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
- [x] C.5 New pane dialog: a machine choice, listing core machines with their gate (UI style per `docs/ui-style.md`,
  help entry if a key is bound)
  Built (with PR D): the endpoint carries `spawn_on_machine`: a primary on Unix adds
  `SPAWN_ON_MACHINE_CAPABILITY` to its welcome's capabilities (a view server does not: it starts no program), the
  client records it per endpoint (`ClientShellState::spawn_on_machine_endpoints`). The client shell has no new-pane
  dialog, so the choice is in the existing pane context menu (`context_menu.rs`): after Split right/down, one
  `Split right on <machine> · <gate>` row per core machine herdr links, `ClientContextMenuAction::SplitOnMachine(n)`
  sending `pane.split` with `on`; the label became a `Cow` for it. The machines and gates are this Mac's
  (`core_machine::command::linked_machines`, the state folders `machine status --core` reads: no core call, no
  dial), offered only while the Local endpoint is active and advertises the capability. No key is bound, so no help
  entry. Tests: `client/shell/tests/core_machine_split.rs` (the rows render with their gates and their click
  target sends `pane.split {on: "lab-vm"}`; without the capability no machine is offered). Plan change: the machine
  list is not carried by the endpoint. A server-side listing needs a new advertised API method, and the client
  already reads and writes these folders for its leases (B.2), so a remote endpoint (a saved machine, a view)
  offers no machine choice; a listing method for remote endpoints is left to PR E with the rest of the fleet path.
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

- [x] D.1 `ServerRole` and `herdr server --view <machine>`; the role's boundaries per D6's table, each a match on the
  role
  Built: `src/view/` (Unix only). `ServerRole { Primary, View(MachineName) }`, set once by `run_view_server`
  (`server/headless/bootstrap.rs`, `--view` after `server`, a `--session` first allowed); `viewed_machine()` is the
  boundary read: socket (`herdr-view-<m>.sock` and `herdr-view-<m>.log` beside the session's own), no JSON API
  socket, `AppPolicy::VIEW` (no restore, save, plugin registry, task store, background update), every core
  publisher off at start and on reload (`view_config`, `reload` keeps them off), no default workspace, no git
  read, no program start (`spawn_command_builder` refuses `PaneLaunch::Command` in a view, so a split, tab or
  workspace made there fails with "this is a view of <m>"), no focus reports, close a detach (`keep_on_drop`,
  `kill()` and `end_program` answer `Live` without a call), input AUTOMATIC_INPUT only (D.3), wake only with a lease
  (`MachineLink::dial`: since the rebase on master's M7 it checks the lease for every role), lifetime (`view::lifetime::IdleExit`, 10 min after the last
  client; `ViewPresence` tells the watcher the client count and the last input). `HeadlessServer::new_at` binds the
  view's socket; `serve_as_view` turns on the lifetime. Tests: `view::tests` (flag, socket and log per machine),
  `view::lifetime::tests` (exit 10 min after the last client; input window). Limits: the idle exit is a unit
  test, not a 10 min live wait; the view keeps the user's config otherwise (keys, theme), so a key bound to a
  local action reaches the refusal, not a hidden menu.
- [x] D.2 Document parser with limits; layout projection; `doc` and `missed` handling; V15, V16
  Built: `view/doc.rs` (`ViewLayout`, `ViewSpace`, `ViewTab`, `ViewNode`): only `herdr_max: 1` spaces; 1 MiB, depth
  32, 64 spaces, 64 tabs, 64 panes a tab, 256 in all, names 256 and folders 4096 characters with control
  characters dropped; space, tab and pane ids must be herdr's own (`w<n>`, `<space>:t<n>`, `<space>:p<n>` or
  `:x<raw>`); a wrong-typed field herdr reads refuses the whole document, unknown fields are ignored; n-ary rows
  fold into binary splits by weight; a leaf on a machine of the far core (`m4m/t3`), a mod or an unknown kind is
  left out. `core_backend/view_link.rs`: one unfiltered `events.subscribe` (measured on hc 0.1.21: a machine's `doc`
  event, `{type: doc, machine, data: {name, version}}`, reaches only a subscription with no filter; a label or
  terminal filter gets none), `doc.get {name, on}`, `machine.list`; facts of the viewed machine only.
  `view/watch.rs` `Watcher<S: ViewSource>`: reads the layout at start, on its `doc` event, on `missed` and when
  the link comes up, sends only a changed layout, keeps the last good one on a refused document (one warning);
  forwards far sizes and 7501 status; the notice line (dormant, waking, needs login, not a machine, publishes no
  spaces, far hc older than 0.1.21 with `hc machine update`) goes in the snapshot's `config_diagnostic`, joined to
  the config diagnostic (D.7; no wire change). `app/view_layout.rs`: the projection keeps each far pane's `PaneId` and terminal across documents,
  attaches new terminals off the loop (`spawn_blocking`, `ViewEvent::Attached`), detaches gone ones, keeps the
  active workspace, tab, focus and zoom by far ids; pane, tab and workspace ids are the far side's (D4).
  `Workspace::for_view`. Tests: V16 unit (`view::doc::tests`: oversized, deep, wrong-typed, bad ids, counts;
  `watch::tests::a_refused_document_keeps_the_last_good_layout`); V15 unit (`watch` reads on the doc event, sends
  only changes; `view_layout` new tab and split, a pane that stays keeps its id and terminal, focus kept); V15
  live (below). Limits: status is mapped to an agent state on the pane (working, blocked, idle) with the app as
  the agent; a far detection-only verdict that the far herdr does not mirror (status mirror off there) is not
  shown. Panes the far herdr keeps on the PTY are not in the document and not shown (D6).
- [x] D.3 The size policy of D6 (`resumed` attach, AUTOMATIC_INPUT only, no DATA or FOCUS, RESIZE only for a changed
  far size from `size` events, read-only fallback); cropped or padded drawing with the remote size in the border;
  V14, V30 (before merge)
  Built: `StartedCore::view` and `view_far_size` (`pane_io.rs`): `terminal.get` just before, attach `resumed` at
  that size with the full history; `observe` when the path does not take AUTOMATIC_INPUT (read-only, refuses
  input). `ViewIo` and `view_writer`: keys, pastes, mouse and submissions as AUTOMATIC_INPUT, no terminal reply
  (D.7, H1); a resize
  carries the far size and goes as RESIZE only when it differs from the last one sent; no FACTS, FOCUS or DATA.
  The view's `TerminalRuntime` keeps the far size whatever its rect (`resize` uses `view_size`), follows `size`
  events (`follow_view_size`, from the watcher's subscription), and drops focus reports; the server renders the
  grid into the rect, cropped or padded; the border label adds `<cols>x<rows> there` when the rect differs, and
  `read-only` (`ui/panes.rs` `view_border_label`). The view never polls when idle: `machine.list` only while a
  client is attached and the machine is away or someone typed within 30 s. Tests: unit (`pane_io`
  `a_view_pane_never_takes_the_far_terminals_size`: resumed at 100x30, four AUTOMATIC_INPUT and one RESIZE 90x25
  for a repeat and a change, no `terminal.kill`; `a_view_without_automatic_input_watches_only`; `watch`
  `an_idle_connected_view_stops_reading_the_machines_state`). Live V14 and V30 below.
- [x] D.4 `ClientEndpointId::Core`, endpoint transport that starts the view server; `herdr --remote core:<m>`;
  exit 10 min after the last client; V17
  Built: `ClientEndpointId::Core(MachineName)` (Unix), `storage_key` `core:<m>`; `EndpointCatalog::core` (the linked
  machines, read at load, never saved) and `has_other_machines`; the shell's `set_core_endpoints` (after the saved
  machines, kept across catalog reloads) and the supervisor's `reconcile_core` and `ConnectTarget::Core`, whose
  connect runs `view::start::ensure_running` (starts `herdr [--session S] server --view M` through the daemon
  launcher when none listens) and connects to the view's socket; reconnects follow #260's dormancy as any
  machine's. `herdr --remote core:<m>` (`main.rs`, `view::start::run_view_client`): ensures the view and runs
  `herdr client` on its socket with the remote-client environment. Any machine core lists can be viewed this
  way (one added by hand too). The endpoints are the linked machines that this session's running primary core
  lists (`linked_machines_of_primary`: one `machine.list`, 1 s at most, never a start): the state folders under
  the passwd home are shared by every herdr-dev session and scratch core of the account, and a folder alone made
  every test client federated (caught by `tests/client_mode.rs` in `just check`). Live V17 below. Limits: core
  machines added after a client started appear in its sidebar after the client restarts (the catalog's file
  watch covers saved machines only); a TUI client starts a view server per listed machine as its endpoint
  transport (no dial without a lease); the 10 min exit is a unit test. A client of a newer build restarts the view
  (D.7, M2).
- [x] D.5 Both-present notice and `[remote] prefer`; V21, V22
  Built: `[remote] prefer = "ssh" | "core"` (`RemotePrefer`, default `ssh`, in the default config and
  `configuration.mdx`); `cli/target.rs` `route_machine`: a saved machine first, a name that is only a core machine
  takes the core path, both: `prefer`. Rebased on PR E (2026-11-23): `MachineRoute::Core` goes to E's `core_route`
  (`fleet::open`, `--dial` as E has it), so `prefer = "core"` now sends a name both kinds share over the core link,
  which lifts E.2's limit "a name both kinds have always goes the old way"; a name no saved machine and no
  primary-listed core machine has still reaches `core_route` as on master (a machine added with `hc` alone). The TUI shows "One Mac, two paths" once per run for a
  saved machine and a core machine on one host (`core_machine::command::same_host_pairs`) and shows both. Tests:
  V22 unit (`prefer_decides_only_when_a_saved_and_a_core_machine_share_the_name`,
  `saved_and_core_machines_on_one_host_pair_up`); V21: the existing `src/remote` and endpoint suites (in `just
  check`) and `view::start::tests` (`--remote <host>` is never a core target, so the old path runs and no view
  starts). Plan change: the endpoint handshake carries no far core id, so the pair is found by the host of the two
  ssh destinations, not by core id.
- [x] D.7 The independent review's findings, after the rebase on origin/master (PR 90's far output, M7's
  lease-gated input wakes, the space-pin PR)
  Built: H1 `view_writer` drops `WriteCmd::Reply` and a resize's reports, and the parser does not even queue a
  view's replies (a history replayed at attach cannot fill the queue ahead of a key); `PaneLaunch::ViewCore` is
  `far_output` (inline Kitty only, no OSC 52, no OSC 7 cwd here) with no `far_cwd`. H7 `presence::lease_pass`:
  a client writes `client-<pid>` only into the folders of the machines its own primary core lists
  (`linked_machines_in` + `listed_by_core`, one `machine.list` on the running primary, only when some machine is
  linked); `HERDR_NO_MACHINE_LEASES` turns the writer off and is set by `tests/support`'s spawn helpers, every
  direct client spawn in `tests/`, and every live script's herdr wrapper except the two lease-test clients of
  `core_lease_live_test.sh`; `Lease::in_force` also needs a `client-`/`cli-` holder's process to run
  (`LeaseHolder::alive`; pid reuse keeps a lease to its expiry, no later than before; it applies to PR E's keep
  leases too, whose two unit tests now hold their lease with the test's own pid instead of 42). M3 no lease at a client's
  start (`ClientPresence::start()` takes no `person_started`; the loop config field is gone); the view's watcher
  wakes only for a new person's input in the view (`ViewPresence::inputs`, stamped on `ClientShellPaneInput` that
  is not only a pointer motion, `lifetime::is_person_input`), once per input, with a lease in force, never on an
  attach or a state read; a new input while away reads the state at once. M2 the view takes an exclusive `flock`
  on `herdr-view-<m>.lock` before it logs and writes `{version, pid}` there; `ensure_running` restarts a running
  view whose build `replaces` orders below the client's (SIGTERM, SIGKILL after 5 s, gone when the lock is free),
  so two builds never restart a view in turn. L3.4 the same lock makes a racing second start exit before it
  touches the log or the socket (the log was already append-only; a second process could rotate it away). M4.1
  `execute_custom_command_binding` refuses every action in a view (`App::view_refusal`). L4.2 a reload clears the
  pane history only when `policy.persist_session`. L4.3 `worktree.create` and `worktree.remove` refused in a
  view. L5.1 `CoreClient::call_bounded`: a reply over its bound is refused from its id alone (`message_id`, no
  `Value`); `layout_document` bounds `doc.get` at 1 MiB + 16 KiB; `doc::parse` no longer re-serializes to measure.
  L5.2 `doc::unique_ids`: a tab, pane or terminal listed twice refuses the document. L5.3 a failed `doc.get`
  waits 1 s, doubling to 30 s. V21 `EndpointCatalog::load()` reads files only; `load_with_core_machines()` (the
  TUI client and the launch's federation check) adds the core machines, so `--remote <host>` and `herdr machine`
  make no core call. Notice: the view's notice is `ViewApp::notice`, joined to the config diagnostic in the
  client-shell snapshot (`view::with_notice`), never written over it. Tests (each failed with its fix reverted,
  all at once, and passed with it): `pane_io` `a_view_pane_sends_no_reply_to_far_queries` and the updated
  `a_view_pane_never_takes_the_far_terminals_size`, `pane` `a_view_panes_output_is_far_data`, `presence`
  `a_client_leases_only_machines_its_primary_lists`, `state` `a_gone_holders_lease_is_not_in_force`, `watch`
  `a_dormant_machine_is_woken_only_by_input_in_the_view` and `a_failing_layout_read_backs_off`, `view`
  `one_view_holds_the_lock_and_records_its_build`, `start` `a_client_restarts_only_an_older_running_view` and
  `a_newer_client_build_replaces_the_running_view`, `client` `a_bounded_call_refuses_a_reply_over_its_bound`,
  `doc` `repeated_tab_pane_and_terminal_ids_are_refused`, `custom_commands` `a_view_runs_no_custom_command`, `app`
  `a_views_reload_leaves_the_primarys_pane_history`, `worktrees` `a_view_makes_no_worktree`, `view_layout`
  `a_far_size_reaches_the_pane_and_its_border`, `lifetime` `a_pointer_motion_is_no_persons_input`. Live:
  `core_lease_live_test.sh` asserts no lease and no dial at the client's start, the lease at its first key, no
  lease in a linked folder the scratch primary does not list (H7), and for V33 no dial at the view's start or
  attach. Limits: V21 has no failing-before test (the old path's core call needs a linked folder under the real
  passwd home to happen at all; the split is by construction); a key in any pane of the primary still leases
  every machine its primary lists (D3 row 1), so typing locally allows, and through core's retry dials, every
  linked machine; narrowing leases to the machine a person types to is a later decision. (Understated when
  written, found by PR D's second read: the lease writer also posted a wake to every listed machine's dialer, so
  any input, a pointer motion included, dialled every dormant linked Mac at once; D.8 item 1 removes that wake.) Two different builds
  with the same order (`0.9.5-ac-beta.161-a` and `-b`) can still restart one view in turn; an older client uses a
  newer view as it is.
- [ ] D.6 Check: `just check` 0, `just core-live-test` 0, CI green
  Local, m4m, 2026-11-23, under the shared lock (the load gate lifted by manager-lab for scanner load), head
  2b439317 for the code (later commits are scripts and openspec only): `just check` 0 (5837 + 158 tests). Live,
  with the binary copied out of the shared target dir: `core_view_live_test.sh` (new, in `just core-live-test`)
  passed at its full 60 s idle: V15 new tab in the view 0.60 s and new split 0.62 s after the far change; V14 50
  keys, a paste, an SGR click and focus in/out reached the far program and the far terminal kept 103x34, then
  after a far viewer took 90x25 it stayed 90x25 while the view typed; frames from the view on streams: 61
  AUTOMATIC_INPUT, 1 CREDIT, 1 RESIZE (90x25), no DATA, FOCUS or FACTS; V30 4 frames in 60 s idle, all CREDIT;
  the far viewer gone, core kept 90x25 (the view's recorded size, not its attach size 103x34), and after a
  SIGKILL of the far herdr it stayed so with the far program's pid unchanged; V17 closing a pane in the view and
  stopping the view left the far program and pane. `core_machine_live_test.sh` with V3_SECS=60, V38_SECS=120,
  V11_SECS=60 passed whole on the last run, including V33's new view part (no dial before the view client's
  lease, one after, connected 0.68 s after the client started). Two earlier runs failed once each in older
  cases, V3 (link bytes after dormancy) and B.5 (the client's lease after a dial), and passed on the next run
  unchanged; not proven, the likely cause is another herdr-dev client of the account writing its lease into every
  machine folder (B.2, as designed) while other builders' live tests ran. One V35 failure was the check's own
  race (retry_spent with no GATE far_core_down line), fixed in the script. `just bench-render-scale`: active
  panes 266 us at 1 pane, 318 us at 15 (1.20x; C.7 had 1.19x); background workspaces 261 and 261 (1.00x); the
  render path gained one `Option` check per pane resize and the view's border label. Not run: the full-length
  V3, V38 and V11 waits, `core_live_test.sh` and the default-session script, CI (not pushed).
  After D.7, rebased on origin/master 724bd1b4 (PR E), m4m, 2026-11-23, code head a2608dde (the openspec record
  follows): `just check` 0 (5899 + 158 tests). Live, binary copied out of the shared target dir:
  `core_view_live_test.sh` with V30_IDLE_SECS=20 (shortened) passed, with the new parts: H1 0 frames from the view
  but CREDIT and RESIZE before a key, the far pane's DA and DSR in its replayed history; L3.4 a second view exited
  1 and left the running one, its lock and its log; M2 a client restarted a view recorded as 0.0.1 under a new
  pid, a client of its own build kept it, the far program ran on. `core_machine_live_test.sh` with V3_SECS=60,
  V38_SECS=120, V11_SECS=60 (shortened) passed whole, PR E's fleet part included: B.2/B.5/M3/H7 no lease and no
  dial at the client's start, its first key's lease before its dial, connected 4.59 s after the start, no lease
  in the unlisted folder; V33/M3 the view's start and attach dialled nothing. Earlier runs on the way: one V3
  failure (12 link-byte lines after dormancy) at 45ceddad that the next two runs did not repeat; a lease watcher
  on the rerun saw only the script's own holders, so its cause is not found (D.6's runs saw it too); one failure
  of PR E's `--dial` ordering check, a core retry's START counted as the dial's, fixed in the script (a2608dde).
  `just bench-render-scale` (5-min load near 100): active panes 268 us at 1 pane, 326 at 15 (1.22x; 1.20x
  before), background workspaces 259 and 266 (1.03x); D.7 adds one `Option` join per client snapshot.

- [x] D.8 Follow-ups of the view's security review (security ac/security 7eff725,
  `docs/reviews/2026-11-23-herdr-view.md`, M1-M8) and of PR D's second read (items 1-6), Gitea #312
  Built:
  M1 (far folders stay far): `handle_tab_bar_status_tasks` runs no status command in a view and
  `next_tab_bar_status_deadline` has none due (`App::is_view`); `worktree.list` and `worktree.open` are refused
  like create and remove (`worktrees/deferred.rs`), with no git read started. Tests `a_view_runs_no_tab_bar_status_command`,
  `a_view_makes_no_worktree` (list and open added).
  M2 (one allowlist): `view::policy::classify` names every client-shell method `Allowed`, `FarLayout` or `Local`;
  `advertised_client_shell_method_names` filters the welcome by it, and `handle_client_shell_endpoint_request`
  refuses (`refused_in_view`) every method the view does not take, before dispatch (`App::view_method_refusal`;
  an unclassified method is refused). Allowed: `client_shell.surface.set`, `server.reload_config`,
  `notification.{list,mark_seen,clear}`, `workspace.focus`, `tab.focus`, `pane.{focus,focus_direction,zoom,scroll,
  scroll_application,clear,clear_scrollback,selection.read,copy_motion,copy_search,input.set,mark_seen,close}`,
  `tab.close`, `workspace.close` (closes are detaches; clear and clear_scrollback touch the view's own grid only).
  Why the rest: layout and names are the far herdr's (D6), and links (open a URL here, resolve a path against a
  far folder), the scrollback editor (a file and an editor here), todos, tasks, issues, asks, integrations and
  dismissals are this Mac's. A view also stages no clipboard image (`ServerEvent::ClientClipboardImage`), clears
  this Mac's release notes and announcement at `start_view`, and the palette offers no send where `agent.message`
  is not advertised (so no Note through the local `note_command`). Tests `view::policy::tests` (every method
  classified, the allowlist pinned, welcome = dispatch), `headless::tests::view_policy` (the named methods through
  the real dispatch: refused, nothing in flight, no editor; an allowed one dispatched; no image staged),
  `palette::an_endpoint_without_agent_message_offers_no_send_or_note`.
  M3 (input bound to the terminal): `ViewIo::instance` pins the far terminal's `herdr.launch` label and pid at the
  first attach; `machine_reattach_check` and `attach_again` take only that instance, and another program at the
  address ends the stream (`view_replaced`, the writer sends nothing after); `App::rebind_replaced_view_pane`
  binds the pane afresh. `ViewApp::bound`/`held`: a pane whose far terminal changed (another address under its
  far id, or a replaced instance) takes no input in `route_synced_pane_input` or `pane.scroll_application` until a
  mouse press in it (not forwarded) or `pane.focus`/`pane.focus_direction` onto it; the border says `another
  terminal now: click to type` (`TerminalState::view_input_held`). Tests
  `pane_io::a_view_reattaches_only_the_far_terminal_it_attached_to`,
  `view_layout::a_pane_whose_far_terminal_changes_holds_its_input_until_focused`,
  `sync_input::a_held_view_pane_takes_no_input_until_a_person_clicks_it`.
  M4 (grid budget): `view::far_size_fits`, 1000 rows, 1000 columns, 300 000 cells (a 6K display at a 6x12 px font
  is about 1000x280; a few MB of grid a pane; the document allows 256 panes). Refused, never clamped (a clamp would
  draw wrongly and reach core as the RESIZE): `view_far_size` fails the attach with `too_large`, and
  `CorePaneIo::follow_far_size` and `App::follow_view_size` ignore a larger size event, keeping the last one.
  Item 6 (part): a view's grid resize (rect, the 2x4 minimum) never reaches the wire; `follow_far_size` sends the
  far size itself. Tests `pane_io::a_far_size_over_the_budget_never_reaches_a_grid_or_the_wire`, the updated
  `a_view_pane_never_takes_the_far_terminals_size`, `view_layout::far_sizes_are_bounded_and_kept_only_for_shown_terminals`.
  M5 (churn): sizes kept only for shown terminals; `LAYOUT_EVERY` 500 ms between layout reads (the last document
  still arrives); one far size and one status per terminal per tick (`coalesce`); `MAX_ATTACHING` 8 at once, the
  rest wait; each attach has a generation and a completion of another is detached. Tests
  `watch::far_churn_is_coalesced`, `view_layout::far_attaches_are_capped_and_obsolete_ones_are_detached`.
  M6: `run_server` takes main's parsed arguments and `view::requested_view` reads the role from them. Tests
  `view::a_session_named_server_does_not_change_the_role`; live (view part) a view in session `server` makes no
  primary socket or session file.
  M7 and items 1, 2, 5 (leases): the client's writer posts no wake; `MachineLink::dial_in` waits up to 3 s
  (`LEASE_GRACE`) for the person's lease; a bare pointer motion (crossterm `Moved`, SGR motion with no button)
  writes no lease (`events_are_person_acts`, `sgr_mouse_is_person_act`; #260's dormancy wake still counts it);
  `presence::writes_leases`: none for a remote client whose target is not `core:`; `lease_pass` needs the
  primary's record to be herdr's own `core-link` (`managed_by_herdr`) and the folder's `primary.json` (new,
  written by `machine add --core`) to name this session's core, or be absent (an older build's folder);
  `LeaseHolder::parse` needs a positive `pid_t`; `MachineDir::leases` drops a lease whose pid now runs a process
  started after the file was last written (`platform::process_started_at`, macOS `proc_bsdinfo`, Linux
  `/proc/<pid>/stat` plus `btime`); `herdr machine status` prints `leases off here: HERDR_NO_MACHINE_LEASES is set`
  (stderr with `--json`). Tests `presence::a_client_leases_only_its_own_primarys_herdr_links`,
  `only_local_and_view_clients_write_leases`, `state::a_reused_pids_lease_is_no_ones`, `holders_are_the_three_kinds`
  (overflow), `panes::a_wake_waits_briefly_for_the_persons_lease`, `client::person_act_tests`,
  `command::machine_status_names_the_lease_opt_out`; live (lease part) the first key in a local pane writes the
  lease and starts no dial.
  M8 and items 4, 6 (lock and restart): `view::open_lock` opens with `O_NOFOLLOW`, only a regular file of this
  account in a folder no one else writes; `running_view_at` reads at most 4 KiB from the locked inode; a probe
  that fails (no flock, `ENOTSUP`) says so once and counts as no view; `ViewRecord::start` holds the process start
  token and `stop_view` signals only while `names_its_process`; `herdr server --view <m> --restart`
  (`view::start::stop_running`) ends a running view of any build first. Tests
  `view::a_symlinked_or_shared_lock_is_refused`, `a_record_names_only_the_process_that_wrote_it`,
  `start::a_restart_never_signals_a_process_that_reused_the_views_pid`.
  Item 3: `catalog_reload::watch_core_machines` reads the primary's linked machines every 30 s and the client adds
  or retires view endpoints (`ClientLoopEvent::CoreMachines`, Local supervised once one shows); an empty listing
  removes none. Test `catalog_reload::core_machine_tests`.
  Design: D3 records why a key in any local pane leases every machine (a person at this Mac can answer Touch ID;
  per Mac, not per pane; free with two Macs), and that leases allow, not wake; D6's table has the allowlist, the
  input binding, the far data bounds, the lock and `--restart`.
  Limits: the far pane identity is checked by instance at reattach, not against the document's far pane id (the
  far herdr's `herdr.pane_id` label can change on a move); a terminal attached before this build's first attach
  record (no launch label and no pid) is not compared; a record of an older view build has no start token, so its
  restart still trusts the held lock and a live pid; a view lock path replaced under a running view splits it
  across inodes (the private folder keeps others out; the account's own processes are trusted); per-pane grid
  budget only, no aggregate across the 256 panes; direct terminal attach (`TerminalHello`) to a view is not gated
  separately (it types through the same AUTOMATIC_INPUT writer and the same binding); a view advertises no
  `pane.link.activate`, so links in a view are copied, not opened; a click that releases a held pane is not sent;
  pre-lock views (an unreleased PR D build) are not found by a client; the 30 s core poll runs one `machine.list`
  per client; `writes_leases` reads the remote client's target from its environment, which a person can set;
  the `rebind_replaced_view_pane` path is covered by its parts' tests (no unit test drives a replaced runtime
  through the App, which needs a real core). Per-machine leases stay a later refinement.
- [ ] D.9 Check for D.8: targeted tests, `just check` 0, `core_view_live_test.sh`, `core_machine_live_test.sh`
  lease part; CI after the push
  Local, m4m, 2026-11-23, under the shared lock, code head faffba19 (later commits are scripts and openspec
  only), based on origin/master 136d563d (PR D): every new test failed with its fix reverted and passed with it
  (M1, M2, M3, M4 with item 6, M5, M7 and items 1, 2, 3, 5, M8; M6's unit test cannot fail on the old code, whose
  fault was in the raw arguments, so its proof is the live check). `just check` 0 (5924 + 158 tests;
  windows-lint skipped on macOS as the justfile says). Live, a debug binary of faffba19 copied out of the shared
  target: `core_view_live_test.sh` with V30_IDLE_SECS=20 (shortened) passed, M6's new check included (a view in
  session `server` made only its lock, log and socket); a `--remote core:` tab on a view a newer client restarted
  ended with it (item 4's question). `core_machine_live_test.sh` with `CORE_MACHINE_PARTS="link lease"`,
  V3_SECS=60 and V38_SECS=120 (shortened) passed: the link part, V2, V7, V3, V38, the client part (its first key in
  a pane of this Mac wrote the lease and started no dial, item 1), V33's wake and view parts (the view's first key
  dialled, connected 8.19 s after its start). Each run left one pane-anchor helper of a scratch server whose
  folder was gone; stopped by hand. Not run: the panes and fleet parts, `core_live_test.sh`, the bench (no render
  or PTY path gained work beyond one hold lookup per view input event), CI (not pushed).

## PR E. Actions and the fleet CLI over the link

- [x] E.1 Remote herdr path and capability probe per link establishment (D7)
  Built: `core_machine/fleet.rs` `far_herdr`. One `machine.exec` of the `env -u` wrapper, `/bin/sh -c` and a fixed
  script (`PROBE_SCRIPT`) with the names as arguments, never as script text: the name this herdr runs as, then
  `herdr`, on the far `PATH` and in `~/.local/bin`, `/opt/homebrew/bin`, `/usr/local/bin` and linuxbrew; the first
  that answers `remote-api-bridge --check` with `herdr-api-bridge-v1` is taken, with its `status client --json`
  (version, protocol, endpoint capabilities). "Per link establishment" is the link's `dial.json` record (the ssh
  pid and start time core-link writes for each dial): the answer is kept in the machine folder as
  `far_herdr.json` with that record and used while it matches, so a redial probes again; a far herdr gone from its
  path (exit 126 or 127) forgets it. `herdr machine status --core` reports it as `far_herdr`. Tests: units for the
  script (finds the named herdr, skips one that fails the check, exit 2 when none), the answer's parse, the argv,
  once per link against the fake core (no second exec on the same dial record, a new one probes), and the failure
  naming what was looked for; loop: one probe for two calls over one link, status reports the path. Limits: mise
  installs are not searched (the old remote's discovery also looks there); the capabilities are read and kept but
  only `remote-api-bridge` is required (E.3's verbs will read the rest); the D9 notice for an old far hc is not here.
- [x] E.2 `--machine` through `machine.exec` with the `cli-<pid>` lease and the `env -u` wrapper; dormant refusal
  (exit 75) and `--dial` through the dialer (D7); `truncated` and `timed_out` as errors, input over 8 MiB refused;
  no retry after send; V18, V19, V31 (exec part), V32, V37
  Built: `cli/target.rs` `Route { Saved, Core }`: a selector that names a saved SSH machine (id or label) goes the
  old way, unchanged (D8); any other is looked up as a core machine (`fleet::open`), which needs herdr's state
  folder (a machine only core knows is refused, exit 2, as one herdr keeps no lease or gate for). `--dial` is a
  prefix flag beside `--machine` (`herdr --machine m --dial pane list`), on each call (E.2a removed the
  environment switch). Connected: the `cli-<pid>` lease as a keep lease (E.2a), the probe, then the requests. Not connected: without
  `--dial`, D7's message and exit 75, no lease, no dial; with it, refused (exit 1) over `needs_check`,
  `auth_stalled`, `auth_failed`, an unreadable gate or a `far_core_down` under 5 min, else the lease with the act
  (it reopens `retry_spent`), a `CLI --dial` line in `link.log`, one `dial_once` (D3a's rules), and a refusal if the
  machine is still not connected. The CLI reaches the far side through `FleetLink`, a 0600 socket in a 0700 folder
  that speaks herdr's API, one request per connection: each request line is one `machine.exec {on, argv: wrapper +
  [far herdr, --session S, remote-api-bridge], input, timeout}`, the reply's stdout goes back untouched. S, the far
  herdr session, is the one whose primary core the machine was added with (`default` for core's default, `X` for
  `herdr-X` or `herdrdev-X`; a core session no herdr session uses is refused). Timeout (as amended by E.2a): the
  status probe 9 s; the request's own `timeout_ms` plus 10 s, 60 to 600 s; none, 600 s. A request over 8 MiB, `unavailable`, any other core refusal, a lost answer
  (it may have run; not sent again), `truncated`, `timed_out` or a far exit other than 0 record a failure and close
  the connection with nothing written, so the CLI fails (exit 1) with that message through `remote_error`; the
  status probe of a core route is never retried (`server_status`). The lease is written before the first exec
  and removed when the CLI ends (its life and renewal: E.2a).
  Supporting: `core_machines::call_running_at` and `CallError` (not sent, refused by kind, no answer), the fake
  core's `machine.exec` (`exec_reply`) and `hang_up_on`. Tests: V19 (the fake core drops the connection on
  `machine.exec`: one error saying it may have run, one exec), V37 (`truncated` and `timed_out` with exit code 0:
  the CLI side gets no bytes and the message; a request over 8 MiB: no exec sent), the request's argv, input and
  timeout, the lease's writes and removal, the gate's refusals, the session mapping, the prefix parse; loop
  (`core_fleet_live_test.sh`, a far scratch core with the six fake `HERDR_*` variables and its own herdr server,
  herdr's link and a fake ssh): V18 the same JSON as the far herdr's own `pane list` and as the old remote's
  `--machine` to the same far herdr through the fake ssh; V31 a control `hc exec --on` lists the six names in the
  far core, every far herdr run through `--machine` got none; V32 dormant: exit 75, D7's message, no lease, no ssh;
  `--dial`: the lease line before the one dial's START, one ssh, the far JSON, the lease gone after; `--dial` over
  `auth_failed`: exit 1 naming `herdr machine login`, nothing written or dialled. Limits: `[remote] prefer` and the
  both-present notice are D.5 (PR D), so a name both kinds have always goes the old way; the lease lives for the
  whole CLI call (its status probe and its request), not one exec; a core retry that lands between the `--dial`
  lease and its connect can make core restart that dial (R4, D3a's accepted limit); the plan's `pane list --json`
  is `pane list` (it prints JSON and takes no `--json`).
- [x] E.2a Independent review of E.1/E.2 (read-only, at cd1de8d4), every finding fixed at its root with a test that
  fails on the old behaviour (each checked by reverting the fix):
  - 2.1 lease life: the `cli-<pid>` lease lives 60 s past its last write and a renewer thread writes it every 30 s
    while the CLI runs (was 120 s plus the call's timeout, up to 720 s after a killed CLI). SIGINT, SIGTERM and SIGHUP
    run a cleanup (`platform::core_link::clean_up_on_termination`, signal-hook, one thread) that removes the lease
    and the bridge folder, then end the CLI as the signal's default does; a renewal racing that exit writes nothing.
    A SIGKILLed CLI's lease ends within 60 s, and the next CLI removes bridge folders of dead pids. Decision on D3's
    "at most once a minute": that rule bounds a client's writes per input; a CLI writes twice a minute for its own
    life only (one small file), and a lease bounded to a minute matters more (design D7 § The lease's life). Tests:
    renewal, kind and removal; the life bound; a child process per signal (SIGTERM, SIGINT, SIGHUP: killed by it,
    lease and folder gone); dead folders swept; loop: a waiting call's lease is a keep lease of at most 61 s, SIGTERM
    exits 143 with the lease and folder gone.
  - 2.2 decision: a **keep lease** (`<expiry> 0 keep`): core-link's watcher counts it (it keeps an up link), `admit`
    does not (`newest_act` counts only leases that dial). A plain call on a connected machine writes one; `--dial`
    writes a dialling lease with its act for its one connect, then turns it into a keep lease. Why not "no lease":
    without one the watcher ends the link mid-call when the other leases run out (a 600 s wait). So core's retry
    after a drop during the call meets no lease of the call's that admits it: D7's "No new ssh" holds (a lease of
    another holder still admits it, as without the call). An older core-link that reads a keep lease as no lease only
    ends such a link sooner. Status lists it as `cli-<pid>(keep)` and `dials: false`. Tests: parse, `admit` refuses
    a keep lease alone (also after `dial_failed`), the watcher keeps the link on one, `prepare` on a connected
    machine writes one and connects nothing; loop: a `--dial` call whose link is killed mid-request fails saying it
    may have run, core's retry is refused as `Dormant`, one ssh in all.
  - 2.3 `--dial` refuses `retry_spent` whose detail is `far_core_down` within 5 min, as `far_core_down`.
  - 2.6 `HERDR_MACHINE_DIAL` removed (code, docs, design, the live script).
  - 3.1 the status probe's run is 9 s and its core call 14 s (`CALL_MARGIN_SECS` 5, was 10), inside the CLI's 15 s
    (`REMOTE_STATUS_TIMEOUT`, a test ties them); dropping the bridge releases the lease and the folder first and
    waits at most 0.5 s for a request in flight (fake core `stall_on`: drop under 2 s, was 14 s+).
  - 3.2 every failure after sending says `it may have run there, and it is not sent again` (far exit other than 0,
    `timed_out`, an answer that does not parse); `truncated` says it ran.
  - 3.3 no `timeout_ms` runs 600 s (core's longest; the old path waits however long); a `timeout_ms` over 590 000 is
    refused unsent with the `--timeout` to use; documented with the exit for longer waits (loop, or a saved machine).
  - 3.4 measured on a throwaway core (hc 0.1.21, direct `machine.exec`): input 8 388 608 bytes runs, 8 388 609 is
    `invalid_params` ("at most 8 MB raw"): the check (`> 8 MiB`) was exact; comment fixed, boundary test added.
  - 3.5 a request's failure is cleared when the next request starts.
  - 3.6 measured with two throwaway cores and a `--via` relay: a link killed mid-`machine.exec` answers
    `unavailable`, `lost the line to loop during the call` (hint "it may have happened there"); a far core stopped
    mid-run answers `unavailable` too. So `unavailable` now says it may have run, with core's message.
  - 4 the probe asks the far herdr about the session the requests go to, and `status client --json` reports
    `core_session`; it must equal the machine's core session, else refused before any request. A far herdr too old to
    report it is accepted for `default`/`herdr-X` and refused for `herdrdev*`.
  - 5 the bridge folder's name carries a per-process random part (`RandomState`) and a taken name is skipped for a
    fresh one (as `platform::fallback`); a start that fails after the folder was made removes it (a guard; not unit
    tested: no way to make `set_permissions` or a thread spawn fail on demand).
  - 6 with no passwd entry, a name core does not list stays an unknown selector (exit 2).
  - 7.1 `far_herdr.json` is trusted 5 min on the same link (`probed_at`), then probed again, so a far herdr upgraded
    in place is read within 5 min; 7.2 a far exit 2 (no `remote-api-bridge`) forgets it, as 126/127 do; 7.4
    `write_atomic` writes a temporary file of its own process and write (`create_new`), never another writer's.
  - Found by the new live checks (the SIGTERM call ended by itself): through `machine.exec` every wait ended at
    once with an empty reply. `remote-api-bridge` half-closes the API connection when stdin ends, the API then ends
    a waiting request, and `machine.exec` closes stdin once the request is in (reproduced with a scratch herdr
    server: `pane.wait_for_output` with stdin closed returned nothing in 0 s; with stdin open it timed out as asked).
    New `remote-api-bridge --request` (reads the request to EOF, never half-closes, passes one reply line;
    `--request --check` prints `herdr-api-request-v1`); the probe takes only a herdr that answers it, every request
    runs it. Tests: a unit test that the API side sees no half-close while the request waits; the probe script skips
    a herdr without it; loop: the SIGTERM and drop checks wait on the far side as they should.
  - The live V35 stale-socket check failed three runs in a row under load, with the base binary too: core's own
    retry dial started as the test opened the gate, the test's connect made core end it (R4), so the stale-socket
    dial recorded `retry_spent (far_core_down)`. The check now accepts that line; failures print the run's link logs.
  Limits: the keep lease needs core-link from this build to count (an older one ends such a link when the other
  leases end); `--dial`'s keep switch comes after core answered `connected`, so a drop within that instant is still
  admitted by the dialling lease; a request refused for its timeout is reported through `remote_error` like any
  other bridge failure.
- [x] E.2b Follow-ups of PR E (https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/310): the second read's
  items and security's two-vendor review (ac/security d9fbd4a, `docs/reviews/2026-11-23-herdr-fleet-cli.md`: no P0
  or P1; M1 P2, M2-M7 and E1 P3). Each fixed at its root with a test that fails on the old behaviour (each checked
  by reverting the fix):
  - M1 far text: `remote::visible_line` (new, `src/remote/far_text.rs`) shows C0 but tab, DEL, C1 and the bidi and
    line-separator marks as `\x1b`-style escapes. Used for the probe's and a request's stderr tail
    (`Ran::stderr_tail`), core's messages in `exec`'s failures, a far herdr's `core_session`, a core route's refusal
    line (`cli::target::refusal_line`), every `remote_error`, the far server's version, core state and macOS
    services context in `status` (now built as lines, `server_status_lines`) and core's state in `machine status
    --core`. Tests: OSC 52, OSC 0 and CSI (and C1) through each of those. Limit: the human list output over
    `--machine` (task, todo, notification, `agent explain`), the old remote's, is
    https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/311.
  - Empty reply: `remote-api-bridge --request` fails when the API closes with no reply line or within one; the
    fleet bridge takes a far exit 0 only with one whole JSON reply line carrying the request's id (M6), else a
    failure that may have run. Probe failures: `exec` returns `ExecFailure` (not sent, may have run, ran); the
    probe says `the request was not sent` for each. Every far exit but 0, and a reply that is not whole, forgets
    `far_herdr.json` (a herdr whose bridge has no `--request` exits 1 with its usage; the test now fakes that).
  - `write_atomic`: a temporary of this process's own name (`<path>.<pid>-<n>.new`) can only be a killed writer's
    leftover: removed and the write made again. The lease renewer reads the wall clock every second (`LeaseTiming`
    `tick` and `clock`) and renews once due by it, so a lease is written within a second after a sleep. A selector
    herdr did not add is an unknown selector with no `machine.list` call; its error names `machine status --core`.
    The `status client --json` `core_session` comment says when it is reported.
  - M2: `far_herdr.json` is read only from a regular file opened with `O_NOFOLLOW`, of this account, 0600, at most
    64 KiB. Not done: pinning the far path at `machine add` (a far binary is still found by the probe, R24's class).
  - M3: the bridge serves only a peer of this account (`platform::core_link::peer_uid`: `getpeereid`, Linux
    `SO_PEERCRED`); one 30 s deadline for the request line and one for the reply (`read_line_by`, `write_all_by`);
    the accept waits on `poll` with a wake pipe that `Drop` closes, never a blocking connect; the sweep takes only
    `herdr-fleet-<pid>-<16 hex>`, 0700 folders of this account, removing the socket and then the empty folder (the
    CLI's own cleanups do the same); making the folder or the lease and registering it for the signal cleanup is
    one step under its lock, and nothing is made once it ran. Not unit tested: the refusal of a peer of another
    uid (no second account in tests) and a full listen queue.
  - M4: `cli-` leases name their kind (`dial` or `keep`), and a `cli-` lease with none is no lease; a lease whose
    expiry is beyond its holder's longest life (60 s, 10 min, 12 h, with 5 s) or whose act is in the future is no
    lease; at most 256 lease files of 128 bytes are read. core-link admits again under `dial.lock`, then turns every
    `cli-` grant in force into a keep lease (`MachineDir::consume_cli_grants`, `GRANT used` in `link.log`); the CLI
    writes its grant once, never renewed (its minute outlasts the dialer's 40 s). The link live test writes its
    `cli-` leases with the kind and a minute.
  - M5: before any lease or exec, core's record of the name must be herdr's link (`command::runs_herdr_link`: `via`,
    a target ending in `core-link <name>`, the session when the record has one), also used by `machine status`.
    The fake core lists whole records (`set_machine_record`).
  - M6: the far bridge forwards one request line within the API's 1 MiB limit (`api::MAX_REQUEST_LINE`) and
    nothing after it, and reads at most 8 MiB of reply; the fleet bridge refuses, unsent, a request that is not one
    line within that limit (was 8 MiB). Not done: a deadline for a directly invoked `remote-api-bridge --request`
    (core's `machine.exec` has its own).
  - M7: `--dial` prints `herdr: --dial: opening ssh to <m> now; a Touch ID or Tailscale check can prompt on this
    Mac` on stderr before its dial.
  - E1: the bridge folder's base must be one no other account can replace (`is_trusted_base`: it and every folder
    above it, symlinks resolved, this account's or root's, a shared one sticky), else `/tmp`, else refused.
  Not fixed: an older core-link watcher on a link that was up before an upgrade reads a keep lease (or a `dial`
  kind) as no lease and ends that link when the other leases end (old binaries; until those links redial). A
  grant that core-link turns into keep just as the CLI removes its lease can leave a keep lease of a gone CLI for
  its minute (it admits no dial).
  Gates, m4m, 2026-11-23, under the shared lock: targeted tests (fleet, state, link, core_link, remote, cli target
  and status) green, each new test failing with its fix reverted; `just check` 0 on 562177f6 (5881 + 158 tests;
  windows-lint skipped on macOS; 9ea6a0cd after it changes the fleet live script only);
  `CORE_MACHINE_PARTS="link lease fleet" core_machine_live_test.sh` 0 on a debug binary of 9ea6a0cd copied out
  of the shared target, with shortened waits (`V3_SECS=150`, `V38_SECS=300`, not 300 and 600): V26, the link part,
  the lease part and the fleet part with the new `--dial` line and grant checks; nothing left running or in the
  passwd home's state.
  Round 3 on PR 95 (an independent read: six low findings; security ac/security 406215a: M5 and M6 again), each
  with a test that fails with its fix reverted:
  - Lease read: the 256-file cap counts holder-named files only (`.new` temporaries, strays and expired leases no
    longer push a live lease out); the dialer's prune also removes expired `cli-<pid>` files whose pid is dead
    (`LeaseHolder::pid`, was `client_pid`).
  - Lease time caps clamp: an expiry beyond `now + max_life` is kept with its expiry set to `now + max_life` (a wall
    clock stepped back after a wake no longer kills a 12 h wake or a keep lease); a future `input_at` is still
    refused.
  - Far text is escaped exactly once, where it is printed: `remote::far_line` / `far_text` (core route only) also
    escape a literal backslash, so far text cannot spell `\x1b`; `visible_line` (unchanged for the local and the old
    paths: a Windows pipe path keeps its backslashes) now also shows U+061C, U+200E and U+200F. The inner layers
    (`check_far_session`, `stderr_tail`, `exec`'s failures) no longer escape; `refusal_line`, `remote_error` for a core
    route (per line, real newlines kept, `\r` escaped) and core's state in `machine status --core` do. A saved-SSH
    machine's `remote_error` is the master text, byte for byte (test: a 3-line ssh stderr with a CR).
  - M5 route check: `target.json` records the `--via` command line `machine add` gave core (`Target.via`, new optional
    field); core's record must carry exactly it (this herdr's path and `core-link <name>`). A target written before
    has no `via` and is checked for the `core-link <name>` end only. Core's `machine.list` does send `session` (`hc
    schema` MachineView: "the hc session there, null: its default"), so a record with no session is the default
    session, and it must equal the target's session (before: a missing session matched any).
  - M6: a request with no readable `id` is refused before it is sent, and no reply matches a request with no id
    (`request_id`; before: any reply passed).
  - The bridge folder is made under the canonical base that `is_trusted_base` checked (`trusted_base`), so a
    symlinked TMPDIR is not followed after the check.
  Gates, m4m, 2026-11-23, under the shared lock: targeted tests green, the 12 new or changed tests failing together
  with their fixes reverted; `just check` 0 on 88af72b3 (5888 + 158 tests; windows-lint skipped on macOS);
  `CORE_MACHINE_PARTS="link lease fleet" core_machine_live_test.sh` 0 on a debug binary built from 88af72b3 and
  copied out of the shared target, with shortened waits (`V3_SECS=150`, `V38_SECS=300`).
- [ ] E.3 View actions (new tab, split, close pane, rename, agent start) as argv `machine.exec` calls; V20
  Waiting for PR D (the view server, `issue-300-d-view-server`): the actions belong to a view, which D builds. E.1's
  probe and `fleet.rs`'s `exec` and wrapper are what they will call.
- [x] E.4 `docs/next`: core machines, dormancy, `machine status` gates, the announcement rule (S4a, S4b:
  `--dial` and `wake` are announced dials), that agents' own `hc --on` on a dormant machine fails, the fallback
  Done: `connecting-machines.mdx` § Herdr Core machines (add from a terminal, no install by herdr, every dial a
  fresh login, never starting the far core and the VM `nc` limit; Dormancy with each lease holder; Status and the
  gate, a table of the seven gates and what reopens each; `--machine` to a core machine, connected, refused with
  75, `--dial`, the far session, the probe, no retry, 8 MiB; Announce a connection first; `hc exec/run --on` and
  `hc machine connect` on a dormant machine); `cli-reference.mdx` § Herdr Core machines (the verbs and `--dial`).
- [ ] E.5 Check: `just check` 0, `just core-live-test` 0, CI green
  Local, m4m, 2026-11-23, under the shared lock, on the code of this branch's head (only tasks.md changed after):
  `just check` 0 (5821 + 158 tests; windows-lint skipped on macOS as the justfile says). `core_machine_live_test.sh`
  with `CORE_MACHINE_PARTS="link fleet"` (the new part switch), a copied debug binary: V26, PR A/B's link part (V1,
  V8, V27, V28, V31, V33, V34, V35, V36, M2) and the fleet part (E.1, V18, V31 exec part, V32, `--dial` over
  `auth_failed`) passed; nothing left running or in the passwd-home state. Not run: the panes and lease parts
  (V3, V38, V11 take 5, 10 and 15 min and nothing in them changed), `core_live_test.sh`, the default-session
  script, the bench (no render, layout, PTY or detection work added) and CI (not pushed).
  After E.2a, m4m, 2026-11-23, under the shared lock: `just check` 0 (5838 + 158 tests; windows-lint skipped on
  macOS) on 41095792; `core_machine_live_test.sh` with `CORE_MACHINE_PARTS="link lease fleet"` (default V3 300 s
  and V38 600 s) passed on a debug binary of 681ccb49 copied out of the shared target (41095792 changes scripts
  only): the link part, V2, V7, V3 (0 link bytes), V38, B.2/B.5, V33, V35 and the fleet part with E.2a's 2.1 and 2.2
  checks. Earlier runs failed on test races, fixed in the scripts: V35's stale-socket dial (core's own retry
  restarted by the test's connect, also with the base binary), B.5's order check (a Windows START counted as the
  client's dial), V32's `--dial` landing on core's fast retries after a link ends (D3a's accepted limit). One V3
  failure and one V38 failure were another session's debug client on this account (pid 26192), whose `client-<pid>`
  lease writer (B.2) writes a fresh act into every machine folder under the passwd home's `herdr-dev` root, these
  tests' folders included: it kept a link up in V3 and reopened `retry_spent` in V38. Not fixed here (B.2's
  documented limit); the live tests share that root with every herdr-dev client of the account.

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
