# notification-center — design decisions

Decisions settled in the 2026-07-18 design discussion, recorded so
implementation does not relitigate them.

## Feed scope (v1)

Exactly the existing toast sources: agent needs-attention, agent finished,
update installed, `notification.show` injections. The hook is a single
`post_notification()` helper replacing the direct `self.toast = Some(…)` sites
(~5 non-test sites in `src/app/actions.rs` / `src/app/mod.rs`), so the log can
never disagree with the toasts the user actually saw, and the existing
pending-notification debounce / focus-suppression / delivery config keep doing
the judgment. Broader or configurable sources are a separate backlog issue.

## Read model: high-water mark

One `last_seen_id` marker in server state; unread = entries newer than the
marker. Opening the panel advances the marker (same path as the
`notification.mark_seen` API). No per-item read flags, no per-item dismiss —
old items age out of the ring buffer (cap 100, constant, not config). The
marker lives server-side so the TUI panel and any external bar always agree on
the unread count.

## Interaction

- Bindable action `open_notification_center` opens the dropdown; default
  `prefix+ctrl+n` (verify no collision at implementation; `prefix+n` and
  `prefix+shift+n` are taken). Help entry in the global group next to
  `open_notification_target`.
- In the panel: up/down and `j`/`k` move selection, Enter jumps to the target
  pane (reuse the toast-click / `open_notification_target` focus path) and
  closes, Esc/`q` closes, row click jumps, click outside or on the indicator
  closes. Rows without a pane target are not actionable.
- New `Mode::NotificationCenter`: added to the `wants_ascii_input` allowlist;
  NOT added to `honors_key_repeat` (held arrows firing once matches the
  navigator; plain `j`/`k` repeat as text).

## In-memory only (v1)

The log lives in server memory: it survives live handoffs to the extent server
state transfers, and empties on a cold server restart. Documented limitation;
persistence (e.g. in `session.json`) is a possible follow-up, deliberately not
in v1.

## Extension feasibility (why this is built-in)

Analyzed against the current plugin surface (`plugin.pane.open/focus/close`,
`plugin.action.*`, socket events):

- A plugin daemon **could** approximate a degraded center: subscribe to
  `PaneAgentStatusChanged`, keep its own log, present a popup-pane TUI, jump
  via `pane.focus`.
- It **cannot**: render a tab-bar indicator (no plugin status-row surface —
  the same gap that made #11 a built-in feature); see `notification.show`
  injections or update notifications (not evented); reproduce the server-side
  notification judgment without drifting from the built-in toasts; or share
  seen state with any other consumer.

Conclusion: the server log + API + `NotificationPosted` event are the
non-extensible core, and they are what make notification UIs extensible
afterwards — the built-in dropdown is consumer #1, an external status bar is
consumer #2 (`herdr notification list --json` to poll, or subscribe for push),
and a future plugin UI could be consumer #3 with full fidelity. If upstream
ever adds a plugin status-segment render surface, the indicator could be
revisited; out of scope here.

## Protocol

Source `PROTOCOL_VERSION` is 18 and the latest released tag (v0.7.4-ac) also
shipped 18, so adding `notification.list` / `notification.mark_seen` /
`EventKind::NotificationPosted` bumps to 19 per the convention. Update
hardcoded protocol expectations and manual fixtures in tests.

## Relationship to issue #11

The indicator is the first tenant of the tab bar's trailing region that #11
plans to fill with catppuccin-style module pills; it should be styled so the
pills can join it without rework. Module pills (load/uptime/clock/host) remain
#11's scope. The "notification unfold" affordance #11 sketched is superseded
by this change's panel.
