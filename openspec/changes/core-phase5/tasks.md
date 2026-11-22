## Ground rules

Planned 2026-11-22 (herdr-core, for herdr; https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/300).
Verifier rows V1-V37 are in design.md § Verifier. Each PR is shippable alone, off until chosen (design D8), and
ends in a check.

Heavy cargo on m4m only under the shared lock: `lockf -k /tmp/m4m-heavy.lock nice -n 15 env CARGO_BUILD_JOBS=6
NEXTEST_TEST_THREADS=6 ZIG=/opt/homebrew/opt/zig@0.16/bin/zig <cmd>`. Live tests use scratch `HC_SESSION`s and
scratch XDG directories, a local `--via` loop and a fake `ssh` first on `PATH` (`probe/`), or real OpenSSH to a
loopback-only server (`probe/fix/sshd_fake.py`, stopped after); no test dials a real Mac, runs `hc update` or `hc machine update`, or touches ac's `default` or `herdr` core. A test Claude Code runs
only with `--settings ~/.config/claude/direct-noconnectors.settings.json`.

Order: A0 (alone, first), A, B, C, D, E, then rollout R. C and D are independent after B.

## 0. Before code

- [x] 0.1 Roundtable on design.md § Roundtable questions; findings F1-F12 folded into design.md (§ Roundtable
  findings, probes R1-R10)
- [ ] 0.2 Verifier review by herdr (the lead)
- [ ] 0.3 Gitea sub-issues for PR A0 and A-E (`[fork]` titles), each linking #300
- [ ] 0.4 Security review request for D11 (the shared link, the ten-minute window per F9's answer) to the security
  agent, `--wake`

## PR A0. Local scans ignore machine terminals (F1; first, alone)

- [x] A0.1 `CoreAddress { machine: Option<MachineName>, terminal }` parsed at the boundary from every core answer
  that carries an address; unit V26 part 1
  Built: `core_backend/address.rs` (`CoreAddress::parse`, `local_address(record)`); parsed at the `terminal.list`
  scans, the `terminal.run` answer and the `reattach` target. Limit: `MachineName` is a bare string newtype until
  PR A adds the validated one; `events.subscribe` has no reader in herdr yet, so nothing to filter there.
- [x] A0.2 `find_labelled_terminals`, `unclaimed_in`, the launch-label recovery in `pane_io.rs` and the
  waiting-pane search keep local addresses only; no machine record's pid reaches a local fact or sweep; fake-core
  V26 part 2
  Built: those three scans filter on `local_address`; the waiting-pane and restore searches go through
  `find_labelled_terminals`; `launched_here` is the launch recovery's filter; `reattach` refuses a machine address
  as `NotOurs` before asking core. Tests: FakeCore with `t1` and `loop/t1` (same labels, `loop/t1` pid a live
  `sleep`): the pane search finds `t1` only, the orphan scan lists no machine terminal, no kill, attach or get
  reaches `loop/`, the process lives; unit tests for the launch filter and the refused reattach. Limit: the
  launch recovery is tested through its filter, not a fake core, because the launch label is random.
- [x] A0.3 Loop case of V26 in `scripts/core_machine_live_test.sh` (a scratch machine added with `hc machine add
  --via`, no herdr link command needed)
  Built: the script (new; PR A extends it) and a line in `just core-live-test`. Two scratch cores, a `via.sh`
  relay, `loop/t1` with pane A's labels and a live far pid; a live handoff, a restart and closing pane A leave it
  listed and running. Checked to fail without the fix (the orphan notice names `loop/t1`). Limit: the far pid is
  a live process on this Mac, which is what makes a wrong signal visible; it is not a second host.
- [ ] A0.4 Check: `just check` 0, `just core-live-test` 0, CI green on Linux, macOS and Windows

## PR A. The link command and core machines (no dormancy yet)

- [x] A.1 Read the Tailscale SSH check banner text and where ssh prints it from Tailscale's docs or source (no
  dial); record it in design D10
  Done: D10 § The check banner. An SSH auth banner from the control server's message (`tailssh.go` `clientAuth`,
  `SendAuthBanner`), reported as `# Tailscale SSH requires an additional check.` / `# To authenticate, visit:
  https://login.tailscale.com/a/<code>`; OpenSSH prints auth banners on stderr only at LogLevel INFO or above, so
  the link argv adds `-o LogLevel=INFO`. Limit: the text is Tailscale's to change; a changed one degrades to
  `auth_stalled`, which also closes the gate. Open question 2 answered.
- [x] A.2 `src/core_machine/`: `MachineName`, `target.json` (with the adding terminal's `SSH_AUTH_SOCK`),
  `gate.json` types parsed at the boundary; the state dir from the passwd home (D2); the ssh argv builder reusing
  `attach.rs`'s managed config and non-interactive options, with `ControlMaster=no`, `ControlPath=none`,
  `ServerAliveInterval=5`, `ServerAliveCountMax=2` (D10), and the allowlisted ssh environment
  Built: `core_machine/name.rs` (`MachineName`, now also the machine part of `CoreAddress`), `state.rs`
  (`MachineDir` 0700 and owner-checked, `Target` with `deny_unknown_fields` and validated destination, session and
  absolute paths, `Gate`/`GateClass`, `LeaseHolder` and lease files written then renamed, `dial.json`, `link.log`
  with one 1 MiB rotation), `ssh.rs` (argv, env, guard, line kind). Shared helpers moved out of `attach.rs`:
  `NONINTERACTIVE_SSH_OPTIONS`, `managed_ssh_config_contents`. Platform calls (getpwuid_r, flock, fork, setsid,
  dup2, exec, poll, kill) in `platform/core_link.rs`. Limits: the root is `herdr-dev` for a debug build (as herdr's
  other folders); the ssh options are fixed in code, not stored in `target.json`; a machine may name its own ssh
  config (`--ssh-config`), which then replaces the managed one.
- [x] A.3 `herdr core-link <name>` per D2 steps 1-8 (`probe/fix/corelink.py` is the reference): POSIX line only,
  gate check, `dial.lock`, fork the watcher, exec ssh; the far guard and `HERDR-LINK-UP`; the 15 s deadline;
  failure classes including `far_core_down` (D10); a lease check that, until PR B, treats every machine as leased.
  A unit test pins core's line shape (V35)
  Built: `core_machine/link.rs`, dispatched first in `main` while single-threaded. Order: line kind, gate, lease
  (always in force until B.1), target, `dial.lock` (2 s, kept across exec), managed config, fork, exec ssh with
  core's stdin and stdout and the watcher's pipe on stderr. The watcher drops core's streams and every other
  descriptor, reads ssh's stderr before checking its parent, records the first class only (`needs_check` ends ssh,
  `far_core_down` and `auth_failed` let it exit, `auth_stalled` at 15 s ends it: SIGTERM, SIGKILL after 1 s, only
  while ssh is its parent), and clears the gate when the link comes up. Not in A: step 7 (lease re-read) and step 9
  (retry budget) are tasks B.3 and B.3a. Limits: `auth_failed` is matched on ssh's stderr text, not its exit code
  (the watcher is ssh's child and cannot wait on it); on a far side whose `nc` has no `-U` the guard falls back to a
  socket-file test (D2 step 5), and `nc -U` on Linux images is unmeasured.
- [x] A.4 `herdr machine add --core`, `rm --core`, `login`, `status [--json]` for core machines; refusal without a
  terminal; no `--install` (V23); add and login write the `manual` lease before calling hc (V33)
  Built: `core_machine/command.rs`, routed from `cli/machine.rs` (`--core`; `login` is core-only; Windows says core
  machines need macOS or Linux). Add: needs a terminal (else prints the command), records the absolute `ssh` on
  PATH and the terminal's `SSH_AUTH_SOCK`, writes the lease, runs `hc machine add --name --session --via
  '<herdr> core-link <name>'`; a failed add prints core-link's log and removes the state. Login: lease, then ssh in
  the terminal (no BatchMode, no master), clears the gate, `machine.connect`. Status: core's `machine.list` (never
  starting core) merged with the state folders: state, gate, leases, last dial, whether core's target is herdr's
  link. Limits: the core session flag is `--remote-session` (herdr's global `--session` takes `--session`); status
  is `herdr machine status --core`, so the saved-machine status output is unchanged; the old-hc notice of D9 comes
  with the link-establishment work of PR B/C. V23 is a source scan (`core_machine/tests.rs`).
- [x] A.5 Loop harness `scripts/core_machine_live_test.sh`: two scratch cores, fake `ssh`, the probe's byte log;
  the loopback OpenSSH server for V34 and V36; V8, V23, V27, V28, V31 (core-link part), V34, V35, V36
  Built: `scripts/core_link_live_test.sh`, sourced by `core_machine_live_test.sh` after V26 (V23 is the unit test).
  Fake ssh with modes relay, slow, hang, banner, denied, logging names only and a byte log; machines named per run
  under the passwd-home state root and removed after. Results on m4m: add refused without a terminal; add and
  login leases before their dials (V33 add and login parts); ssh pid = core-link pid; V31 allowlisted names with
  the recorded agent under a foreign HOME/XDG_STATE_HOME/SSH_AUTH_SOCK; V28 busy refusal in 2.05 s, Windows line
  refused, one ssh; V8 three classes, `auth_stalled` 15.1 s, login reopens; V27 through core 15.2 s, kill -9 leaves
  no watcher; V35 stopped and stale far core give `far_core_down`, no far core starts; V34 no control socket, the
  redial authenticates again; V36 unreachable 25.9 s after SIGSTOP, `terminal.list` 3.0 s per call until then.
  Limits: V35's "no dial for 5 min" is shown by an immediate refused dial plus the unit test of the expiry, not a
  5 min wait, and the reopening is by `machine login` (`machine wake` is B.3); V34 ends the link by SIGTERM of ssh
  (the lease end is B.3); the loopback server is paramiko through `uv`.
- [ ] A.6 Check: `just check` 0, `just core-live-test` 0, CI green on Linux, macOS and Windows

## PR B. Leases and dormancy (#260 on core machines)

- [x] B.1 Lease files (`leases/<holder>`, write-then-rename, expiry) and the gate's lease check (D3); V1, V5
  Built: `state.rs` `Lease { holder, expires_at, input_at }`, a file of `<expires_at> <input_at>` (a one-field
  file from PR A takes its mtime as the act); holders are `manual`, `client-<pid>`, `cli-<pid>` only;
  `newest_act` is the newest act behind a lease in force. `link.rs` `admit` reads the line, then the gate, then
  the leases. Tests: V1 unit (no lease, expired leases, the Windows line first) and loop (no lease and an
  expired one exit 75 as `Dormant`, no ssh); V5 unit (two clients and a CLI writer in threads, a reader sees
  whole files only; a dead holder's lease ends by its expiry). Limit: `cli-<pid>` is written from PR E on.
- [x] B.2 The client writes `client-<pid>` from `InputClock`, at most once a minute; V24
  Built: `core_machine/presence.rs`. The client loop stamps each key, text, paste or mouse input
  (`wake_remotes_on_input`, #260's events); `LeaseClock` lets one stamp a minute through a `try_send` to a writer
  thread, which writes the lease (input + 10 min) in every machine folder with a `target.json`, prunes expired
  leases of dead clients, and wakes that machine's dialer. Tests: V24's count as a unit (inputs every 100 ms for
  10 min write 10 leases, a minute apart); loop: a TUI client in a pty, keys every 0.5 s for 8 s after its start
  write nothing more. Limits: V24's `just bench-render-scale` was not run; nothing was added to a render, layout
  or PTY loop (one `Instant` comparison per input batch). The throttle can end a lease up to 59 s before last
  input + 10 min, and a key after a `retry_spent` reopens the gate at the next write, at most a minute later.
  Every herdr-dev or herdr client of the account writes into every machine folder of its build's root, as D3 says.
- [x] B.3 `core-link`'s watcher re-reads leases every 5 s and ends ssh; `herdr machine wake` (lease, then connect,
  reopens `far_core_down`); V2, V3, V7 (answer keys per R3)
  Built: the watcher's lease re-read (`LEASE_RECHECK`), a lease end is no failure; `command.rs` `wake` (`--for`
  90s/30m/2h, 30 min default, 12 h at most; refuses over a login class and names `machine login`; clears
  `far_core_down`). Loop results on m4m: V2 link ended 20.0 s after a 20 s lease began, status `dormant`; V7 a
  core restart while dormant gave only refused POSIX and Windows STARTs, no ssh; V3 (5 min) zero link bytes and
  zero ssh starts after dormancy, refused pairs with the Windows line < 0.1 s after its POSIX line (see B.6 for
  the gate run's numbers); wake from dormant wrote its lease before the dial and connected; a wake reopened
  `far_core_down`. Limit: status reports an unreadable `gate.json` as open (security M8, later).
- [x] B.3a Retry budget (D2 step 9): failures and drops recorded in `gate.json`; one retry on core's own
  initiative, then `retry_spent` until a lease newer than the failure (input, wake, login, `--dial`); V38
  Built: `GateClass::DialFailed` (closes nothing) and `RetrySpent`; `next_gate` reads the gate as recorded when
  the dial ends; a link counts as up 5 s past `HERDR-LINK-UP` (`UP_SETTLE`), so core's add check connection and
  a far `connect` that fails at once are failures. V38 loop: real OpenSSH to a second loopback server on a
  second far core, which drops each connection at its auth request; a client lease renewed every 30 s with its
  old act time; 10 min: 2 ssh starts, 2 refused auths, gate `retry_spent`; one key (its lease, then its wake):
  exactly one more dial, then none for 40 s. Limits: a `Permission denied` refusal is `auth_failed` (D10, V8),
  which waits for `machine login`, so V38's far side drops the connection instead; the two dials came 0.6 s
  apart (core's own backoff), so a prompting far side still asks twice; `--dial` is PR E.
- [x] B.4 The dialer (D3a): one thread per machine per process, `try_send` wake requests, `connect.lock`, 20 s
  timeout, `machine.list` polling when another process dials; V29
  Built: `core_machine/dialer.rs` (`Dialer`, `dial_once`, the `CoreLink` trait over the primary core). A
  connected machine is not dialled; one already `connecting` is waited on, not re-dialled (R4). V29 unit: a
  connect that blocks for the whole dial, 100 keys from two panes and a view wake each post in < 1 ms; a second
  process's dialer (its own `connect.lock` descriptor) with two wakes polls `machine.list` and sends nothing;
  exactly one connect. `machine wake` and `machine login` run `dial_once` on their own thread.
- [x] B.5 Wake: the first input that reaches a server for a machine's pane, or a view server's first client with a
  lease, posts to the dialer; a client started by a person's command writes its lease at start; V4, V33
  Built: a client started by a person (not a reconnect after a handoff, not a re-exec) writes its lease at start;
  each lease write wakes every machine's dialer. Loop: a TUI client started while the machine was dormant (core's
  retries 30 s apart) wrote its lease before the dial and the machine connected within 4 s; V33's wake part.
  Limits: no pane runs on a machine before PR C and no view server exists before PR D, so the server-side wake for
  a machine's pane, V4's repaint and V33's view part come with them; until then a client's input wakes every core
  machine (which its lease already allows within core's next retry).
- [x] B.5a Security review of PR A (security `docs/reviews/2026-11-22-herdr-core-link.md`), M1-M3 before R.2
  M1: the sandbox measurement in `probe/README.md` § M1 (throwaway cores; Codex seatbelt, two `sandbox-exec`
  profiles and nono's `claude-gpt` refuse; Claude Code sessions, nono `default` and Codex with network connect;
  what was not measurable is named there). M2: design D10 § No forwarding; argv unit tests, an `ssh -G` unit
  test, and the loop's loopback server logging forward and env requests under a config that asks for all of
  them. M3: design D10 § Check-free paths only and task R.2.0. M4-M8 are later changes.
- [ ] B.6 Check: `just check` 0, `just core-live-test` 0, CI green

## PR C. A herdr-max pane on a machine

- [ ] C.1 `Host` on `CorePaneIo`; pid facts, sweep and close by host (D5); V12
- [ ] C.2 Spawn with `on`; label `<label>@<core id>`; integration variables stripped and the `env -u` wrapper on
  argv, line and default shell (D5); V9, V10, V31 (pane part)
- [ ] C.3 `CoreWaiting` causes `machine_dormant` and `machine_unreachable`; `machine.list` poll while a lease holds;
  input held and wake on first key; V6, V11
- [ ] C.4 API `on` and capability `spawn_on_machine`; CLI `--on`; pane JSON `machine`, `core_address`; V13
- [ ] C.5 New pane dialog: a machine choice, listing core machines with their gate (UI style per `docs/ui-style.md`,
  help entry if a key is bound)
- [ ] C.6 `docs/next`: running a pane on a machine, what a remote pane lacks (hooks, detached-job sweep)
- [ ] C.7 Check: `just check` 0, `just core-live-test` 0, `just bench-render-scale` 1 and 15 panes reported, CI green

## PR D. The view server

- [ ] D.1 `ServerRole` and `herdr server --view <machine>`; the role's boundaries per D6's table, each a match on the
  role
- [ ] D.2 Document parser with limits; layout projection; `doc` and `missed` handling; V15, V16
- [ ] D.3 The size policy of D6 (`resumed` attach, AUTOMATIC_INPUT only, no DATA or FOCUS, RESIZE only for a changed
  far size from `size` events, read-only fallback); cropped or padded drawing with the remote size in the border;
  V14, V30 (before merge)
- [ ] D.4 `ClientEndpointId::Core`, endpoint transport that starts the view server; `herdr --remote core:<m>`;
  exit 10 min after the last client; V17
- [ ] D.5 Both-present notice and `[remote] prefer`; V21, V22
- [ ] D.6 Check: `just check` 0, `just core-live-test` 0, CI green

## PR E. Actions and the fleet CLI over the link

- [ ] E.1 Remote herdr path and capability probe per link establishment (D7)
- [ ] E.2 `--machine` through `machine.exec` with the `cli-<pid>` lease and the `env -u` wrapper; dormant refusal
  (exit 75) and `--dial` through the dialer (D7); `truncated` and `timed_out` as errors, input over 8 MiB refused;
  no retry after send; V18, V19, V31 (exec part), V32, V37
- [ ] E.3 View actions (new tab, split, close pane, rename, agent start) as argv `machine.exec` calls; V20
- [ ] E.4 `docs/next`: core machines, dormancy, `machine status` gates, the announcement rule (S4a, S4b:
  `--dial` and `wake` are announced dials), that agents' own `hc --on` on a dormant machine fails, the fallback
- [ ] E.5 Check: `just check` 0, `just core-live-test` 0, CI green

## R. Rollout (both Macs in one step)

- [ ] R.1 Open question 1: ac-mbm5's herdr publishes its spaces to its `default` core (ask on the desk if a config
  change is needed there)
- [ ] R.2.0 Precondition (security review M3, design D10): for each target, record its ssh path and whether that
  path has a Tailscale SSH `check` rule. Only a check-free path (an `accept` rule for ac to ac between ac's own
  devices, or OpenSSH over the tailnet) is added as a core machine; a target with a check rule gets no
  background dials (login or an announced wake only). The M1 sandbox results (`probe/README.md` § M1) go into
  the announcement: which sandboxes reach the primary core socket, and so the link
- [ ] R.2 Announcement S1 through the gestore desk: "herdr-core needs ssh ac@ac-mbm5 from m4m's Herdr Core now, to
  add ac-mbm5 as a core machine; later dials happen when you use the view or a pane there after 10 min idle, each a
  fresh ssh login (1Password may ask for Touch ID), and never start Herdr Core on ac-mbm5". Wait for ac's
  acknowledgement
- [ ] R.3 Beta with A-E on m4m and ac-mbm5 by live handoff, panes kept; then ac runs `herdr machine add --core
  ac-mbm5` on m4m himself (S1); record the 1Password prompt's app name (open question 3)
- [ ] R.4 V25: one night with the view attached; `pmset -g log` on ac-mbm5 read by mac-m5; results on #300
- [ ] R.5 Close #300 after V25 passes in a running beta, with the check described
