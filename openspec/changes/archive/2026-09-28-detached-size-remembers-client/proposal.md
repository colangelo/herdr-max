## Why

When the last client detaches, herdr resizes every pane to the no-client size:
the size a live handoff carried, or `server.headless_cols/rows` (default
120×40). After a server restart, panes start at 120×40 until a client
attaches. Tools that read agent screens while ac is away see a narrow screen
that ac never uses. The 04:30 nightly (CONTEXT herdr-compact `nightly.py` /
`idle_maintain`) reads Claude Code screens detached, and at 120 columns their
footer and right-aligned notification row clip. On 2026-09-28 a clipped footer
made a restart drop the permission flag. ac asked for the detached size to come
from the real screen instead.
https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/102

Planned 2026-09-28, NOT to be built until ac batches it with other items.

## What Changes

- The server remembers the size of the last real client: the foreground
  client's terminal size, updated on attach, on every resize, and when the
  foreground client changes.
- On detach, the no-client size is that remembered size. Precedence: a size a
  live handoff carried (it is the same size, just fresher), then the remembered
  client size, then `server.headless_cols/rows`, then 120×40.
- The remembered size is written to `session.json` as an optional field, so it
  survives a server restart or a cold start. `session.json` is per machine, so
  each Mac remembers its own screen; nothing goes into the shared dotfiles.
- Sizes smaller than a floor (80×24) are not remembered, so a brief attach
  from a phone or a tiny split does not shrink every pane for the night.
- Nothing changes while a client is attached. Splits made while detached keep
  the current rule (divide the target pane's real size).

## Impact

- `src/server/headless.rs` (`detached_size`, `sync_foreground_client_state`,
  client resize handling), `src/app/state.rs`, `src/persist/snapshot.rs`
  (an optional `last_client_size`, serde default, no version bump),
  `src/app/session.rs` (restore it on startup).
- No wire or protocol change, no handoff manifest change (the manifest already
  carries `client_size`).
- Config: optional `server.remember_client_size` (default `true`); `false`
  restores today's behaviour exactly. Documented in docs/next configuration.
