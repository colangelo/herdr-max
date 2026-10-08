Ordered so the safety net lands first and the riskiest move lands last. Groups
1–5 are ordinary local refactors, each green on its own and each *reducing* fork
lines. Group 6 is refactor-risk under `AGENTS.md` and has its own protocol.

Prerequisite: `todo-editing-keyboard` group 1 (the `TextField` primitive), which
group 5 adopts. Nothing else here depends on it.

## 1. Snapshot harness (the safety net, before anything moves)

- [x] 1.1 Promote `row_text(buffer, rect)` from `src/ui/todo_panel.rs`'s tests into a shared test utility that renders an `AppState` into a `TestBackend` at a fixed size and returns the rows of a rect
- [x] 1.2 Write a rendered-layout test for each overlay against **current** behaviour: notification center (both positions, empty and populated), todo panel (empty, populated, narrow), navigator, keybind help, settings, context menu, global menu, release notes, onboarding
- [x] 1.3 Land group 1 green before touching any geometry. A later group whose snapshot changes has found a real difference; do not update the expected rows to make it pass without saying why in the commit

## 2. Anchored panel geometry

- [x] 2.1 Add `AnchoredPanelSpec` / `PanelGeometry` to `src/ui/widgets.rs` (or a new `src/ui/overlay/geometry.rs` if `widgets.rs` is getting long)
- [x] 2.2 Express `notification_center_rect` (`src/app/input/mouse.rs:1471`) in it, keeping the accessor and its signature so callers do not move. Cover both `top-right` and `bottom-right`, including opening above the floating indicator
- [x] 2.3 Express `pane_todo_panel_rect` (`mouse.rs:1589`) in it, keeping its pane anchor and the `None`-when-the-pane-is-gone guard
- [x] 2.4 Delete the per-panel inner/list/footer derivations that the geometry now returns
- [x] 2.5 Tests: unit tests for the resolver (clamping, anchor edges, above/below placement, too-small screen); group 1's snapshots unchanged

## 3. Button rows

- [x] 3.1 Add `ButtonRow<B>` over the existing `action_button_*` helpers, with drop priority and a never-dropped dismiss button
- [x] 3.2 Move `pane_todo_panel_button_rects` + `PaneTodoPanelButtonRects::hit`/`row_y` (`src/ui/todo_panel.rs:45-121`) onto it
- [x] 3.3 Move `notification_center_button_rects` + its `hit`/`row_y` (`src/ui/notification_center.rs:41-100`) onto it
- [x] 3.4 Keep the near-miss rule (a click on the row but not on a button is inert) in the row rather than in each overlay's mouse handler
- [x] 3.5 Tests: narrow rows drop by priority and keep the dismiss button; hit-test equals drawn rects; group 1's snapshots unchanged, including the narrow todo panel that drops `clear done`

## 4. One list cursor

- [x] 4.1 Merge `MenuListState` and `SelectionListState` (`src/app/state.rs:1132`, `:1159`) into one `ListCursor`, with `window()` and `row_at()`
- [x] 4.2 Move the overlays keeping a bare `selected: usize` onto it: notification center, todo panel, and the pickers
- [x] 4.3 Replace the per-overlay "keep the selection visible" implementations (navigator, notification center, todo panel) with `window()`
- [x] 4.4 Leave `src/ui/list_motion.rs` alone — bubble motion is a display-order transform that composes with the cursor, not part of it
- [x] 4.5 Tests: nearest-edge reveal rather than recentering; clamping at both ends; `row_at` is the inverse of what rendered; group 1's snapshots unchanged

## 5. One keymap, one text field

- [x] 5.1 Extract the list chords into one shared matcher accepting arrows, `j`/`k`, `ctrl+j`/`ctrl+k`, `ctrl+n`/`ctrl+p`, half-page, first/last, with a flag for whether plain characters are text
- [x] 5.2 Adopt it in the navigator (both states), keybind help, notification center, todo panel, settings, and the pickers
- [x] 5.3 Adopt `TextField` in the rename modals (`handle_rename_edit_key`), worktree create, the navigator search box, and the keybind-help search box, deleting each one's append-only editing
- [x] 5.4 Use the field's word-boundary definition everywhere, deleting the duplicate `word_delete_class` usage
- [x] 5.5 Tests: each list-bearing overlay moves on every chord; modified chords work with a search box focused while plain characters are text; each converted input supports the shared editing set; group 1's snapshots unchanged

## 6. One overlay value (refactor-risk — read `AGENTS.md` first)

- [x] 6.1 Before moving anything: name the protected behaviours in the change (mode/state pairing, input-source selection, key-repeat gating, help-panel coverage, mouse dispatch on mode) and confirm each has a characterization test, adding what is missing
- [x] 6.2 Run a roundtable per `AGENTS.md`; this touches core state and UI/input state projection
- [x] 6.3 Add `enum Overlay` carrying today's overlay state structs; replace the parallel `Option<XState>` fields on `AppState` with `overlay: Option<Overlay>`
- [x] 6.4 Derive `wants_ascii_input` from the variant, leaving `Mode` only the five non-overlay modes. There is no `Mode::honors_key_repeat`: key repeat is gated by `App::terminal_input_context`, which returns `Some` only for `Terminal`, `Copy`, `AppScroll` and a popup pane, so it is already structural and has no allowlist to delete — see `roundtable.md`
- [x] 6.5 Derive keybind help entries from the variant; add a guard test asserting every variant contributes at least one, mirroring the existing serialize/deserialize keybinding guard
- [x] 6.6 Keep `Mode` for non-overlay modes and keep input dispatch keyed on it; the variant supplies the mode
- [x] 6.7 Tests: `AppState::assert_invariants_for_test` with `AppState::test_with_adversarial_identity_state` covering mode/overlay agreement; a test that an overlay with no help entry fails; group 1's snapshots unchanged
- [x] 6.8 Run `just check` in full, including the Windows lint leg

## 7. Close out

- [x] 7.1 No changelog entry unless something user-visible moved. If the uniform list chords are user-visible — they are a superset, so an overlay may now accept a chord it did not — say so in one line under Unreleased
- [x] 7.2 Record in `AGENTS.md` that new overlays are built on the kit, replacing the "remember to add a help entry" rule with the enforced one
- [x] 7.3 Note the fork-surface effect in the change: groups 1–5 reduce fork lines, group 6 adds conflict-prone edits to `Mode` and `AppState`; flag group 6 in `herdr-sync-upstream` as a known conflict site
