# Design

## D1. The keeper

`herdr __pane-job --socket <path> --id <n> --token <t>` is submitted per pane with `launchd_spawn` (a run-once job in
gui/<uid>, label `dev.herdr.pane.<server-pid>.<id>.<nanos>`), but unlike the server it does NOT fork out
(`leave_launchd_job` is not called): the keeper stays the job's process.

1. It connects to the server's pane-job socket (per session, mode 0600, next to the API socket), sends `Hello{id, token}`.
2. It receives the command and the PTY slave fd over SCM_RIGHTS (the pane anchor's `Request::Spawn` codec and
   `recv_with_fds`, reused).
3. It forks; the child does the PTY library's setup (setsid, TIOCSCTTY, dup2 0/1/2, close the rest) and execs the program.
4. It reports `Spawned{pid}`, then waits, then reports `Exited{status}`. If the server connection dropped (a live
   handoff), it reconnects to the socket path (the importer re-binds it) and re-sends `Exited` until acknowledged or
   10 s pass, then exits with the program's status.
5. It ignores SIGHUP and SIGINT so terminal signals reach only the program; it forwards SIGTERM to the program.

The keeper is a few hundred lines in `src/platform/macos/pane_job.rs`; it runs before logging and threads start,
like the anchor.

## D2. The server side

A new `PaneIo` backend on macOS (`Actor` variant kept for the PTY master; the child handle is a `JobChild`): spawn =
submit job + wait for `Spawned{pid}` (timeout 10 s, then fall back to the anchor/direct spawn and log once). `kill`
signals the pid (process-session check as in M2). `wait` resolves on `Exited` or, if the keeper vanished, on PTY EOF
plus `process_exists(pid) == false`. Handoff passes the pane's job label, pid and PTY master as today's panes; the
importer serves the same socket path so keepers reconnect.

Stale finished jobs are removed with launchd_spawn's existing `remove_stale_jobs` (label prefix `dev.herdr.pane.`).

## D3. Suspended panes

A pane whose program ended without an `Exited` from its keeper, while the keeper's job is gone, is marked
`Suspended` (the keeper died: logout, or a stop of that pane's coalition). It stays in the layout with its saved
resume plan and shows a "suspended" state; it is not closed and its workspace stays. Resume:

- **The server survives logout** (to be measured): on the next client attach, or when `gui/<uid>` accepts jobs again,
  each suspended pane respawns in place; an agent pane runs its resume plan (as session restore does today).
- **The server ends at logout**: session.json already holds the layout and resume plans; the next start restores.

A burst of keeper deaths (>= 3 within 2 s) is logged once at warn with the count (logout looks like this).

## D4. TCC responsibility

A launchd-started keeper is its own responsible process, and so is the program it starts. Privacy answers then belong
to the pane's program and outlive server upgrades, which is what the anchor (#246) gave; the anchor stays for the
fallback path only.

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
