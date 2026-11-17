# Design

## D1. Spawn through launchd

`spawn_handoff_server` on macOS calls `macos::spawn_launchd_job` first (it writes a run-once job into
`gui/<uid>` and returns its label). The job process runs `leave_launchd_job`, forks the real server into its own
session and exits, exactly as for a client-started server, so the server is not a running GUI job that logout
stops. On error (SSH, a command launchd cannot carry, no GUI domain) it logs at info and uses the existing
`spawn_disclaimed_daemon`, then the plain detached spawn.

`spawn_handoff_server` keeps returning `io::Result<u32>`; 0 means "launchd started it, pid unknown".

## D2. The replacement's pid

The replacement connects to the handoff socket after it has forked out of the job. When the spawn returned 0, the
handoff takes the pid from the accepted connection (`platform::socket_peer_pid`, already used for API peers) and
uses it wherever it used the spawn pid. If the peer pid is unknown it continues; rollback then cannot signal the
replacement, and logs that at warn.

## D3. Rollback

`cleanup_failed_import_child(pid)` waits on and kills its own child today. For a replacement that is not our child
(launchd path), it sends SIGKILL to the peer pid only if that pid's session leader is the pid itself (the forked
server called setsid) and it is not ourselves, and does not wait. With pid 0 it does nothing and warns.

## D4. Environment

The launchd job carries the command's program, arguments, environment changes and working directory
(`job_plist`), so `HERDR_SESSION`, the removed socket overrides and `--handoff-import <socket> <token>` arrive
unchanged. The token stays out of logs (the plist lives in a private 0700 folder that is removed with the job).

## Verifier table

| # | Requirement | Test | Pass |
|---|---|---|---|
| V1 | a handoff replacement on macOS is in its own coalition | live: throwaway herdr whose server is in coalition X, live handoff, `lsappinfo` member lists of a running app's coalition show the new server pid outside X | new server not in the old coalition |
| V2 | an app launched from a pane after the handoff leaves no stale LaunchServices entry | live: run the probe app (UIElement and BackgroundOnly) from a new pane, `lsappinfo` after exit | no entry |
| V3 | handoff still keeps every pane and program | `just core-live-test` handoff step and the existing handoff tests; live: tick loop and pids across the handoff | all kept |
| V4 | the replacement's pid comes from the socket peer when launchd started it | unit: pid 0 from spawn + peer pid N gives N to the manifest/rollback path | N used |
| V5 | rollback never signals a pid that is not the replacement | unit: not our child, peer pid whose session leader differs -> no signal; pid 0 -> no signal, one warn | no signal |
| V6 | SSH or launchd failure falls back to today's spawn | unit/integration: SSH_CONNECTION set -> disclaimed spawn path | old path |
