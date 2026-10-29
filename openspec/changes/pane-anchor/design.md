## Context

macOS attributes a process's privacy (TCC) requests to its responsible process,
which a child inherits from its parent unless the parent disclaims it at spawn
(`responsibility_spawnattrs_setdisclaim`, used since #240 for the handoff
replacement server). The answer to an AppData prompt is kept for the life of the
responsible process (Apple DTS forum thread 742147; measured on #240 with the
tracer `~/.local/state/tcc-trace/requests.jsonl`). A pane's shell and everything it
runs, `op` included, therefore re-prompt after every live handoff: their
responsible process, the old server, is gone.

The fix is to make the responsible process of panes something that outlives the
server. Panes already survive handoff without being children of the new server:
an imported pane's exit is seen as PTY EOF (`TerminalRuntime::from_handoff_fd`).

## Goals / Non-Goals

**Goals:** one answer per anchor lifetime (until logout, reboot or the session's
last pane closes), unchanged pane behaviour, exit statuses kept for panes the
current server started, safe fallback.

**Non-Goals:** changing which identity the prompt names (it stays the herdr
binary), Linux or Windows, re-parenting panes that already run under a dead
server (they are respawned once after the first install, as today).

## Decisions

### D1. One anchor per session, started disclaimed

The server, on macOS, connects to `<api socket dir>/herdr-pane-anchor.sock`. When
nothing answers it starts `<exe> __pane-anchor --socket <path>` with
`spawn_disclaimed_daemon` (new session, stdio on /dev/null, its own responsible
process) and connects with a short retry. A socket path over the platform limit, a
failed start or a failed connect means no anchor: the server spawns panes itself
and logs the reason once.

### D2. The anchor runs from a link the package manager cannot remove

Homebrew deletes the old keg on upgrade. A running process whose executable is
deleted may fail a code-signing check when tccd evaluates it, which would bring the
prompt back. The server hard-links its executable to
`<config dir>/pane-anchor/<version>/<exe file name>` (a copy when linking fails)
and starts the anchor from there. The file name stays the executable's, so the
prompt still says "herdr-beta". The anchor writes `anchor.pid` next to the link;
at start the server removes version directories whose `anchor.pid` names no live
process, never its own.

### D3. The server keeps the PTY; the anchor only forks

The server opens the PTY as today and keeps the master. It sends the anchor one
request: the `CommandBuilder` (serde, `portable-pty`'s `serde_support`) and the
slave descriptor over `SCM_RIGHTS`. The anchor builds a slave PTY from the
descriptor (`portable_pty::unix::slave_from_fd`, the one vendored addition) and
calls its `spawn_command`, so session setup, controlling terminal, signal reset,
descriptor closing and umask are the library's own code, not a copy. The anchor
answers with the pid or the error text. The server closes its slave copy after the
answer.

Protocol: newline-delimited JSON on a Unix stream socket, one connection per
server. `Hello {version}` both ways first; a different version means no anchor
(D1 fallback). Then `Spawn {id, command}` + one descriptor → `Spawned {id, pid}` or
`Failed {id, error}`, and unsolicited `Exited {pid, code, signal}`.

### D4. Exit statuses go to the server that spawned the pane

The anchor reaps every child (`waitpid` in a dedicated thread) and sends `Exited`
on the connection that spawned it. The server's `Child` for an anchored pane
(`AnchoredChild`: pid, kill by `SIGHUP` as the library's child does, `wait` on a
channel fed by the connection reader) turns it into the `portable_pty::ExitStatus`
the pane watcher already classifies. When the spawning connection is gone (a live
handoff), the exit is dropped: the new server sees those panes as imported and
learns of their end by PTY EOF, exactly as today. When the connection breaks
while the server lives, every waiting `AnchoredChild` returns a wait error, which
the watcher already reports as `WaitFailed`.

### D5. Lifetime

The anchor exits when it has no live children and no connection for 10 seconds,
so a stopped session leaves nothing behind and a starting server has time to
connect. It ignores `SIGHUP` and `SIGINT`; `SIGTERM` ends it.

### D6. Where it lives

`src/platform/macos/pane_anchor.rs` holds the protocol, the anchor loop and the
client. `src/pty/backend/unix.rs` asks `platform::pane_anchor_spawn` first and
falls back on `None`. Other platforms return `None`. `HERDR_PANE_ANCHOR=off`
disables it (tests that count processes, and a way out if it misbehaves).

## Risks / Trade-offs

- **The premise.** If tccd keyed answers by something other than the responsible
  process's life, this would not help. Evidence for the premise is #240's
  measurements; the live check after install is ac answering once and seeing no
  prompt after the next upgrade.
- **A crashed anchor** leaves its panes running (they are not killed with it) but
  attributed to a dead process: the old symptom, until those panes restart. The
  next pane spawn starts a new anchor.
- **First install** still needs one respawn wave: panes started before the anchor
  existed hang off old servers.

## Verification

- Unit: protocol round trip; an in-process anchor loop spawns `/bin/echo` on a PTY,
  the master reads its output, the exit status arrives; a version mismatch falls
  back.
- Integration (macOS): a server starts the anchor; a new pane's shell has the
  anchor as its responsible process (`responsibility_get_pid_responsible_for_pid`);
  after a live handoff the anchor pid is unchanged and a new pane is attributed to
  it too; the anchor exits after the session stops.
- Live: install on mbm5 then m4m, respawn panes once, ac answers one prompt, then
  the next beta's install brings no prompt.
