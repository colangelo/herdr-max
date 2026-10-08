# Tasks

## 1. Setup and existing key support

- [x] 1.1 Verify worktree writes and `cargo check --bin herdr` with assigned Zig; record outcome in progress.
- [ ] 1.2 Verify ac's two prefixes and retained letter range dispatch/help; cover client target selection past index nine and prefix consumption.
- [ ] 1.3 Connect retained keybind warnings to non-fatal startup/reload client diagnostics; test warning text, intentional unbind and command preservation.

## 2. Copy and application scrolling

- [ ] 2.1 Port Ctrl+g and viewport-only Ctrl+k/j with fork behavior tests, selection clamp/offset coalescing and mode-bar help.
- [ ] 2.2 Wire six directional/granularity entry actions with help and held-entry repeat handoff; test exact movement, no-scrollback entry and downward no-op.
- [ ] 2.3 Complete protocol/input roundtable against frozen codecs and original fork tests before adding application scroll API; record findings in tracked plan.
- [ ] 2.4 Implement/advertise single-target alternate-screen scroll intent; test wheel/no-support/lost-screen cases and legacy/kitty generated key bytes.
- [ ] 2.5 Implement client SCROLL state, keys, repeat target guards, Claude agent precedence, exits and fork instruction bar; test focus/boot loss and no sync fanout.

## 3. Runtime controls

- [ ] 3.1 Wire retained layout balance/preset APIs into client actions/help; test config overrides, per-axis balance and cycle order.
- [ ] 3.2 Rename fork history-purge advertisement to `pane.clear_scrollback`, preserving `pane.clear`; test full visible screen preservation and client default/binding.
- [ ] 3.3 Wire pane respawn with client confirmation and external response semantics; test key dispatch, unavailable method and lifecycle identity.
- [ ] 3.4 Advertise existing pane.move and wire break/adjacent-tab actions/help; test stable target IDs, bounds/no-op and existing move reconciliation.

## 4. Sync and pin input

- [ ] 4.1 Advertise retained group APIs and optional revision-bound state; test old/missing/stale feature data and unchanged generation-1 fixtures.
- [ ] 4.2 Fan out typed semantic key/text/paste through headless runtime routing with original held recipients; test heterogeneous peers, membership/focus changes, excluded origin, other tabs and popup/mouse isolation.
- [ ] 4.3 Wire sync toggle/member/pair menu input/help; test passthrough-first right-click matrix and empty-group grace.
- [ ] 4.4 Wire pin/unpin key/menu/marker actions using chrome-compatible stable-target helpers; test marker precedence, pin ordering data and missing API.
- [ ] 4.5 Disable workspace drag reorder under priority sort while preserving focus and linked-worktree/endpoint guards; test drag/focus distinction.

## 5. Chrome input integration

- [ ] 5.1 Connect display-window key and host/sidebar/split/keyboard-resize gestures to chrome helper; test held/release deadlines and defaults.
- [ ] 5.2 Connect overflow edge-row page clicks and copy-feedback source identity; test covered/stale hit regions and async source preservation.
- [ ] 5.3 Report out-of-range indexed agent target with final displayed ordering; test no focus/input request and diagnostic context.

## 6. Later overlay dependencies

- [ ] 6.1 Port common list/search/capitalization and editor deltas after daily-use verification; test Ctrl+j/k priority, Shift text/actions, Esc order, undo and word-arrow editing.
- [ ] 6.2 Integrate notification/todo/board/move-picker input only after overlay models/render/hits exist; port named fork tests and add hover/near-miss/paste/repeat coverage.

## 7. Integration

- [ ] 7.1 Rebase onto lead's test-clean tag and run each topic's authored/ported tests plus `just check`; record actual results, never mark merely compiled tests passing.
- [ ] 7.2 Review all newly advertised shapes/optional controls against unchanged generation-1 fixtures and endpoint-local failure behavior.
- [ ] 7.3 Report topic SHAs, tests and visible differences to lead via progress; add completion timestamp only when seat checks pass.
