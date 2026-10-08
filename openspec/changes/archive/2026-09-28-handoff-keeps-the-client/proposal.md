## Why

A live handoff disconnects every client, and the client exits. Nothing brings it
back, so an agent-driven handoff while ac is away leaves the session without a
client. Worse, the importing server's first frame is computed at the headless
default (120×40) and resizes every pane to it, so agent panes stay narrow until
someone relaunches `herdr`. Even when someone does, every handoff reflows every
pane twice. https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/94

## What Changes

- The handoff manifest carries the exporting server's effective size, meaning
  the attached client's size, or its own carried size when none was attached.
  The importing server uses it as the no-client size until a client attaches, so
  panes keep their width through a handoff.
- An app client that receives the handoff shutdown restores the terminal, waits
  up to 30 s for the new server, and reattaches at its current size. Any other
  shutdown, a detach, or a direct terminal attach exits as before.

## Impact

- `src/server/handoff.rs`, `src/server/headless.rs`, `src/client/mod.rs`,
  `src/protocol/wire.rs` (a shared reason constant; no wire change, no
  protocol bump).
- The manifest field is optional, so older and newer servers interoperate. A
  fixed client also reconnects after a handoff done by an older server, because
  the reason text is unchanged.
