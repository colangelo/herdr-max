Ordered so each group lands green on its own. Groups 1 and 2 are the correctness
fixes and must both precede group 5: shipping the picker without group 1 creates
links that silently vanish on save, and without group 2 it shows a good label in
the picker then persists `w1:p3`.

## 1. Session-wide public pane id (correctness)

- [x] 1.1 Add a session-wide public pane id lookup in `src/app/ids.rs` that locates the pane's own workspace, mirroring the search `AppState::pane_todo_link_target` already performs
- [x] 1.2 Use it in `save_pane_todo_edit_via_api` (`src/app/input/modal.rs`) instead of resolving against `self.state.active`
- [x] 1.3 Test: saving a link whose target is in another workspace stores the link and round-trips through snapshot restore. Mutation-check by reverting 1.2 and confirming the test fails

## 2. Link label chain (correctness)

- [x] 2.1 Share `launch_label` out of `src/app/actions.rs` rather than duplicating it
- [x] 2.2 Add the launched-command fallback to `resolve_link` (`src/app/api/todos.rs`) so the chain is `manual_label → agent label → launched command → raw public pane id`
- [x] 2.3 Tests: linking to a shell pane stores `zsh`/`npm` rather than `w1:p3`; the manual-label and agent-label cases still win over it; a target with no argv at all still falls back to the raw id. Cover it through the API so the CLI path is covered too

## 3. Always-on indicator

- [x] 3.1 Change `pane_todo_indicator_label` (`src/ui/panes.rs`) to always return a glyph, with the count only when outstanding todos exist
- [x] 3.2 Give the three states distinct tones: priority colour (outstanding), normal dim (all done), dimmest (empty); keep `ui.show_pane_todo_indicator = false` suppressing all three
- [x] 3.3 Keep both existing guards: no indicator when the pane draws no top border, and the indicator wins a width squeeze (title drops itself), omitted only when the pane cannot carry the glyph plus its two border corners
- [x] 3.4 Tests: empty pane draws the glyph and is clickable; empty and all-done render in different tones; drawn cells still equal `pane_todo_indicator_rect`; configured-off draws nothing on a pane holding todos; a pane with no top border draws nothing; the title yields before the indicator does

## 4. Add from the panel

- [x] 4.1 Bind `a` in `handle_pane_todos_key_via_api` (`src/app/input/modal.rs`) to `AppState::open_new_pane_todo` for the panel's pane
- [x] 4.2 Add the matching footer button in `src/ui/todo_panel.rs`, and render the footer in the empty state so an empty panel is not a dead end
- [x] 4.3 Tests: `a` opens the edit modal on a new todo and saving returns to the panel with it listed; the empty-state panel offers the add affordance

## 5. Navigator selection mode

- [x] 5.1 Add a navigator purpose to `AppState` (goto vs. selecting a link target for a todo), defaulting to goto and reset on close
- [x] 5.2 In selection mode, resolve on pane rows only; make workspace and tab rows expand/collapse without resolving
- [x] 5.3 Offer a synthetic clear-the-link entry, and exclude the todo's own pane
- [x] 5.4 Return to `Mode::PaneTodoEdit` on both resolve and dismiss, leaving the staged link untouched on dismiss, mirroring `close_pane_todo_edit_and_return`
- [x] 5.5 Title the overlay for the selection purpose rather than reusing the goto title
- [x] 5.6 Show the navigator's pane label chain in the picker rows (client-side display only — the stored label is group 2)
- [x] 5.7 Point the edit modal's link control (key and click) at the picker; delete `cycle_pane_todo_edit_link` and `pane_link_candidates`
- [x] 5.8 Tests: a pane row stages the target; workspace/tab rows do not; the clear entry clears; own pane absent; dismiss preserves the prior link; the label shown in the picker for a shell target matches what group 2 persists

## 6. Docs and validation

- [x] 6.1 Update `docs/next/website/src/content/docs/keyboard.mdx` for the always-on indicator, the panel add key, and the link picker replacing the cycle
- [x] 6.2 Add the changelog entry under `docs/next/CHANGELOG.md`, and add `help_entry` rows in `src/ui/keybind_help.rs` for any new action
- [x] 6.3 `just check` green apart from the known macOS `live_handoff` failure (fork issue #33), plus the changelog duplicate checks from the sync skill
- [x] 6.4 Dogfood on the `-ac-beta` channel: click an empty pane's indicator, add a todo from the panel, and link it to a pane in another workspace
