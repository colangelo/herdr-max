## Why

With no client attached, a new pane is spawned at `estimate_pane_size()` (the
first pane of whatever tab the server shows) and is never resized until a client
attaches. Agent-driven workspaces created while ac is away ran their agents at an
arbitrary width: both panes of a down split came out 39×46.
https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/95

## What Changes

- While no client is attached, `estimate_pane_size()` returns the no-client size:
  `server.headless_cols/rows`, or a size a live handoff carried. Upstream's
  `pane_created_after_detach_uses_configured_headless_size` already expects this.
- A split made while no client is attached sizes the new pane from the target's
  real PTY size divided by the ratio, and shrinks the target to its share.
- With a client attached nothing changes: its next frame lays panes out as today.

## Impact

- `src/app/state.rs`, `src/app/creation.rs`, `src/app/api/{panes,layouts}.rs`,
  `src/server/headless.rs`.
