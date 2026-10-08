Ordered so each group lands green on its own. Group 1 is pure data and carries
most of the test weight; group 2 is the dispatch that makes the new destinations
reachable; group 3 is what the user sees.

No prerequisite change. `overlay-ui-kit` will later absorb the picker's geometry
into the shared panel kit — keep the rendering in group 3 expressed through
`src/ui/widgets.rs` helpers so that absorption stays a move, not a rewrite.

## 1. Destinations as data

- [x] 1.1 Replace `PaneMoveTargetEntry`'s tab-only shape in `src/app/state.rs` with an entry carrying the display fields plus a `PaneMoveTarget` (`Tab { tab_id }` / `NewTab { workspace_id }` / `NewSpace`) and the id of the space it belongs to
- [x] 1.2 Add the space-heading rows to `PaneMoveTargetPickerState` as non-selectable items, so render and selection read one list rather than deriving headings twice
- [x] 1.3 Rewrite `pane_move_target_picker_for_state` (`src/app/input/navigate.rs`) to enumerate every space: own space first, then the rest in sidebar order; tabs in tab order, then that space's new-tab entry; the new-space entry last
- [x] 1.4 Keep the existing exclusions (source tab, zoomed tabs) and keep returning `Err` — not an empty picker — when nothing is offerable. Note that the new-space destination means "nothing offerable" is now reachable only when the pane cannot be moved at all
- [x] 1.5 Selection movement skips headings in both directions, and clamps without landing on one at either end
- [x] 1.6 Tests on `AppState::test_new()`: ordering across three spaces; source tab excluded; headings unselectable and skipped by next/prev; a single-space single-tab session whose tab holds more than one pane still offers new-space (the sole-pane case is the spec's suppression scenario, so it asserts the picker stays shut); a zoomed source still errors

## 2. Dispatch

- [x] 2.1 Map `PaneMoveTarget` onto `PaneMoveDestination` at submit — `Tab`, `NewTab { workspace_id }`, `NewWorkspace { label: None, tab_label: None }` — all with `focus: true`
- [x] 2.2 Route every variant through the existing `dispatch_pane_move_with_feedback` so `pane.move` rejections surface exactly as they do for tab moves
- [x] 2.3 Confirm the active space follows the pane for cross-space and new-space moves, and that a source tab emptied by the move does not linger
- [x] 2.4 Tests: each destination variant produces the expected `pane.move` params; a cross-space move leaves the destination space active with the pane focused; moving the last pane of a tab to a new space leaves no empty tab

## 3. Picker rendering and discoverability

- [x] 3.1 Render space headings in the existing modal language — a dim, non-selectable row, matching the sidebar's section-heading weight rather than inventing a style
- [x] 3.2 Size the picker for the grouped list, with the existing scroll behaviour when it exceeds the available height
- [x] 3.3 Confirm `prefix+m`'s `help_entry` still reads correctly now that it moves more than "to tab"; reword if it does not, keeping the entry present while unbound
- [x] 3.4 Render tests: headings render dim and unselected; a destination in another space shows its space; the new-space row renders last

## 4. Docs

- [x] 4.1 Update `docs/next/website/src/content/docs/keyboard.mdx` for what `prefix+m` now reaches
- [x] 4.2 Changelog entry under `docs/next/CHANGELOG.md`
- [x] 4.3 ja/zh-cn parity: nothing owed. The fork is English-only and the heading-parity gate was removed in `405d313f`; `release-docs-check` enforces file-set parity only, and this change adds no new page

## 5. Verification

- [x] 5.1 `just check` green
- [x] 5.2 Dogfood on the `-ac-beta` channel: move a running agent pane to another space and to a new space, and confirm the process and scrollback survive both. Ran on `0.8.0-ac-beta.56-cambiaso` after a live handoff that preserved all 18 panes. A throwaway pane kept its terminal id, its scrollback (including history from before the first hop), and a responsive shell across both a cross-space move and a new-space move; the emptied source space closed rather than lingering
- [x] 5.3 Confirm the diff stays free of fork-only styling, so the change can be lifted as a UI-only upstream PR (refs the wayfinder proposal pack). No fork config knob or editorial-sidebar branch is touched; the only fork-only symbols the diff leans on are `FOOTER_ROWS` / `footer_split`, which the lift inlines as a two-row footer reservation. `workspace_list_entries_expanded` and `display_name_from_terminals` both exist upstream
