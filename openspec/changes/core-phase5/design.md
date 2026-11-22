# Core phase 5: design

Phase 5 of the herdr-core migration (`openspec/changes/herdr-core-client/design.md` § 6.4 item 5, "Remote
through core machines"). Issue: https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/300. Drafted
2026-11-22 for the roundtable; no code yet.

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
session on both Macs). `herdr machine add --core <target> [--name N] [--session S]` runs `hc machine add` with
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
versioned path). Core runs it for every dial: add, retries, core start, `machine.connect`. `core-link`:

1. Reads the leases of D3. With none in force it exits 75 at once, writes nothing and opens no socket. Core
   retries locally every 30 s at most (P4).
2. Otherwise it starts `ssh` with herdr's background options (D10) and the target recorded by `herdr machine
   add --core`, with ssh's stdin and stdout inherited, so no byte is copied by herdr. It keeps only ssh's stderr.
3. Every 5 s it re-reads the leases; when none is in force it ends ssh and exits. The link closes; core marks the
   machine unreachable and its next retries hit step 1.
4. It watches stderr and the first byte from the far side for the failure classes of D10 and records them in the
   machine's gate state.

The link state lives in herdr's state directory, `core-machines/<name>/` (mode 0700): `target.json` (the ssh
target and options), `leases/`, `gate.json` (the last refusal or failure and its time).

Alternatives:

- **`machine.rm` on dormancy, `machine.add` on wake.** Rejected: rm drops the machine's terminals from every
  client of the core, each wake is a full add (an install check, two dials, P1), and a rename between them
  changes every address.
- **A plain ssh target plus an upstream idle policy.** Rejected for now: an agent may not file a feature request
  (AGENTS.md § External contributor guardrail). ac may post a Discussion; if core ever grows an idle setting, D2
  shrinks to a plain target.
- **herdr kills the ssh process from outside.** Rejected: core owns the process; finding and signalling it from
  the herdr server races with core's own retries. The lease file is read by the one process that needs it.

### D3. Dormancy is a set of leases, one file per holder

A lease is a file `leases/<holder>` holding an expiry time. The link may be up while any lease is in force. Each
holder writes only its own file (write to `.new`, then rename), so no two processes write one file. A crashed
holder's lease ends by its expiry.

| Holder | Writes | Expiry |
|---|---|---|
| a TUI client (`client-<pid>`), for every core machine | on human input (#260's `InputClock` events: key, text, paste, mouse), at most once a minute | last input + 10 min (`REMOTE_DORMANT_AFTER`) |
| a fleet CLI call through the machine (`cli-<pid>`, D7) | before it sends, removed after the reply | 120 s, longer by its own timeout |
| `herdr machine wake <name> [--for D]` (`manual`) | once | the given duration, 30 min by default, at most 12 h |

Not holders: a herdr server, a restore or a live handoff (a 3 a.m. beta install must not wake ac-mbm5), the
phone, Herdr.app and agents' own `hc` calls. They see the machine `unreachable` while it is dormant, which the
docs state. The first input after dormancy (any herdr server receiving input for a pane on that machine, or a view
server getting a client, D6) also calls `machine.connect`, so wake does not wait for core's 30 s retry.

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
- A link is `core_terminal {session, address}` as today, with the address qualified (`ac-mbm5/t7`) and the session
  the local primary that added the machine.

### D5. A herdr-max pane on a core machine

`terminal.run` with `on: <machine>` from the primary connection. What changes against a local core pane, as one
type: `CorePaneIo` gains `host: Host`, `enum Host { Local, Machine(MachineName) }`, and every pid- or
host-dependent call matches on it.

- **Ownership label.** `herdr.session = "<label>@<primary core id>"` (core id from `hello.core.id`), plus
  `herdr.pane_id`. Every herdr build matches `herdr.session` by equality, so ac-mbm5's own herdr (label `herdr`)
  never claims, lists as orphan or kills a terminal m4m placed there, with no build of ac-mbm5 changed first. The
  orphan scan runs per machine with that value.
- **Environment.** The integration variables (`HERDR_ENV`, `HERDR_PANE_ID`, `HERDR_TAB_ID`, `HERDR_WORKSPACE_ID`,
  `HERDR_SOCKET_PATH`, `HERDR_CLIENT_SOCKET_PATH`) are not passed. On the far side the `herdr` on `PATH` is that
  Mac's own server, and a hook report with m4m's pane id would land on a pane of ac-mbm5 with the same id.
  Status comes from OSC 7501 (Claude Code and Codex report it, § 4b) and from herdr's screen detection on the
  bytes; hook-only signals are absent for remote panes, which the docs state.
- **Process facts.** For `Host::Machine` the pid is the far side's: pid-based facts answer `None`, the close sweep
  (core-backend D8) does not run, and core's `foreground` record (argv, cwd) is the only process fact.
- **Close** is `terminal.kill`. Detached jobs of the program survive on the far side, as core's own kill leaves
  them. A remote sweep through `machine.exec` was rejected for phase 5: it needs the far side's session and holder
  pids for core-backend D8's guard, which `server.status` does not give for another machine.
- **Restore and live handoff** reattach by (session, address) as today. When the machine is not `connected`, the
  pane is `CoreWaiting` (core-phase3 A1.3) with the cause `machine_dormant` or `machine_unreachable`, never started
  fresh, and the reattach is tried again when `machine.list` says `connected`. The server reads `machine.list`
  every 5 s while it has a pane on a machine and some lease is in force; P5 showed no event for a loss.
- **In-band notices.** Core's `[line to <m> lost; reconnecting...]` and the CAN-led repaint (P8) go through the
  parser like any output. The CAN repaint resets the screen; no filtering.
- **Input while not connected** is not sent. The first key wakes the machine (D3) and the pane shows "waking
  <machine>" until the reattach; keys typed before it are dropped, so nothing lands on a stale screen.
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
| Lifetime | as today | started on demand by the endpoint transport; exits 10 min after its last client; stateless, so a herdr update needs no handoff: a client reconnect with a new version restarts it |

**Size.** Two herdr servers now view one terminal, and core gives the size to the last active viewer. The view
never imposes its own size: it attaches with the terminal's current `cols`/`rows` (from `terminal.get` just
before), follows `size` events and answers each with a RESIZE of the same size, so typing on m4m never resizes
ac-mbm5's pane. The m4m pane draws the remote grid at its own size, cropped or padded, with the remote size in the
pane border when they differ. A "fit here" action that resizes the remote terminal is out of scope for phase 5.

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
  at most 600 s); streaming verbs stay unavailable as today.
- In a view (D6), new tab, split, close pane, rename and agent start become the same `machine.exec` calls of the far
  side's CLI, argv only, never a shell line built from names. The far side's herdr changes its layout and
  republishes; the view follows the document.
- The remote herdr path and its capabilities are read once per link establishment (`machine.exec` of `<bin> status
  client --json`, the same probe `attach.rs` uses). A missing verb disables that action with a notice; it never
  disconnects the view.

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

`core-link`'s ssh argv is the old remote's: `apply_managed_ssh_options` (`-C -F <managed config> -S <scoped
ControlPath> -o ControlMaster=auto -o ControlPersist=600`, `attach.rs` 1466-1486) and
`apply_noninteractive_ssh_options` (`BatchMode=yes`, `NumberOfPasswordPrompts=0`, `StrictHostKeyChecking=yes`,
`ConnectTimeout=10`, `ConnectionAttempts=1`, `attach.rs` 1452-1464), then `-T <target> <line>`. So a link and an
old-remote tab to the same target should share one authenticated master (inferred from the shared helpers; task
A.2 asserts the two ControlPaths are equal). No `ServerAliveInterval`: core's 3 s poll
(P2) finds a dead link while a lease holds.

Failure classes `core-link` records in `gate.json`, each closing the gate to every holder but `herdr machine login`
until a person acts:

| Class | Seen as | Why it closes the gate |
|---|---|---|
| `needs_check` | stderr contains a Tailscale SSH check banner (`Tailscale SSH requires an additional check`, exact text verified in task A.1) | each redial can open a new sign-in request in ac's browser |
| `auth_stalled` | no byte from the far side 20 s after ssh started | an unanswered 1Password Touch ID or another prompt on this Mac |
| `auth_failed` | ssh exit 255 with `Permission denied` or a host-key error | redials cannot succeed |

`herdr machine login <name>` runs the ssh in the person's terminal (so the check, the Touch ID or the host key is
answered there), clears the gate and calls `machine.connect`. `herdr machine status [--json]` reports, per core
machine: `state` (core's), `gate` (`open`, `dormant`, `needs_check`, `auth_stalled`, `auth_failed`), the lease
holders and their expiry, the last dial time. Agents read this to announce before acting, and the TUI shows the
gate in the endpoint row.

Every path that opens an SSH connection, and when it must be announced (house rule
`no-unannounced-tailscale-ssh-check`; ac's Mac-to-Mac ssh goes through the 1Password agent, CONTEXT
`PATTERNS/environment.md`):

| # | Path | Who starts it | Prompt risk | Announcement |
|---|---|---|---|---|
| S1 | `herdr machine add --core <target>` (two dials, P1) | a person, in a terminal | 1Password Touch ID on this Mac; a Tailscale check if the target has a check rule | before the first add on each pair of Macs and for each VM: "herdr needs ssh ac@<target> now, to add it as a core machine" through the gestore desk |
| S2 | `core-link` while a lease holds: core start, retries, `machine.connect` | core, in the background | none while herdr's ControlMaster lives; a new master after more than 10 min dormant can prompt | covered by S1's announcement, stated in it; a prompt nobody answers becomes `auth_stalled` within 20 s, so no unattended run waits on it |
| S3 | `herdr machine login <name>` | a person | as S1 | the person is at the terminal; agents ask through the desk first |
| S4 | `--machine` over the link (D7), `hc exec/file/run --on` | rides the existing link | none: no new ssh | none |
| S5 | the old remote | unchanged | unchanged | unchanged |

Live tests (verifier) dial nothing: they use a local `--via` loop as `probe/` does, with a fake `ssh` first on
`PATH` that runs the line in a scratch core. The first real add (task R.2) waits for ac's acknowledgement of the
S1 announcement.

### D11. Security surface

- **The link widens who reaches the other Mac.** While a lease holds, every process that can open the primary core
  socket (agents on m4m, Herdr.app, the phone through m4m's core) can run commands on ac-mbm5 with no per-connection
  Touch ID. Today the 1Password prompt gates each new ssh master. D3's leases bound the window to ten minutes after
  human input. The security agent reviews this before task R.2.
- `core-link` reads only its own state directory; leases are timestamps, never commands. Its argv comes from
  `target.json`, written by `herdr machine add --core`, never from the far side.
- The far side's `herdr-app` document is untrusted input to the view: parsed at the boundary with size, depth and
  id-format limits, unknown fields ignored, a bad document leaves the last good layout and logs once.
- D7 sends argv vectors only. Pane, tab and workspace names from the far side never reach a shell.
- Forged labels: a far-side terminal labelled `herdr.session=<label>@<our core id>` is adopted only by the pane
  whose link names its exact address and whose `herdr.pane_id` matches, as core-phase3's open question 3 already
  covers.
- Status shown in a view comes from the far side's 7501 records. A view takes no action on status: fleet tools on
  m4m act on ac-mbm5 only through `--machine`, which asks ac-mbm5's own herdr.

### D12. Delivery order and rollback

Each PR is off until chosen (D8) and reversible by config. No PR needs a build on the other Mac first: D5's label
value is invisible to every existing build, and a view only reads. Removing a machine is `herdr machine rm --core
<name>` (`hc machine rm`: its terminals keep running there; panes on it wait, then show "machine removed" and can be
closed). Rollback to a build without phase 5 leaves remote panes as unclaimed links that wait (core-phase3 D1b),
never fresh starts.

## Risks

- **The view's size policy.** If core applies a viewer's stale size when it becomes active, typing on m4m resizes
  ac-mbm5's pane. V14 measures it before PR D merges.
- **Lease and core retry interplay.** A lease that expires mid-dial, or core holding a hung via process, could
  leave the link up or a dial in flight while dormant. V5-V8.
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
| V1 | no lease, no dial: `core-link` exits 75 and opens no socket | unit over a lease dir; loop with a `connect(2)`-logging fake ssh | D2 step 1 | exit 75; fake ssh never ran |
| V2 | a lease lets the link up and its expiry closes it within 5 s | loop, lease with a 20 s expiry | D2 step 3 | link connected; closed 20-25 s after start; machine `unreachable` |
| V3 | dormant means zero bytes | loop with the probe's byte log; the test writes a client lease that expires 30 s later (the lease file is the interface, no test-only setting), then waits 5 min | P2 pattern while active | after dormancy: only refused STARTs, at most one per 30 s, and zero link bytes |
| V4 | first input wakes at once | loop: dormant, then a key through a TUI client | P6 | `machine.connect` sent; pane repainted within 2 s |
| V5 | no holder writes another's lease; a dead holder's lease expires | unit: two clients and a CLI writer in threads | D3 | each file intact; expired leases ignored |
| V6 | restore and live handoff do not wake a dormant machine | loop: dormant, then `herdr server restart` and a live handoff | D3 non-holders | no dial; remote panes `CoreWaiting(machine_dormant)`; they reattach on wake with the same program pid on the far side |
| V7 | core start dials only with a lease | loop: dormant, `hc server restart` of the local core | D2 | one refused START, no link |
| V8 | failure classes close the gate | fake ssh printing the Tailscale banner and hanging; one silent for 25 s; one `Permission denied` | D10 table | `gate` = `needs_check`, `auth_stalled`, `auth_failed`; no further ssh until `machine login` |
| V9 | a pane on a machine: label, env, facts | loop: `herdr pane split --on loop` with a program that prints its env | D5 | `herdr.session=<label>@<core id>`; no `HERDR_*` integration variable; pid facts `None`; `machine` and `core_address` in pane JSON |
| V10 | the other Mac's herdr ignores our remote panes | loop: a far-side herdr server (debug, scratch session) beside m4m's remote pane | D5 label | far side claims nothing, lists no orphan, kills nothing |
| V11 | a remote pane across a 15 min drop | loop: lease kept, via killed and refused 15 min, then allowed | P8 | pane keeps its place; same far-side pid after; one repaint |
| V12 | close of a remote pane | loop | D5 close | `terminal.kill` sent; no local sweep; the far-side core and its other terminals alive |
| V13 | the old CLI and an old server with `on` | fake server without `spawn_on_machine` | D5 API | the CLI refuses `--on` with a message; no local pane started |
| V14 | the view never resizes the far side | loop: far-side herdr pane at 100x30, view pane 160x50, type 50 keys in the view, then resize the far side to 90x25 and type again | D6 size | far-side terminal stays 100x30, then 90x25; view's grid follows |
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
| V25 | live on both Macs | beta on m4m and ac-mbm5 in one step; m4m views ac-mbm5 and runs one pane there; one night with the view attached | Goals 1-3 | ac-mbm5 `pmset -g log` shows no dark-wake run from the link; panes intact in the morning; the first key wakes the view |

## Roundtable questions

1. **The view server (D6).** Is a second herdr process in a `ServerRole::View` mode the right home for another
   Mac's panes, against viewed panes inside the primary server, given the generation 1 endpoint contract and the
   blast radius? Which `ServerRole` boundaries are missing from D6's table?
2. **The gate (D2, D3).** Is a via wrapper reading lease files sound against core's retry loop (a 30 s cap,
   dials at core start, a hung via process), and is closing the link the only way to stop P2's 3 s poll with core
   as shipped?
3. **Who holds a lease (D3).** Client input only, as #260, or should a view with a busy agent, or the phone, keep
   the link up? Is a 3 a.m. live handoff that leaves remote panes waiting until morning acceptable?
4. **Size between two herdr servers (D6, V14).** Does follow-size hold under core's "last active viewer" rule,
   and is a cropped remote grid acceptable UX against the old remote's resize-to-fit?
5. **Ownership by label value (D5).** `herdr.session=<label>@<core id>` against a new `herdr.owner` label: what
   breaks when a core's state is wiped (new id) or two herdr sessions share a core?
6. **Remote panes without herdr hooks (D5).** Is dropping the `HERDR_*` variables and hook reports the right
   trade, and does any fleet tool depend on a hook-only fact (`agent_session`, `blocked_reason`) for a pane on a
   machine?
7. **Security of the shared link (D11).** Is a ten-minute window of unprompted access to the other Mac for every
   local core client acceptable, or must the link live in a core session only herdr uses despite D1's costs?
8. **`--machine` over `machine.exec` (D7).** Any semantic drift from the ssh `remote-api-bridge` (timeouts,
   stdin size limit of 8 MB, environment of the far-side core) that a fleet tool would notice?

## Open questions

1. Does ac-mbm5's herdr publish its spaces into its `default` core's `herdr-app` document today (core-spaces-doc
   switch)? D6 needs it on.
2. The exact Tailscale SSH check banner and whether ssh prints it before or after `ConnectTimeout` (task A.1).
3. Which app 1Password names when core's child ssh asks for the key (Herdr Core or herdr), and whether one approval
   covers later dials.
4. Does core ever classify a via failure as `needs_login` (the 2, 5, 10 min ladder), and should `core-link` exit
   in a way that gets it?
5. Do Herdr Mobile and Herdr.app resolve a `herdr-app` leaf with a qualified address (`ac-mbm5/t7`) for a pane
   herdr-max owns on a machine?
