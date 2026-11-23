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
  Local, m4m, 2026-11-22, under the shared lock: `just check` 0 (5754 + 158 tests). `just core-live-test`'s
  steps 0, run with the built binaries copied out of the shared target dir first (another session rebuilt that
  dir's `debug/herdr` from its own source mid-run): V3 300 s dormant, 0 link bytes, 0 ssh starts, 13 refused
  pairs, Windows line at most 0.077 s after its POSIX line, pairs after the cap at least 30.06 s apart; V38
  600 s, 2 ssh starts, 2 refused auths, `retry_spent`, one key and its wake gave exactly one more dial; V2
  24.7 s; the client connected the dormant machine 0.67 s after it started. The live scripts now take their
  herdr from `CARGO_TARGET_DIR` when set. CI not run (not pushed).

## PR C. A herdr-max pane on a machine

- [x] C.1 `Host` on `CorePaneIo`; pid facts, sweep and close by host (D5); V12
  Built: `pane_io.rs` `Host { Local, Machine(MachineName) }`, taken from the parsed address at attach. For a
  machine the pid is not published (child pid 0, no `ProgramIdentity`, no pinned terminal), so pid facts,
  process detection and the OSC 7501 status write give nothing for it; `end_program` is `terminal.kill` only (no
  `server.status`, no sweep). A machine's program ends the pane only when core reports its record finished, and
  then nothing is killed; a suspend cause makes it wait for its machine (C.8), since resuming would start it on
  this Mac. Tests: fake-core units
  (owner label and no pid at start, kill-only close, reattach by address); loop V12: closing the pane ended its
  far terminal only; the far core, another far terminal and a `nohup` job the program left run on. Limit: close
  while the machine is away gets `unavailable` from core and leaves the far program running; it then shows in
  the orphan scan once the machine is back.
- [x] C.2 Spawn with `on`; label `<label>@<core id>`; integration variables stripped and the `env -u` wrapper on
  argv, line and default shell (D5); V9, V10, V31 (pane part)
  Built: `core_backend/machine_spawn.rs` (`MachineSpawn`, `MachineProgram { Shell, Line, Argv }`,
  `/usr/bin/env -u ... --` on every form, the shell as `/bin/sh -c 'exec "${SHELL:-/bin/sh}" -l'`);
  `PaneLaunchEnv`/`LaunchExtra` carry the `MachineTarget` through the workspace and tab constructors;
  `PaneIo::spawn_on_machine` never falls back to the PTY. Only `TERM`, `COLORTERM`, `TERM_PROGRAM(_VERSION)` and
  the request's own `env` (less the six names) go; no folder of this Mac goes, only a `--cwd` the request names.
  The core id comes from `hello.core.id`. Loop V9 (far core started with the six fake variables): label
  `herdrdev-v26@<id>`, argv wrapped, the far shell listed no `HERDR_*`, started in `--cwd`, no process facts or
  `foreground_cwd`; V31 a far `herdr pane report-agent` failed and reached no pane; V10 a far herdr server with
  the same session label and core session name left the pane's terminal alone. Limits: `agent.start` takes no
  `on` (it types into an existing pane, and a machine pane has no local shell facts, so it is refused there);
  the far program's `PATH` is the far core's.
- [x] C.3 `CoreWaiting` causes `machine_dormant` and `machine_unreachable`; `machine.list` poll while a lease holds;
  input held and wake on first key; V6, V11
  Built: `CoreWaitReason::MachineAway { machine, dormant }`; restore and live handoff never search this Mac's
  terminals or start fresh for a machine link (`machine_ended`, link held, only when core reports it finished;
  `machine_terminal_missing` while core has the machine connected but not its terminal; C.8); the saved
  folder check is skipped for them. `app/core_machine_panes.rs` polls every 5 s while a pane runs on or waits for
  a machine, on its own thread, reading `machine.list` only when a lease is in force or herdr does not gate the
  machine; it marks `core_machine::panes::MachineLink` and attaches waiting panes. Input to a live pane whose
  machine is not connected is refused and posts a wake (at most one per 5 s) to the server's dialer
  (`connect.lock`, only for a machine with a herdr `target.json`). The orphan scan lists the owner label's
  machine terminals. Loop: V6 unreachable (restart and live handoff wait, no far program started, same far pid
  after); V6 dormant (gated folder, no lease: waits as `machine_dormant`, the poll reads nothing for 7 s with the
  machine connected, a lease lets it attach); V4 server part (core's retries at their cap, a key was not sent and
  the server's wake connected the machine within 1 s); V11 (see C.7 for the 15 min run). Limits: V4's repaint
  within 2 s through a TUI client and the view part of V33 need PR D; a key to a pane that waits (no runtime)
  wakes through the client's own lease and dialer (B.2), then the poll attaches it within 5 s; the first key
  after a reconnect can still find the machine marked away until the next poll.
- [x] C.4 API `on` and capability `spawn_on_machine`; CLI `--on`; pane JSON `machine`, `core_address`; V13
  Built: optional `on` on `pane.split`, `tab.create`, `workspace.create`; `ServerCapabilities.spawn_on_machine`
  (absent when false, true on Unix servers); the CLI refuses `--on` before sending when the server does not
  advertise it (exit 1, no pane). Pane JSON: optional `machine`, `core_address` (every core pane) and
  `core_waiting_cause`. The three new request shapes are frozen in the endpoint shape test beside `pane.close`'s
  `force`; `tests/fixtures/endpoint-method-shapes-v1.json` is unchanged. V13 unit: refused without the
  capability (and for an old server's capabilities), sent with it. Limit: the client shell sends no `on` (C.5).
- [ ] C.5 New pane dialog: a machine choice, listing core machines with their gate (UI style per `docs/ui-style.md`,
  help entry if a key is bound)
  Not built. The client shell has no new-pane dialog (a split is a key or a context-menu item), and a safe machine
  choice there needs two things the client-shell endpoint does not carry: whether the server takes `on` (the
  endpoint advertises method names, not `spawn_on_machine`; an older server would start the pane locally), and the
  core machines with their gates (a server-side listing). Both are endpoint additions to make with PR D's
  `ClientEndpointId::Core` work; until then panes go on a machine through the CLI and API.
- [x] C.6 `docs/next`: running a pane on a machine, what a remote pane lacks (hooks, detached-job sweep)
  Done: `connecting-machines.mdx` § Panes on a Herdr Core machine (`--on`, the JSON fields, waiting and wake, no
  hooks or integration variables, no process facts or `agent start`, close ends the program only);
  `cli-reference.mdx` gains `--on` on the three create commands.
- [ ] C.7 Check: `just check` 0, `just core-live-test` 0, `just bench-render-scale` 1 and 15 panes reported, CI green
  Local, m4m, 2026-11-22, under the shared lock, binaries copied out of the shared target dir before each live
  step: `just check` 0 on the head (5778 + 158 tests). `just core-live-test`'s steps 0: steps 1-3 (16 core
  backend live tests, `core_live_test.sh`) in one run; step 4 failed twice on races in PR A/B's loop cases, each
  fixed in its own commit (`machine rm --core` removing the folder while the link winds down; V35 against core's
  own retry), then steps 4-6 passed on the same code: PR C's V9, V10, V12, V6 (unreachable and dormant), V4's
  server part (the wake connected the machine within 1 s), V11 with the full 900 s drop (same far pid after),
  and A/B's cases (V3 300 s, 0 link bytes; V38 600 s, 2 dials). `just bench-render-scale`: active panes server
  surface median 254 us at 1 pane, 303 us at 15 (1.19x); background workspaces 254 us at 1 and 15 (1.00x);
  nothing was added to a render, layout, PTY or detection loop (the input path reads one atomic for a machine
  pane). CI not run (not pushed).
  After C.8's second round, head 493026a0, same lock and copied binary: `just check` 0 (5804 + 158 tests);
  the 16 core backend live tests (`HERDR_TEST_HC=1`) passed; `core_machine_live_test.sh` through its V26 and
  PR C parts (link and lease parts not run) passed: V26, V9, V31 (pane part), V10, V12, V6 unreachable and
  dormant, V4's server part (wake within 1 s), V11 with a 120 s drop (same far pid after). Not run this round:
  `core_live_test.sh`, the default-session script, the PR A/B parts, the bench (no render, layout, PTY or
  detection work added) and CI. Still open for the tick: CI green, and a full `just core-live-test`.
- [x] C.8 Review findings on PR C (independent read-only review, 2026-11-22): W1, C1-C6, C8-C10 fixed, C7 left as
  a limit; a second read (2026-11-23) found N1-N4 and C10's rest, all fixed (below); a third round (2026-11-23,
  the downgrade check live on beta 212) moved a machine pane's saved link out of `core_terminal` (N3); C5's check
  is replaced
  W1: `LaunchExtra::from` builds a literal per target (Windows has no `on`, so the rest pattern was a needless
  update under `-D warnings`). C1: a respawn starts on this Mac, so `pane.respawn` and `pane.respawn_agent`
  refuse a pane on a machine (live, waiting or ended) with `pane_on_machine`, and `prepare_respawn` and
  `respawn_pane_agent` refuse it for every other path (the exit-action shell, the resume queue, the move);
  carrying the `MachineTarget` through a respawn was not taken: it needs the far program's own end first, which
  pid 0 cannot confirm. C2: an ended pane keeps its link as `CoreWaitReason::MachineEnded` (`machine_ended`), so
  every save writes its link (`core_machine_terminal` since C.8's third round, N3) and a restore asks core about
  that terminal again and starts nothing. C3: `pane.split`, `tab.create` and `workspace.create` with `on` arm
  the machine poll. C4: a probe carries `read`;
  one whose `machine.list` failed changes neither the machine's mark nor its panes. C5: a `not_found`, a
  non-live record or a label mismatch for a machine's terminal is final only while core lists the machine
  `connected` before and after the question (restore, the waiting attach and a live pane's reattach loop);
  otherwise the pane waits. C6: a suspend cause on a machine pane's stream goes up as it is, and the server
  makes the pane wait for its machine (`MachineAway`) with its link instead of suspending it. C8: a key posts
  the machine to one wake thread (`try_send`); the passwd lookup, folder checks and the dialer's thread start
  there. C9: a wake reports the state it found and the lease read after the dial, so a dial refused for no
  lease stays `machine_dormant`. C10: the machine is marked connected after `terminal.attach` answers.
  Herdr Core facts behind C5 (hc 0.1.21 `hc schema` from a throwaway session, its strings, and P5): `CoreInfo.id`
  is a "permanent random id, reset only when state is wiped" (kept in `core.json`; a disk copied from another
  machine gets a new identity); `terminal.get` errors are `not_found` and `machine_unknown` only, and core drops
  a machine's terminals from its lists while the machine is not connected (P5), so a known machine that is not
  linked yet answers `not_found`; `machine.list` states are `connecting`, `connected`, `needs_login` and
  `unreachable`. A wiped primary (new id) makes every far label a mismatch: with the machine connected those
  panes end (`machine_ended`, never started) and their programs show as orphans there.
  Limits: C7, far calls on the server loop: attaching a waiting machine pane runs `machine.list`,
  `terminal.get`, `machine.list` and `terminal.attach` on the loop, each up to 5 s, per waiting pane per poll;
  moving them off the loop joins https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/302. The core
  label follower (`herdr.session` for seen and turn tracking) does not see machine panes, whose label carries
  `@<core id>`. The orphan notice's `hc ls --label herdr.session=<label>` command does not find machine orphans
  (their label is `<label>@<core id>`). A live pane whose machine stays away keeps its reattach loop and shows
  the "Herdr Core is not answering" line after the limit, not the machine wait notice. Between core listing the
  machine `connected` and its terminals arriving, an attach can still see `not_found`; the before-and-after
  check covers a link that changes during the question, not a lag inside one `connected` state (superseded by
  N2 below: a `not_found` is no longer final at all).
  Second read of the fixed head (2026-11-23): N1-N4, C10's rest and the address parser fixed, one more found
  (a server stop). N1: `wait_for_machine_after_exit` keeps the core terminal before the runtime's shutdown,
  so a suspend cause no longer sends `terminal.kill` to the program it then waits for. N2 (replaces C5's
  before-and-after check): for a pane on a machine only positive evidence is final, a `terminal.get` record
  with the pane's labels in state `finished`. A `not_found`, a record labelled for another pane or in another
  state is `ReattachError::Missing`: the pane waits as `machine_terminal_missing`
  (`CoreWaitReason::MachineMissing`) while core lists the machine connected, as `machine_unreachable` while it
  does not, and every poll asks again; the live reattach loop retries on anything but a live or finished
  record; a stream CLOSE ends a machine pane only with a finished record or a suspend cause. Whatever ends a
  machine pane's stream sets keep-on-drop, so no automatic end sends `terminal.kill`; only a person's close of
  a pane with a runtime does. N3: a saved machine pane has `cwd` `/dev/null/herdr-pane-on-a-core-machine`, and
  its link is saved only as `core_machine_terminal` (`{session, address, machine_cwd}`: the same
  `PaneCoreTerminalSnapshot` with no pid fields, which are the machine's), never as `core_terminal`, so the
  previous beta (beta 212, origin/master 1087d68d) sees no link at all: a plain "Saved directory is
  unavailable" pane, with no hold, no probe, no label search and no attach. Found live on beta 212
  (2026-11-23, /tmp/dg212-variantB.log): with the link in `core_terminal` that build held the pane as
  `DirectoryUnavailable`, got `NotOurs` for `ac-mbm5/t7`, ran `find_pane_core_terminals(session,
  public_pane_id)` and attached a terminal of this Mac labelled with this herdr session and the pane's id.
  `PaneSnapshot::core_link` is the one reader (restore, the live handoff's restore and `program_cwd`): it takes
  `core_terminal`, else a `core_machine_terminal` whose address names a machine; one that names none is no link,
  never a terminal of this Mac. A pane on this Mac saves byte for byte what it saved before. Tests:
  `a_machine_link_is_saved_apart_from_a_local_one`, `a_saved_machine_pane_restores_waiting_for_its_machine`
  (twice through restore and save, a labelled terminal of this Mac present, no `terminal.list`),
  `a_saved_machine_pane_is_never_started_by_the_previous_beta` (beta 212's restore decision on the new save,
  parsed with that build's field set: unavailable, no hold; the link in `core_terminal` was held and searched
  for; the real folder started the pane fresh) and `a_machine_link_without_a_machine_is_no_link`, each checked
  failing with its fix reverted. Live (2026-11-23, a copy of the installed beta 212, throwaway herdr session
  `dg300x` and core session `herdr-dg300x`, scratch XDG folders, a local terminal labelled `herdr.pane_id=w1:p1`,
  saves written by this build's `capture`; log /tmp/dg300-live-212.log): with the link moved back to
  `core_terminal` (control) 212 attached the terminal within 10 s (resized 200x120 to 119x40) and ended it at
  its stop; with `core_machine_terminal` it showed the pane "Saved directory is unavailable", and the terminal
  kept its pid and size, never attached, and ran on after the stop. N4: `move_facts` gives `on_machine` before
  any other refusal and `move_pane` refuses
  before any kill; `prepare_resume` refuses a machine pane, which waits for its machine with its suspension's
  link. C10: a machine's first link starts not connected. `CoreTerminalLink::machine()` parses through
  `CoreAddress`. Found on the way: every stop but a restart ended all core programs, so `herdr server stop`
  or a logout killed programs on machines; `keep_core_programs_for_stop` keeps those on a machine (the saved
  link attaches them again).
  Herdr Core facts behind N2 (hc 0.1.21 `hc schema` from a throwaway `HC_SESSION`, its core stopped after,
  no machine dialled): `Terminal.state` is "`live` or `finished`"; `reason` is one of `exited`, `killed`,
  `server_stop`, `host_shutdown`, `signaled`, `holder_lost`, `stopped_by_restart` (and `spawn_pending` while
  its spawn may still run); a finished record carries `finished_ms`, `exit_code` or `signal`; `terminal.get`
  and `terminal.kill` errors are `not_found` and `machine_unknown`; `terminal.attach` adds `finished`,
  `unavailable` and `needs_framed`; a stream CLOSE carries the finished record (or nothing for a drop).
  Limits: a machine pane that core never lists again waits forever (a person closes it, which sends nothing);
  the live reattach loop checks state, not labels, so a far core whose state was wiped and reused the address
  would be attached (fixed in C.9); after a downgrade to a build before PR C the old build drops the pane's link
  (`core_machine_terminal`, a field it does not know) from its next save (rule R.0); a far program whose pane a
  stop kept, and whose next server never comes, runs on as an orphan there.
- [ ] C.9 (PR C2, https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/308) Security review M1-M8 of PR C
  (ac/security 050caaa, `docs/reviews/2026-11-22-herdr-machine-panes.md`) and the third read's findings 1-7.
  Built, each with a test that fails with its fix reverted (all 19 checked in one reverted build):
  M1, output from a machine pane is far data: `PaneLaunch::far_output()` (a `Command` with `machine`, a
  `ReattachCore` whose link names a machine) gives the terminal `KittyMedia::InlineOnly`
  (`enable_kitty_graphics_with`, ghostty-vt: file and shared-memory off, no temp-file folder, on both screens;
  t=f, t=t and t=s load nothing and the qualifying temp file stays, t=d draws); `publish_output_requests`
  drops its OSC 52 writes and keeps its OSC 7 folder in `persistence_cwd` only (never `reported_cwd`, never
  `TerminalCwdReported`), so `cwd()`, `cached_cwd()`, `follow_cwd()` stay `None`; `--cwd` with `--on`
  (`pane.split`, `tab.create`, `workspace.create`) goes to the machine only, the pane's folder here is a new
  pane's; the far folder travels as `CoreTerminalLink.machine_cwd` (filled from `persistence_cwd` by
  `PaneRuntime::core_terminal`, seeded from the request or the saved link) into
  `core_machine_terminal.machine_cwd`, and `PaneSnapshot::local_cwd()` (replacing `program_cwd()`) is `None`
  for a machine pane, so restore, the workspace's `identity_cwd`, git status and a local split never see it.
  Tests: `inline_only_kitty_graphics_refuse_local_media_and_still_draw_inline`,
  `far_output_sets_no_clipboard_and_its_folder_stays_save_metadata`,
  `a_machine_folder_never_becomes_a_folder_here` (a far `--cwd` that this Mac also has).
  M2 and finding 1, the launch label: `CoreTerminalLink.launch` (saved as `core_machine_terminal.launch`) is
  the run's `herdr.launch`; for a machine pane `Shared.identity` is the owner value and that launch, checked
  on the run answer, the attach answer (`AttachResult.terminal`, so the stream itself is bound), a CLOSE's
  finished record, `machine_reattach_check` and the live loop's attach, and before `terminal.kill`
  (`terminal.get` first: a finished record kills nothing, any other terminal is not killed). A mismatch waits
  (`Missing`/`Unreachable`), never ends, never kills; a link without a launch waits. Tests:
  `a_restored_machine_pane_attaches_only_to_its_launch`, `a_live_machine_pane_never_takes_another_run_at_its_address`
  (another run with the pane's owner and pane labels: not attached, not its end, not killed),
  `a_machine_close_without_the_launch_label_is_not_an_end`, and `only_a_finished_record_ends_...` and
  `a_dropped_machine_pane_ends_only_on_a_finished_record` with labelled records (finding 1's test).
  Finding 2: closing a waiting machine pane sends nothing; chosen: the notice, not a pending kill. The
  `machine_dormant` and `machine_unreachable` waits now say that closing sends nothing there and the program
  shows as an orphan once the machine is back, as `machine_terminal_missing` did. A pending kill would be a
  second, delayed kill path outside the person's act, kept only in memory (a restart would drop the promise
  silently) and fired at an address whose launch must be checked again then; the notice plus the orphan scan
  covers it with no new automatic action. Test: `a_machine_wait_says_closing_sends_nothing_there`.
  Finding 3: the pane `cwd` no host has is never a folder (`local_cwd()`), and `on_machine()` and `link()`
  never take it for `machine_cwd`. Test: `the_machine_pane_cwd_is_never_a_folder`.
  Finding 4: `PaneMoveRefusal::Unknown` with `serde(other)`; schema regenerated. Test:
  `an_unknown_move_refusal_reads_as_unknown`.
  M5 and finding 5: a failed attach or thread start of a machine pane detaches and kills nothing
  (`abandon_setup`; this Mac's panes are killed as before). Test: `a_machine_pane_setup_failure_kills_nothing`.
  Finding 6: `finish` keeps the program only after a finished record; after a suspend cause the server's
  `wait_for_machine_after_exit` keeps it, so a person's close racing the CLOSE still kills (label-checked);
  `suspend_pane_after_exit` makes a machine pane wait before its overlay and worktree-removal checks; a
  worktree removal does not stop a machine pane's runtime (it holds no folder here). Tests:
  `a_close_racing_a_suspend_cause_still_ends_the_machine_program`,
  `a_machine_pane_waits_even_where_a_pane_would_close`, `a_worktree_removal_leaves_a_machine_pane_running`.
  Finding 7: `wait_for_machine_instead_of_resume` requests a render. Test: the render assertion in
  `a_suspended_machine_pane_is_never_resumed_here`.
  M3: only this Mac's own terminal marks the primary stopped by a person. Test:
  `a_machine_streams_server_stop_is_not_this_macs_core_stop`.
  M6: restore decides the host from the saved address first; `machine_cwd` is never a local folder, and a pane
  saved with the machine `cwd` and no machine link is held unavailable, never started (on every platform).
  Test: `a_machine_folder_is_never_a_local_one`.
  M7: decided: API input may wake a machine only while a lease is in force, as keys; the wake thread checks
  the lease before it dials (API input writes no lease), and the 5 s per-machine limit applies to both. Test:
  `input_without_a_lease_does_not_dial_a_gated_machine`.
  M8: `HC_*` is dropped from a request's env to a machine. Test: the env names in
  `a_machine_spawn_passes_no_integration_variable_and_no_local_environment`.
  Limits: M4 (machine RPCs on the server loop) stays with https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/302;
  hc 0.1.21 has no kill bound to a label, so the launch check and `terminal.kill` are two calls (a reset
  between them could still give the address away); a machine pane saved before this change (no launch) waits
  and never attaches, so close it and open it again; the orphan notice's `hc ls --label` command still does not
  find machine orphans (C.8); the inline-only and clipboard rules apply to machine panes only, not to an `ssh`
  typed in a local pane (security's P3 for panes in general; a setting for every pane is not built).
  Check (m4m, 2026-11-23, under the shared lock, head 3af9f2a9, binary copied out of the shared target dir as
  `herdr-h308-3af9f2a9`): `just check` 0 (5823 + 158 tests; windows-lint skipped on macOS); the 16 core backend
  live tests (`HERDR_TEST_HC=1`) passed; `core_machine_live_test.sh` through its V26 and PR C parts (link and
  lease parts not run) passed: V26, V9, V31 (pane part), V10, V12 (close: launch label checked, then killed),
  V6 unreachable and dormant (the saved launch attached the same far pid after a restart and a live handoff),
  V4's server part (wake within 1 s, with a lease in force), V11 with a 120 s drop (same far pid after). Not
  run: `core_live_test.sh`, the default-session script, the PR A/B parts, the bench, Windows lint, CI (not
  pushed). Security reads this change once before R.2.

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

- [ ] R.0 Rule: before rolling a Mac back to a build without PR C (downgrade or live handoff to it), close the panes
  on core machines or move them (start them again with `--on` after). Since C.8's third round such a build never
  starts a saved machine pane here and never attaches a terminal of this Mac to it: the link is saved as
  `core_machine_terminal`, which it does not read, and the `cwd` is a folder no host has, so it shows the pane as
  "Saved directory is unavailable" with no hold and no search. But it drops the link from its next save: back on
  PR C the pane cannot attach, and its program on the machine runs on with no pane (never killed). Stated in
  `docs/next` (connecting-machines).

- [ ] R.1 Open question 1: ac-mbm5's herdr publishes its spaces to its `default` core (ask on the desk if a config
  change is needed there)
- [ ] R.2.0 Precondition (security review M3, design D10): for each target, record its ssh path and whether that
  path has a Tailscale SSH `check` rule. Only a check-free path (an `accept` rule for ac to ac between ac's own
  devices, or OpenSSH over the tailnet) is added as a core machine; a target with a check rule gets no
  background dials (login or an announced wake only). The M1 sandbox results (`probe/README.md` § M1) go into
  the announcement: which sandboxes reach the primary core socket, and so the link. The announcement also names
  herdr's own API socket among who reaches the far machine (security review of PR C, M8): any process that can
  open it can start a program there with `pane.split --on` while the machine's link is up, so the same sandbox rule (or the note that
  the same network rule covers it) applies to it
- [ ] R.2 Announcement S1 through the gestore desk: "herdr-core needs ssh ac@ac-mbm5 from m4m's Herdr Core now, to
  add ac-mbm5 as a core machine; later dials happen when you use the view or a pane there after 10 min idle, each a
  fresh ssh login (1Password may ask for Touch ID), and never start Herdr Core on ac-mbm5". Wait for ac's
  acknowledgement
- [ ] R.3 Beta with A-E on m4m and ac-mbm5 by live handoff, panes kept; then ac runs `herdr machine add --core
  ac-mbm5` on m4m himself (S1); record the 1Password prompt's app name (open question 3)
- [ ] R.4 V25: one night with the view attached; `pmset -g log` on ac-mbm5 read by mac-m5; results on #300
- [ ] R.5 Close #300 after V25 passes in a running beta, with the check described
