# Core phase 5: design

Phase 5 of the herdr-core migration (`openspec/changes/herdr-core-client/design.md` § 6.4 item 5, "Remote
through core machines"). Issue: https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/300. Drafted
2026-11-22, revised the same day after the roundtable: its twelve findings F1-F12 are answered in the decisions,
each with a Verifier row (§ Roundtable findings lists where), and the hc behaviour each answer relies on was
probed (`probe/README.md` § Roundtable probes, R1-R10). No code yet.

## Context

### What herdr-max's remote is today

Two surfaces, both in `src/remote/` and `src/client/endpoint/` (read 2026-11-22):

- **`herdr --remote <host>`** runs one TUI client against one remote herdr server. ac runs it as a tab on m4m
  (`herdr-beta --remote ac-mbm5`). The target goes to OpenSSH unchanged, through a managed config that only
  `Include`s the user's (`attach.rs` 3224-3263), with `-S /tmp/<hash> -o ControlMaster=auto -o ControlPersist=600
  -T` (`attach.rs` 1466-1486). herdr binds a private local socket and runs one `ssh ... <bin> remote-client-bridge`
  per accepted connection (`attach.rs` 3093-3163); the bridge splices stdio to the remote server's client socket
  (`host.rs`). It installs a binary only when no remote candidate is compatible or `HERDR_REMOTE_BINARY` is set
  (`attach.rs` 1567-1630), and live-hands-off the remote server only with `--handoff` (`restart_policy.rs` 39-62,
  163-178). It never reconnects: a lost link ends the client with the reattach command (`client/errors.rs` 61-67).
- **Saved machines** (`herdr machine add`) are extra endpoints in the local TUI, `ClientEndpointId::Ssh(ProfileId)`
  (`client/endpoint.rs` 78). The endpoint supervisor reconnects with a backoff from 0.5 s to 120 s; auth and
  host-key failures become Attention, retried every 30 s (`supervisor.rs`, `saved.rs` 155-180). Background
  attempts use `BatchMode=yes`, `StrictHostKeyChecking=yes`, `ConnectTimeout=10` (`attach.rs` 1452-1464). The
  fleet CLI reaches one with `herdr --machine <label|id> <cmd>` through `ssh <bin> --session S remote-api-bridge`
  (`cli/target.rs` 27-101, `remote/saved.rs` 68-122); a request is never replayed after a failure, except the
  read-only status probe (`target.rs` 117-148). Pane JSON carries no host field.

Remote dormancy (#260, beta 181): after 10 min with no key, text, paste or mouse input on the client
(`client/endpoint/dormancy.rs` `InputClock`, `REMOTE_DORMANT_AFTER`), remote endpoints get no health ping and no
automatic reconnect; the first input probes at once and retries from the bottom of the backoff. herdr adds no ssh
`ServerAliveInterval`. Unix only. Its limit: a `--remote` tab has no herdr pings, so its idle link may drop across
a NAT.

### What herdr-core gives (hc 0.1.21; `docs/machines.md`, `docs/security.md`, `schema/README.md`, `hc schema`)

- `machine.add {target | via, name, session, temporary}` keeps a link from this core to another core; the far side
  runs `herdr-core connect` in the named session. Terminals there are `name/t7` everywhere on this core: `run --on`,
  `exec --on`, `file --on`, `attach`, `read`, events, `doc.get {on}`.
- `machine.connect` dials now. There is no pause, idle policy or keepalive setting.
- Lost links are retried; an SSH auth failure is `needs_login`, retried after 2, 5, then every 10 min; `hc machine
  login` needs a terminal.
- `machine add --install` and `machine update` copy hc there and restart that core.
- Adding a machine gives every client of this core that machine's account (`security.md` § Remote trust).

### Measured for this change (`probe/README.md`, local `--via` loop, scratch sessions)

- A connected link polls **every 3 s** (about 70 bytes out, 283-457 back) plus a 15 s heartbeat, with no client
  and no viewer: about 1 400 round trips an hour (P2). #260's 5 s herdr ping was enough to keep ac-mbm5 in dark
  wakes, so an always-on core link would undo #260.
- A via command that fails at once is retried after 0.3 s doubling to a **30 s cap**, all local (P4).
- While unreachable, the machine's terminals leave `terminal.list` and **no event says so** (P5). Wake by
  `machine.connect` is immediate and yields `missed` with a snapshot (P6).
- An attach stream **survives** a 25 s drop: core writes `[line to <m> lost; reconnecting...]` into it, then a
  CAN-led full repaint on reconnect (P7, P8).

### Measured after the roundtable (`probe/README.md` R1-R10)

- A far terminal is in the local `terminal.list` and the `events.subscribe` snapshot with the same labels as a
  local one and no machine field; only the `<machine>/` address prefix tells it apart. `terminal.list` has no
  machine filter (R1).
- Core SIGKILLs a silent via after about 19.8 s, its pid only (F2). Each retry runs the POSIX line and then a
  Windows line 76-100 ms later (R3). A `machine.connect` during a dial ends that dial and starts a new one (R4).
- `machine.connect` blocks its caller for the whole dial, 20-22 s on a silent one (F4).
- A dial through `herdr-core connect` starts a stopped far core under the ssh login (F10); a far-side `nc -U` test
  tells a live socket from a missing or stale one without starting anything (R5).
- Terminals and `machine.exec` on a machine inherit the far core's environment; `env` in `terminal.run` overrides
  but cannot unset; `env -u` in the argv does (R6).
- A typing attach takes the size; `resumed: true` plus AUTOMATIC_INPUT frames types without taking it; FOCUS takes
  it (R7). `size` events reach the local core within 2 ms (R8).
- With `ControlPersist=600` the master outlives the link and is reused without auth (F9). With `ControlMaster=no`
  and `ServerAliveInterval=5`, a frozen link is unreachable after 26 s instead of about 45 s; meanwhile every
  `terminal.list` on the primary costs 3 s (R9, F11).
- `machine.exec` cuts output at 8 MiB with `truncated: true` and exit 0 (F12).

### What phases 2-4 give

- Core panes behind `PaneIo::Core`, one framed connection per core session (`CoreSessions`,
  `core_backend/sessions.rs`), links `core_terminal {session, address}` re-adopted after restart and live handoff
  (core-backend D10, core-phase3 D6). A core that does not answer is waiting, never gone; nothing starts fresh on a
  guess (core-phase3 D1a, D1b, `CoreWaiting`).
- Ownership is the `herdr.session` label by equality, plus `herdr.pane_id` (core-phase3 D5). The release default
  herdr session places panes in core session `default` on both Macs (PR C).
- Pid-based process facts and the close sweep assume the pid is on this host (core-backend D8, D11: "Phase 5
  (remote) must decide again").
- herdr-max publishes its own spaces, tabs and splits into the `herdr-app` document of its primary core, marked
  `herdr_max: 1`, leaves `content.terminal = <address>` (core-spaces-doc D1). Status is mirrored into core as
  OSC 7501 (core-status-mirror).

## Goals and non-goals

Goals:

1. herdr-max on m4m shows and drives ac-mbm5's herdr-max panes through a core machine link.
2. A herdr-max pane can run on a core machine (`--on`), which is the VM harness goal (§ 4d).
3. #260 carries over: no traffic to a machine after 10 min without human input, the first input wakes it.
4. The fleet CLI and JSON API keep their names and fields; pane ids stay herdr's; machine and core address are
   extra optional fields (§ 6.5).
5. The old remote keeps working, unchanged, as the fallback until phase 6.
6. herdr-core is used as shipped. No patch, no feature request by an agent.
7. Every SSH dial is one ac has been told about, and none happens unattended where it can prompt.

Non-goals: deleting `src/remote/` (phase 6); installing herdr-beta over the link; Windows; editing a remote
machine's layout from m4m beyond the verbs in D7; the computer-use host (it stays on the PTY, § 6.5).

## Decisions

### D1. The machine lives in the primary core; herdr adds it, core owns the link

A remote Mac or VM is a core machine of the herdr session's primary core (`default` for the release default
session on both Macs). `herdr machine add --core <target> [--name N] [--remote-session S]` (`--session` is herdr's own global flag) runs `hc machine add` with
herdr's link command (D2) in the person's terminal, so hc's own install question is asked there. Core then owns
the connection, retries, the merged terminal list and `--on`. On ac's Macs: m4m adds `ac-mbm5` (its session
`default`), and ac-mbm5 may add `m4m` the same way.

Alternatives:

- **Carry herdr's client protocol through a core terminal** (a raw-mode bridge program on the far side, attached
  from here). Keeps every remote feature with almost no new code. Rejected: core answers and strips terminal
  queries in program output and injects a repaint on reconnect, so the stream needs armouring and resync; core
  parses and stores the protocol bytes as terminal history on both Macs; the phone shows a garbage terminal; and
  nothing moves toward § 6.2's end state.
- **A dedicated core session for machine links** (`herdr-machines`). Isolates the link from Herdr.app and the
  phone. Rejected: `--on` works only through the core that added the machine, so remote panes (D5) would need a
  second primary, and two links to one Mac double the poll of P2. Sessions are not a security boundary anyway
  (`security.md` § Sessions).
- **Keep herdr's own ssh** (status quo). It stays as the fallback (D8).

### D2. Every dial goes through herdr's link command, which is the dormancy gate

The machine is added with `--via '<herdr bin> core-link <name>'` (absolute path of the installed herdr, never a
versioned path). Core runs it for every dial: add, retries, core start, `machine.connect`, with core's far-side line
as the last argument. `core-link` (prototype: `probe/fix/corelink.py`, R2-R5, R9):

1. **Line kind.** Only core's POSIX line (`sh -c '...'`) is dialled. Core's Windows fallback line, which follows
   every failed POSIX dial (R3), exits 75 at once: far sides are macOS and Linux (Windows is a non-goal). So a retry
   is one dial, not two.
2. **Gate and leases.** A closed gate (D10) or no lease in force (D3): exit 75 at once, start no process, open no
   socket. Core retries locally every 30 s at most (P4).
3. **Single flight.** It takes `dial.lock` (`flock`, waiting at most 2 s, so the check connection of `machine add`
   and core's next dial hand over). The descriptor is inherited by ssh, so the lock lasts exactly as long as the
   link. A second `core-link` that cannot take it in 2 s exits 75 without dialling (F3).
4. **Process shape.** It forks a watcher, then **execs ssh** (D10's options) with core's stdin and stdout, so the
   via pid core holds, and SIGKILLs after about 19.8 s, is ssh itself: nothing can be orphaned, on macOS without a
   pdeathsig (F2; R2: `kill -9` of the via pid left no process). The watcher holds none of core's stdio, only the
   read end of ssh's stderr; it exits when ssh's stderr closes or its parent pid changes, and signals ssh only while
   `getppid()` is still ssh's pid.
5. **Far guard.** The remote command is `sh -c '<guard>' && <core's line>`. The guard finds the far session's
   socket by core's endpoint rules (`XDG_RUNTIME_DIR`, `/run/user/<uid>`, else `~/.local/state/herdr-core/<s>/`),
   tests it with `nc -U -w 2 <socket> </dev/null`, and when that fails prints `herdr-link: herdr-core is not running
   there` and exits 69; otherwise it prints `HERDR-LINK-UP` on stderr. So a dial never starts the far core (F10;
   R5, including the stale socket a killed core leaves). Where the far `nc` has no `-U` (some Linux VM images),
   the guard falls back to `[ -S <socket> ]`, which a stale socket passes: on such a VM a dial after a far core
   crash can start that core under the ssh login. Accepted for VMs, which have no desktop herdr to object (D7 of
   core-phase3 concerns a Mac's own herdr).
6. **Deadline.** No `HERDR-LINK-UP` within 15 s of the start: gate `auth_stalled`, ssh ended (SIGTERM, then
   SIGKILL after 1 s), before core's own kill (R2: gate at 15.0 s, `machine.connect` answered after 16.3 s).
7. **Lease end.** Every 5 s the watcher re-reads the leases; when none is in force it ends ssh. The link closes;
   core marks the machine unreachable and its next retries hit step 2.
8. **Classes.** The watcher reads ssh's stderr for the classes of D10 and writes them to `gate.json`.
9. **Retry budget (herdr's review, 2026-11-22).** With `ControlMaster=no` (F9), every dial authenticates, which can
   mean a Touch ID prompt. So core's own reconnects must never turn a sleeping or flapping far side into a stream of
   prompts. Any dial that ends without a link (an `auth_*` class, a refusal, `far_core_down`, or a drop of a link
   that was up) is written to `gate.json` with its time. After that, `core-link` allows **one** more dial on core's
   own initiative. A second failure closes the gate as `retry_spent`, and every later dial exits 75, until a human
   action gives a lease newer than the gate: input in a view or a far pane (the client lease's mtime, D3), `herdr
   machine wake`, `herdr machine login`, or a CLI call with `--dial` (D7). A lease that was only kept alive by its
   10-minute window, with no new input since the failure, does not reopen it. A link that comes up clears the
   budget. As built (PR B): "comes up" is 5 s up past `HERDR-LINK-UP` (`UP_SETTLE`), so a far side that answers
   the guard and drops at once (a failing far `connect`, core's own check connection of `machine add`) is a
   failure, not a link; a lease file holds `<expires_at> <input_at>`, and "newer than the gate" compares
   `input_at` (the human act), so a renewal that is no new act keeps its old `input_at`. The login classes
   (`needs_check`, `auth_stalled`, `auth_failed`) still close until `herdr machine login`; a second
   `far_core_down` within the budget becomes `retry_spent`.

The link state lives in `<home>/.local/state/herdr/core-machines/<name>/` (mode 0700), where `<home>` is the
account's passwd home (`getpwuid`), never `HOME` or `XDG_STATE_HOME` from the environment core passes: that is the
environment of whoever started the core (F6). It holds `target.json` (the ssh target, options and the
`SSH_AUTH_SOCK` of the person who added it), `leases/`, `dial.lock`, `connect.lock` (D3a) and `gate.json` (the
last refusal or failure and its time). ssh runs with an allowlisted environment (`HOME` from passwd, `USER`,
`LOGNAME`, a fixed `PATH`, `LANG`, `LC_*`, `TMPDIR`) plus `target.json`'s `SSH_AUTH_SOCK`: herdr's own core start
drops `SSH_AUTH_SOCK` (`CORE_START_ENV_KEEP`), and a core someone else started passes theirs.

Alternatives:

- **`machine.rm` on dormancy, `machine.add` on wake.** Rejected: rm drops the machine's terminals from every
  client of the core, each wake is a full add (an install check, two dials, P1), and a rename between them
  changes every address.
- **A plain ssh target plus an upstream idle policy.** Rejected for now: an agent may not file a feature request
  (AGENTS.md § External contributor guardrail). ac may post a Discussion; if core ever grows an idle setting, D2
  shrinks to a plain target.
- **herdr kills the ssh process from outside.** Rejected: core owns the process; finding and signalling it from
  the herdr server races with core's own retries. The lease file is read by the one process that needs it.
- **core-link as ssh's parent, with a guardian holding a pipe or a kqueue `NOTE_EXIT` watch** (the roundtable's
  suggestion for F2). Rejected: it works only while the guardian lives, and core's SIGKILL hits the parent, so
  ssh survives it unless a third process watches; exec makes the pid core kills the one that holds the link.
- **Refuse in core-link when the far core is down, tested from here.** Not possible without a dial: only the far
  side can test its socket, so the guard runs there, after authentication.

### D3. Dormancy is a set of leases, one file per holder

A lease is a file `leases/<holder>` holding an expiry time. The link may be up while any lease is in force. Each
holder writes only its own file (write to a temporary file of its own, then rename), so no two processes write one
file. A crashed holder's lease ends by its expiry. A **keep lease** (`<expiry> 0 keep`, PR E review) keeps a link that
is up and admits no dial: core-link's watcher counts it, its admission does not (D7).

| Holder | Writes | Expiry |
|---|---|---|
| a TUI client (`client-<pid>`), for every core machine | on human input (#260's `InputClock` events: key, text, paste, mouse), at most once a minute | last input + 10 min (`REMOTE_DORMANT_AFTER`) |
| a TUI client started by a person's command (`herdr --remote core:<m>`, `herdr` from a terminal) | also at its start; a client re-created by a reconnect or a live handoff does not | start + 10 min |
| a fleet CLI call through the machine (`cli-<pid>`, D7) | before its `machine.connect` or `machine.exec`, every 30 s while the CLI runs, removed when it ends and on SIGINT, SIGTERM or SIGHUP | last write + 60 s; a keep lease (D7) except for `--dial`'s own connect |
| `herdr machine add --core`, `herdr machine login` (`manual`) | before they call hc (F8: both dial at once) | 30 min |
| `herdr machine wake <name> [--for D]` (`manual`) | before it calls `machine.connect` | the given duration, 30 min by default, at most 12 h |

Not holders: a herdr server, a restore or a live handoff (a 3 a.m. beta install must not wake ac-mbm5), the
phone, Herdr.app and agents' own `hc` calls. They see the machine `unreachable` while it is dormant, which the
docs state. The first input after dormancy (any herdr server receiving input for a pane on that machine, or a view
server getting a client, D6) also calls `machine.connect`, so wake does not wait for core's 30 s retry.

**A lease comes before every connect (F8).** No herdr code calls `machine.connect`, `hc machine add` or `hc
machine login` without a lease in force for that machine, written by itself or checked: without one, core-link
refuses (exit 75) and the caller would report a failure for a machine that is only dormant. The wake path is
"input → the client writes its lease → the server or view server is told → D3a's dialer calls `machine.connect`";
a view server whose first client has not written a lease shows "dormant, press a key to wake" and dials nothing.

### D3a. `machine.connect` is a background job, one at a time (F3, F4)

`machine.connect` blocks its caller for the whole dial, 20-22 s on a silent one (F4), and a second call during a
dial makes core end that dial and start a new one, each a fresh ssh authentication (R4). So:

- Each herdr process (primary server, view server, CLI) has one dialer per machine: a thread that owns every
  `machine.connect` call. Input, render and API handlers only post a wake request to it (`try_send` on a bounded
  channel of one; a full channel means a wake is already pending). No `machine.connect` runs on an input, key,
  render or API thread.
- Across processes the dialer takes `connect.lock` (`flock`, non-blocking) around the call. When another herdr
  process holds it, the dialer does not call; it polls `machine.list` every 1 s until the state leaves
  `connecting`, at most 20 s. So herdr makes one `machine.connect` per wake however many panes, clients or CLI calls
  want it.
- The call's timeout is 20 s (core-link's own deadline is 15 s, D2 step 6). The outcome goes back as an event; the
  pane or view shows "waking <machine>" meanwhile, and keys typed are dropped (D5).
- Accepted limit: other core clients (an agent's `hc machine connect`, Herdr.app) can still restart a dial in
  flight; core-link's `dial.lock` keeps that to one ssh at a time, and core ends the old one before it starts the
  new one (R4).

#260's own rules stay: the client's health pings and endpoint reconnects still follow `InputClock`, and the
client writes its lease from the same clock, so one clock decides both.

Alternatives: one shared lease file (rejected: several clients and CLI calls write it at once); the herdr server
as the only holder (rejected: it does not see input that goes to a view server, and it must not hold the link up
by being alive).

### D4. Identity and addresses

- The machine's name is core's name for it (`ac-mbm5`), the first part of every address (`ac-mbm5/t7`). herdr
  never calls `machine.rename`; a person who does makes links to the old name unreachable, and those panes wait.
- Pane ids stay herdr's. A pane herdr-max owns on a machine (D5) keeps a local id (`w3:p2`). A viewed pane (D6)
  keeps the id its own herdr gave it (`w1:p1`), scoped by the endpoint as today's saved machines are.
- Pane JSON and `pane.get` gain two optional fields: `machine` (absent for this Mac) and `core_address` (the full
  address as this core names it). Nothing existing changes meaning.
- A link is `{session, address}` as today, with the address qualified (`ac-mbm5/t7`) and the session the local
  primary that added the machine. A session save writes it as `core_machine_terminal` (with the folder there),
  never as `core_terminal`, so a build from before D5 does not take it for a local link (D5).
- **Local scans see only local terminals (F1).** Once a machine is added, every `terminal.list` and
  `events.subscribe` answer of the primary also lists the far core's terminals, with the far herdr's own labels
  (`herdr.session=herdr`, `herdr.pane_id=w1:p1`, the same values as ours) and the far pids, and nothing but the
  address prefix marks them (R1; core offers no machine filter). So every address herdr reads from core is parsed
  at the boundary into one type, `CoreAddress { machine: Option<MachineName>, terminal: String }` (`t7` → local,
  `ac-mbm5/t7` → that machine), and each scan states which host it wants: the restore label search
  (`find_labelled_terminals`), the waiting-pane search (`attach_waiting_pane` → `find_pane_core_terminals`), the
  orphan scan (`unclaimed_in_session`), the start recovery by launch label (`pane_io.rs` `terminal.list`), and every
  future subscriber keep **local addresses only** for local panes; D5's scans keep only their machine's. A pid from
  a record with a machine is never used for a local fact or a sweep. This ships first, alone (PR A0, before any
  `machine add` can exist), because ac's primary core already gains ac-mbm5's terminals the moment anything adds
  the machine, including `hc machine add` typed by hand.

### D5. A herdr-max pane on a core machine

`terminal.run` with `on: <machine>` from the primary connection. What changes against a local core pane, as one
type: `CorePaneIo` gains `host: Host`, `enum Host { Local, Machine(MachineName) }`, and every pid- or
host-dependent call matches on it.

- **Ownership label.** `herdr.session = "<label>@<primary core id>"` (core id from `hello.core.id`), plus
  `herdr.pane_id`. Every herdr build matches `herdr.session` by equality, so ac-mbm5's own herdr (label `herdr`)
  never claims, lists as orphan or kills a terminal m4m placed there, with no build of ac-mbm5 changed first. The
  orphan scan runs per machine with that value.
- **Environment.** The integration variables (`HERDR_ENV`, `HERDR_PANE_ID`, `HERDR_TAB_ID`, `HERDR_WORKSPACE_ID`,
  `HERDR_SOCKET_PATH`, `HERDR_CLIENT_SOCKET_PATH`) are not passed, **and are removed** from what the far core
  passes: a terminal on a machine inherits the far core's environment, and `terminal.run`'s `env` can override a
  variable but not unset it (F6). A far core started by ac-mbm5's own herdr has none of them (herdr starts its core
  with `CORE_START_ENV_KEEP` only), but one started by a bare `hc` inside a herdr pane there carries that pane's.
  So the argv on a machine is always `["env", "-u", "HERDR_ENV", "-u", "HERDR_PANE_ID", "-u", "HERDR_TAB_ID", "-u",
  "HERDR_WORKSPACE_ID", "-u", "HERDR_SOCKET_PATH", "-u", "HERDR_CLIENT_SOCKET_PATH", "--", <argv>]`; a shell line
  becomes `<that> sh -c <line>`, and the machine's own shell `<that> /bin/sh -c 'exec "${SHELL:-/bin/sh}" -l'`
  (R6: the unwrapped program saw five fake `HERDR_*` names, the wrapped one none; macOS and GNU `env` both take
  `-u` and `--`). The same wrapper goes on every `machine.exec` argv (D7). On the far side the `herdr` on `PATH` is
  that Mac's own server, and a hook report with m4m's pane id would land on a pane of ac-mbm5 with the same id;
  without `HERDR_PANE_ID`, a far-side `herdr pane report-agent` (or any verb that defaults to "this pane") resolves
  to no pane and fails, as it does outside herdr.
  Status comes from OSC 7501 (Claude Code and Codex report it, § 4b) and from herdr's screen detection on the
  bytes; hook-only signals are absent for remote panes, which the docs state.
- **Process facts.** For `Host::Machine` the pid is the far side's: pid-based facts answer `None`, the close sweep
  (core-backend D8) does not run, and core's `foreground` record (argv, cwd) is the only process fact.
- **Close** is `terminal.kill`. Detached jobs of the program survive on the far side, as core's own kill leaves
  them. A remote sweep through `machine.exec` was rejected for phase 5: it needs the far side's session and holder
  pids for core-backend D8's guard, which `server.status` does not give for another machine.
- **Only a person's close ends a far program (decision, PR C, 2026-11-22).** A `herdr server stop`, a logout or a
  restart on this Mac leaves programs on core machines running, and the saved link attaches them again when a
  server starts; local core panes keep core-backend D10's stop behaviour. A pane on a machine ends only when core
  reports its terminal `finished`; `not_found`, a label mismatch or a drop make it wait. Reason: a stop or logout
  on one Mac must not kill agents running on the other, and an answer that may only mean "not listed yet" must
  not end one either (herdr, lead, OK on 671bd81d).
- **Restore and live handoff** reattach by (session, address) as today. When the machine is not `connected`, the
  pane is `CoreWaiting` (core-phase3 A1.3) with the cause `machine_dormant` or `machine_unreachable`, never started
  fresh, and the reattach is tried again when `machine.list` says `connected`. The server reads `machine.list`
  every 5 s while it has a pane on a machine and some lease is in force; P5 showed no event for a loss. Only
  positive evidence ends such a pane: a record with its labels that core reports `finished`. A `not_found`, or
  a record labelled for another pane, is never final (core can list a machine `connected` before its
  terminals); the pane waits (`machine_terminal_missing`) and is asked again. Nothing automatic sends
  `terminal.kill` to a machine: not an end, a lost stream, a stop of either core, or a stop of this server;
  only a person's close of the pane does (tasks C.8).
- **Far output is far data (security review of PR C, M1; tasks C.9).** A machine pane's terminal takes inline
  kitty graphics only, drops OSC 52 clipboard writes, and keeps the folder its program reports (OSC 7), and a
  request's `--cwd`, as far metadata on its link (`machine_cwd`), never as this Mac's cwd, a git root, the
  workspace's identity or a local split's folder.
- **Launch label (M2).** The link saves the run's `herdr.launch`; attach, the live reattach loop, an end and a
  kill take only a terminal shown to carry the owner value and that launch. Anything else waits.
- **In-band notices.** Core's `[line to <m> lost; reconnecting...]` and the CAN-led repaint (P8) go through the
  parser like any output. The CAN repaint resets the screen; no filtering.
- **Input while not connected** is not sent. The first key posts a wake to the dialer (D3, D3a) and the pane shows
  "waking <machine>" until the reattach; keys typed before it are dropped, so nothing lands on a stale screen.
- **API and CLI.** `pane.split`, `tab.create`, `workspace.create` and `agent.start` take an optional `on`. The
  server advertises the capability `spawn_on_machine`; the CLI sends `on` only when it is advertised, because an
  older server would ignore it and start the pane locally while reporting success (AGENTS.md § Stable client
  endpoint contract). The CLI flag is `--on <machine>`.

Alternative for the label: a new `herdr.owner=<core id>` label beside `herdr.session`. Rejected for phase 5: every
herdr build on every Mac would have to learn it before m4m places a pane on ac-mbm5, or ac-mbm5 reads the pane as
its own. The `@` value needs no order.

### D6. Viewing another Mac's herdr: a view server per machine

To show and drive ac-mbm5's own panes, m4m runs `herdr server --view <machine>`: the herdr binary in a second role,
one process per viewed machine, with its own client socket. The TUI connects to it as an endpoint
(`ClientEndpointId::Core(MachineName)`), so the sidebar, the endpoint supervisor, #260's client dormancy and the
generation 1 endpoint protocol are reused unchanged, and a fault in the view cannot take down the primary server.
`herdr --remote core:<machine>` opens a client on it, which is the tab ac uses today.

The role is one type, `enum ServerRole { Primary, View(MachineName) }`, read at the boundaries that differ:

| Concern | Primary | View |
|---|---|---|
| Layout | `session.json` | the far side's `herdr-app` document (`doc.get {name: "herdr-app", on}`), spaces with `herdr_max: 1` only, re-read on its `doc` event, on `missed`, and on wake |
| Panes | spawned or restored | one per terminal leaf; ids from the document's block ids |
| Status | detection, hooks, 7501 | the terminal's 7501 record from core (ac-mbm5's herdr mirrors its verdict there); no detection |
| Persistence, orphan scan, spaces publisher, status mirror | yes | none |
| JSON API socket | yes | none; the fleet uses `--machine` (D7) |
| Close | ends the program | detaches; the program and the far side's pane stay |
| Input | DATA frames | AUTOMATIC_INPUT frames only, never DATA or FOCUS (Size below) |
| Wake | — | only with a lease in force (D3); `machine.connect` through D3a's dialer |
| Lifetime | as today | started on demand by the endpoint transport; exits 10 min after its last client; stateless, so a herdr update needs no handoff: a client reconnect with a new version restarts it |

**Size (F5).** Two herdr servers now view one terminal, and core gives the size to the active viewer. The
roundtable measured the first draft's follow-size (attach at the far size, echo every `size` event) to fail three
ways: the attach itself resized, the first key snapped the far terminal back to a stale size, and two viewers
echoing each other produced 149 000 size events in 12 s. The view instead never becomes an active viewer
(R7, R8, `probe/fix/sizeprobe.py`, `echoprobe.py`):

- It attaches with `resumed: true` ("sets the size only once active again") at the far terminal's current
  `cols`/`rows` from `terminal.get` just before, and checks that the attach result says `automatic_input: true`
  (else it attaches `observe: true`, read-only, and says so in the pane border).
- It sends every key, paste and mouse report as AUTOMATIC_INPUT frames (kind 6). Core delivers them to the program
  and does not make the view active: in R7 the far terminal kept 100x30 and followed the far herdr to 90x25 while
  the view typed. It never sends DATA (the first DATA key resized the far terminal to the view's 160x50) and never
  sends FOCUS (one FOCUS frame did the same).
- It learns the size from `size` events of `events.subscribe {terminal: <address>}` on the local core (within 2 ms
  of the far resize, R8) and answers a size **different from the last one it sent** with a RESIZE of exactly that
  size. Core records it without applying it, so no new event follows (no storm). The recorded size matters once:
  when the far herdr detaches (its server restarts or hands off), core applies the view's recorded size, which
  is then the far one (R8: 120x40 kept; without the echo, the view's stale attach size won).
- An idle view sends nothing but CREDIT for bytes it receives.
- The m4m pane draws the remote grid at its own size, cropped or padded, with the remote size in the pane border
  when they differ. A "fit here" action that resizes the remote terminal is out of scope for phase 5.

Accepted limits, with their cost:

- Core takes AUTOMATIC_INPUT as no person's typing, so `terminal.send`'s "wait until the person sends or clears
  their draft" does not see ac typing in the view: an agent's `hc send` to that terminal can land in the middle of
  his line. ac-mbm5's own herdr shows the same pane, so its typing there is still seen.
- Programs get no focus reports from the view.
- If the far terminal resized while the link was down, the view's recorded size is stale until the `missed`
  snapshot after the reconnect (P6) gives the new size, which it echoes; a far herdr detach inside that window
  applies the stale size until the far herdr attaches again (R8: its attach sets the size at once).

Panes on the far side that are not in core (PTY panes, `core_pty_panes`) are not in the document and are not shown;
the old remote still shows them.

Alternatives:

- **Viewed panes inside the primary server's `AppState`.** No new process. Rejected: every fleet verb, the
  persistence, the publisher and the orphan scan would need a "foreign pane" exception, a bug there reaches ac's
  main server, and the TUI would need new protocol fields for a second machine's workspaces.
- **A plain `hc attach` per pane** (no herdr layout). Rejected: it shows one terminal, not ac-mbm5's workspaces.

### D7. The fleet CLI and structural actions over the link

- `herdr --machine <name> <cmd>` resolves a saved herdr machine first, unchanged. When the name is a core machine
  only (or `[remote] prefer = "core"`, D8), the request goes through `machine.exec {on, argv: [<remote herdr>,
  "remote-api-bridge", "--session", S], input}`: the same request bytes `remote-api-bridge` reads from ssh today,
  the same reply, so the output is the far side's own, with its pane ids. The command list of `target.rs` 291-320
  is unchanged. A request is never retried once sent, as today. `exec`'s timeout is the call's own (default 60 s,
  at most 600 s); streaming verbs stay unavailable as today. The argv carries D5's `env -u` wrapper (F6).
- **`machine.exec` never dials (F7).** On a machine that is not `connected` it fails at once (`unavailable`, R10),
  and a lease alone would only let core's next background retry dial. So the CLI decides the dial itself:
  - `connected`: write the `cli-<pid>` lease as a keep lease, `machine.exec`, remove the lease. No new ssh, and
    none after a drop during the call: core's retry meets no lease that admits a dial on the call's account (a
    lease of any other holder still admits it, as it would without the call).
  - not connected, without `--dial`: refuse with exit 75 and `ac-mbm5 is dormant; reaching it opens ssh (a Touch
    ID or Tailscale check can prompt). Announce it, then rerun with --dial, or run herdr machine wake ac-mbm5.` No
    lease is written, nothing dials.
  - not connected, with `--dial`: refuse when the gate is closed (D10, naming the class and `herdr machine
    login`) or the far core was found down within 5 min (`far_core_down`, or `retry_spent` over it); else write
    the lease (one that admits a dial, with the act), post a wake to D3a's dialer and wait for it (20 s at most),
    turn the lease into a keep lease, then `machine.exec`. The one announced dial is the connect; a drop after it
    is not redialled on the call's account.
  This is stricter than the old remote, whose `--machine` dials silently on every call; the house rule wants
  every dial announced, and an agent cannot pass `--dial` without deciding to. There is no environment switch for
  it (PR E review: `HERDR_MACHINE_DIAL=1` let every agent of a shell dial without deciding, and was removed).
- **The lease's life (PR E review).** The `cli-<pid>` lease lives 60 s past its last write and the CLI writes it
  every 30 s while it runs, so its life follows the process: SIGINT, SIGTERM and SIGHUP remove it (and the bridge
  folder) before the CLI ends as the signal's default does, and a SIGKILL leaves one that ends within a minute (the
  watcher ends the link 5 s later at most). This replaces "120 s, longer by its own timeout", up to 720 s after a
  killed CLI. A long call writes twice a minute, not D3's once a minute for clients: the client rule bounds writes
  per input, and a CLI's renewals are one small file per 30 s for its own life only.
- **Cut and timed-out replies are errors (F12).** `machine.exec` cuts stdout at 8 MiB with `truncated: true` and
  exit code 0 (roundtable: 9 000 000 bytes → 8 388 608). The CLI treats `truncated: true` or `timed_out: true` as
  a failure: exit 1, `the reply from <m> was cut at 8 MiB` (or `timed out after <n> s`), and prints none of the
  partial output as a result. A request over 8 MiB raw (8 388 608 bytes: core's own input limit, measured; its
  schema says "8 MB") is refused before it is sent. Every failure after sending says the request may have run:
  no answer, `timed_out`, a far exit other than 0, and core's `unavailable`, which core also answers for a link lost
  during the run (`lost the line to <m> during the call`, hc 0.1.21).
- **Timeouts.** The status probe's run is 9 s and its core call 14 s, inside the CLI's 15 s probe; a request with
  `timeout_ms` runs that long and 10 s more (at least 60 s); one with none (a wait with no `--timeout`, which the
  old remote waits out however long) runs core's longest, 600 s, then fails as timed out. A `timeout_ms` over
  590 000 is refused before it is sent. A CLI that leaves with a request in flight does not wait for it.
- In a view (D6), new tab, split, close pane, rename and agent start become the same `machine.exec` calls of the far
  side's CLI, argv only, never a shell line built from names. The far side's herdr changes its layout and
  republishes; the view follows the document.
- The remote herdr path and its capabilities are read once per link establishment (`machine.exec` of `<bin> status
  client --json`, the same probe `attach.rs` uses), again when the answer is 5 min old (so a far herdr upgraded in
  place is read anew) and when the far herdr exits 2, 126 or 127 (no `remote-api-bridge`, or gone). A missing verb
  disables that action with a notice; it never disconnects the view. The probe asks the far herdr for the session
  the requests go to, and `status client --json` reports `core_session`, the core session that herdr session uses:
  it must be the one the machine was added with, so a release `herdr` there is never sent a debug build's
  `herdrdev-X` session's requests (an older far herdr that does not report it is taken for a release build).

### D8. Fallback and selection

- The old remote is untouched: `herdr --remote <host>`, saved herdr machines and `--machine` to them work as in
  today's betas. `src/remote/` gains no code in this phase except shared helpers D10 reuses.
- The new path is chosen explicitly: `--remote core:<machine>`, an endpoint added with `herdr machine add --core`,
  or `--machine` to a name that is only a core machine.
- `[remote] prefer = "ssh" | "core"`, default `"ssh"`, decides `--machine <name>` when a saved herdr machine and a
  core machine share the name. Phase 6 flips the default and then deletes the ssh path.
- When the TUI has a saved herdr machine and a core view for the same far-side core id, it shows one notice per
  run naming both, and shows both.

### D9. Installing and updating hc and herdr

- herdr never runs `hc update`, `hc machine update` or `machine add --install`. `herdr machine add --core` runs
  `hc machine add` in the person's terminal, where hc asks before copying itself. Unattended, it refuses with the
  command to run.
- After each link establishment the server reads the machine's `version` from `machine.list`; below 0.1.21 it raises
  one notice with `hc machine update <name>` and what that does (restarts Herdr Core there; its terminals keep
  running; that Mac's herdr re-adopts its panes per core-phase3 D3).
- herdr-beta on the far side is installed by the house's beta install, as today (both Macs in one step). A view
  needs only the document; D7's actions need the far side's CLI and check its capabilities first.
- Correction to the house memory "remote attach upgrades mbm5": the old `--remote` installs only when no remote
  binary is compatible or `HERDR_REMOTE_BINARY` is set, and hands off only with `--handoff`
  (`attach.rs` 1567-1630, `restart_policy.rs` 39-62). Phase 5 does not carry that installer into the core path.

### D10. SSH dials, prompts and announcements

`core-link`'s ssh argv is the old remote's managed config and non-interactive options with the master removed
and keepalives added: `-C -F <managed config>` (`attach.rs` 3224-3263), `apply_noninteractive_ssh_options`
(`BatchMode=yes`, `NumberOfPasswordPrompts=0`, `StrictHostKeyChecking=yes`, `ConnectTimeout=10`,
`ConnectionAttempts=1`, `attach.rs` 1452-1464), then **`-o ControlMaster=no -o ControlPath=none -o
ServerAliveInterval=5 -o ServerAliveCountMax=2`**, plus `-o LogLevel=INFO` (task A.1: OpenSSH prints an auth
banner only at INFO or above, and a user config may set less), then `-T <target> <guard && line>` (D2 step 5).
A machine added with `--ssh-config <file>` reads that file instead of the managed config (the loopback tests use it).

- **No master (F9).** With the old remote's `ControlMaster=auto` and `ControlPersist=600`, the master outlived the
  link by ten minutes and a new ssh reused it with no authentication, so the dormancy gate did not bound access.
  With `ControlMaster=no` nothing outlives ssh (R9: no control socket), and the authenticated window is the link's
  life, which the leases bound. Cost: the link and an old-remote tab no longer share one authentication, and every
  dial authenticates; with the 1Password agent a dial can ask for Touch ID when its approval for the app has
  lapsed (open question 3). Dials happen on wake, which follows a person's input, or while a lease holds; an
  unanswered prompt becomes `auth_stalled` in 15 s.
- **Keepalives (F11).** A frozen link (sleeping Mac, dead NAT entry) stayed `connected` about 45 s, and every
  `terminal.list` on the primary core cost 3 s meanwhile, for every client of that core. With the keepalives ssh
  ends 15 s after the far side goes silent; core's redial then ends at `ConnectTimeout` (R9: unreachable at 26 s,
  `terminal.list` 3.00 s per call until then, 0.00 s after). Accepted cost: up to about 25 s of 3 s calls per
  freeze, so herdr's own 5 s timeouts on the primary hold. A healthy link sends no keepalive: ssh sends one only
  after 5 s with no data from the server, and core's 3 s poll answers (P2) keep it from firing, so #260's idle
  count is unchanged.

- **No forwarding, no sent variables (security review M2, PR B).** Both the link and `herdr machine login` pin
  `-o ForwardAgent=no -o ForwardX11=no -o ClearAllForwardings=yes -o PermitLocalCommand=no`; ssh keeps the first
  value it obtains and the command line comes first, so no `Host` block or included file turns them back on.
  `SetEnv` also keeps its first value, and OpenSSH 10.5 refuses an empty one (`no argument after keyword
  "setenv"`), so the argv pins an inert `-o SetEnv=HERDR_LINK=1` in its place (a far sshd drops it unless its
  `AcceptEnv` names it). `SendEnv` accumulates instead, and a command-line `SendEnv=-*` is applied before the
  config adds names, so the link's own `ssh_config` (an `Include` of the user's, the system's, or the
  `--ssh-config` file) ends with `Host *` / `SendEnv -*`. Checked with `ssh -G` (unit) and against the loopback
  server, which logs every forward and env request (loop).
- **Check-free paths only (security review M3; rollout policy).** Background dials (S2: core start, retries, a
  wake through D3a) may go only to a target whose ssh path has no Tailscale SSH `check` rule: an `accept` rule for
  ac to ac between ac's own devices, or OpenSSH over the tailnet. A target whose path has a check rule gets no
  background dials: it is reached only through `herdr machine login` or a wake a person announced, and is not
  added as a core machine until it has a check-free path. The R.2 announcement names each target, its ssh path
  and whether that path has a check rule (task R.2's precondition).

Failure classes `core-link` records in `gate.json`, each closing the gate to every holder but `herdr machine login`
until a person acts (`far_core_down` only for 5 min):

| Class | Seen as | Why it closes the gate |
|---|---|---|
| `needs_check` | stderr contains a Tailscale SSH check banner: `# Tailscale SSH requires an additional check.` then `# To authenticate, visit: https://login.tailscale.com/a/<code>` (task A.1; core-link matches either line) | each redial can open a new sign-in request in ac's browser |
| `auth_stalled` | no `HERDR-LINK-UP` on stderr 15 s after ssh started (D2 step 6; core kills the via at about 19.8 s) | an unanswered 1Password Touch ID or another prompt on this Mac |
| `auth_failed` | ssh exit 255 with `Permission denied` or a host-key error | redials cannot succeed |
| `far_core_down` (timed, 5 min, then open) | the guard's `herdr-link: herdr-core is not running there`, exit 69 (D2 step 5) | a dial authenticates before the guard runs; without the pause every retry while a lease holds would be a new authentication. It never starts the far core; ac-mbm5's herdr starts it from the desktop (core-phase3 D7) and `herdr machine wake` reopens the gate at once |

`herdr machine login <name>` runs the ssh in the person's terminal (so the check, the Touch ID or the host key is
answered there), clears the gate and calls `machine.connect`. `herdr machine status [--json]` reports, per core
machine: `state` (core's), `gate` (`open`, `dormant`, `needs_check`, `auth_stalled`, `auth_failed`,
`far_core_down`), the lease holders and their expiry, the last dial time. Agents read this to announce before acting, and the TUI shows the
gate in the endpoint row.

**The check banner (task A.1, read from public source and reports, no dial).** Tailscale's SSH server sends a
`check` rule's hold as an SSH **auth banner** during user authentication, before any session:
`ssh/tailssh/tailssh.go`, `(*conn).clientAuth`, the `HoldAndDelegate` branch, `SendAuthBanner(action.Message)`.
The text is the control server's message, not a constant in the client source; as users report it (paramiko
issue 2425), it is `# Tailscale SSH requires an additional check.` and `# To authenticate, visit:
https://login.tailscale.com/a/<code>`, and after approval `# Authentication checked with Tailscale SSH.`. OpenSSH
prints an auth banner on **stderr**, only when its log level is INFO or above (`sshconnect2.c`,
`input_userauth_banner`), hence `LogLevel=INFO` in the argv. `ConnectTimeout` does not bound the hold: TCP and the
key exchange are done, and the hold lasts until approval or Tailscale's own limit, so the banner arrives within a
second or two of the dial and the watcher ends ssh then; the 15 s deadline (`auth_stalled`) covers a banner whose
text changes. Because the message comes from the control server, the match is on text Tailscale may change: a
changed banner degrades to `auth_stalled`, which also closes the gate.

Every path that opens an SSH connection, and when it must be announced (house rule
`no-unannounced-tailscale-ssh-check`; ac's Mac-to-Mac ssh goes through the 1Password agent, CONTEXT
`PATTERNS/environment.md`):

| # | Path | Who starts it | Prompt risk | Announcement |
|---|---|---|---|---|
| S1 | `herdr machine add --core <target>` (two dials, P1) | a person, in a terminal | 1Password Touch ID on this Mac; a Tailscale check if the target has a check rule | before the first add on each pair of Macs and for each VM: "herdr needs ssh ac@<target> now, to add it as a core machine" through the gestore desk |
| S2 | `core-link` while a lease holds: core start, retries, wake through D3a | core, in the background, after a person's input or an explicit wake | every dial authenticates (no master, F9): 1Password Touch ID when its approval lapsed; a Tailscale check if the target has a check rule | covered by S1's announcement, stated in it; a prompt nobody answers becomes `auth_stalled` in 15 s and a check banner `needs_check`, so no unattended run waits on it and none repeats it |
| S3 | `herdr machine login <name>` | a person | as S1 | the person is at the terminal; agents ask through the desk first |
| S4 | `--machine` (D7), `hc exec/file/run --on` on a **connected** machine | rides the existing link | none: no new ssh | none |
| S4a | `--machine` on a dormant machine | refused without `--dial` (D7) | none | none: the refusal names the dial |
| S4b | `--machine --dial`, `herdr machine wake` | a person or an agent, explicitly | as S2 | an agent announces before it passes `--dial` or runs `wake`, through the desk, as for S1; a person at the terminal needs none |
| S5 | the old remote | unchanged | unchanged | unchanged |

Agents' own `hc machine connect`, `hc exec --on` and `hc run --on` on a dormant machine are core's, not herdr's:
the first fails at core-link's gate unless a lease holds, the others fail at once (R10). The docs tell agents to
go through `herdr --machine` or `herdr machine wake`, which apply the rule above.

Live tests (verifier) dial nothing: they use a local `--via` loop as `probe/` does, with a fake `ssh` first on
`PATH` that runs the line in a scratch core, or real OpenSSH to a loopback-only server (`probe/fix/sshd_fake.py`). The first real add (task R.2) waits for ac's acknowledgement of the
S1 announcement.

### D11. Security surface

- **The link widens who reaches the other Mac.** While a lease holds, every process that can open the primary core
  socket (agents on m4m, Herdr.app, the phone through m4m's core) can run commands on ac-mbm5 with no per-connection
  Touch ID. Today the 1Password prompt gates each new ssh master. D3's leases bound the window to ten minutes after
  human input, and with no ssh master (D10, F9) nothing authenticated outlives the link. The security agent
  reviews this before task R.2.
- herdr's own API socket reaches the far machine too: whoever can open it can start a program there with
  `pane.split --on` while the link is up (security review of PR C, M8). R.2's announcement names it.
- The link never starts the far core (D2 step 5, F10), so the far core's environment and its parent stay what
  ac-mbm5's own herdr gave them; panes and `machine.exec` there still strip `HERDR_*` (D5, F6).
- `core-link` reads only its own state directory, found from the passwd home, never from the environment core
  passes (F6); leases are timestamps, never commands. Its argv comes from `target.json`, written by `herdr machine
  add --core`, never from the far side; the only part from core is core's own line, appended after the guard.
- The far side's `herdr-app` document is untrusted input to the view: parsed at the boundary with size, depth and
  id-format limits, unknown fields ignored, a bad document leaves the last good layout and logs once.
- D7 sends argv vectors only. Pane, tab and workspace names from the far side never reach a shell.
- Forged labels: a far-side terminal labelled `herdr.session=<label>@<our core id>` is adopted only by the pane
  whose link names its exact address and whose `herdr.pane_id` matches, as core-phase3's open question 3 already
  covers.
- Status shown in a view comes from the far side's 7501 records. A view takes no action on status: fleet tools on
  m4m act on ac-mbm5 only through `--machine`, which asks ac-mbm5's own herdr.

### D12. Delivery order and rollback

PR A0 (D4's local-address filter) ships first and alone: it changes nothing until a machine exists, and from then
on it is what keeps ac's primary core from handing m4m's herdr ac-mbm5's panes (F1). Each later PR is off until
chosen (D8) and reversible by config. No PR needs a build on the other Mac first: D5's label
value is invisible to every existing build, and a view only reads. Removing a machine is `herdr machine rm --core
<name>` (`hc machine rm`: its terminals keep running there; panes on it wait, then show "machine removed" and can be
closed). Rollback to a build without phase 5 leaves remote panes as unclaimed links that wait (core-phase3 D1b),
never fresh starts.

## Risks

- **The view's size policy** rests on core treating AUTOMATIC_INPUT and `resumed` as R7-R8 measured on hc 0.1.21.
  A core update that makes either take the size would resize ac-mbm5's panes from m4m; V14 and V30 run against
  every hc version herdr accepts.
- **Lease and core retry interplay.** A lease that expires mid-dial, or core holding a hung via process, could
  leave the link up or a dial in flight while dormant. D2's exec shape, lock and 15 s deadline answer F2-F3;
  V5-V8, V27, V28.
- **Core's dial line is core's.** The guard (D2 step 5) is prepended to whatever line core sends; a core update
  that changes the line's shape (not `sh -c` first) makes core-link refuse every dial until herdr learns it. V35
  pins the shape so the failure is a test, not a silent outage.
- **Shared default core.** The machine shows in Herdr.app and the phone on m4m. Intended, but a person removing it
  there removes it for herdr.
- **Hook-only signals are lost on remote panes** (D5). An agent that reports neither 7501 nor a detectable screen
  shows `unknown` there.
- **Attach lifetime past a long drop** is unmeasured beyond 25 s (probe "Not measured"). V11 covers 15 min.

## Verifier: required tests

"Loop" means a live test against two scratch cores on one Mac joined by a `--via` loop and a fake `ssh` on `PATH`
(`probe/` shows the pattern), `#[ignore]` unless `HERDR_TEST_HC=1`, run by `just core-live-test`. "Fake core" is the
in-process test server of core-backend. No test dials a real Mac.

| # | Proves | Where it meets the system | Answer key | Pass |
|---|---|---|---|---|
| V1 | no lease, no dial: `core-link` exits 75 and opens no socket | unit over a lease dir; loop with a `connect(2)`-logging fake ssh | D2 step 2 | exit 75; fake ssh never ran |
| V2 | a lease lets the link up and its expiry closes it within 5 s | loop, lease with a 20 s expiry | D2 step 7 | link connected; closed 20-25 s after start; machine `unreachable` |
| V3 | dormant means zero bytes | loop with the probe's byte log; the test writes a client lease that expires 30 s later (the lease file is the interface, no test-only setting), then waits 5 min | P2 pattern while active; R3 retry shape (F12) | after dormancy: only refused core-link STARTs, in pairs (POSIX line, then core's Windows line < 100 ms later), at most one pair per 30 s once the backoff reaches its cap; zero ssh starts; zero link bytes |
| V4 | first input wakes at once | loop: dormant, then a key through a TUI client | P6 | `machine.connect` sent; pane repainted within 2 s |
| V5 | no holder writes another's lease; a dead holder's lease expires | unit: two clients and a CLI writer in threads | D3 | each file intact; expired leases ignored |
| V6 | restore and live handoff do not wake a dormant machine | loop: dormant, then `herdr server restart` and a live handoff | D3 non-holders | no dial; remote panes `CoreWaiting(machine_dormant)`; they reattach on wake with the same program pid on the far side |
| V7 | core start dials only with a lease | loop: dormant, `hc server restart` of the local core | D2; R3 (F12) | one refused pair (POSIX and Windows line), no ssh start, no link |
| V8 | failure classes close the gate | fake ssh printing the Tailscale banner and hanging; one silent for 25 s; one `Permission denied` | D10 table | `gate` = `needs_check`, `auth_stalled` (written ≤ 15.5 s after the START), `auth_failed`; no further ssh until `machine login` |
| V9 | a pane on a machine: label, env, facts | loop: the far core started with the six `HERDR_*` integration variables set to fake values; `herdr pane split --on loop` with a program that prints its variable names | D5, R6 (F6) | `herdr.session=<label>@<core id>`; argv starts with D5's `env -u` wrapper; no `HERDR_*` integration variable in the program; pid facts `None`; `machine` and `core_address` in pane JSON |
| V10 | the other Mac's herdr ignores our remote panes | loop: a far-side herdr server (debug, scratch session) beside m4m's remote pane | D5 label | far side claims nothing, lists no orphan, kills nothing |
| V11 | a remote pane across a 15 min drop | loop: lease kept, via killed and refused 15 min, then allowed | P8 | pane keeps its place; same far-side pid after; one repaint |
| V12 | close of a remote pane | loop | D5 close | `terminal.kill` sent; no local sweep; the far-side core and its other terminals alive |
| V13 | the old CLI and an old server with `on` | fake server without `spawn_on_machine` | D5 API | the CLI refuses `--on` with a message; no local pane started |
| V14 | the view never resizes the far side | loop: far-side herdr pane at 100x30, view pane 160x50; type 50 keys, paste, click and focus the view's pane; resize the far side to 90x25 and type again | D6 size, R7 (F5) | far-side terminal stays 100x30, then 90x25; every key reaches the program; the view sent no DATA and no FOCUS frame, only AUTOMATIC_INPUT, CREDIT and one RESIZE per changed far size; view's grid follows |
| V15 | view layout follows the far side | loop: far-side herdr creates a tab and splits; view open | D6 layout | the view shows the new tab and split within 2 s of the `doc` event |
| V16 | a bad far-side document | fake core: oversized, deep, wrong-typed, unknown fields | D11 | last good layout kept; one warn; unknown fields ignored |
| V17 | view close is a detach | loop | D6 close | far-side program and pane unchanged |
| V18 | `--machine` over the link equals the old path | loop: `herdr --machine loop pane list --json` with the far side as a saved herdr machine (fake ssh) and as a core machine | D7 | identical JSON |
| V19 | no retry after send | fake core drops the link after `machine.exec` is sent | D7 | one error, no second exec |
| V20 | structural actions in a view use argv | unit: a workspace named `$(touch x); w` | D7, D11 | argv contains the name as one element; no shell |
| V21 | the old remote unchanged | the existing `src/remote` and endpoint suites; `--remote <host>` against fake ssh | D8 | green; no core call made |
| V22 | `[remote] prefer` and the both-present notice | unit | D8 | resolution as D8; one notice |
| V23 | herdr never installs or updates hc | architecture test over `src/` | D9 | no `machine.update`, no `--install`, no `hc update` |
| V24 | dormancy cost on the client | `just bench-render-scale` with 1 and 15 panes, lease writes counted | D3 at most once a minute | render median within noise; at most one lease write per minute per client |
| V26 | local scans ignore machine terminals (F1, PR A0) | unit: `CoreAddress` parse (`t7` and `m/t7` accepted; `m/`, `/t7`, `m/n/t7` rejected); fake core listing `t1` and `loop/t1` with the same `herdr.session` and `herdr.pane_id`, and `loop/t1` with a pid that is a live local process; then loop with a real machine | D4, R1 | restore, the waiting-pane search, the orphan scan and the launch-label recovery each see only `t1`; nothing attaches, lists, kills or signals `loop/t1` or its pid |
| V27 | core-link answers before core kills it, and leaves nothing (F2) | loop: fake ssh silent; then the same with `kill -9` of the via pid 3 s after its START | D2 steps 4 and 6, R2 | gate `auth_stalled` ≤ 15.5 s after START; `machine.connect` answers < 17 s; after either run no ssh, watcher or core-link pid is alive (pids from the link log) |
| V28 | one dial at a time (F3) | loop: a slow fake ssh (5 s); a second `core-link` started while the first holds `dial.lock`; core's Windows line given to `core-link` | D2 steps 1 and 3, R3, R4 | the second refuses `busy` within 2.1 s and starts no ssh; the Windows line exits 75 with no ssh start; at most one ssh process per machine at any sample |
| V29 | `machine.connect` never runs on an input path (F4) | unit: dialer whose connect blocks 20 s; 100 keys to two panes on the machine, a view wake and two CLI wakes from two processes against one `connect.lock` | D3a | each key handled < 1 ms; exactly one `machine.connect` sent; the second process polls `machine.list` and sends none |
| V30 | an idle view sends nothing, and keeps the far size (F5) | loop: view attached 60 s with no input; then the far herdr detaches and reattaches | D6 size, R8 | 0 frames from the view in 60 s except CREDIT; after the far detach the far terminal has the last far size, not the view's |
| V31 | remote programs and the link do not depend on who started the core (F6) | loop as V9; `machine.exec` of `env`; a far-side `herdr pane report-agent` with no target in a remote pane; `core-link` run with `HOME`, `XDG_STATE_HOME` and `SSH_AUTH_SOCK` pointing elsewhere | D2 state, D5 env | exec output has no `HERDR_*` name; the far CLI reports no pane; core-link uses the passwd-home state dir and target.json's `SSH_AUTH_SOCK`; ssh's environment has only the allowlisted names (names compared, never values) |
| V32 | `--machine` on a dormant machine (F7) | loop: dormant; `herdr --machine loop pane list --json` without and with `--dial` | D7 | without: exit 75, the dial message, no lease, no ssh start; with: lease written before `machine.connect`, one dial, the far side's JSON, lease removed after |
| V33 | a lease comes before every connect (F8) | loop: `herdr machine add --core`, `machine login` (pty), `machine wake`, and a view's first client, each from dormant | D3 | in each, the lease file's mtime precedes the first core-link START, and that START dials |
| V34 | nothing authenticated outlives the link (F9) | real OpenSSH to `probe/fix/sshd_fake.py` on 127.0.0.1; link up, then the lease ends | D10 options, R9 | no control socket ever exists; after the link ends no ssh process remains, and a new `core-link` authenticates again (server AUTH count + 1) |
| V35 | a dial never starts the far core (F10) | loop: far core stopped; far core SIGKILLed (stale socket); lease held | D2 step 5, R5 | far socket stays absent or dead and no far core process starts; gate `far_core_down`; no further dial for 5 min; after the far core is started, `herdr machine wake` connects; a unit test pins core's POSIX line shape (`sh -c '...connect'`) |
| V36 | a frozen link ends in bounded time (F11) | real OpenSSH to the loopback server, then SIGSTOP of the server | D10 keepalives, R9 | machine `unreachable` ≤ 30 s after the freeze; `terminal.list` latency on the primary recorded each 2 s in the test log |
| V37 | cut or timed-out exec replies are errors (F12) | fake core answering `machine.exec` with `truncated: true` and with `timed_out: true`, exit code 0 | D7 | the CLI exits 1 with the message and prints no partial output; an input over 8 MiB is refused before sending |
| V38 | a refusing far side costs at most 2 dials (herdr's review) | loop with real OpenSSH to `probe/fix/sshd_fake.py` set to refuse auth, and a client lease renewed every 30 s by a timer (no input); 10 min; then one simulated key in a far pane | D2 step 9 | at most 2 ssh starts (server AUTH attempts at most 2) in the 10 min; gate `retry_spent` after the second; the key gives exactly one more dial |
| V25 | live on both Macs | beta on m4m and ac-mbm5 in one step; m4m views ac-mbm5 and runs one pane there; one night with the view attached | Goals 1-3 | ac-mbm5 `pmset -g log` shows no dark-wake run from the link; panes intact in the morning; the first key wakes the view |

## Roundtable findings

Roundtable of 2026-11-22 on ea5d5eb5: four reviewers, twelve findings, each proven by a run against hc 0.1.21 and
re-verified (`~/.local/state/herdr-core-p1/p5-roundtable.md`, repros in `/tmp/arena-p5/`). Each is answered by a
decision or an accepted limit, and each has a required Verifier row.

| # | Finding | Answer | Kind | Verifier |
|---|---|---|---|---|
| F1 | far terminals merge into the local `terminal.list` with the far herdr's labels; restore, close, the sweep and the orphan scan act on them | D4: every address parsed into `CoreAddress`; local scans keep local addresses only; PR A0 ships it first, alone | fix | V26 |
| F2 | core SIGKILLs a silent via at about 19.8 s; `auth_stalled` never recorded; ssh orphaned | D2 steps 4 and 6: core-link execs ssh (the killed pid is ssh), a watcher beside it writes the gate at 15 s | fix, prototyped (R2) | V8, V27 |
| F3 | concurrent `machine.connect` → several vias; two dials per retry | D2 steps 1 and 3: `dial.lock` held by ssh; the Windows line refused locally. D3a: one connect per wake across herdr processes. Limit: other core clients can still restart a dial (one ssh at a time) | fix + limit, prototyped (R3, R4) | V28, V29 |
| F4 | `machine.connect` blocks 20 s | D3a: a dialer thread per machine; input only posts a request | fix | V29 |
| F5 | follow-size storms, snaps back and resizes on attach | D6 Size: `resumed` attach, AUTOMATIC_INPUT only, no FOCUS, RESIZE only for a changed far size. Limits: `hc send` cannot see typing in the view; no focus reports | fix + limits, prototyped (R7, R8) | V14, V30 |
| F6 | remote terminals and exec inherit the far core's `HERDR_*`; core-link's paths follow the core starter | D5: `env -u` wrapper on every remote argv; D2: state dir from the passwd home, ssh env allowlisted plus the recorded `SSH_AUTH_SOCK` | fix, prototyped (R6) | V9, V31 |
| F7 | `machine.exec` never dials; S4's "none" was wrong | D7: the CLI dials only with `--dial` (or `machine wake`), else refuses; D10 S4a, S4b | fix | V32 |
| F8 | add, login and view wake had no lease and were refused | D3: a lease before every connect; add and login write `manual` | fix | V33 |
| F9 | `ControlPersist=600` master outlives the gate and is reused without auth | D10: `ControlMaster=no`, `ControlPath=none`. Cost: every dial authenticates; no shared master with the old remote | fix + cost, prototyped (R9) | V34 |
| F10 | a dial restarts a stopped far core as a child of the ssh login | D2 step 5: far guard tests the socket with `nc -U` and refuses; gate `far_core_down` for 5 min. Limit: VMs without `nc -U` fall back to a socket-file test | fix + limit, prototyped (R5) | V35 |
| F11 | a frozen link stays connected about 45 s; primary `terminal.list` 3 s per call | D10: `ServerAliveInterval=5`, `ServerAliveCountMax=2`. Cost: up to about 25 s of 3 s calls per freeze | fix + cost, prototyped (R9) | V36 |
| F12 | V3/V7 keys missed the second dial per retry; `machine.exec` cuts at 8 MiB with exit 0 | V3, V7 keys corrected (R3); D7: `truncated` and `timed_out` are errors | fix | V3, V7, V37 |

## Roundtable questions

All answered on 2026-11-22; the F-numbers are the findings above.

1. **The view server (D6).** Is a second herdr process in a `ServerRole::View` mode the right home for another
   Mac's panes, against viewed panes inside the primary server, given the generation 1 endpoint contract and the
   blast radius? Which `ServerRole` boundaries are missing from D6's table?
   **Answered (F5, F8):** no finding against the second process. Two boundaries were missing and are now in D6's
   table: input (AUTOMATIC_INPUT only) and wake (only with a lease, through D3a).
2. **The gate (D2, D3).** Is a via wrapper reading lease files sound against core's retry loop (a 30 s cap,
   dials at core start, a hung via process), and is closing the link the only way to stop P2's 3 s poll with core
   as shipped?
   **Answered (F2, F3, F9, F10, F11, F12):** sound once core-link execs ssh, holds a lock, answers within 15 s,
   refuses the Windows line, keeps no master, never starts the far core and ends frozen links by keepalive.
   Closing the link stays the only way to stop the poll (P3).
3. **Who holds a lease (D3).** Client input only, as #260, or should a view with a busy agent, or the phone, keep
   the link up? Is a 3 a.m. live handoff that leaves remote panes waiting until morning acceptable?
   **Answered (F7, F8):** client input and a person's client start, a CLI call that is connected or passes
   `--dial`, add, login and wake. Not a busy agent in a view, not the phone. A 3 a.m. handoff leaving remote panes
   waiting is accepted (V6).
4. **Size between two herdr servers (D6, V14).** Does follow-size hold under core's "last active viewer" rule,
   and is a cropped remote grid acceptable UX against the old remote's resize-to-fit?
   **Answered (F5):** it did not hold; D6's new policy makes the view never active (R7, R8). The cropped grid stays;
   "fit here" stays out of scope.
5. **Ownership by label value (D5).** `herdr.session=<label>@<core id>` against a new `herdr.owner` label: what
   breaks when a core's state is wiped (new id) or two herdr sessions share a core?
   **Answered (F1):** the live risk was the other direction, our local scans adopting the far herdr's panes, fixed
   by D4's address filter. The `@<core id>` value stays: a wiped core (new id) leaves m4m's remote panes unclaimed
   links that wait (core-phase3 D1b), and two herdr sessions on one core have different labels.
6. **Remote panes without herdr hooks (D5).** Is dropping the `HERDR_*` variables and hook reports the right
   trade, and does any fleet tool depend on a hook-only fact (`agent_session`, `blocked_reason`) for a pane on a
   machine?
   **Answered (F6):** dropping is not enough, they must be unset (D5's wrapper). The trade stays. The roundtable
   named no fleet tool that needs a hook-only fact on a machine, and none was audited for it: task C.6 documents
   the facts a remote pane lacks, so a tool that needs them uses a local pane.
7. **Security of the shared link (D11).** Is a ten-minute window of unprompted access to the other Mac for every
   local core client acceptable, or must the link live in a core session only herdr uses despite D1's costs?
   **Answered (F9, F6):** the window was larger than stated (the master's ten minutes on top); with no master it is
   the lease window. The ten-minute window for every local core client is kept and goes to the security review
   (task 0.4) as it is.
8. **`--machine` over `machine.exec` (D7).** Any semantic drift from the ssh `remote-api-bridge` (timeouts,
   stdin size limit of 8 MB, environment of the far-side core) that a fleet tool would notice?
   **Answered (F7, F12, F6):** three: a dormant machine needs `--dial`; a cut or timed-out reply is an error, not
   exit 0; the far core's environment is stripped of `HERDR_*`.

## Open questions

1. Does ac-mbm5's herdr publish its spaces into its `default` core's `herdr-app` document today (core-spaces-doc
   switch)? D6 needs it on.
2. ~~The exact Tailscale SSH check banner and whether ssh prints it before or after `ConnectTimeout` (task A.1).~~
   Answered in D10 (the check banner): an auth banner from the control server, printed on stderr at LogLevel INFO,
   after the connection is made, so `ConnectTimeout` does not bound it.
3. Which app 1Password names when core's child ssh asks for the key (Herdr Core or herdr), and whether one approval
   covers later dials. With no master (D10, F9) this decides how often a wake prompts.
4. Does core ever classify a via failure as `needs_login` (the 2, 5, 10 min ladder), and should `core-link` exit
   in a way that gets it?
5. Do Herdr Mobile and Herdr.app resolve a `herdr-app` leaf with a qualified address (`ac-mbm5/t7`) for a pane
   herdr-max owns on a machine?
