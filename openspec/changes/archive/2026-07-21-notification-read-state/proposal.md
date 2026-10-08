# Notification Read State

## Why

The notification center tracks unread with a single `last_seen_id` high-water mark: opening the dropdown marks everything seen at once, every row renders identically (kind-colored dot, plain title), and nothing records which entries the user actually visited. Design agreed in Gitea issue https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/25.

## What Changes

- `NotificationEntry` gains server-owned `read: bool`. Activating an entry (Enter or row click, which jumps to its pane) marks that entry read. Opening the panel no longer marks anything.
- The `◆ N` indicator counts unread entries and decrements per visited entry (sticky badge). `Clear all` still empties the log; a new `r` key marks all read while keeping history.
- Row styling: unread = kind-colored dot + bold title; read = blank dot column + regular-weight title dimmed to the muted gray. Selected-row styling unchanged.
- Footer adopts the settings button language: filled boxes with the shortcut hint inside — `c clear all` + `esc close` (+ `r mark read` when width allows), right-aligned, surface at rest, accent on hover.
- API/CLI parity: `notification.list` entries gain `read`; `notification.mark_seen` accepts an optional `id` (absent = mark all, as today). `herdr notification list` shows a read marker.
- The notification center is unreleased, so `notification.mark_seen` / `notification.list` shapes may change without compatibility shims. Check `PROTOCOL_VERSION` against the latest released tag; no bump expected (already ahead at 19).

## Capabilities

### New Capabilities

<!-- none -->

### Modified Capabilities

- `notification-center`: per-entry read state replaces the open-marks-all-seen model; row styling, footer buttons, `r` action, and API field additions.

## Impact

- `src/app/state.rs`: `NotificationEntry`, `NotificationLog` (`unread_count`, `mark_read(id)`, `mark_all_seen`), `open_notification_center`, activation paths.
- `src/app/input/modal.rs`, `src/app/input/mouse.rs`: `r` key, activation marks read, footer button hit areas.
- `src/ui/notification_center.rs`: row styles, footer buttons.
- `src/app/api.rs`: `notification.list` read field, `notification.mark_seen` optional `id`.
- `src/main.rs` / CLI: notification list read marker.
- Docs: amend the unreleased notification-center changelog entry.
