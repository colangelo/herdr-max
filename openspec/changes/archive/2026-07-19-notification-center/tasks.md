## 1. Server-owned notification log

- [x] 1.1 Add the notification log to server state: ring buffer (cap 100) of `{id, kind, title, context, target, timestamp}` plus `last_seen_id`, with unread-count math
- [x] 1.2 Add `post_notification()` and replace the direct `self.toast = Some(…)` sites in `src/app/actions.rs` / `src/app/mod.rs` so every toast also appends to the log (toast behavior unchanged)

## 2. API, event, CLI

- [x] 2.1 Add `notification.list` and `notification.mark_seen` socket methods (schema, server dispatch, app handlers)
- [x] 2.2 Emit `EventKind::NotificationPosted` on append through the existing subscription stream
- [x] 2.3 Add `herdr notification list [--json]` CLI verb
- [x] 2.4 Bump `PROTOCOL_VERSION` 18 → 19 (source equals latest released 18) and update protocol expectations/fixtures in tests
- [x] 2.5 Add `notification.clear` socket method + `herdr notification clear [--json]` CLI verb (no protocol bump; source 19 already exceeds released 18)

## 3. Top-right indicator

- [x] 3.1 Render the indicator (icon + unread pill when unread > 0) in the tab bar trailing region with a click hit-area that toggles the panel, coexisting with the new-tab/scroll controls

## 4. Panel + keyboard

- [x] 4.1 Add `Mode::NotificationCenter` (in the `wants_ascii_input` allowlist; not in `honors_key_repeat`) and the anchored dropdown panel rendering (newest-first rows: kind icon, title, context, relative time) reusing existing overlay/list styling
- [x] 4.2 Add the `open_notification_center` `KeysConfig` action (default `prefix+ctrl+n`, collision-verified) with config template entry and a `prefix+?` help entry next to `open_notification_target`
- [x] 4.3 Panel input: open marks all seen; Up/Down + `j`/`k` selection; Enter jumps to the target pane via the existing toast-click focus path and closes; Esc/`q` close; row click jumps; targetless rows not actionable
- [x] 4.4 Footer "Clear all" button (also bound to `c`, hover-highlighted) that empties the log via `AppState::clear_notifications` and keeps the panel open on the empty state
- [x] 4.5 `ui.notification_center_position` config option (`top-right` default, `bottom-right`) wired through startup + live reload, with config template and reference entries

## 5. Tests

- [x] 5.1 Log unit tests: append/cap eviction, monotonic ids, marker/unread math, mark-seen idempotence
- [x] 5.2 API tests: `notification.list` newest-first + unread count, `mark_seen`, `NotificationPosted` event emission, protocol fixture updates
- [x] 5.3 State/TUI tests via `AppState::test_new()`: toast sites append to the log; opening the panel marks seen; selection movement; Enter focuses the target workspace/tab/pane; targetless Enter is a no-op
- [x] 5.4 Clear tests: `NotificationLog::clear` empties + keeps ids monotonic; `notification.clear` API; panel `c` key clears and keeps panel open; footer/list rect split geometry

## 6. Docs

- [x] 6.1 Document the center, indicator, keybinding, and CLI/socket surface in `docs/next` (keyboard.mdx, cli-reference.mdx, socket-api.mdx, config reference)
- [x] 6.2 Add a `docs/next/CHANGELOG.md` Unreleased entry

## 7. Verification

- [x] 7.1 `just check` green
- [x] 7.2 Dogfood on the beta build: watch notifications accrue, open via indicator click and via the keybinding, navigate and jump with Enter, confirm the unread pill clears, and confirm `herdr notification list --json` serves the same feed
