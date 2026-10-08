## 1. Honor key repeat in copy mode

- [x] 1.1 Add `Mode::honors_key_repeat(self) -> bool` (true for `Terminal | Copy`) in `src/app/state.rs`, with a doc comment explaining the modal-leak rationale
- [x] 1.2 In `src/app/runtime.rs`, gate the `Press`-time `suppressed_repeat_keys` insert and the `Repeat`-dispatch branch on `popup || mode.honors_key_repeat()` instead of `== Mode::Terminal`
- [x] 1.3 In `src/app/mod.rs::route_client_events`, do the same, and decouple the repeat decision from handler routing so a `Repeat` in copy mode is routed to `handle_non_terminal_key_headless`, not the terminal handler

## 2. Ctrl-K / Ctrl-J viewport scroll in copy mode

- [x] 2.1 Add `scroll_copy_mode_viewport_line(direction)` in `src/app/input/copy_mode.rs`: shift `offset_from_bottom` by one line, keep the cursor anchored to the same buffer row, clamp the cursor to the viewport edge, no-op at history top / live bottom
- [x] 2.2 Bind `Ctrl-K` (reveal older) and `Ctrl-J` (toward bottom) in `handle_copy_mode_key`
- [x] 2.3 Add a `^k/^j scroll` hint to the copy-mode overlay footer in `src/ui/menus.rs`

## 3. Tests

- [x] 3.1 Unit test `Mode::honors_key_repeat` (true for `Terminal`/`Copy`, false for others)
- [x] 3.2 Copy-mode test: `Ctrl-K` scrolls the viewport one line and leaves the cursor's buffer row unchanged
- [x] 3.3 Copy-mode test: `Ctrl-J` scrolls back toward the bottom after scrolling up
- [x] 3.4 Copy-mode edge tests: `Ctrl-J` at the bottom and `Ctrl-K` at history top are no-ops
- [x] 3.5 Integration test: a `Press` then `Repeat` of a held key in copy mode dispatches the repeat (proves the suppression-insert + dispatch-gate fix together)

## 4. Docs

- [x] 4.1 Document `Ctrl-K` / `Ctrl-J` copy-mode scroll and the now-working held-key repeat in `docs/next/website/src/content/docs/keyboard.mdx`
- [x] 4.2 Add a `docs/next/CHANGELOG.md` Unreleased entry

## 5. Verification

- [x] 5.1 `just check` green
- [x] 5.2 Dogfood on the beta build: shipped `0.7.4-ac-beta.20260718165308` via the beta loop and live-handed-off the running server onto it (panes preserved). Behavior confirmed in the running session: held `Ctrl-U` / `Ctrl-D` pages continuously and `Ctrl-K` / `Ctrl-J` scroll the viewport line-wise with the cursor anchored. (Automated coverage: `just check` unit + integration tests; hold-to-repeat itself needs a physical Kitty-protocol terminal, so it is not automatable.)
