## 1. Config actions

- [x] 1.1 Add `copy_mode_page_up` (default `prefix+pageup`), `copy_mode_half_page_up` (default `prefix+ctrl+u`), and `copy_mode_line_up` (default `prefix+ctrl+k`) `BindingConfig` fields to `KeysConfig` in `src/config/model.rs`, with doc comments, profile/apply wiring, and parse-test rows
- [x] 1.2 Add commented entries to the generated config template in `src/main.rs`, noting that `prefix+ctrl+b` is available for `copy_mode_page_up` only when the prefix is not `ctrl+b` (send-prefix shadows it)

## 2. Dispatch and behavior

- [x] 2.1 Add `NavigateAction::{CopyModePageUp, CopyModeHalfPageUp, CopyModeLineUp}` variants, map the new `KeysConfig` fields in the binding lookup, and add them to the copy-mode-survives set so re-invocation does not cancel copy mode
- [x] 2.2 Add an entry helper in `src/app/input/copy_mode.rs`: if copy mode is active on the focused pane, scroll only; otherwise cancel stale copy mode, `enter_copy_mode`, then scroll via `scroll_copy_mode_page(-1, half)` / `scroll_copy_mode_viewport_line(-1)`
- [x] 2.3 Wire the three action arms in `execute_tui_navigate_action` to the helper

## 3. Discoverability

- [x] 3.1 Add three `help_entry` rows to the panes group in `src/ui/keybind_help.rs`

## 4. Tests

- [x] 4.1 Config parse test covers the three new `[keys]` entries
- [x] 4.2 Gesture test: from terminal mode, prefix + PageUp enters copy mode scrolled one page; half-page and line variants scroll their amounts
- [x] 4.3 Repeat test: a second gesture scrolls further, preserves `entry_offset_from_bottom`, and exit restores the pre-gesture scroll position
- [x] 4.4 Characterization test: `prefix+prefix` (send-prefix) still passes the literal prefix through and fires no scroll action

## 5. Docs

- [x] 5.1 Document the gesture in `docs/next/website/src/content/docs/keyboard.mdx` (copy mode section + keybinding table if applicable)
- [x] 5.2 Add a `docs/next/CHANGELOG.md` Unreleased entry

## 6. Verification

- [x] 6.1 `just check` green
- [x] 6.2 Dogfood on the beta build: shipped `0.7.4-ac-beta.20260718211457` via the beta loop and live-handed-off the running server onto it (panes preserved). Verified live in the running session (2026-07-18): `prefix+PageUp` / `prefix+ctrl+u` / `prefix+ctrl+k` enter copy mode pre-scrolled, repeating scrolls further, `q` restores the live view, and `prefix+prefix` send-prefix still passes through
