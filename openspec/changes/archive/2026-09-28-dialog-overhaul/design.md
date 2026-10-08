## Findings

**#103: why the new space is called "master".** There are two ways to name a
space. `Workspace::display_name_from` (`src/workspace.rs`) resolves the root
pane's *live* cwd through the runtime (`Tab::cwd_for_pane` → `rt.cwd()`, a
lock and a clone of what the detection task already resolved). The agent panel
uses it (fixed upstream for #264; test
`all_workspaces_agent_panel_entries_use_live_root_runtime_cwd_for_workspace_label`).
`Workspace::display_name_from_terminals` reads `TerminalState::cwd`, which only
an OSC 7 report updates. A pane spawned in the "master" folder that `cd`'d to
`CONTEXT` without reporting keeps "master" there. The move picker's space
headings use the stale one (`pane_move_target_picker_for_state`,
`src/app/input/navigate.rs`). The move itself makes it worse:
`ResolvedPaneMoveDestination::NewWorkspace` (`src/app/api/panes.rs`) seeds the
new space's `identity_cwd` from the same stale `TerminalState::cwd`. Reproduced
by `move_picker_names_a_space_made_by_a_move_the_way_the_sidebar_does`
(`src/app/input/navigate.rs`, committed `#[ignore]`d). The source space
"master" is given three panes. A real runtime has live cwd `CONTEXT` over a
stored cwd of `master`. The pane moves to a new space (`NewSpace`), and the
picker reopens from "master". The headings come out `["master", "master"]`;
expected `["master", "CONTEXT"]`.

**#106: why the detail shows in some spaces only.** It has nothing to do with
the space, the window or the pane's position. `pane_todo_detail_rows`
(`src/ui/todo_panel.rs`) returns 0 unless the *selected* todo's text contains a
newline. It is called only for `todos.get(panel.list.selected)`
(`src/app/input/mouse.rs`), and the selection opens on the first todo in
display order (high priority first). So "herdr"'s top todo is multi-line and
"master"'s is not; arrowing onto a multi-line todo in "master" shows it there
too. A truncated one-line todo never shows it. Checked with a throwaway test at
310x56: short one-line, none; 60-column cut line, none; `"a\nb"`, 3 rows; a
one-line High above a multi-line Normal, none. Height is not the cause: the
panel is at most 12 + 2 + 2 + 7 = 23 rows and slides up to fit. The block is
drawn as a `─` rule plus `subtext0` text, with no box, and the panel's height
changes as the selection moves on and off a multi-line todo.

## Decisions

**Naming (#103).** The picker names every space with `display_name_from`, the
sidebar's function. `pane_move_target_picker_for_state` takes the runtime
registry; its test-only caller passes an empty one. A move to a new space seeds
`identity_cwd` from the runtime's live cwd, falling back to the stored cwd.
While building, audit the other `display_name_from_terminals` callers that show
a space's name to the user (todo board headings in `todo_board_items`, the
sidebar space-row height, the window title) and move each onto the live name,
or leave a comment saying why not.

**One search piece (#104, #107, #108).** Add `src/ui/overlay/search.rs` with:
- `ListSearch { query: TextField, focused: bool }`, built by the existing
  `search_query_field()`;
- `matches(&self, text)`, which is today's `text_matches_query`: every
  whitespace-separated word must appear, case-insensitive;
- `handle_key(&mut self, key) -> SearchKey { Edited, Left, Accept, Chord(ListChord), Ignored }`,
  which routes `list_chord(.., PlainChars::AreText)` and then `apply_text_key`;
- `render_search_row(frame, rect, &ListSearch, placeholder, count, palette)`,
  moved out of `navigator.rs::render_search` along with `set_search_caret`.

The navigator moves onto it first, with `snapshot_navigator` unchanged. That is
the proof that the extraction changes nothing. The move picker and the todo
board then adopt it. KeybindHelp and the worktree-open picker keep their own
search for now; they are follow-ups, not part of this change.

What each dialog matches on:
- **Move picker:** the space name, tab number and label, `new tab` and
  `new space`. A heading stays, dimmed, when any of its destinations match. A
  matching heading keeps all its destinations. Headings stay unselectable.
- **Todo board:** the todo text, `#id`, and the pane heading (`space · label`).
  A matching heading keeps all its todos; a matching todo keeps its heading.
  `GroupGap` rows between surviving groups only.
- When a filter leaves nothing, the dialog says "no match" and keeps the query.

**Key map.** It is the same in all three, taken from the navigator:

| Key | List focus | Search focus |
|---|---|---|
| `/` | focus the search | types `/` |
| typing | the dialog's own commands | edits the query, selects the first match |
| arrows, ctrl+j/k/n/p, Home, End | move (`list_chord`, AreChords) | move (`list_chord`, AreText) |
| Enter | the dialog's accept (move / open pane / go to) | accept the selected row |
| Esc | close; if a query is set, clear it first | leave the search, keep the query |

The todo board's letter commands (`e`, space, `g`, `d`, `c`, `q`) work in list
focus only, so they never collide with typing. The navigator's own letters
(`b`/`w`/`i`/`d`/`a` state filters, space expand) are unchanged. Esc clearing
the query before it closes is new for all three; the navigator gets it too, so
the three match.

**Placement and width (#105).** `AnchoredPanelSpec` gains a centred placement
(`VerticalAnchor::Centered`, with the anchor ignored horizontally). The
navigator, move picker and todo board resolve their outer, list, detail and
footer rects through it, instead of percentage margins, `centered_popup_rect`,
and `TodoBoardGeometry`. Each dialog measures its own content (the kit's rule),
and all three share `LIST_DIALOG_MAX_WIDTH = 120`.
- **Navigator:** widest row (tree prefix, icon, label) plus the metadata column
  (28) plus chrome, bounded to 64..120. Height keeps today's rule.
- **Move picker:** widest heading or destination row plus chrome, bounded to
  48..120. Height is the item count plus chrome, up to the screen.
- **Todo board:** today's measured content, bounded to 80..120 (was 80..140).

At 310x56 the navigator goes from 272 to at most 120 columns. The pane-todo
dropdown stays anchored to its pane, which is its point.

**Detail box (#106).** One kit helper decides whether a detail shows and draws
it. The pane-todo panel and the todo board both use it.
- **Shown when** the selected todo has text its row cannot show: more than one
  line, or a first line wider than the row's text budget (the width
  `render_pane_todo_row` truncates to).
- **Drawn as** an inner bordered box (`render_panel_shell` in `surface_dim`
  inside the panel), with the text wrapped to the panel width minus 6, up to 6
  rows. It costs 2 + text rows.
- **Height** is reserved for the largest need among the visible todos, so the
  panel does not jump as the selection moves. A row whose todo needs no detail
  shows the box empty, with a dim "full text shown above".
- **Too little room:** `PanelGeometry::detail` returns `None` when fewer than 3
  rows are free, rather than a lone rule.

**Move picker onto the kit.** `ButtonRow` replaces `action_button_row_rects`,
and a near-miss on the button row no longer closes the picker, as on the todo
surfaces. `ListCursor::window` replaces the hand-rolled `selected - (max_rows-1)`
in both the renderer and the mouse code. Half a page becomes the visible rows,
not `items.len()/2`. It gets its first `overlay_snapshot` test.

**Mouse, made uniform.** Hover selects. A click on a row activates it. A click
inside the dialog but off its rows and buttons does nothing, and a click
outside closes. That is the navigator's rule; today the picker and both todo
surfaces close on an inside click. The todo board keeps its select-then-activate
double step, because activation there leaves the board.

## Open questions (with recommendation)

1. **Maximum width: 120 or 100?** Recommend 120: the navigator's metadata
   column wants 28, and todo text reads better wide. Revisit after ac sees it.
2. **Should a cut one-line todo open the detail too?** Recommend yes: the
   detail's job is "the text the row hides", and a cut line hides text.
3. **Reserved detail height** (no jumping) **vs. a detail only when needed**
   (a smaller panel)? Recommend reserved: ac's complaint is that it is
   inconsistent, and jumping is the same problem.
4. **Esc clears a set query before closing:** a small change to the navigator.
   Recommend yes, for one rule in all three dialogs.
