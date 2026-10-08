Ordered so each group lands green on its own. Group 1 is pure state and fully
testable with `AppState::test_new()`; group 2 wires the effects; group 3 is
presentation and docs.

## 1. State: mode, diversion, and the pending-send queue

- [x] 1.1 Add `Mode::AppScroll` to the `Mode` enum (`src/app/state.rs`) and
      `AppState.app_scroll: Option<AppScrollState { pane_id }>` plus
      `AppState.pending_app_scroll_keys: Vec<TerminalKey>`. Mirror the
      copy-mode pairing invariant (`app_scroll.is_some()` iff mode is
      `AppScroll`) in `assert_invariants_for_test` alongside the existing
      copy-mode invariant if one exists.
      *Resolution:* no invariant added. The copy-mode invariant asserts pane
      liveness only, and copy mode itself can hold a stale pane between
      cleanup points; instead of asserting a pairing that external mode
      transitions can transiently break, every consumer of `app_scroll` (key
      handler, drain, overlay) is defensive and exits on a missing or
      unfocused pane
- [x] 1.2 In `enter_copy_mode_scrolled` (`src/app/input/copy_mode.rs`): when
      the focused pane's runtime reports `alternate_screen_active()`, enter the
      passthrough mode instead — set `app_scroll`, set the mode, and queue the
      entry key (`PageUp` for `Page`/`HalfPage`, nothing for `Line`). Runtime
      unresolvable → existing copy-mode entry path unchanged
- [x] 1.3 `AppState::handle_app_scroll_key`: release events ignored; translate
      per the design table (`ctrl+u`/`pgup`→`PageUp`, `ctrl+d`/`pgdn`→
      `PageDown`, `g`/`home`→`Home`, `shift+g`/`end`→`End`) into the pending
      queue; `esc`/`q`/`enter` exit to `Mode::Terminal`; prefix chord enters
      `Mode::Prefix`; anything else swallowed. Pinned pane no longer focused →
      exit without queueing
- [x] 1.4 Tests (state-only, no PTYs): diversion on an alt-screen pane vs
      copy-mode entry on a primary-screen pane; entry queueing per gesture;
      each translation; swallowing; exit keys; prefix precedence; focus-moved
      exit; invariants hold under
      `AppState::test_with_adversarial_identity_state()` if the copy-mode
      invariant is covered there

## 2. App: draining the queue into the pane

- [x] 2.1 `App::dispatch_pending_app_scroll_keys()`: resolve the pinned pane's
      runtime, `encode_terminal_key(key)`, send via `lookup_runtime_sender`;
      drop silently (and exit the mode) if the pane or runtime is gone.
      Call it wherever `dispatch_pending_clipboard_write` is called after the
      dispatch paths that can queue: the `Mode::AppScroll` arm in
      `App::handle_key` (`src/app/input/mod.rs`) and both navigate dispatchers
      (`App::execute_tui_navigate_action`,
      `execute_navigate_action_in_context` callers)
- [x] 2.2 Route `Mode::AppScroll` in `App::handle_key` to the state handler +
      drain
- [x] 2.3 Confirm the headless/state-level dispatch path
      (`execute_navigate_action_in_context`) queues identically and its caller
      drains; add a test through `handle_key` on a real test `App` with a
      terminal runtime fed `\x1b[?1049h` (alternate screen on), asserting the
      encoded `PageUp` bytes reach the runtime sender, mirroring how existing
      forwarding tests capture sent bytes
- [x] 2.4 Check mouse routing while the mode is active behaves sanely (no
      special handling expected — wheel routing is already per-pane-state);
      note findings in the PR/commit if a follow-up is warranted.
      *Resolution:* `mode_bar_covers_tab_row` now includes `Mode::AppScroll`
      so the overlay row shields the bottom tab bar, matching copy mode. A
      click that refocuses another pane is caught by the focus-mismatch exit
      on the next key; wheel events keep their per-pane routing untouched

## 2b. Line-granular scrolling (added after live dogfood feedback)

- [x] 2b.1 Extend the pending queue to `AppScrollSend { Key, WheelUp,
      WheelDown }` (`src/app/state.rs`); map `ctrl+k`/`k`/`Up` and
      `ctrl+j`/`j`/`Down` to wheel ticks, and the line entry gesture to one
      wheel-up tick
- [x] 2b.2 Drain encodes a tick per the pane's wheel routing: mouse report at
      the pane's centre (`pane_mouse_position` widened to `pub(super)`),
      alternate-scroll arrows under DECSET 1007, dropped otherwise
- [x] 2b.3 Tests: wheel reports on a mouse-capturing pane (SGR 64/65), arrows
      under 1007, dropped with 1007 explicitly disabled (it defaults on in
      ghostty-vt), line entry sends one tick where supported

## 3. Presentation and docs

- [x] 3.1 Indicator on the focused pane while the mode is active, naming the
      exit key (reuse pane badge/overlay styling in `src/ui/panes.rs`)
- [x] 3.2 Config template comments for the three entries mention the
      alt-screen behavior (`src/config/` template strings), without adding any
      new config key
- [x] 3.3 Docs: extend the copy-mode/scrollback page under
      `docs/next/website/src/content/docs/` with the passthrough behavior and
      key table
- [x] 3.4 `just check` green; update this file's checkboxes and the fork issue
      (https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/59)
