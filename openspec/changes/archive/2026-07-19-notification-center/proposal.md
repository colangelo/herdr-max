## Why

Herdr notifications today are transient singletons: `AppState.toast` holds one
`ToastNotification` at a time, replaced or expired per the configured per-kind
durations, then gone. There is no history anywhere — step away for ten minutes
and there is no way to see which agents finished or blocked while you were
gone. The planned top-right status widget (fork issue #11) reserves a
"notification unfold" click target, but as scoped it could only re-show the
single current toast, which is not a notification center.

Separately, external status bars (tmux-catppuccin-style top bars) have no
faithful way to show herdr notification state: the event stream carries
`PaneAgentStatusChanged`, but `notification.show` injections and update
notifications are not evented, and the server-side judgment that turns raw
status changes into user-worthy notifications (pending-notification debounce,
focus suppression, delivery config) is not observable at all.

## What Changes

- **Server-owned notification log**: a bounded in-memory ring buffer (cap 100)
  of notifications — `id` (monotonic), kind, title, context, optional
  workspace/pane target, timestamp — plus a `last_seen_id` high-water mark. A
  single `post_notification()` helper replaces the direct `self.toast = Some(…)`
  sites so every toast also lands in the log; transient toast behavior is
  unchanged. v1 sources are exactly the toast sources: agent needs-attention,
  agent finished, update installed, and `notification.show` injections.
- **Socket API + events**: `notification.list` (entries newest-first, unread
  count, marker), `notification.mark_seen` (advance the marker), and a new
  `EventKind::NotificationPosted` on the existing subscription stream. CLI:
  `herdr notification list [--json]`. Protocol version bumps 18 → 19 (source
  protocol equals the latest released tag).
- **Top-right indicator**: a compact icon in the tab bar's trailing region with
  an accent-colored unread-count pill when unread > 0, styled so issue #11's
  catppuccin module pills can visually join it later. Click toggles the panel.
- **Dropdown panel + keyboard**: a new `Mode::NotificationCenter` overlay
  anchored under the indicator, reusing the existing overlay/list UI language.
  Opening marks all seen. Up/down (and `j`/`k`) move the selection; Enter jumps
  to the notification's target pane (same focus path as today's toast click)
  and closes; Esc/`q` closes; row click jumps. New bindable action
  `open_notification_center` (named alongside the existing
  `open_notification_target`), default `prefix+ctrl+n` (collision-verified at
  implementation), discoverable in the `prefix+?` help panel.
- **Non-goals**: issue #11's data module pills (load/uptime/clock/host — remain
  #11's scope; the indicator is the first tenant of that trailing region),
  configurable notification sources (follow-up backlog issue), log persistence
  across cold server restarts, and a plugin status-segment render surface.

## Capabilities

### New Capabilities

- `notification-center`: a server-owned bounded notification log with
  seen-marker semantics, exposed through the socket API, subscription events,
  and CLI, presented in the TUI as a top-right unread indicator with a
  keyboard-navigable dropdown that jumps to notification targets.

### Modified Capabilities

<!-- None. Toast behavior, toast config, and the plugin API are unchanged. -->

## Impact

- **Server state** (`src/app/state.rs`, `src/app/actions.rs`, `src/app/mod.rs`):
  notification log struct + `post_notification()` replacing direct toast
  assignments.
- **API** (`src/api/schema.rs`, `src/api/server.rs`, `src/api/subscriptions.rs`,
  `src/app/api.rs`): two new methods, one new event kind.
- **Protocol** (`src/protocol/wire.rs`): 18 → 19, with test fixture updates.
- **CLI** (`src/cli/spec.rs`, `src/cli/…`): `herdr notification list`.
- **TUI** (`src/ui/tabs.rs`, new panel module under `src/ui/`,
  `src/app/input/…`, `src/ui/keybind_help.rs`, `src/config/model.rs`,
  `src/config/keybinds.rs`, `src/main.rs`): indicator, panel, mode, action,
  binding, help entry, template entry.
- **Boundary guardrail**: the log, marker, methods, and event are shared
  runtime facts with neutral names (notification log/list/seen); the
  indicator, panel, selection, and colors are TUI/client presentation.
- **Extension feasibility** (analyzed 2026-07-18, recorded in `design.md`): a
  plugin could approximate a degraded center today (subscribe to
  `PaneAgentStatusChanged`, own log, popup-pane UI, `pane.focus` to jump) but
  cannot render a tab-bar indicator (no plugin status surface — same
  conclusion as #11), cannot see `notification.show` or update notifications,
  would drift from the server's notification judgment, and cannot share seen
  state. The server core in this change is exactly the part that cannot be an
  extension — and once it exists, external bars and future plugin UIs become
  full-fidelity consumers of the same feed.
- **Upstream candidate**: the server log + API + event core is generic and
  fork-agnostic; note for a potential upstream contribution, but nothing is
  opened on upstream.
