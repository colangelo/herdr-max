## Context

Today `HeadlessServer::detached_size()` returns `handoff_client_size`, or
`headless_size` (from `server.headless_cols/rows`). `sync_foreground_client_state`
uses it whenever there is no foreground client, sets
`app.state.detached_pane_size`, and resizes the shared runtime to it. So a
detach reflows every pane from the real size down to 120×40. The first
attached client clears `handoff_client_size`, so after one attach/detach
cycle even a handoff-carried size is gone.

## Decisions

### Where the size lives

A new `last_client_size: Option<(u16, u16)>` on the server, mirrored into
`AppState` so the session save can write it. It is set in
`sync_foreground_client_state` from the foreground client's `terminal_size`,
and on every resize of the foreground client, if the size is at least the floor.
`detached_size()` becomes `handoff_client_size.or(last_client_size).unwrap_or(headless_size)`.
A change marks the session dirty (the existing debounced save), so a resize
storm costs one write.

### Persistence

`SessionSnapshot` gains `#[serde(default)] last_client_size: Option<(u16, u16)>`.
It is additive and optional, so an older build reads a newer file and a newer
build reads an older file, with no version bump. On startup the restored value
seeds `last_client_size` before the first `detached_pane_size` is set
(`HeadlessServer::new`, where it is set to `headless_size` today). The file is
`~/.config/herdr/session.json`, which is not chezmoi-managed, so each Mac keeps
its own.

### Open question 1: several clients, whose size wins

Recommendation: **the foreground client's size, last change wins.** herdr
already sizes panes from the foreground client while attached, so remembering
exactly that keeps detach invisible: panes stay at the size they had the moment
before. When the foreground moves to another client, its size becomes the
remembered one. A background client never changes it.

### Open question 2: tiny or mobile clients, floor and ceiling

Recommendation: **a floor of 80×24, no extra ceiling.** A size below the floor
in either dimension is used while attached, as today, but not remembered: the
previous remembered size stays. So a quick look from a phone over ssh does not
leave every agent at 50 columns for the night. No ceiling beyond `u16`: a huge
screen is a real screen, and the nightly benefits from width. The floor is a
constant, not a config knob, unless someone asks.

### Open question 3: live handoff

Recommendation: **keep the manifest as it is, and let the handoff size win
while it is fresher.** The exporting server's `effective_size` is already the
foreground client's size (or its own no-client size), so in practice it equals
`last_client_size`. The importing server uses `handoff_client_size` first, then
its own restored `last_client_size`. Both servers share one `session.json`, so
the importing server has the value too, but the manifest is written at the
handoff instant and never older. No manifest change is needed; mixed-version
handoffs behave as today.

### Opt-out

`server.remember_client_size = true` by default. `false` gives exactly today's
behaviour (handoff size, then headless config). An explicit
`server.headless_cols/rows` does not disable remembering: it stays the fallback
for a first-ever start and for `false`. Documented so a user who set headless
sizes on purpose knows to flip the knob.

## Risks

- A remembered size from a big external monitor applies when ac later works on
  the laptop screen with no client attached. That is harmless, because the first
  attach resizes to the real client, as today.
- Restore reads the size before any client exists, so a corrupt value must fall
  back cleanly: a zero or out-of-range size is ignored.
