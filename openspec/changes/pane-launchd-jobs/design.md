# Design

## D1. The keeper

`herdr __pane-job --socket <path> --id <n> --token <t>` is submitted per pane with `launchd_spawn` (a run-once job in
gui/<uid>, label `dev.herdr.pane.<server-pid>.<id>.<nanos>`), but unlike the server it does NOT fork out
(`leave_launchd_job` is not called): the keeper stays the job's process.

1. It connects to the server's pane-job socket (per session, mode 0600, next to the API socket), sends `Hello{id, token}`.
2. It receives the command and the PTY slave fd over SCM_RIGHTS (the pane anchor's `Request::Spawn` codec and
   `recv_with_fds`, reused).
3. It starts the program with `posix_spawn` and `responsibility_spawnattrs_setdisclaim`, as
   `src/platform/macos/disclaimed_spawn.rs` does for the server, so the program is its own responsible process
   (resp(program) == program). The PTY setup is done by the spawn attributes: a new session, the slave as fds
   0/1/2 and controlling terminal, default signal dispositions, every other descriptor closed. A plain fork from
   the keeper would make the program responsible to the keeper, whose executable (the herdr-beta clone) changes
   path with every beta, so privacy answers would not stick.
4. It reports `Spawned{pid}`, then waits, then reports `Exited{status}`. If the server connection dropped (a live
   handoff), it reconnects to the socket path (the importer re-binds it) and re-sends `Exited` until acknowledged or
   10 s pass, then exits with the program's status.
5. It ignores SIGHUP and SIGINT so terminal signals reach only the program; it forwards SIGTERM to the program.

The keeper is a few hundred lines in `src/platform/macos/pane_job.rs`; it runs before logging and threads start,
like the anchor.

## D2. The server side

Off by default: `[experimental] pane_jobs = true` turns the job backend on for new panes (applied on config load
and reload, like `[experimental] runtime`). With it off, panes use the anchor or a direct spawn as today.

A new `PaneIo` backend on macOS (`Actor` variant kept for the PTY master; the child handle is a `JobChild`): spawn =
submit job + wait for `Spawned{pid}` (timeout 10 s, then fall back to the anchor/direct spawn and log once). `kill`
signals the pid (process-session check as in M2). `wait` resolves on `Exited` or, if the keeper vanished, once the
program's recorded session is gone: `platform::process_session(pid)` no longer equals the session recorded at spawn,
the check `sweep_targets`/`signal_session` in `src/pane/runtime_io/pty.rs` use. PTY EOF plus
`process_exists(pid) == false` is not used: a reused pid would keep a stranger's process looking like the pane's
(security review M2). Handoff passes the pane's job label, pid and PTY master as today's panes; the
importer serves the same socket path so keepers reconnect.

Stale finished jobs are removed with launchd_spawn's existing `remove_stale_jobs` (label prefix `dev.herdr.pane.`).

The pane-job socket is mode 0600 and each keeper presents its token. The server also checks that the connecting
peer's uid equals its own (`platform::unix_stream_peer_uid`, PR 50) and drops any other connection unanswered.

**Rollback.** A live handoff to an older beta without the job backend must keep job panes working. The importer
holds the PTY master fd as for any pane and ignores the job fields of the handoff state. The keeper loses its
server, keeps waiting for its program, and after the exit retries the socket for 10 s and exits. The older server
sees the pane end by PTY EOF, as every imported pane does.

## D3. Suspended panes

A pane whose program ended without an `Exited` from its keeper, while the keeper's job is gone, is marked
`Suspended` (the keeper died: logout, or a stop of that pane's coalition). It stays in the layout with its saved
resume plan and shows a "suspended" state; it is not closed and its workspace stays. Resume:

- **The server survives logout** (to be measured): on the next client attach, or when `gui/<uid>` accepts jobs again,
  each suspended pane respawns in place; an agent pane runs its resume plan (as session restore does today).
- **The server ends at logout**: session.json already holds the layout and resume plans; the next start restores.

A burst of keeper deaths (>= 3 within 2 s) is logged once at warn with the count (logout looks like this).

The suspended state is a shared runtime fact, so clients learn it through the stable client endpoint contract. It
is added as an optional field or a new enum value with an `Unknown` fallback that older clients ignore, never by
changing a frozen generation-1 codec or fixture. A client that does not know the state shows the pane as it shows
any pane without one.

## D4. TCC responsibility

A launchd-started keeper is its own responsible process, and so is the program it starts, because the keeper
disclaims responsibility when it spawns it (D1 step 3). Privacy answers then belong
to the pane's program and outlive server upgrades, which is what the anchor (#246) gave; the anchor stays for the
fallback path only.

V1-V3 hold for panes started with the backend on; panes that existed before stay on the anchor until respawned.

## Verifier table

| # | Requirement | Test | Pass |
|---|---|---|---|
| V1 | a new pane's program is in its own coalition, not the server's | live, throwaway herdr: `lsappinfo` coalition lists after a probe app launched from the pane | pane coalition contains no server or other-pane pid |
| V2 | no stoppable entry after an app launched from a pane exits | live: Probe.app (UIElement) and ProbeBG.app from a pane, `lsappinfo` after exit | no entry |
| V3 | stopping one pane's coalition ends only that pane | live: `kill` the keeper's job (`launchctl remove <label>`) of pane A | pane A suspended, pane B and the server untouched |
| V4 | panes work: input, resize, exit status, clipboard, `osascript`, `open` | live + existing pane tests on the job backend | all pass |
| V5 | a live handoff keeps job panes (pids, output) and keepers re-report exits to the importer | live: tick loop across `live-handoff`; exit a program after the handoff | kept; exit status arrives |
| V6 | a suspended pane is kept and resumes | unit/fake: keeper gone without Exited -> Suspended, layout kept; live: `launchctl remove` a Claude pane's job, reattach -> resumed by its plan | kept and resumed |
| V7 | logout then login: panes come back by resume | live, one real logout on m4m (asks row a1114-14, reworded) | every agent pane resumed |
| V8 | SSH / no GUI domain falls back to the anchor or direct spawn | unit: SSH_CONNECTION set -> anchor path | old path |
| V9 | cost: spawn latency and CPU stay reasonable | live: 30 panes spawn time and V12 busy profile vs today | spawn < 300 ms p50, busy CPU within 1.1x |
| V10 | a job pane's program is responsible for itself | live: `~/.local/state/herdr-resp/resp <program pid>`; unit: responsibility lookup on a program the keeper started | resp(pid) == pid, not the keeper or the server |
| V11 | the backend is off unless `[experimental] pane_jobs = true` | unit: config default and gate; live: a throwaway with it unset starts panes through the anchor | off by default |
| V12 | a handoff to an older beta keeps job panes | live, once: `live-handoff --import-exe` an older beta; the pane keeps output, then exits | kept; the pane ends by PTY EOF |
| V13 | only our uid reaches the pane-job socket | unit: the peer-uid gate | another uid is dropped |
| V14 | a suspended pane reaches old clients safely | unit: frozen generation-1 fixtures unchanged; an old client decodes a snapshot with the new state | decodes, shows the pane |
