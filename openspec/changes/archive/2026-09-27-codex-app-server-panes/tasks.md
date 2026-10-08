## 1. Layer A

- [x] 1.1 `[agents.codex]` config with `app_server`, `app_server_socket`, `name_threads`
- [x] 1.2 Launch arguments helper (`--remote unix://…`, `-C cwd`), caller arguments win
- [x] 1.3 Apply to `agent start --kind codex` and to the deferred `codex resume`

## 2. Layer B

- [x] 2.1 Minimal JSON-RPC-over-WebSocket client for a unix socket
- [x] 2.2 Name jobs: resume by id, start by discovery, session report, rename; off the app loop
- [x] 2.3 Skip ephemeral and sub-agent threads; ambiguous discovery names none

## 3. Tests and docs

- [x] 3.1 Unit tests: argv for start and resume, caller precedence, off by default
- [x] 3.2 Client tests against a fake WebSocket daemon on a unix socket
- [x] 3.3 Discovery selection tests (one, none, ambiguous, ephemeral, sub-agent)
- [x] 3.4 Config reference + configuration.mdx

## 4. Verification

- [x] 4.1 `just check` green
- [x] 4.2 Live, no Codex turn: thread in `thread/loaded/list`; `thread/read` shows pane cwd and name; `agent-bell who` lists it as codex; `agent-bell send` arrives queued

Verified 2026-09-27 on m4m, codex-cli 0.157.1, no Codex turn: `agent start` argv
carries `--remote unix://…` and `-C <pane cwd>`; two panes started one after the
other in one directory each got their own thread named after the agent, while an
older unnamed thread there was left alone; a pane in another directory got that
directory as the thread's cwd; `agent rename` renamed the thread; with
`codex-bridge` running, `agent-bell who` listed the panes as `codex` and
`agent-bell send --to cx-one` reached `injected` with the thread still at 0 turns.
The resume path is covered by unit tests only (a resumable session needs a turn).

Found on the way: `createdAt` is not a creation time for threads without turns
(the daemon refreshes it), which made discovery name an older thread. Discovery
uses the time in the thread's UUIDv7 id instead.
