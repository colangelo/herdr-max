# Launch Codex panes on the shared app-server daemon

## Why

agent-bell (ac/infra#184, D3′) pushes peer messages into agent sessions by
name. Its Codex receiver, `codex-bridge`, talks to the shared Codex app-server
daemon over its control socket, so it can only reach threads that live on that
daemon, and it addresses them by the thread's name. herdr starts and resumes
Codex panes, so herdr is where both have to happen
(https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/85).

Measured with codex-cli 0.157.1: `codex --remote unix://…/app-server-control.sock -C <dir>`
creates a thread on the daemon at launch, before any turn, with the given cwd.
Without `-C`, a `--remote` thread takes the daemon's cwd. The thread starts
unnamed; `thread/name/set` names it; ephemeral threads refuse it. herdr's Codex
hook reports the session id only at the first turn, so at launch herdr does not
know its pane's thread id.

## What Changes

Two layers, each behind its own switch, both off by default:

- **Layer A** (`agents.codex.app_server = true`): `agent start --kind codex` and
  the restore path's `codex resume <id>` add
  `--remote unix://<agents.codex.app_server_socket>` and `-C <pane cwd>`. The
  socket defaults to `~/.codex/app-server-control/app-server-control.sock`.
  Arguments the caller already passes (`--remote`, `-C`/`--cd`) win.
- **Layer B** (`agents.codex.name_threads = true`, only with layer A): herdr
  names the pane's thread after the pane's agent name over the same socket
  (JSON-RPC over WebSocket at `ws://localhost/rpc`). For a resume the thread id
  is the saved session id. For a new start herdr finds the thread itself: the
  one unnamed, non-ephemeral, top-level thread in the pane's cwd created since
  the launch. If there are several, it names none rather than guess, and names
  the thread once the Codex hook reports the session id. Renaming the agent
  renames its thread. Ephemeral threads are skipped quietly. Every call runs off
  the app loop, and a failure is logged and never fails the pane.

Turning B off leaves A exactly as it is.

## Impact

- New config section `[agents.codex]`; documented in `docs/next`.
- New module `src/codex_app_server.rs`: launch arguments, and a small
  JSON-RPC-over-WebSocket client for a unix socket (no new dependency: the
  daemon is local, and the client needs text frames only).
- Touches `agent start` (src/app/agents.rs), deferred resume
  (src/app/agent_resume.rs), agent rename, and the agent-session report.
- Unix only; on other platforms the switches do nothing.

## Non-goals

- Running or supervising `codex-bridge` (infra/macos-setup).
- Changing how herdr detects Codex state.
