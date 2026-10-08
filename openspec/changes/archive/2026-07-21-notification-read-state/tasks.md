# Tasks: notification-read-state

## 1. Server state

- [x] 1.1 `src/app/state.rs`: `NotificationEntry.read: bool`; `unread_count()` counts unread entries; `mark_read(id) -> bool`; `mark_all_seen()` sets every entry read; drop the `last_seen_id` unread semantics
- [x] 1.2 `open_notification_center()` no longer marks seen; activation paths (modal Enter local + App twin, mouse row click) mark the activated entry read; targetless entries stay unread
- [x] 1.3 `r` in the panel marks all read, panel stays open (modal key handler + App twin)

## 2. API + CLI

- [x] 2.1 `src/app/api.rs`: `notification.list` entries gain `read`; `notification.mark_seen` accepts optional `id` (present = one entry, absent = all); update API tests
- [x] 2.2 CLI `herdr notification list`: read/unread marker in text output, `read` in `--json`
- [x] 2.3 Verify `PROTOCOL_VERSION` is already greater than the latest released tag (expect 19 > released; no bump)

## 3. TUI rendering

- [x] 3.1 `src/ui/notification_center.rs`: unread rows = kind dot + bold title; read rows = blank dot column + dim regular title; selected row unchanged
- [x] 3.2 Footer: settings-style `render_action_button` boxes with hints — `c clear all` + `esc close`, plus `r mark read` when width allows; right-aligned; hover keeps accent fill; update `src/app/input/mouse.rs` hit rects

## 4. Tests

- [x] 4.1 State: open leaves unread; activate marks one read; `r`/mark-all reads all; targetless stays unread; unread_count per-entry
- [x] 4.2 API: mark_seen with/without id; list carries `read`
- [x] 4.3 Buffer: unread vs read row styling; footer buttons render settings-style; button hit areas

## 5. Validation

- [x] 5.1 Docs: amend the unreleased notification-center entry in `docs/next/CHANGELOG.md`
- [x] 5.2 `cargo fmt` + `just check`
- [x] 5.3 Dogfood: beta, upgrade, user verifies (badge stays on open, decrements per click, `r` silences, dim read rows, new buttons); then resolve+close issue #25 and archive
