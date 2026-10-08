## 1. Actions and config keys

- [x] 1.1 Add `NavigateAction` variants `BreakPaneToNewTab`, `MovePaneToTab`, `MovePaneToNextTab`, `MovePaneToPrevTab` in `src/app/input/navigate.rs`
- [x] 1.2 Add matching `KeysConfig` fields in `src/config/model.rs` with defaults: `break_pane = "prefix+!"`, `move_pane_to_tab = "prefix+m"`, and `move_pane_next_tab` / `move_pane_prev_tab` (candidate `prefix+>` / `prefix+<`, per design Open Questions)
- [x] 1.3 Wire each new `KeysConfig` field to its `NavigateAction` in the keymap binding table (`src/app/input/navigate.rs:~1467`)
- [x] 1.4 Confirm no default-chord collision with existing bindings; adjust or leave adjacent-move unbound if unresolved

## 2. Break pane to new tab

- [x] 2.1 Implement the `BreakPaneToNewTab` handler that calls the existing `pane.move` client path with a `new_tab` destination (current workspace), focusing the new tab per spec
- [x] 2.2 Guard the single-pane-source case as a no-op with a non-blocking toast
- [x] 2.3 Surface `pane.move` rejections (e.g. `zoomed_tab`) as a non-blocking toast, leaving layout unchanged

## 3. Move pane to tab via picker

- [x] 3.1 Add a tab-target picker modal reusing the existing picker/modal infrastructure (as used by the workspace picker), listing the workspace's other tabs by number/label and excluding the current tab and invalid targets
- [x] 3.2 Implement the `MovePaneToTab` handler: open the picker; on select, call `pane.move` with a `tab` destination and default split `right` next to the target tab's focused pane
- [x] 3.3 Handle cancel (no change) and the no-other-tabs case (do not open an empty picker; show a non-blocking toast)

## 4. Quick move to adjacent tab

- [x] 4.1 Implement `MovePaneToNextTab` / `MovePaneToPrevTab` handlers resolving the adjacent tab in workspace tab order and calling `pane.move` with a `tab` destination and default split
- [x] 4.2 Make no-adjacent-tab a no-op (no wrap, no tab creation) with a non-blocking toast

## 5. Tests

- [x] 5.1 Add unit tests covering handler target resolution and edge cases (single-pane source, no other/adjacent tab, rejection passthrough) using `AppState::test_new()` / `Workspace::test_new()` without PTYs
- [x] 5.2 Add a keymap test asserting the four new `KeysConfig` fields map to the correct `NavigateAction` variants
- [x] 5.3 Assert no new protocol `Method`/`Request` variant is introduced and `PROTOCOL_VERSION` is unchanged

## 6. Docs and validation

- [x] 6.1 Document the new bindings in `docs/next/website/src/content/docs/keyboard.mdx` and the `[keys]` block in `configuration.mdx`
- [x] 6.2 Run `just check` (fmt + nextest + maintenance script tests) and fix any failures
- [x] 6.3 Dogfood live: break a pane to a new tab, move a pane via the picker, and quick-move to an adjacent tab; confirm process/terminal preservation and toast feedback on no-op paths
