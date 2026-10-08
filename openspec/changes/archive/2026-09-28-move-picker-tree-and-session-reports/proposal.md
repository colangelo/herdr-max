## Why

ac's feedback on beta.104 (the dialog overhaul), 2026-09-28, with two
screenshots, relayed by herdr-relay as JOB 10:

- https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/109: the session
  navigator is now too narrow. At 73 columns its right status column is cut
  (`macos-relay-3 · i…`, `keyboard-shortcut…`). Its width should come from the
  content, the full status column included, and stay capped in wide windows.
- https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/110: the move-pane
  dialog is much less readable than the navigator. Space names do not stand
  out, `tab 1` is one flat colour, and there is no tree. ac: "put some effort
  in thinking how to make it pleasurable and informative to the eyes".

Two agent-session bugs were added to the same job, because they ship in the
same beta:

- https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/111: a pane moved to
  another space keeps its old `HERDR_PANE_ID` in its shell. After a restart or
  a live handoff the old id no longer finds the pane, so a Claude started there
  never gets its session recorded.
- https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/112: a Claude started
  with `pane run` never gets its session recorded, although its pane id is
  right.

## What Changes

- **Navigator status column is measured** (#109). The column is as wide as the
  widest status the rows can show, counting the longest state word, up to 40.
  The renderer uses that measured column instead of fixed 28/20/14 steps. The
  label column gives way first when the window is narrower. `1 panes` becomes
  `1 pane`.
- **The move picker becomes a tree** (#110), in the navigator's language:
  bold accent space headings with a pane count and a state icon, `├──`/`└──`
  branches, tab rows as `tab` (dim) + number (bold) + name, a pane-names
  column, the pane's own tab shown greyed with `◆` and "you are here" but not
  selectable, `+ new tab` / `+ new space` drawn as actions, the navigator's
  full-row accent selection, blank lines between spaces, a scrollbar, and a
  one-line detail above the buttons naming every pane in the selected tab.
  The title names the pane being moved and where it is. `/` search keeps
  working, with tree branches redrawn over the filtered rows.
- **A moved pane's old ids survive a restart and a handoff** (#111). Each pane
  carries its former public ids in the session file. They are rebuilt into the
  alias map on restore and on handoff. A live id always wins over an alias.
- **A restarted agent keeps its new session report** (#112). Reproduced
  first: the new Claude's startup report is refused while the old session is
  on record, and the old one clears only when detection sees the old process
  exit. Detection now marks a same-agent process in a new process group as a
  replacement, and the refused report is held and adopted when it is seen.

## Impact

- #109 and #110 are TUI presentation only: `src/ui/navigator.rs`,
  `src/ui/dialogs.rs`, `src/app/input/navigate.rs` (picker items),
  `src/app/state.rs` (`PaneMoveTargetItem`), `src/app/input/mouse.rs`
  (geometry). No API, protocol or config change, and no new keybinding.
- #111 adds an optional field to the pane snapshot (`former_public_ids`),
  omitted when empty, so old session files read unchanged and new ones stay
  readable by older builds (serde ignores unknown fields). No protocol bump.
- #112 touches `src/terminal/state.rs`, `src/pane.rs` (detection) and an
  internal event. It is a server/runtime fact, not TUI state; no protocol
  change.
- Docs: none user-facing beyond the look. No keyboard change.
