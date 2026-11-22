# Core phase 3: design

Phase 3 of the herdr-core migration (`openspec/changes/herdr-core-client/design.md` § 6.4, "Restore and update
through core"). Issue: https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/295. Revised 2026-11-21 after the
four-model roundtable (§ Roundtable 2026-11-21).

## Context

What phase 2 already gives (`openspec/changes/core-backend/design.md`):

- A core pane is a core terminal behind `PaneIo::Core`, labelled `herdr.session` and `herdr.pane_id` (D4).
- A restart or live handoff detaches from core panes and the next server reattaches by the saved link
  `core_terminal {session, address}`, after checking the labels (D10). Unclaimed labelled terminals are left
  running and named in one notice.
- The core session is `herdr`, `herdr-<name>` or `herdrdev[-<name>]` (D2, `core_session_name` in
  `src/pane/runtime_io/core_backend/mod.rs`). The server starts the core only before its first connection
  (`EVER_CONNECTED`), never runs `hc server stop`, `restart` or `update`.
- Exit mapping (D7, `exit_reason` in `core_backend/pane_io.rs`): `holder_lost`, `server_stop`, `host_shutdown`,
  `stopped_by_restart` all map to `ChildExitReason::Interrupted`, so the pane closes after a checkpoint.
- Restored panes start on the PTY (`pty_only` in `src/pane.rs`, live test `core_v1_a_restored_pane_stays_on_the_pty`).

Three phase 2 behaviours this change must undo, found by the roundtable:

- `reattach_core_pane` in `src/persist/restore.rs` starts the pane fresh on **any** reattach error, an
  unreachable core included. With the holder alive, a herdr restart or handoff then runs a second copy of the
  agent beside the first.
- `reattach()` in `pane_io.rs` reports `WaitFailed` after `REATTACH_LIMIT` (60 s unreachable), and `try_reattach`
  reports `WaitFailed` on `not_found`. Both mean "gone" today, though only the second can be.
- `PaneIo::reattach_core` refuses a link whose session is not `core_session_name()`, and `PaneIo::core_terminal`
  writes `core_session_name()` into the link instead of the pane's own session.

What pane-launchd-jobs gives (`openspec/changes/pane-launchd-jobs/design.md` D3, `src/app/pane_suspension.rs`):
a pane whose program the session ended keeps its place as Suspended, keeps its agent's resume plan, and starts
again in place 5 s after the last loss (30 s after a burst of three within 2 s), or at once on a client attach,
once `pane_jobs_ready_probe` says launchd accepts jobs. Resume runs `respawn_pane_agent`, else the launch
command, else a shell, all synchronously in `resume_suspended_panes_if_due` on the server loop. `suspended` is an
optional pane fact, so old clients are safe.

What herdr-core 0.1.21 does (core `schema/README.md` § Events, `crates/herdr-core/src/SKILL.md`, `docs/security.md`):

- The default session is named `default`. `HC_SESSION` selects another core, terminals and state directory. The
  client socket is `<session>.sock` under `$XDG_RUNTIME_DIR/herdr-core/`, else the state directory.
- A finished terminal's `reason` is one of `exited`, `killed`, `server_stop`, `host_shutdown`, `holder_lost`,
  `stopped_by_restart`. `terminal.keep` replaces a lost program with a shell and never re-runs the command.
  `terminal.kill` "returns the observed result".
- On macOS a desktop start runs the core as a one-shot launchd job; an SSH start stays a direct child.

Measured for this change on m4m, hc 0.1.21, throwaway `HC_SESSION=p3probe<pid>` with scratch `XDG_RUNTIME_DIR` and
`XDG_STATE_HOME` under `/tmp`, removed afterwards (first table 2026-11-21 morning, second table during the
revision):

| Action | Result |
|---|---|
| `server.status` with two terminals | core server ppid 1, `~/.local/bin/hc`; one holder, ppid the core server, `Herdr Core.app/Contents/MacOS/herdr-core` |
| SIGTERM to the holder | both terminals `finished`, `reason: holder_lost`; `holders: []` |
| SIGKILL to the core server | the holder (now ppid 1) and its program kept running; the next `hc` call started a core, which listed the holder and the terminal `live` |
| SIGKILL to that holder | its terminal `finished`, `holder_lost`; the program was gone |

| Probe (revision) | Result |
|---|---|
| six `hc server start` at once on a fresh session | all exit 0, all report the same server pid; one listener on the socket |
| `hc server restart` with 50 live terminals, socket polled every 2 ms, 3 runs | socket **missing** 393, 396 and 400 ms, never refusing; the command took 0.55-0.58 s |
| `hc server restart` with three `hc server start` 50 ms into it | each start exits 5, `unavailable`, "the core restarted while this ran (an update?)"; one server after, holder with 50 terminals |
| SIGKILL to the core server with 50 terminals | the socket **file stays**; a connect is refused (`ECONNREFUSED`) |
| `hc server start` after that SIGKILL | 134 ms; the new core reconciled the holder, 50 of 50 terminals live |
| a desktop `hc server start` from a herdr pane | core ppid 1, and no launchd label carries the session name afterwards (the one-shot job is gone) |

`hc update` was not run live: it swaps the per-user install under `~/.local/share/herdr-core` and registers the
bundle with LaunchServices, which reaches ac's running cores. Its socket gap is the restart's (the swap comes
first); phase 0 Q6 measured 0.6 s with few terminals. V46 records the gap of the next real update from herdr's
own log.

So the process Activity Monitor shows as Herdr Core is the holder. Quitting or killing it ends every program it
holds. The core server itself can die without losing a program, provided a core is started again; it then leaves
a socket file that refuses connections.

## Goals and non-goals

Goals:

- A core loss never costs a pane its place, its agent conversation or its workspace, and never runs a second copy
  of a program that still runs.
- Herdr Mobile and the Herdr apps see herdr-max's panes with no shell bridge.
- Every pane on both Macs runs on core, moved without interrupting an agent's work, except the panes the server's
  config keeps on the PTY.
- The CPU cost of core is re-measured and gated before the rollout.

Non-goals: a holder per terminal (herdr-core's code, D4); layout in a `herdr-max` core document (phase 4);
remote machines (phase 5); deleting the PTY path (phase 6); Windows.

## Decisions

### D1. A core loss suspends the pane

A core pane's exit is a suspension when the runtime ended its program, not the program itself:

| core says | today | phase 3 |
|---|---|---|
| `exited` with a code | `Exited` | `Exited` (unchanged) |
| `exited` or `killed` with a signal | `Interrupted` | `Interrupted` (unchanged: the program died of a signal) |
| `killed` with no signal (our close, `hc kill`, a phone close) | `Exited` | `Exited` (unchanged: a person or tool ended it) |
| `holder_lost`, `host_shutdown`, `stopped_by_restart` | `Interrupted` | `Suspended(cause)` |
| `server_stop` | `Interrupted` | `Suspended(CoreStopped)`, held until the core returns (D3b) |
| stream lost, core answers `not_found` or `finished`, program gone (D1a) | `WaitFailed` | `Suspended(TerminalGone)` |
| stream lost, core does not answer, or answers but the program still runs | `WaitFailed` after 60 s | `Unreachable`: never an exit (D1a) |

`exit_reason` keeps the first five rows in one match and one table test. The suspend cause is a closed enum
(`HolderLost`, `HostShutdown`, `CoreStopped`, `StoppedByRestart`, `TerminalGone`, `PaneJobEnded`) carried to the
notice text (D2) and the loss guard (D3).

### D1a. Gone, unreachable and reattached are three outcomes

`reattach()` in `pane_io.rs` returns `ReattachOutcome`:

```rust
enum ReattachOutcome { Reattached, Gone(SuspendCause), Unreachable }
```

- `Reattached`: `terminal.get` says `live`, the attach succeeded.
- `Gone`: the core answered (`not_found`, or a finished record) **and** the program is not running: its pid,
  checked against the `ProcessIdentity` recorded at attach (pid plus start token, `src/platform/unix_common.rs`),
  is absent or reused. A finished record maps its reason by D1; `not_found` maps to `TerminalGone`.
- `Unreachable`: anything else. The core does not answer (socket missing, refused, call timeout), or it answers
  `not_found` while the identity still matches a running process (a core that lost track of a live holder), or
  the identity is unknown (pid 0 from core).

`Unreachable` never ends the pane. The loop keeps retrying with its backoff (50 ms doubling to 2 s, then every
10 s after `REATTACH_LIMIT`). At `REATTACH_LIMIT` (60 s) it sets the pane fact `core_unreachable_since` and the
pane shows "Herdr Core is not answering; the program keeps running" in place; the fact clears on reattach. The
deadline is a field of `Shared` so V3 drives the real loop with a short limit.

Alternatives:

- `terminal.keep` on every pane, so core replaces a lost program with a shell. Rejected: core never re-runs the
  command, so an agent pane comes back as a bare shell, and the kept record stays `live`, which our re-adoption
  would then attach to as if it were the agent.
- Keep `Interrupted` and rely on the next server start to restore. Rejected: the panes and an emptied workspace
  close now, and nothing restarts them until ac restarts herdr.

### D1b. The link records the program, and restore never starts a pane fresh on a guess

`PaneCoreTerminalSnapshot` gains two optional fields:

```rust
pub struct PaneCoreTerminalSnapshot {
    pub session: String,
    pub address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid_started: Option<u64>,
}
```

They are written from the `ProcessIdentity` taken at attach. No persisted struct uses `deny_unknown_fields`, so a
phase 2 build reads the new JSON and drops the fields, and a phase 3 build reads a phase 2 link as `pid: None`
(V31). The handoff snapshot carries the same struct, so restart and live handoff share the rule.

`reattach_core_pane` becomes a match on `RestoreReattach`:

| outcome | restore does |
|---|---|
| attached | the pane shows its program (today) |
| core answered gone, and the recorded identity is not running | starts the pane fresh, by D8's runtime choice |
| core answered gone, and no identity was recorded (a phase 2 link) | searches the known sessions (below); fresh only if nothing is found |
| core unreachable, identity running or unknown | the pane is restored **waiting**: no program starts, the pane shows "Waiting for Herdr Core (session `<s>`)"; the server retries the attach every 10 s and on each core start; `herdr pane respawn` is the person's escape |
| core unreachable, identity recorded and not running | starts the pane fresh (the program is gone whatever core says) |
| the address is live but labelled for another pane | treated as gone for this pane, then the known-session search |

**Known-session search (also D14's rollback guard).** Before any fresh start, restore lists live terminals
labelled `herdr.session=<label>` and `herdr.pane_id=<public id>` in every session it knows and can reach without
starting one: the primary, the link's session, `herdr` and `default` (release default session only). One match is
attached (the pane's link is rewritten to it). Two or more matches start nothing and raise one notice naming them.
None starts the pane fresh.

A waiting pane is not Suspended: its program may run, so no resume timer touches it. It is a separate state,
`CoreWaiting`, so the suspended-pane resume (D2) cannot pick it up.

**Changed by the security follow-ups (#297, review M3):**

- `CoreWaiting` is the link's durable home and carries why it holds: `Unreachable`, `Conflict` (the several
  matches, which used to drop the link at the next save), or `DirectoryUnavailable` (a saved directory that is
  gone, which also used to drop it). Every save keeps the link until the pane attaches or its program is proven
  gone. A conflict resolves when a person closes the extra terminals; a gone directory only ever attaches.
- "Not running" needs proof. The probe answers `Started`, `NoSuchProcess` or `Unknown`; a probe that fails is
  `Unknown`, so with core unreachable the pane waits. The link also records the agent detection saw (a shell's
  child with its own pid), and a running agent counts as a running program.
- The known-session search reports sessions that did not answer. Nothing found while one did not answer waits;
  a session with no socket has no core and counts as answered.

### D2. Resume waits for its backend, proves the old program is gone, and runs off the server loop

**Readiness.** `pane_jobs_ready_probe` becomes a readiness check for the backend a resumed pane will start on:
with `runtime = "core"` and the pane not excluded (D13), the primary core answers `hello`; otherwise today's
pane-jobs probe. The timing stays pane-launchd-jobs D3's: 5 s settle, 30 s after a burst, retry every 10 s, at
once on a client attach. A `CoreStopped` pane waits for the core in every case (D3b).

**Dead-program check before every resume.** For every suspend cause (`HolderLost`, `HostShutdown`, `CoreStopped`,
`StoppedByRestart`, `TerminalGone`, and the PTY `PaneJobEnded`), resume first checks the last program's
`ProcessIdentity` and, where detection knows one, the agent process's identity (an agent started by a shell has
its own pid). If either still runs, the pane is not resumed: it stays suspended with "The old program (pid N)
still runs" and is checked again at the next retry. A pid of 0 or an identity never recorded counts as **may
run**: such a pane is not resumed automatically, and shows "Cannot confirm the old program ended; run `herdr
pane respawn`".

**Changed by the security follow-ups (#297).** A failed probe of either identity counts as may run (M3). A
person's stop of a secondary session holds its panes until that session answers, with the primary up or not, and
that session is only asked, never started (M7). A pane whose program the worker is starting is reserved from the
launch to the install: a respawn of it starts nothing, `pane.respawn` answers `pane_resuming`, and a live handoff
is refused until the worker's answers land (M3).

**Off the server loop.** Today `resume_suspended_panes_if_due` spawns every pane synchronously, and a core spawn
is a `terminal.run` plus an attach with 5 s timeouts each: 50 panes on a slow core block the loop for minutes. In
phase 3 the loop only builds each pane's `SpawnSpec` (pure, in-memory) and hands batches of at most 8 to one
`resume-worker` thread. The worker runs `terminal.run` and the attach, and sends each result back as an
`AppEvent::PaneResumed`, which the loop installs like a respawn. The next batch leaves only when the last one has
answered. A pane in flight shows "Resuming"; input to it is refused until it lands.

Cardinality note (AGENTS.md § Multiplicative performance paths): this runs per loss burst times suspended panes,
never per render or per byte. The loop's work per pane is one spec build and one install; core round trips are
off the loop. V43 measures the loop's longest turn while 1 and 16 suspended panes resume against a fake core
that answers `terminal.run` after 2 s, and reports both.

**Notices, one per burst, by cause:**

| cause | notice |
|---|---|
| `HolderLost` | "Herdr Core quit or crashed and ended N panes; they resume in place" |
| `HostShutdown` | "The login session ended N core panes; they resume in place" |
| `CoreStopped` | "Herdr Core was stopped; N panes resume when it runs again" |
| `StoppedByRestart`, `TerminalGone` | "Herdr Core lost N panes' programs; they resume in place" |
| unreachable 60 s (D1a) | "Herdr Core is not answering; N programs keep running and reattach when it answers" |

Log lines carry the core reason. The cause is an optional pane fact `suspend_cause` (a string, with older clients
ignoring it), so the TUI can show it on the pane.

Alternative: a separate resume path for core panes. Rejected: the suspended state, its timing, its client fact
and its tests already exist; a second path would differ only in the probe.

### D3. Starting the primary core again

**What counts as down.** `connect_session` starts a core only when the socket file is missing. A SIGKILLed core
leaves the file and refuses (measured), so today herdr would never start it again. Phase 3 treats a refused
connect like a missing file. Only the primary session (D6) may be started.

**The 3 s rule.** The server starts its primary core when the socket has been missing or refusing for at least 3 s
without a break, with the clean environment of core-backend D2 through `hc server start`. Measured: a restart
with 50 terminals leaves the socket missing about 0.4 s and never refusing; a start that lands during a restart
exits 5 without a second core; six starts at once give one core. The 3 s window is therefore not what keeps the
core single (hc's start is exclusive); it stops herdr from adding a start to every planned restart and update.
A start that exits 5 (`unavailable`, restarted while it ran) is a success for the rule: the socket is polled again.

**Changed by the security follow-ups (#297).** The start gets an allow-list of herdr's environment, not a
deny-list (M4): `HOME`, `PATH`, `USER`, `LOGNAME`, `SHELL`, `TMPDIR`, `LANG`, `LC_*`, `XDG_*`, `TERM`,
`TERMINFO`, `TERMINFO_DIRS`, `COLORTERM`, `HC_SOCKET` and `HC_INSTALL_DIR`, plus the `HC_SESSION` herdr sets.
herdr's own panes get their full environment from herdr's spawn, so only terminals other clients start lose the
rest. `hc` is `~/.local/bin/hc` when it is there, else the first on `PATH` with a warning, and a start that runs
past 10 s is stopped (M5). A core start whose outcome is unknown (the `terminal.run` answer lost, or an attach
that failed with a kill core did not confirm) never falls back to the PTY; the run carries a random
`herdr.launch` label, and a lost answer is looked up by it and attached (M3).

**D3a. `CoreLossGuard` counts losses, not starts.** A holder that keeps dying starts no core (the core server
lives on and makes a new holder at the next `terminal.run`), so counting starts would never trip. The guard counts
**loss bursts**: every suspension of core panes, and every start of the primary core after it went down, is a loss
event; events within 2 s of each other are one burst. Pure, clock passed in:

- `Allowed` until the third burst within ten minutes;
- then `GaveUp`: no more core starts, resumes go to the PTY (`spawn_preferred` gets `RuntimeChoice::Pty(GaveUp)`,
  D13), one notice "Herdr Core keeps ending; panes resume on the PTY", and `herdr server status` shows
  `core: gave_up` (an optional field in its JSON). A config reload resets it.

The guard lives in the server and is not carried through a live handoff or restart: a new server starts
`Allowed`. This is stated in the docs; a handoff is a person's act, and three more bursts trip the new guard.

**D3b. `server_stop` is a person's stop.** Terminals finished with `server_stop` are suspended as `CoreStopped`.
The server records `stopped_by_person` for that core session and never starts it while the record stands. The
panes resume when the core answers again (someone ran `hc server start`, or Herdr.app started it), or on the next
herdr start, whose first connection may start the core as today. The record clears on either.

A core-server-only loss (the SIGKILL rows) repairs itself: after 3 s the server starts the core, which reconciles
the live holder; the panes' reattach loops find their terminals `live`; every program keeps its pid; nothing is
suspended.

herdr-max still never runs `hc server stop`, `restart` or `update`, and never starts a session other than its
primary.

Alternatives:

- Never start the core again. Rejected: suspended panes would wait forever, and new core panes would fall back to
  the PTY until herdr restarts.
- Run `hc server restart` on a hung core. Rejected: it acts on every terminal in the session, including other
  clients' in the default core (D5).
- Close `server_stop` panes. Rejected: a stop for an update or a reboot would cost every agent its pane.

### D4. What only herdr-core can do, and how to ask

The holder owns every program's terminal and sweeps them on loss, so herdr-max cannot keep a program alive
through a holder loss. Making one quit end one pane needs a holder per terminal, or per-terminal responsibility
and coalition at spawn. Both are herdr-core code.

The upstream ask goes through AGENTS.md § External contributor guardrail, which herdrdev applies to herdr-core:
herdr-max agents may help file a **verified, reproducible bug** with the bug template and nothing else, and
never a feature request. Two candidate bugs, each measured first in a throwaway core:

1. A GUI app launched from a core terminal, after a `hc server restart`, leaves a Dock "Running in Background"
   entry whose stop ends every terminal of that holder (the #288 mechanism applied to core).
2. Activity Monitor "Quit" of Herdr Core ends every terminal with no prompt.

Only a reproduced case becomes a one-screen report (current, expected, shortest repro, impact, version), sent as
an `ASK:` to manager-lab's desk when ready; ac decides whether to submit it, and submits it himself. A design wish (holder per terminal) is a GitHub Discussions
post by ac, not an agent's issue. Until upstream changes, the docs and the herdr skill say: quitting Herdr Core
suspends every core pane and they resume; do not stop "Running in Background" entries of apps launched from a
pane.

### D5. Ownership is a label, placement is a session

Two functions replace one:

- `herdr_session_label()`: the value written to and checked in `herdr.session`. It keeps today's formula
  (`herdr`, `herdr-<name>`, `herdrdev`, `herdrdev-<name>`), so every terminal phase 2 started still matches.
- `core_session_name()`: where new panes go. Release builds put the **default herdr session in core session
  `default`**. A named session keeps `herdr-<name>`; a debug build keeps `herdrdev[-<name>]`. The default core is
  only ever chosen for the release default session.

Every call site, and which name it takes:

| call site | today | phase 3 |
|---|---|---|
| `pane_labels` (`core_backend/mod.rs`) | `core_session_name()` | `herdr_session_label()` |
| `unclaimed_terminals` list filter (`core_backend/mod.rs`) | `core_session_name()`, one connection | label filter, run once per connected session, results as `(session, address)` |
| `connector()` (`core_backend/mod.rs`) | connects `core_session_name()` | `CoreSessions` entry for a named session (D6) |
| `PaneIo::reattach_core` (`runtime_io/mod.rs`) | refuses a link whose session is not `core_session_name()` | connects the link's session through `CoreSessions` (secondary, never started) |
| `PaneIo::core_terminal` (`runtime_io/mod.rs`) | writes `core_session_name()` | writes the session stored on `CorePaneIo` |
| `report_unclaimed_core_terminals` notice (`src/app/core_terminals.rs`) | `HC_SESSION=<s> hc ls --label herdr.session=<s>` | one line per session: `HC_SESSION=<session> hc ls --label herdr.session=<label>` |
| claimed list in `report_unclaimed_core_terminals` | addresses | `(session, address)` pairs |
| `terminate_session` sweep guard (`core_pids`) | the one client | the pane's own session's client |
| herdr-app publisher (`spaces_doc.rs`, PR 59) | `existing_core_connector()` | the primary session only (D7) |
| `socket_path` (`client.rs`) | `HC_SOCKET` for any session | `HC_SOCKET` for the primary only (D6) |

A terminal is ours when it carries `herdr.session=<label>`; reattach also checks `herdr.pane_id` (core-backend
D10). In the default core, terminals without our label (Herdr.app's, the phone's, an agent's `hc run`) are never
claimed, never listed as orphans and never killed.

Alternatives:

- Rename the label to `herdr-max.session`. Rejected for phase 3: every live terminal carries `herdr.session`, so a
  rename needs a dual-read period for no present conflict. Revisit if the apps ever define `herdr.session`.
- A config switch for the session move. Rejected: re-adoption across sessions (D6) makes the move reversible in
  code, and D14's delivery order covers rollback.

### D6. One connection per core session, links carry their session

The single `CLIENT` slot becomes a `CoreSessions` registry keyed by session name:

- **primary**: `core_session_name()`; may be started (D3); new panes go here.
- **secondary**: any other session a restored or handed-off pane's link names, plus `herdr` and `default` when
  their sockets exist (release default session only). Never started; connected only while its socket answers;
  dropped when no pane uses it and its orphan scan is empty.

Each `CorePaneIo` stores its session. `CoreTerminalLink.session` comes from the pane. Claims are `(session,
address)` pairs, because addresses are per core: `t7` can exist in both.

`HC_SOCKET` applies to the primary session only; a secondary's socket always follows core's path rules
(`socket_path_from` with `hc_socket: None`). A unit test sets `HC_SOCKET` and asserts the secondary path ignores it
(V42).

PR B ships this registry with `default` already read as a secondary while placement stays `herdr` (D14), so the
build a rollback lands on can see panes in `default`.

Alternative: move every legacy terminal to the default core at once and keep one connection. Rejected:
terminals cannot move between cores, so "moving" means restarting every agent at once, which is what the idle
move (D9) exists to avoid.

### D7. Sharing the default core with other clients

- **Launch context.** The default core may already run, started by Herdr.app or by the phone over SSH. A parent
  pid of 1 is not evidence either way: the probe's desktop start also shows ppid 1, and a direct child whose
  launcher exits is reparented to 1. The server raises the notice only on positive evidence: the core process's
  session leader (`getsid`) is `sshd`/`sshd-session` or has one as an ancestor. Then one warning and one notice:
  "Herdr Core in session default was started over SSH; restart it from a desktop terminal". `server.status` has
  `installed_by` and `keeps_running_after_logout` (both null in 0.1.21); once core fills them they replace the
  process test. The server never restarts that core.
- **The `herdr-app` document.** PR 59 (`openspec/changes/core-spaces-doc/`, D5 and D6) already writes only
  spaces marked `herdr_max: 1`, merges by compare-and-set, re-merges on conflict and on "off" removes only its
  own spaces. What it does not know is sessions: in phase 3 a pane in the legacy `herdr` core has an address
  that means nothing in `default`. So `spaces_document` takes a leaf filter: only panes whose link names the
  primary session are terminal leaves; every other pane is left out the way a PTY pane is (core-spaces-doc D2).
  The publisher writes to the primary session only. That is all PR D adds.
- **The status mirror** (`openspec/changes/core-status-mirror/`) writes to the pane's terminal device, so it works
  in whichever core holds the pane. No change.

### D8. Restored and respawned panes start on core

With `runtime = "core"`, a restored pane with no live saved terminal (D1b's fresh-start rows) starts on the
primary core, with its saved history seeded into herdr's parser as today, unless D13 keeps it on the PTY. Core's
own screen starts with the fresh program, so the phone shows no pre-restore history (accepted). A pane adopted by
fd in a live handoff stays on the PTY: its program is alive and cannot change backend.
`core_v1_a_restored_pane_stays_on_the_pty` is replaced by a test that states the new rule.

`PaneLaunch::Command`'s `pty_only: bool` becomes `RuntimeChoice { Preferred, Pty(PtyReason) }` with reasons
`Excluded` (D13), `GaveUp` (D3a) and `FdAdopted`, so every caller of `spawn_preferred` states why a pane skips core.

### D9. Moving an idle pane

A new server method, `pane.respawn_if_idle {pane, check_only}`, answers `{moved, refusals}`. A new method name is
required because an older server would ignore an `if_idle` field on `pane.respawn` and respawn a busy agent
(AGENTS.md § Stable client endpoint contract, on method shapes). The CLI is `herdr pane respawn --if-idle
[--check]`.

The server decides in one app-loop turn, from facts it already holds, through a pure `MoveEligibility` function:

| Fact | Refusal |
|---|---|
| the pane runs on the primary core | `already_on_primary` |
| the pane matches `core_pty_panes` (D13) | `excluded` |
| suspended, waiting for core, an overlay or popup pane, or a worktree removal is stopping it | `not_movable` |
| any input in the last 60 s (below) | `recent_input` |
| agent pane: its manifest lacks the move facts (below) | `unsupported_agent` |
| agent pane: state not idle for 60 s | `agent_busy` |
| agent pane: `background_activity` or `background_count > 0` | `background_work` |
| agent pane: the manifest's `prompt_box_body` is not empty | `draft_in_box` |
| agent pane: no resume plan | `no_resume_plan` |
| agent pane: the terminal's foreground process is not the agent, or a process in the pane's session is outside the agent's process tree | `foreign_process` |
| shell pane: not `shell_at_prompt`, a foreground job, or more than the shell in its session | `shell_busy` |
| shell pane: input in the last 10 minutes | `shell_recent` |

**Input, every path.** `recent_input` reads one per-pane stamp. Today `Pane::stamp_input` runs for
`try_send_bytes` and `try_send_paste` (client keys, `pane.send_text`, `pane.send_keys`, `pane.send_input`), but
not for `queue_user_input_submission` (the queued Submit behind `agent.message` typed mode and API submits), and
`agent.message` note mode never touches the pane. Phase 3 stamps the queued Submit when it is queued and when it
is delivered, counts a Submit still in the writer queue as input now, and stamps `agent.message` in note mode when
the note command answers `queued`, `held` or `injected`. Resume and restore launches stay untracked
(`try_send_bytes_untracked`). A peer message that reaches Claude outside herdr (agent-bell's own channel) is not
visible to the server; it starts a turn, which `agent_busy` sees within its 60 s window, and the driver asks
agent-bell for queued messages to the pane's session before each move (step 2 below).

**Which agents.** A manifest declares the move facts by having both a background region (`background_count`) and
the `prompt_box_body` region. Today only `claude.toml` has them, so every other agent is `unsupported_agent`. The
check reads the loaded manifest, so a manifest that gains both regions becomes movable without code.

**Claude's subagents.** A foreground Task subagent keeps Claude's own state working (`agent_busy`). A background
agent appears in Claude's background list, which `background_count` counts with shells, monitors and MCP tasks
(`TerminalState::background_count`). So no Claude subagent passes as idle by the server's facts. V40 proves it live
with a real Claude Code (no-connector settings) running a background agent.

**The move, with a confirmed kill.** The move kills the old program first and starts the replacement only after
the kill is confirmed:

0. Agent pane: the agent's identity gets a SIGTERM, so it saves its session, and up to 3 s to exit; if it
   outlives that, a SIGKILL of it and every process below it, and up to 2 s more. What an agent that ended on
   its SIGTERM leaves running below it is ended too (SIGTERM, then SIGKILL after 0.5 s): no longer in its
   terminal's foreground, it would miss the PTY's hangup. An agent behind a wrapper's nested PTY leads a session
   of its own, which the pane's sweep does not reach (fork issue 301).
1. PTY pane: the existing terminate and sweep; confirmed when the child pid's identity no longer runs.
   Core pane (legacy session): `CorePaneIo::kill` returns `KillOutcome` from `terminal.kill`'s observed result.
   Confirmed only for `finished` or `not_found` **and** the pid's identity gone.
2. A sent signal is a point of no return (fork issue 301). Only a shell pane whose kill core answered `live`
   (nothing signalled, nothing swept) refuses with `core_unreachable` and stays as it was. In every other
   unconfirmed case (no answer, an agent already stopped, a program still running after 2 s) the old runtime is
   shut down and the pane is suspended (`TerminalGone`, with the program and agent identities), and the answer
   is `moved`. D2 resumes it in place once both identities are gone. Leaving the runtime there would let the
   exit of the killed program close the pane: on 2026-11-22 that closed 13 Claude panes.
3. Confirmed: `respawn_pane_agent` (agent) or a shell in the pane's directory (shell), on the primary core, with
   herdr's parser history kept as in D8. A spawn that fails after a confirmed kill suspends the pane
   (`TerminalGone`), so D2 resumes it.

The killed program's exit is absorbed (`respawn_replaced_runtimes`) while the pane runs a runtime that has not
exited, and any exit is ignored while the pane has no runtime and waits to start again (suspended, or its agent's
resume pending). A mark whose runtime never reported (a core stream forgotten before its CLOSE) does not swallow
the exit of the runtime the pane runs later.

A moved shell loses its process state (variables, jobs, history in memory). That is accepted for shells idle for
10 minutes and stated in the docs; its directory and herdr's scrollback are kept.

The driver, `scripts/core_move_idle_panes.sh`, is the lever a person reruns:

1. lists panes not on the primary core; the server already refuses `excluded` panes, and `--exclude <glob>` only
   adds names for this run;
2. moves one canary, after `agent-bell` reports no queued message for its session, then verifies it: same agent
   session id once its first hook report lands, same pane name, the new process's argv carries the plan's
   `--model`, `--effort`, `--permission-mode` and `--settings`, and the agent's status line shows the same model
   before and after;
3. moves the rest one at a time, verifying each, and stops at the first failure.

Alternative: the driver alone, over `pane get` and `pane respawn --resume`. Rejected: between the script's check
and its respawn, ac can start typing; only the server sees input and state in one turn.

### D10. The default stays per machine

`[experimental] runtime` keeps its code default `"pty"`. Both Macs set `"core"` in their configs (m4m since
2026-11-16, ac-mbm5 since 2026-11-21).

- For: upstream herdr's default is the PTY, and fork work stays easy to upstream; a machine without `hc` (CI,
  Linux boxes, Windows) gets no fallback notice on every start; ac's two Macs already opt in.
- Against: two code paths stay live. They stay live anyway until phase 6 deletes the PTY.

Alternatives: flip the code default to `"core"` with a silent fallback, or add `"auto"` (core when `hc` 0.1.21+
is present). Both change behaviour for every build to serve two machines that already have the setting. The code
default flips in phase 6.

**Rollback rule.** The rollback target after PR C is the PR B beta (D14), which reads `default`, `herdr` and the
primary, so a rollback re-adopts every pane. Rolling back further, to beta 202 or earlier, keeps core-backend D10's
hazard: that build looks only in `herdr` and would restore a `default` pane fresh. D1b's known-session search
does not exist there, so the rule for such a rollback stays: close core panes or list them first with
`HC_SESSION=default hc ls --label herdr.session=herdr` and `HC_SESSION=herdr hc ls --label herdr.session=herdr`.

### D11. The CPU gate

Before the rollout, `scripts/core_busy_profile.sh` runs PTY and core alternately, **eight runs a side**, in both
profiles (default, and `V12_ONE_PROCESS=1`), each with `V12_TOP` counters and the load average recorded, release
build, under the m4m heavy lock.

- Gate 1: median herdr server CPU on core within 1.3x of the PTY in both profiles.
- Noise band: the ratio of the PTY medians of runs 1-4 and 5-8. When the core ratio is within that band of 1.3x
  (for example 1.24x with a 1.08 band), eight more runs a side decide; the band and every run go in the record.
- Gate 2: total CPU on core (herdr server, core server and holder) within 1.2x of phase 2's measured total for the
  same profile (`core-backend/v12-results.md`, 1.92-2.52 s). This is a regression tripwire against core itself,
  not a PTY comparison: core does the runtime's work, so its total is about twice the PTY server by design.
- Recorded, not gated: the idle cost of a second running core (legacy `herdr` beside `default`).

Results go to `v12-phase3.md` in this change. Over either gate: the conditional PR E replaces the per-chunk hop
with a reader that hands whole batches to each pane's parser (core-backend `v12-results.md`, "Not changed"). It
must keep one pane's parse off the shared reader and show that no caller waits on a core reply while holding a
terminal lock, and it profiles 1 and 15 panes (AGENTS.md § Multiplicative performance paths).

### D12. Removing the bridge and the old core

On each Mac, in order:

1. The phase 3 beta runs; new panes land in `default`.
2. The idle move (D9) empties the legacy `herdr` core of herdr-max panes, except panes that are never idle; those
   end on their own or wait for ac.
3. The bridge line is removed from chezmoi `dot_zshenv` per host: the relay to the dotfiles owner asks for the line
   to become `dot_zshenv.tmpl` with a per-host condition, so each Mac drops it when its own step 2 is done, not
   both at once. Never edited from this repo. The app's recorded probe command, run through `zsh -c` with an
   SSH-like environment and a `terminal.list`, then lists the default core's terminals, herdr-max's included. ac
   checks the phone (V24). Panes still in the legacy core stay off the phone until they move or end (D7).
4. With the legacy core holding no live terminal, the Mac's agent (mac-m4 on m4m, mac-m5 on ac-mbm5) announces the
   stop, then runs it: `HC_SESSION=herdr hc server stop`. herdr-max drops
   the secondary connection when the socket goes. A legacy core is a secondary, so D3b's stop record does not
   apply and no pane is suspended (none remain).

### D13. Some panes stay on the PTY: `core_pty_panes`

herdr asked for this explicitly. The computer-use host panes (`manager-lab-hlp` on m4m, `manager-aruba-hlp` on
ac-mbm5) hold Screen Recording and Accessibility grants that belong to the PTY program; a core pane's privacy
prompts name Herdr Core (phase 0 Q9). A script-only exclusion would not stop a restore, a resume or a respawn from
putting them on core.

`[experimental] core_pty_panes = ["*-hlp"]`: a list of glob patterns, server config, reloadable, default empty in
code. ac's configs on both Macs set `["*-hlp"]`.

A pane matches when a pattern matches its **stable name**, the first that exists of:

1. the `--name`/`-n` value in the pane's saved agent launch flags (`PaneAgentLaunchSnapshot.flags`, the flag
   `launch_flags.rs` records as `name`, which `restore_argv` re-types);
2. the pane's herdr agent name when its owner is explicit (set by `herdr agent name` or the API), never the name an
   agent reported about itself or the terminal title.

A pane with neither never matches. One pure function, `excluded_from_core(&patterns, &stable_name) -> bool`, is
the only matcher. It is consulted by every path into `spawn_preferred`: new panes, restore (D8), suspended-pane
resume (D2), `pane respawn`, and the move (`MoveEligibility` refusal `excluded`). A match gives
`RuntimeChoice::Pty(Excluded)`. A pane already on core that matches is not moved off by the server (no reverse
move); the docs say to respawn it after adding a pattern.

### D14. Delivery order and rollback safety

PRs land and ship in this order, each with its own check:

1. **A1** loss outcomes and identity: D1, D1a, D1b (restore never fresh on a guess), D2's dead-program check.
2. **A2** core start and resume: D2's off-loop batches and notices, D3 (refused as missing, the 3 s rule, D3a, D3b).
3. **S** `core_pty_panes` (D13) and `RuntimeChoice`; before F and G, because both spawn on core.
4. **B** `CoreSessions` (D6) with `default` and `herdr` read as secondaries, the call-site table (D5), placement
   unchanged. **Installed as its own beta on both Macs** (R0) before C merges, so the rollback target after C reads
   `default`.
5. **C** placement in `default` (D5), the SSH-start notice (D7).
6. **D** the herdr-app leaf filter (D7), after PR 59.
7. **M** the CPU re-measure (D11), then **E** only if a gate fails.
8. **F** restored panes on core (D8). **G** the idle move (D9).
9. **R** rollout.

Restore's known-session search (D1b) ships in A1 already, so from A1 on a pane whose link names the wrong session
is found rather than started fresh.

## Risks

- **Double agents.** A resume or restore while the old program lives duplicates a conversation. D1a's three
  outcomes, D1b's no-fresh-on-a-guess restore and D2's dead-program check before every resume guard it (V3, V29,
  V30, V35).
- **Restart loops.** A core or holder that keeps dying. `CoreLossGuard` counts loss bursts (V4, V16, V36).
- **Wrong pane, wrong core.** With two sessions, an address alone can match a terminal in the other core. Claims and
  links are `(session, address)` pairs (V6, V7, V45).
- **Other clients' terminals.** The default core is shared. Every call is label-scoped (V6, V17).
- **Moving a busy agent.** `MoveEligibility` is built from heuristics. Unsupported agents are refused, every input
  path is counted, the kill is confirmed before the replacement, one canary, stop at first failure (V8, V20, V39,
  V40, V25).
- **Rollback duplicates.** The PR B beta is the rollback target (D14); older builds keep the D10 rule (V21).
- **Persisted state and identity.** Two optional link fields and a new `core_terminal.session` value. Refactor-risk
  under AGENTS.md: invariants with adversarial identity state (V13) and backward reading (V31).
- **Waiting panes that never come back.** A pane restored `CoreWaiting` for a core nobody starts (a secondary) waits
  until a person acts; the notice names `herdr pane respawn` and the session.
- **Security.** New: the server starts its core again after a loss, and a new method can respawn panes (same reach as
  `pane respawn` today). Two-vendor review at ship.

## Verifier: required tests

"Fake core" is the in-process test server of core-backend (`core_backend/fake.rs`), extended to serve two sessions.
"Fake process table" is an injected `ProcessIdentity` lookup. "Live core" runs against a real hc 0.1.21+ in a
scratch `HC_SESSION` and scratch XDG directories (`just core-live-test`); any test that uses session `default` first
asserts `XDG_RUNTIME_DIR` and `XDG_STATE_HOME` are under its own temp folder, so it can never reach ac's default
core. "Live server" is a throwaway herdr through `scripts/core_live_test.sh`. "On the Mac" runs on ac's machines
after a beta, by the named actor.

| # | Proves | Where | Answer key | Pass |
|---|---|---|---|---|
| V1 | exit mapping (D1) | unit, `core_backend/pane_io.rs` | D1 table | each reason maps as the table, `server_stop` to `CoreStopped`; `killed` with no signal stays `Exited` |
| V2 | a holder loss suspends and resumes (D1, D2) | fake core: CLOSE `holder_lost` on three panes in one tab | pane-launchd-jobs D3 | three panes suspended, layout and workspace kept, one `HolderLost` notice; after the core answers and the pids are gone, each resumes by its plan |
| V3 | three reattach outcomes (D1a) | unit, the real `reattach` loop with `Shared`'s limit set to 200 ms, fake connector and fake process table | D1a | connector always failing: no exit after the limit, `core_unreachable_since` set, loop still retrying; `not_found` with the identity running: `Unreachable`; `not_found` with it gone or reused (start token differs): `Gone(TerminalGone)`; pid 0: `Unreachable` |
| V4 | `CoreLossGuard` (D3a) | unit, clock passed in | D3a | no start before 3 s of missing or refused socket; three bursts in ten minutes give `GaveUp`; events within 2 s are one burst; reload resets; a secondary session is never started |
| V5 | label and session names (D5) | unit, `core_backend/mod.rs` | D5 | release default: session `default`, label `herdr`; named: `herdr-<name>` both; debug: `herdrdev[-<name>]` both |
| V6 | claims across two sessions (D6) | fake core with sessions `default` and `herdr`, `t7` in both | D6 | each `t7` claimed by its own pane; an unlabelled terminal in `default` is not claimed, listed or killed; orphans listed per session with the right `HC_SESSION` and label in the notice |
| V7 | a link keeps its session (D6) | unit, `src/persist/snapshot.rs` | D6 | a pane reattached in `herdr` saves `"session": "herdr"` after the primary became `default` |
| V8 | `MoveEligibility` (D9) | unit, pure function | D9 table | each fact gives its refusal; an idle Claude pane with an empty box, no background work, no foreign process and a plan is movable |
| V9 | the move is atomic and old servers are safe (D9) | unit, `src/app/api.rs`; CLI against a server without the method | D9 | input inside 60 s refuses; `check_only` changes nothing; a server without the method answers unsupported and nothing respawns |
| V10 | restored panes on core (D8) | live core, replaces `core_v1_a_restored_pane_stays_on_the_pty` | D8 | `runtime = "core"`: a restored pane runs on core with its history; `"pty"`: on the PTY; an fd-adopted pane stays PTY |
| V11 | spaces leaves from the primary only (D7) | unit, `core_spaces_doc.rs` | D7, core-spaces-doc D2 | a pane linked to `herdr` while the primary is `default` is left out like a PTY pane, its split collapsed; a primary pane is a leaf |
| V12 | endpoint compatibility | existing fixture tests | AGENTS.md § Stable client endpoint contract | generation-1 fixtures and digests unchanged; only the new JSON API method and optional fields are added |
| V13 | identity invariants | unit, `AppState::assert_invariants_for_test` on `test_with_adversarial_identity_state` | AGENTS.md § Testing | invariants hold after suspend, wait, resume and move of core panes |
| V14 | live: holder loss | live core: SIGTERM the scratch core's holder with two shells and an agent stand-in (compiled stub `claude`, reported with `--resume sess-p3`) | D1, D2 | all three suspended, then running on a new holder; the stub shows `args=[--resume sess-p3]`; the workspace kept |
| V15 | live: core server loss | live core: SIGKILL the scratch core server | D3 | the socket refuses; herdr starts a core after 3 s or more; panes reattach; program pids unchanged; nothing suspended |
| V16 | live: holder loop | live core: kill the holder three times, each after the resume | D3a | no core start counted; the third burst gives `GaveUp`; the third resume runs on the PTY; one "keeps ending" notice; `herdr server status` shows `core: gave_up` |
| V17 | live: two sessions | live core, scratch `default` and `herdr`: a pane started under phase 2 naming, then a build with D5 | D5, D6 | the old pane is re-adopted from `herdr`; a new pane goes to `default`; an unlabelled `default` terminal is untouched |
| V18 | live: restore on core | live core: `server stop`, then start from the saved session | D8 | restored panes on core; the agent's resume command typed once |
| V19 | live server: mixed handoff | `scripts/core_live_test.sh`: one PTY pane, one pane in `herdr`, one in `default`; live handoff, restart, stop | core-backend V10 | pids kept across handoff and restart; stop ends both core sessions' panes |
| V20 | live server: idle move | `scripts/core_live_test.sh`: an idle shell (no input for 10 min, clock injected), a shell running `sleep 30`, a pane typed into 10 s before, an idle agent stub | D9 | idle shell moved with its directory; the busy and typed-into panes refused with their reasons; the stub moved with identical plan arguments |
| V21 | live server: rollback | `live-handoff --import-exe` to the PR B beta with a pane in `default`; then to beta 202 | D10, D14 | PR B beta re-adopts the `default` pane, nothing fresh, pid kept; beta 202 restores it fresh and its terminal keeps running; the next phase 3 build lists it as an orphan |
| V22 | CPU gate | `scripts/core_busy_profile.sh`, D11 protocol | PTY runs of the same series; phase 2 totals | gate 1 and gate 2 met in both profiles, with the noise band recorded; else PR E, then this row again |
| V23 | render path, if PR E lands | `just bench-render-scale`, 1 and 15 panes | `openspec/changes/core-backend/bench-render-phase2.txt` | median within noise; scaling delta reported |
| V24 | on the Mac: the phone (ac) | Herdr Mobile on m4m, then ac-mbm5, bridge removed | #293 row 16 | herdr-max panes listed from `default`; the Inbox shows their states |
| V25 | on the Mac: canary move (mac-m4 on m4m, mac-m5 on ac-mbm5, announced first) | `scripts/core_move_idle_panes.sh` on m4m, then ac-mbm5 | D9 | the canary keeps its conversation id, name, model, effort and permission mode; then the batch; `*-hlp` panes refused `excluded`; no PTY pane left outside the exclusions |
| V26 | on the Mac: quit drill (ac or a computer-use session) | a named throwaway herdr session (own core `herdr-<name>`); Activity Monitor "Quit" on that core's holder pid | D1, D2 | its panes suspended and resumed; ac's own sessions untouched |
| V27 | on the Mac: retire the legacy core (mac-m4 on m4m, mac-m5 on ac-mbm5, announced first) | D12 steps 3 and 4 | D12 | probe command lists `default`; legacy core stopped; herdr logs no error; no orphan notice |
| V28 | upstream evidence (herdr-core) | throwaway core: D4 cases 1 and 2 | D4 | each case recorded as reproduced or not, with versions; a reproduced one drafted in bug-template form and sent as an `ASK:` to manager-lab's desk for ac to submit |
| V29 | live server: unreachable core is never a fresh start (D1b) | `scripts/core_live_test.sh`, two cases. (a) primary: SIGKILL the scratch core server, then within 3 s a herdr restart, and separately a live handoff. (b) secondary: a pane in scratch `herdr` while the primary is scratch `default`; SIGKILL that core server, never started again; herdr restart, then live handoff | D1a, D1b | every program's pid unchanged and no `terminal.run` and no PTY spawn for those panes; (a) reattached once the primary core is back; (b) panes `CoreWaiting` through both, then reattached after `hc server start` by hand |
| V30 | restore outcome table (D1b) | unit, `src/persist/restore.rs`, fake core and fake process table | D1b table | each row gives its action; a phase 2 link (no pid) with the core unreachable waits; gone with no identity runs the known-session search first |
| V31 | link fields backward compatible (D1b) | unit, `snapshot.rs` | D1b | a phase 2 link parses with `pid: None`; a phase 3 link parses into a copy of the phase 2 struct; `pid` absent from JSON when `None` |
| V32 | `core_pty_panes` matching (D13) | unit, `excluded_from_core` and its callers | D13 | `["manager-lab-hlp", "manager-aruba-hlp"]` and `["*-hlp"]` match panes named so by `--name` and by an explicit agent name; a title or a self-reported name `manager-lab-hlp` does not match; new pane, restore, resume, respawn give `Pty(Excluded)`; `MoveEligibility` gives `excluded`; reload changes the answer |
| V33 | live server: an excluded pane stays PTY (D13) | `scripts/core_live_test.sh`, `runtime = "core"`, `core_pty_panes = ["*-hlp"]`, stub `claude --name test-hlp` | D13 | after restart it restores on the PTY; after a holder-loss-shaped suspension (pane job ended) it resumes on the PTY; `pane respawn --if-idle` refuses `excluded`; its pid is a PTY child throughout |
| V34 | `not_found` with the program alive | fake core answering `not_found` for the pane's address while the identity recorded at attach is a real `sleep` the test started (real process table, not a fake) | D1a | pane stays `Unreachable`, no suspension, no second program; after the test kills the `sleep`, the next retry gives `Gone(TerminalGone)` |
| V35 | dead-program check before resume (D2) | unit, `pane_suspension.rs`, fake process table | D2 | for each of the six causes: identity running gives no resume and the "still runs" text; gone gives resume; pid 0 gives no automatic resume and the "cannot confirm" text; a running detected agent pid blocks even when the terminal pid is gone |
| V36 | guard visible and reset (D3a) | unit and `herdr server status` JSON | D3a | `GaveUp` appears as `core: gave_up`; one notice; a new server (handoff) starts `Allowed`, as the docs say |
| V37 | `server_stop` (D3b) | live core: `hc server stop` on the scratch primary | D3b | panes `CoreStopped`, one notice; no core start for 60 s; `hc server start` by hand, then each resumes after the pid check; a second run ends with a herdr restart instead, which starts the core and resumes |
| V38 | start rules (D3) | unit over `connect_session` with a socket file that refuses; the probe tables above as the answer key | D3, Context probes | a refused socket counts as missing for the primary only; a start that exits 5 is polled again; a secondary is never started |
| V39 | confirmed kill (D9) | unit, `CorePaneIo::kill` against the fake core | D9 move steps | `terminal.kill` `live`: refused `core_unreachable`, no replacement, pane unchanged; timeout or error: suspended, no replacement; `finished` with the pid gone: replaced; `finished` with the pid running: suspended, its exit keeps the pane (fork issue 301) |
| V40 | move facts per agent and input paths (D9) | unit for the paths; on the Mac live smoke for Claude | D9 | a Codex pane: `unsupported_agent`; `pane.send_text`, a queued Submit still pending, and `agent.message` note `queued` each give `recent_input`; a resume launch does not; real Claude Code (`--settings ~/.config/claude/direct-noconnectors.settings.json`) with a background agent running: `background_work`; with a foreground subagent: `agent_busy`; record the CLI version |
| V41 | foreign process in an agent pane (D9) | unit over a fake process table | D9 | a `sleep &` started from the agent's shell escape, outside its tree: `foreign_process`; the agent's own MCP children: no refusal |
| V42 | `HC_SOCKET` is the primary's only (D6) | unit, `client.rs` | D6 | with `HC_SOCKET` set, the primary's path is it and a secondary's path follows core's rules |
| V43 | resume off the loop (D2) | unit, fake core answering `terminal.run` after 2 s | D2 | batches of at most 8, one batch in flight; the loop's longest turn while 1 and while 16 panes resume stays under 50 ms; both numbers reported |
| V44 | PR B beta installed first (D14) | R0 record on #295 | D14 | the PR B beta number on both Macs, by live handoff, before PR C merges |
| V45 | known-session search (D1b) | fake core with `default` and `herdr` | D1b | a pane whose link names `herdr` but whose labelled terminal is in `default` attaches there and its link is rewritten; two matches start nothing and raise one notice; none starts fresh |
| V46 | the next real `hc update` (herdr) | herdr's log on m4m when ac next updates hc | D3 | the reattach loop's logged gap is under 3 s and no core start was made; recorded in `v12-phase3.md` |

## Roundtable 2026-11-21

A four-model adversarial roundtable reviewed the first draft (3be26585). Each item it asked to act on, and what
resolves it:

| # | Finding | Resolved by |
|---|---|---|
| 1 | A restore or handoff starts a pane fresh on any reattach error, duplicating live agents | D1b (identity in the link, restore table, `CoreWaiting`); V29, V30, V31; specs "The saved link records the program" and "Restore never starts a pane fresh while its program may run"; tasks A1.2-A1.4 |
| 2 | The skip-list must be server config, honoured on every spawn path | D13 (`core_pty_panes`, stable name, `RuntimeChoice`); V32, V33; specs "Some panes stay on the PTY by server config" and "An excluded pane stays on the PTY on every path"; PR S |
| 3 | `WaitFailed` conflates gone and 60 s unreachable | D1a (`ReattachOutcome`); V3, V34; specs "A lost stream is gone only when the program is gone" and "An unreachable core never suspends a pane"; A1.1 |
| 4 | A pid and start-time check before every resume | D2 (dead-program check, pid 0 may run, agent pid); V35; spec "Every resume checks that the old program ended"; A1.5 |
| 5 | The loss guard counts core starts, which a holder loop never makes | D3a (loss bursts, `core: gave_up`, reset documented); V16, V36; A2.3 |
| 6 | `server_stop` behaviour left open | D3b (suspend, no herdr start, resume when the core returns or on herdr restart); V37; removed from open questions; A2.4 |
| 7 | A rollback after the placement move lands on a build that cannot see `default` | D14 (PR B reads `default`, its own beta before C), D1b known-session search; V21, V44, V45; R0 |
| 8 | The idle move starts a replacement without a confirmed kill | D9 move steps (`KillOutcome`); V39; spec "A move starts the replacement only after a confirmed kill"; G.2 |
| 9 | `MoveEligibility` gaps: unsupported agents, foreign processes, input paths, Claude subagents | D9 table and paragraphs; V8, V40, V41; specs "Input on every path is recent input" and "What idle means for an agent pane"; G.1 |
| 10 | Core start rules: a SIGKILLed core leaves its socket; start races; the 3 s window untested | D3 (refused as missing), Context probe table; V15, V38, V46; A2.2 |
| 11 | Spaces document after the move points legacy addresses at the wrong core | D7 (primary-only leaves; PR D shrunk to that); V11; PR D |
| 12 | Label and socket call sites not listed | D5 call-site table, D6 `HC_SOCKET`; V6, V42; B.3 |
| 13 | Mass resume blocks the server loop | D2 (resume worker, batches of 8, cardinality note); V43; A2.1 |

Considered and decided:

- **CPU gate power.** Eight runs a side, a stated noise band with eight more runs inside it, and a second gate on
  core's total CPU against phase 2's own total (D11).
- **SSH-started core detection.** ppid 1 is not evidence (measured); positive evidence only, from the core's
  session leader, and core's own status fields once filled (D7).
- **The zshenv bridge.** Removed through a per-host chezmoi template, each Mac after its own move (D12).
- **Notice text.** One text per suspend cause (D2 table).
- **Idle-move state loss.** A moved shell's process state is lost, accepted for shells idle for 10 minutes and
  documented; agents keep their conversation by the resume plan (D9).

## Open questions

1. **`hc update` gap with many terminals.** Not run live (Context). V46 records it from the next real update; if
   it exceeds 3 s, the window is widened before PR A2 ships to the Macs.
2. **Peer messages outside herdr.** agent-bell's own channel to Claude is invisible to the server; the driver's
   pre-move check depends on agent-bell reporting a session's queued messages. If it cannot, such panes rely on the
   60 s idle rule alone.
3. **Forged labels in the shared default core.** Another same-user client can label a terminal
   `herdr.session=herdr` and a pane id. The known-session search and orphan scan would then offer it. Same-user
   trust is phase 2's boundary; whether `default` changes that is for the security review at PR C.
4. **Restored history and core's screen.** Restored history lives only in herdr's parser (D8). Whether any fleet
   tool reads core's screen for a restored pane and expects the earlier history is unconfirmed.
