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

**Revised (phase 1 build, 2026-11-17).**

- The command line is `herdr __pane-job --socket <path> --label <label>`. The token travels in the job's environment
  (`HERDR_PANE_JOB_TOKEN`), because other users can read a process's arguments but not its environment. The label is
  the key the keeper presents: a per-server `--id` would collide across a live handoff.
- Step 5 forwards SIGHUP, not SIGTERM. Interactive shells ignore SIGTERM (bash(1): "in the absence of any traps, it
  ignores SIGTERM"), so a forwarded SIGTERM would leave the pane's shell running. SIGHUP is what a closed terminal
  sends. Live: `launchctl remove` of a pane's job ended its bash with `Hangup: 1`.
- The PTY setup in step 3 is done by spawn attributes: `POSIX_SPAWN_SETSID`, then opening the slave's path as fd 0
  (macOS runs the file actions after the new session exists, so the open makes it the controlling terminal), dup to
  1 and 2, `POSIX_SPAWN_CLOEXEC_DEFAULT`, default dispositions for the signals the PTY library resets. The resolved
  program, argv, environment and working directory come from the vendored `CommandBuilder::unix_spawn_parts`
  (portable-pty patch 0006), from which `as_command` is now built too.
- The exit report carries `stopped` (the keeper was told to stop). A stopped job and a lost keeper end the pane as
  interrupted, so the session is checkpointed; D3 keys its suspended state on them.

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

**Revised (phase 1 build, 2026-11-17).**

- Start timeout: 10 s before the server serves (restored panes, as the anchor waits), 3 s after, because a spawn
  holds the app loop (fork issue 249). After a timeout the job is removed and new panes skip the backend for 60 s.
  Measured on m4m: a keeper connects and its program starts 42-74 ms after the submit.
- The dead-keeper check compares the program's session and its process start time with the values recorded at
  spawn. A session check alone passes a reused pid that leads a session of its own.
- Handoff: the job travels in the handoff state as `pane_job {label, token}`. The importer re-binds the socket when
  it adopts a job pane, and when the pane's PTY closes it waits up to 3 s for the keeper's report. Without one the
  pane ends as `Handoff`, as before.

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

**Revised (task 2.1 build, 2026-11-17).**

- The keeper's report counts a stopped job first: `Exited{stopped}` reads as `pane job stopped` even when the program
  died of the hangup the keeper sent it (it usually does: live, bash ended with `Hangup: 1`). Before, that status
  read as the program's own signal and closed the pane.
- Both keeper statuses classify as `ChildExitReason::Suspended` (Unix only, like `Handoff`). Only job panes report
  them, so with `pane_jobs` off nothing changes; a job pane started before `pane_jobs` was turned off still suspends.
- On a suspended exit the server saves the session first (the checkpoint an interrupted exit takes, with the pane's
  history), keeps the pane's last directory, drops its runtime and marks its terminal `suspended`. The agent is not
  released, by the app or by the server's own exit forwarding (which releases every exited pane's agent first, and
  so dropped the plan in the first live run), so its session and resume plan stay on record. A detected agent reads
  as idle, with no "done" mark for the cut-off work; a hook-reported state stays as reported until the pane resumes.
  Popup panes, overlay panes and panes a worktree removal is stopping close as before.
- The pane shows a notice in place, drawn by the server, so every client shows it. `pane get`/`pane list` carry
  `suspended: true`, and endpoint clients get the optional `pane_process` resource fact (`suspended`, else `Unknown`).
- Resume: 5 s after the last keeper ended (30 s after a burst, so panes do not restart into a logout), or at once
  on a client attach, the server starts every suspended pane again if launchd accepts pane jobs: pane jobs off or
  unavailable, or not in the 60 s fallback window, and `launchctl print gui/<uid>` answers. Else it looks again
  every 10 s. A pane with a resume plan runs it (as `pane respawn --resume`), any other re-runs its launch command
  or a shell. A pane that cannot start shows why and stops waiting.
- The burst warning is logged when the third keeper ends within 2 s of the previous one, with that count; the
  resume logs how many panes started again.
- Not kept across a restart: `suspended` is not saved. A restore or live handoff starts every pane anyway, the
  suspended ones by their saved plans. A suspended pane has no history in later saves, only in the checkpoint.

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

## Security review (2026-11-17)

Security's two-vendor review (`security/docs/reviews/2026-11-17-herdr-pane-launchd-jobs.md`) found no P0 or P1;
H1 is the handoff rollback (see `handoff-own-coalition`). What changed here:

- **H2, signal forwarding after the reap.** The keeper sees its program's exit with `waitid(WNOWAIT)`, clears the
  pid its SIGTERM handler forwards to, and only then reaps. The keeper is single-threaded, so the handler runs on
  the thread that clears the pid and needs no signal mask. Unit: `forwarding_is_disarmed_before_the_program_is_reaped`.
- **H3, a failed handshake.** D1 gains a step: the server answers `Spawned` with `Ack`. A keeper with no `Ack`
  (the server timed out and hung up) kills its program's session group and exits, and the server removes the job
  on every failure after the keeper connected. Unit: `a_keeper_ends_a_program_no_server_took` (a program that
  ignores SIGHUP, no `Ack`).
- **H4, environment removals.** Does not apply to keepers: a pane program's environment is the complete one the
  server sends over the socket, not the job's. The server's own launchd start is fixed in `handoff-own-coalition`.
- **H6, descriptors.** Received descriptors are marked `FD_CLOEXEC` at once, a transfer with `MSG_CTRUNC` is refused
  with what arrived of it closed, and a keeper given anything but one descriptor reports `Failed`. The pane anchor
  shares `recv_with_fds`. Unit: `a_keeper_refuses_anything_but_one_descriptor` (0, 2 and 9),
  `received_descriptors_close_on_exec_and_a_cut_transfer_is_refused`.
- **H7, the socket's mode.** The pane-job socket is bound under umask 077 (`bind_owner_only`, which the anchor now
  shares), so it is owner-only from the moment it exists; it is 0700, not chmodded to 0600. The plist folder is
  unchanged (mkdtemp 0700 under the user's private `TMPDIR`). Unit: `the_pane_job_socket_is_owner_only_when_bound`.
