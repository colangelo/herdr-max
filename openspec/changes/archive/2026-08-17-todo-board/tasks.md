Ordered so each group lands green on its own. Group 1 is pure data and carries
the ordering tests; group 2 is the surface; group 3 makes rows actionable.

Prerequisites, all met — `todo-editing-keyboard`, `pane-todos-ux` and
`overlay-ui-kit` are archived, and the footer convention (`FOOTER_ROWS` /
`footer_split`) came with the kit.

Because `overlay-ui-kit` landed first, the board is built on the kit rather than
migrated onto it later: the footer row is `ButtonRow` / `ButtonSpec`, selection
and scroll are a `ListCursor`, movement comes from `list_chord`, the board is a
variant in the `overlays!` list in `src/app/state.rs`, and `overlay_help` in
`src/ui/keybind_help.rs` will not compile until it declares its help entries.
Centred geometry stays on the modal path (`centered_popup_rect` /
`render_modal_shell` / `footer_split`) — the kit's `AnchoredPanelSpec` places a
panel against an anchor, and the board has none by design.

## 1. The aggregate, as data

- [x] 1.1 Add a board projection on `AppState`: every pane holding todos, grouped space > tab > pane, each group carrying the pane's addressable id and label and its todos in existing presentation order
- [x] 1.2 Reuse `pane_todos_in_display_order` per pane rather than re-deriving ordering, so the board and the panel cannot drift
- [x] 1.3 Omit panes with no todos
- [x] 1.4 Board state: a `ListCursor` over one item list with headings interleaved as non-selectable items, so render and selection read the same list (the `PaneMoveTargetPickerState` shape)
- [x] 1.5 Selection movement skips headings in both directions and clamps without landing on one at either end
- [x] 1.6 Tests on `AppState::test_new()`: grouping order across two spaces; within-pane ordering matches the panel's; empty panes omitted; headings skipped by next/prev; selection survives a todo being removed underneath it

## 2. The board surface

- [x] 2.1 Add `src/ui/todo_board.rs` rendering a centred modal from `render_modal_shell` / `render_modal_header` and a kit `ButtonRow` footer, reserving the footer block
- [x] 2.2 Render group headings with the addressable id leading, then the label, in the dim non-selectable style the move picker uses
- [x] 2.3 Render rows identically to the pane todo panel's — same marker, priority colour, done styling, link chip — so a todo looks the same in both surfaces
- [x] 2.4 Scroll window when the list exceeds the available height, keeping the selection visible
- [x] 2.5 Empty state when no pane holds a todo, with the footer still offering close
- [x] 2.6 A variant in the `overlays!` list, a `KeysConfig` action shipping unbound, and its `overlay_help` arm in `src/ui/keybind_help.rs` (present while unbound), plus the board's own chords in the help panel's fixed-chord section
- [x] 2.7 Render tests: heading style and content; a row matching the panel's rendering; the separator row above the footer blank; the empty state

## 3. Actions

- [x] 3.1 Wire the panel's actions at board scope under the same keys: toggle done, edit, remove, clear done, follow link, close — falling through to `list_chord` for movement exactly as the panel does
- [x] 3.2 Activation (Enter) focuses the todo's owning pane through the existing focus path and closes the board
- [x] 3.3 Keep `g` targeting the *linked* pane, so a linked todo's two destinations stay distinct
- [x] 3.4 Editing from the board opens the existing edit modal and writes against the owning pane
- [x] 3.5 Mouse: click to select, click a footer button, click outside to dismiss — matching the panel's hit-testing rules
- [x] 3.6 Tests: activation focuses the owner across a space boundary and closes the board; `g` reaches the link, not the owner; a toggle from the board is visible in that pane's panel; removing the last todo of a pane drops its heading

## 4. Docs

- [x] 4.1 Update `docs/next/website/src/content/docs/keyboard.mdx` with the board action and its chords
- [x] 4.2 Changelog entry under `docs/next/CHANGELOG.md`
- [x] 4.3 N/A — this fork is English-only and the heading-parity gate was removed on 2026-08-13; there are no ja/zh-cn trees to keep in step

## 5. Verification

- [x] 5.1 `just check` green
- [x] 5.2 Confirm the board adds no per-render work to the pane-scaled paths — it renders only while open, and the projection is built on open, not per frame
- [x] 5.3 Dogfood on the `-ac-beta` channel: todos on panes in two spaces, triage from the board, jump to an owner across a space boundary
