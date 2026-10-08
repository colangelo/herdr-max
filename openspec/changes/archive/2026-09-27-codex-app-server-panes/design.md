# Design: Codex panes on the shared app-server daemon

## Finding the thread

The thread id is the Codex session id (`thread/read` returns `sessionId == id`),
but herdr's Codex hook reports it only at the first turn. So:

- **Resume**: `codex resume <id>` reuses thread `<id>`; name it directly once it
  is loaded (poll `thread/read` briefly).
- **Start**: poll `thread/loaded/list` for up to 20 s; for each id, `thread/read`;
  candidates are top-level (`parentThreadId` null), non-ephemeral, unnamed,
  `cwd` equal to the pane's cwd (canonicalised) and created no earlier than the
  launch (1 s slack). Creation time comes from the thread id, a UUIDv7, not
  from `createdAt`: the daemon refreshes `createdAt` on threads with no turns,
  and an old empty thread then looks brand new. Exactly one candidate: name it. Several: stop and
  leave it to the session report.
- **Session report** (first turn): if the thread is still unnamed, name it.
- **Rename**: with a known session id, rename that thread; otherwise rename the
  thread in the pane's cwd whose name is the old agent name.

## Client

A blocking client in a background thread: connect to the unix socket, send an
HTTP/1.1 upgrade for `/rpc`, check `101`, then masked text frames out and
unmasked frames in (text, continuation, ping answered with pong, close). Each
job opens its own connection, sends `initialize` + `initialized`, does its
calls with a read timeout, and closes. No new crate: the daemon is local and
trusted, only text frames are needed, and the handshake's accept key is not
checked (nothing in between to guard against).

## Config

`[agents.codex]` with `app_server` (bool, false), `app_server_socket` (string,
the default path), `name_threads` (bool, false). Read live from config, so a
reload applies to the next launch.
