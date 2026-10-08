## Findings

**#109: why the status column is cut.** `navigator_content_width`
(`src/ui/navigator.rs`) sizes the box as the widest label plus a fixed 28-column
status column, then floors it at 73. In ac's session the labels are short, so
the box lands on the floor, 73. But the renderer does not use 28: its
`metadata_width` steps the column by the box width (28 from 90 columns, 20 from
68, 14 from 52). At 73 it hands the column 20, which is 18 characters of text,
so `macos-relay-3 · idle` (20) and `keyboard-shortcuts · idle` (25) are cut.
The measurement and the drawing disagree. Also, a tab's status reads `1 panes`.

**#110: why the move picker reads flat.** Every destination is one
`subtext0` string (`tab 1`, `1 · cc`, `new tab`), headings are `overlay0` bold
(dimmer than the rows under them), there is no tree, no status and no gap
between spaces. The pane's own tab is left out, so nothing says where the pane
is now. `new tab` and `new space` look like places. The selection is a
`surface0` bar with `›`, not the navigator's accent bar.

**#111: why a moved pane's old id stops working.** A move across spaces
already records `old public id → pane` in `AppState::public_pane_id_aliases`
(`src/app/api/panes.rs`), and `parse_pane_id` (`src/app/ids.rs`) reads it first.
So right after a move the old id works. But that map lives only in memory. It is
not in `SessionSnapshot`, so a restart or a live handoff (every beta upgrade)
starts with it empty. The shell in the moved pane still exports the old id, so
every hook report from it (`report_agent_session`, `report_agent`,
`report_metadata`, `todo --current`) answers `pane_not_found`. Checked live on
m4m (beta.104): `herdr-beta pane get w2:p1X` → `pane_not_found`, while the pane
is `wV:p1`.

**#112: what the code rules out, and the suspect.** `pane run` types the
command into the pane's shell (`pane.send_input`), so the Claude it starts has
the same environment as one started by hand: `HERDR_ENV`, `HERDR_SOCKET_PATH`
and `HERDR_PANE_ID` are the shell's. For `herdr:claude` a session report does
not wait for detection: `set_agent_session_ref_for_session_start`
(`src/terminal/state.rs`) records it whether or not Claude is detected yet
(Claude is not a full-lifecycle source). So "dropped because not detected yet"
does not match the code. What does wipe a recorded Claude session is detection
observing a Claude process exit: `set_detected_state_with_screen_signals_at`
clears `persisted_agent_session` for that agent when `process_exited`. The
typical `pane run` use is a restart recipe (end the old Claude, then
`pane run <pane> "claude … --name X"`). If the new Claude's SessionStart report
lands before detection's next poll notices the old Claude left, that late exit
clears the new session. A person typing `claude` is slower than one poll, which
would explain why hand-started sessions keep theirs. This is a hypothesis; the
build pins it before fixing (task 1.8).

## Decisions

### Navigator width (#109)

- **Measure the status column.** `navigator_content_width` returns the label
  width and the status width separately, and the navigator state keeps both.
  The status width is the widest `row.meta` among the rows, measured with its
  state word swapped for the longest one (`working`, `blocked`, `unknown`: 7).
  An `idle` pane turning `working` then does not overflow. It is capped at 40,
  so one very long agent name cannot eat the box.
- **Draw what was measured.** `metadata_width(width)` goes away. The renderer
  gives the status column its measured width. When the box is narrower than
  measured (a small window), the label column gives way first, down to 16
  columns, then the status column is cut with `…`.
- **Box width** = label + status + border, bounded to 73..120 as today. In ac's
  session that is still 73, now with nothing cut. With longer statuses it grows,
  up to 120.
- `1 panes` → `1 pane`, in the tab rows and the detail line.

### Move picker as a tree (#110)

Rows, top to bottom. Colours are palette names; ASCII cannot show them, so each
mockup has a legend.

- **Title:** `move pane` (text, bold) + the pane's label (accent, bold) +
  `from <space> › tab <n>` (overlay0).
- **Search row:** unchanged (`/ search destinations`, count on the right). The
  blank header row becomes a rule, as in the navigator.
- **Space heading:** state icon (its state colour), name and `(pane count)` in
  accent bold, exactly as the navigator draws a space. Status column: the
  space's activity (`1 working`), overlay0. No fold caret: the picker does not
  fold.
- **Tab row:** `├──` in surface1 (the navigator's branch colour), the tab's
  state icon, then `tab` in overlay0, the number in text bold, and `· name` in
  text when the tab has one. Status column: the names of the tab's panes,
  comma-separated, subtext0, ending `+N` when they do not fit. The unnamed-tab
  `tab 1` and the named `1 · cc` become one shape: `tab 1` / `tab 1 · cc`.
- **The pane's own tab:** shown, in its place, as `◆ ├── ◐ tab 1 · cc  you are
  here`, the `◆` in accent and the rest overlay0. It is context, not a
  destination: the cursor skips it like a heading, a click on it does nothing,
  and it does not count in "N destinations". Moving a pane into its own tab is
  a no-op, which is why the old picker left it out; greyed, it tells you where
  you are.
- **Actions:** `└── + new tab` under each space and `+ new space` last, after a
  gap. `+` in accent, the words in overlay1, no status. They read as things to
  do, not places.
- **Gaps:** one blank line between spaces, as in the navigator. Not selectable.
- **Selection:** the navigator's full-row accent bar with contrast text, bold.
- **Scrollbar:** the navigator's, when the list is taller than its rows.
- **Detail line** above the buttons, overlay0, middle-elided: for a tab, every
  pane in it (`macOS › tab 1: macos-relay-2, macos-relay-3, vpn-morning-issues-2`);
  for `+ new tab`, `a new tab in macOS`; for `+ new space`, `a new space holding
  herdr-fixes`.
- **Width:** measured like the navigator's (tree + icon + label, plus a status
  column of the widest pane list, capped at 36), bounded 48..120. The pane list
  is measured once at open, so filtering does not resize the box.
- **Search:** unchanged rules (a matching heading keeps its tabs; a matching
  tab keeps its heading). The tab row also matches on its pane names, so `/
  keyboard` finds the tab holding `keyboard-shortcuts`. Branch glyphs (`├──` vs
  `└──`) are worked out over the rows left after filtering, as the navigator
  does, so the last visible child always gets `└──`. The "here" row stays when
  its space is shown.

Data: `PaneMoveTargetItem` gains `Here { label }` (context row) and a `Gap`,
and `SpaceHeading` gains `pane_count` and `status`/`seen`. `PaneMoveTargetEntry`
gains `pane_names: Vec<String>` and `status`/`seen` for tab rows. The picker is
built once at open, so none of this is per-frame work. Only the rows in view
are drawn. `ListCursor` skips `SpaceHeading`, `Here` and `Gap`.

### Mockups (#109 and #110)

Generated from ac's session in the 2026-09-28 screenshots, by the same width
rules the build will use, so the columns are the real ones. `◀ selected` marks
the row with the accent bar. The last column (`▐`, `▕`) is the scrollbar.

Style legend (both dialogs): space name and `(n)` = accent bold; icons =
state colour (`✓` idle green, `◐` working, `·` shell); `├──` = surface1;
`tab` = overlay0; tab number = text bold; tab name = text; pane names and
statuses = subtext0 (navigator statuses keep their state colour); `◆` = accent;
"you are here" row = overlay0; `+` = accent, `new tab`/`new space` = overlay1;
detail line = overlay0.

#### Move pane, 310x56 window

```
move picker in a 310x56 window: box 62x56, centred; 49 list rows, 47 visible
╭────────────────────────────────────────────────────────────╮
│ move pane  herdr-fixes  from herdr › tab 1 · cc            │
│ / search destinations                      26 destinations │
│────────────────────────────────────────────────────────────│
│   ◐ herdr (3)         1 working                           ▐│
│ ◆ ├── ◐ tab 1 · cc    you are here                        ▐│
│   └── + new tab                                           ▐│
│                                                           ▐│
│   ✓ master (1)                                            ▐│
│   ├── ✓ tab 1         master                              ▐│
│   └── + new tab                                           ▐│
│                                                           ▐│
│   ✓ CONTEXT (2)                                           ▐│
│   ├── ✓ tab 1         context-relay                       ▐│
│   ├── · tab 2         pane 2                              ▐│
│   └── + new tab                                           ▐│
│                                                           ▐│
│   ✓ infra (2)                                             ▐│
│   ├── ✓ tab 1         infra-relay                         ▐│
│   ├── · tab 2         pane 2                              ▐│
│   └── + new tab                                           ▐│
│                                                           ▐│
│   ✓ macOS (5)                                             ▐│
│   ├── ✓ tab 1         macos-relay-2, macos-relay-3, +1    ▐│  ◀ selected
│   ├── ✓ tab 2         keyboard-shortcuts, install tuicr   ▐│
│   └── + new tab                                           ▐│
│                                                           ▐│
│   ✓ clipper (3)                                           ▐│
│   ├── ✓ tab 1         clipper-relay-3, clipper-astra-2, +1▐│
│   └── + new tab                                           ▐│
│                                                           ▐│
│   ✓ cchv (3)                                              ▐│
│   ├── ✓ tab 1         cchv-relay, ac@docker: ~, pane 3    ▐│
│   └── + new tab                                           ▐│
│                                                           ▐│
│   ✓ direction (5)                                         ▐│
│   ├── ✓ tab 1         direction-closing, direction-5, +1  ▐│
│   ├── ✓ tab 2         direction-astra-2, direction-7      ▐│
│   └── + new tab                                           ▐│
│                                                           ▐│
│   ✓ siai (1)                                              ▐│
│   ├── ✓ tab 1         siai-relay                          ▐│
│   └── + new tab                                           ▐│
│                                                           ▐│
│   ✓ jev-lab (1)                                           ▐│
│   ├── ✓ tab 1         jev-lab                             ▐│
│   └── + new tab                                           ▐│
│                                                           ▐│
│   ✓ monitoring (1)                                        ▐│
│   ├── ✓ tab 1         monitoring-relay                    ▕│
│   └── + new tab                                           ▕│
│────────────────────────────────────────────────────────────│
│ macOS › tab 1: macos-relay-2…relay-3, vpn-morning-issues-2 │
│                                                            │
│                [ ↵ move ]   [ esc cancel ]                 │
╰────────────────────────────────────────────────────────────╯
```

#### Move pane, 100x30 window

```
move picker in a 100x30 window: box 62x30, centred; 49 list rows, 21 visible
╭────────────────────────────────────────────────────────────╮
│ move pane  herdr-fixes  from herdr › tab 1 · cc            │
│ / search destinations                      26 destinations │
│────────────────────────────────────────────────────────────│
│   ◐ herdr (3)         1 working                           ▐│
│ ◆ ├── ◐ tab 1 · cc    you are here                        ▐│
│   └── + new tab                                           ▐│
│                                                           ▐│
│   ✓ master (1)                                            ▐│
│   ├── ✓ tab 1         master                              ▐│
│   └── + new tab                                           ▐│
│                                                           ▐│
│   ✓ CONTEXT (2)                                           ▐│
│   ├── ✓ tab 1         context-relay                       ▕│
│   ├── · tab 2         pane 2                              ▕│
│   └── + new tab                                           ▕│
│                                                           ▕│
│   ✓ infra (2)                                             ▕│
│   ├── ✓ tab 1         infra-relay                         ▕│
│   ├── · tab 2         pane 2                              ▕│
│   └── + new tab                                           ▕│
│                                                           ▕│
│   ✓ macOS (5)                                             ▕│
│   ├── ✓ tab 1         macos-relay-2, macos-relay-3, +1    ▕│  ◀ selected
│   ├── ✓ tab 2         keyboard-shortcuts, install tuicr   ▕│
│────────────────────────────────────────────────────────────│
│ macOS › tab 1: macos-relay-2…relay-3, vpn-morning-issues-2 │
│                                                            │
│                [ ↵ move ]   [ esc cancel ]                 │
╰────────────────────────────────────────────────────────────╯
```

#### Session navigator, 310x56 window

```
navigator in a 310x56 window: box 73x46, centred; 56 list rows, 40 visible
╭───────────────────────────────────────────────────────────────────────╮
│ / search panes                                               36 panes │
│───────────────────────────────────────────────────────────────────────│
│   ▾ ✓ master (1)                                                     ▐│
│   └── ✓ master                           claude · idle               ▐│
│                                                                      ▐│
│   ▾ ✓ CONTEXT (2)                                                    ▐│
│   ├── ✓ 1                                1 pane                      ▐│
│   │  └── ✓ context-relay                 context-relay · idle        ▐│
│   └── · 2                                1 pane                      ▐│
│      └── · pane 2                        shell                       ▐│
│                                                                      ▐│
│ ◆ ▾ ◐ herdr (3)                          1 working                   ▐│
│   ├── ✓ herdr-relay                      herdr-relay · idle          ▐│
│ ◆ ├── ◐ herdr-fixes                      herdr-fixes · working       ▐│  ◀ selected
│   └── ✓ herdr-tests                      herdr-tests · idle          ▐│
│                                                                      ▐│
│   ▾ ✓ infra (2)                                                      ▐│
│   ├── ✓ 1                                1 pane                      ▐│
│   │  └── ✓ infra-relay                   infra-relay · idle          ▐│
│   └── · 2                                1 pane                      ▐│
│      └── · pane 2                        shell                       ▐│
│                                                                      ▐│
│   ▾ ✓ macOS (5)                                                      ▐│
│   ├── ✓ 1                                3 panes                     ▐│
│   │  ├── ✓ macos-relay-2                 claude · idle               ▐│
│   │  ├── ✓ macos-relay-3                 macos-relay-3 · idle        ▐│
│   │  └── ✓ vpn-morning-issues-2          claude · idle               ▐│
│   └── ✓ 2                                2 panes                     ▐│
│      ├── ✓ keyboard-shortcuts            keyboard-shortcuts · idle   ▐│
│      └── ✓ install tuicr                 claude · idle               ▐│
│                                                                      ▕│
│   ▾ ✓ clipper (3)                                                    ▕│
│   ├── ✓ clipper-relay-3                  clipper-relay-3 · idle      ▕│
│   ├── ✓ clipper-astra-2                  clipper-astra-2 · idle      ▕│
│   └── · pane 7                           shell                       ▕│
│                                                                      ▕│
│   ▾ ✓ cchv (3)                                                       ▕│
│   ├── ✓ cchv-relay                       cchv-relay · idle           ▕│
│   ├── · ac@docker: ~                     shell                       ▕│
│   └── · pane 3                           shell                       ▕│
│                                                                      ▕│
│   ▾ ✓ direction (5)                                                  ▕│
│ herdr-fixes · herdr-fixes · working · ~/_sync/dev/herdr               │
│ enter switch  / search  b/w/i/d/a states  j/k/^j/^k/↑↓ move  esc close│
╰───────────────────────────────────────────────────────────────────────╯
```

#### Session navigator, 100x30 window

```
navigator in a 100x30 window: box 73x24, centred; 56 list rows, 18 visible
╭───────────────────────────────────────────────────────────────────────╮
│ / search panes                                               36 panes │
│───────────────────────────────────────────────────────────────────────│
│   ▾ ✓ master (1)                                                     ▐│
│   └── ✓ master                           claude · idle               ▐│
│                                                                      ▐│
│   ▾ ✓ CONTEXT (2)                                                    ▐│
│   ├── ✓ 1                                1 pane                      ▐│
│   │  └── ✓ context-relay                 context-relay · idle        ▕│
│   └── · 2                                1 pane                      ▕│
│      └── · pane 2                        shell                       ▕│
│                                                                      ▕│
│ ◆ ▾ ◐ herdr (3)                          1 working                   ▕│
│   ├── ✓ herdr-relay                      herdr-relay · idle          ▕│
│ ◆ ├── ◐ herdr-fixes                      herdr-fixes · working       ▕│  ◀ selected
│   └── ✓ herdr-tests                      herdr-tests · idle          ▕│
│                                                                      ▕│
│   ▾ ✓ infra (2)                                                      ▕│
│   ├── ✓ 1                                1 pane                      ▕│
│   │  └── ✓ infra-relay                   infra-relay · idle          ▕│
│   └── · 2                                1 pane                      ▕│
│ herdr-fixes · herdr-fixes · working · ~/_sync/dev/herdr               │
│ enter switch  / search  b/w/i/d/a states  j/k/^j/^k/↑↓ move  esc close│
╰───────────────────────────────────────────────────────────────────────╯
```

In both navigator mockups the status column is 30 wide, measured from
`keyboard-shortcuts · working` (the widest status with the longest state word);
the box is 73 because of the footer floor. Nothing in it is cut.

### Moved pane ids survive a restart and a handoff (#111)

Options weighed:

1. **Persist the aliases** (chosen). `PaneSnapshot` gains
   `former_public_ids: Vec<String>`, skipped when empty. At snapshot time each
   pane writes the aliases that point at it. On restore and on handoff (both go
   through `SessionSnapshot`), each pane's ids go back into
   `public_pane_id_aliases` under its new `PaneId`. The ids travel with the pane,
   so a later move or a raw-id renumbering on restore cannot misdirect them, and
   closing the pane drops them (already done on close).
2. Resolve the caller from its process tree (peer pid → which pane's PTY it
   descends from). It needs peer credentials per platform and a process walk
   per report, and it changes what `pane_id` means in the API. Rejected for now;
   noted as a follow-up if a case appears that aliases cannot cover.
3. Have hooks send a move-stable key (the terminal id). Needs a new env var,
   an integration version bump for every agent, and it cannot fix shells that
   are already running. Rejected.
4. Refresh the id on move. A running shell's environment cannot be changed.
   Not possible.

Two rules come with option 1:

- **A live id wins.** `parse_pane_id` looks up live ids first and aliases
  second (today it is the other way). Workspace ids can be reused after a
  restart (`reserve_workspace_ids` counts only live spaces), so an old
  `w2:p1X` could one day name a new pane; if it does, it means the new pane.
  On restore, an alias equal to a live id is dropped.
- **Other reports gain it for free.** Every API call that takes a pane id goes
  through `parse_pane_id`, so `report_agent`, `report_metadata` and
  `todo --current` from a moved pane work too.

Known limit: shells whose alias was already lost before this ships (such as
m4m's `wV:p1`, env `w2:p1X`) stay stale until their shell is restarted
(`pane respawn`) or the session is re-reported with the right id. Their
`HERDR_WORKSPACE_ID` and `HERDR_TAB_ID` also stay stale; nothing we ship
reports with those, so they are out of scope.

### A session report outlives an older process exit (#112)

First, the build reproduces it in a throwaway session on m4m, with a stand-in
agent rather than a paid Claude: a script named `claude` that the detector
recognises and that runs the real Claude hook with a SessionStart payload.
Restart it the way the recipe does: exit, then `pane run` at once. Check
`agent_session` after the next detection poll. The server log is no help
(`pane.report_agent_session` is a routine method, logged at debug only), so the
test reads the pane.

If the race is confirmed, the fix is in `src/terminal/state.rs`: a recorded
session carries the time it was reported, and a process exit only clears
sessions reported before the exited process was last seen running. A report
that arrived after that belongs to the new process and stays. If the cause is
something else, the finding and the new fix go to herdr-relay before the build
goes on, since that changes the design.

## Open questions (with recommendation)

1. **Blank lines between spaces in the picker.** They cost about 10 rows, so at
   310x56 the list scrolls by 2. Recommend keeping them: they are most of what
   makes the navigator easy to scan.
2. **Tab status column: pane names or a count?** Recommend names. The heading
   already has the count, and names tell you which tab is which.
3. **Show the pane's own tab (greyed) or leave it out?** Recommend showing it.
   It answers "where is this pane now?", which the old picker left to memory.
4. **Navigator cap stays 120.** Recommend yes. With the column measured,
   ac's session sits at 73 and nothing is cut.
