# Core phase 3: design

Phase 3 of the herdr-core migration (`openspec/changes/herdr-core-client/design.md` § 6.4, "Restore and update
through core"). Issue: https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/295.

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

What pane-launchd-jobs gives (`openspec/changes/pane-launchd-jobs/design.md` D3, `src/app/pane_suspension.rs`):
a pane whose program the session ended keeps its place as Suspended, keeps its agent's resume plan, and starts
again in place 5 s after the last loss (30 s after a burst of three within 2 s), or at once on a client attach,
once `pane_jobs_ready_probe` says launchd accepts jobs. Resume runs `respawn_pane_agent`, else the launch
command, else a shell. `suspended` is an optional pane fact, so old clients are safe. V7 (a reboot on m4m) kept
49 of 51 panes and resumed 32 of 34 Claude sessions with the same id.

What herdr-core 0.1.21 does (core `schema/README.md` § Events, `crates/herdr-core/src/SKILL.md`, `docs/security.md`):

- The default session is named `default`. `HC_SESSION` selects another core, terminals and state directory. The
  client socket is `<session>.sock` under `$XDG_RUNTIME_DIR/herdr-core/`, else the state directory.
- A finished terminal's `reason` is one of `exited`, `killed`, `server_stop`, `host_shutdown`, `holder_lost`,
  `stopped_by_restart`. `terminal.keep` replaces a lost program with a shell and never re-runs the command.
- Unexpected holder loss sweeps the programs' detached processes too.
- On macOS a desktop start runs the core as a one-shot launchd job; an SSH start stays a direct child, and
  "SSH versus launchd is still chosen by that launcher".

Measured for this change (2026-11-21, m4m, hc 0.1.21, throwaway `HC_SESSION=p3probe` with scratch XDG
directories, removed afterwards):

| Action | Result |
|---|---|
| `server.status` with two terminals | core server pid 9604 (ppid 1, `~/.local/bin/hc`); one holder, pid 11393, ppid 9604, `Herdr Core.app/Contents/MacOS/herdr-core`, `terminals: 2` |
| SIGTERM to the holder | both terminals `finished`, `reason: holder_lost`; `holders: []` |
| SIGKILL to the core server | the holder (now ppid 1) and its program kept running; the next `hc` call started a core, which listed the holder with `terminals: 1` and the terminal `live` |
| SIGKILL to that holder | its terminal `finished`, `holder_lost`; the program was gone |

So the process Activity Monitor shows as Herdr Core is the holder. Quitting or killing it ends every program it
holds. The core server itself can die without losing a program, provided a core is started again.

## Goals and non-goals

Goals:

- A core loss never costs a pane its place, its agent conversation or its workspace.
- Herdr Mobile and the Herdr apps see herdr-max's panes with no shell bridge.
- Every pane on both Macs runs on core, moved without interrupting an agent's work.
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
| `holder_lost`, `host_shutdown`, `server_stop`, `stopped_by_restart` | `Interrupted` | `Suspended` |
| connection lost, then the terminal is gone (D1a) | `WaitFailed` | `Suspended` |

The table lives in one function, `exit_reason`, so the mapping is one match and one table test.

**D1a. No suspension while the program may still run.** A suspension leads to a resume, and a resume starts the
agent again. If the old program still runs (its holder alive, the core unreachable) the result is two copies of
one conversation. So the "gone" row needs two facts: a core that answers (`terminal.get` says `not_found` or
`finished`), and the program's pid, checked with the start time recorded at attach, no longer running (the
pid-reuse-safe check of pane-launchd-jobs D2). While the core does not answer, the pane stays in its reattach
loop (core-backend D5) and shows "Herdr Core unreachable" in place. It is never resumed on a guess.

Alternatives:

- `terminal.keep` on every pane, so core replaces a lost program with a shell. Rejected: core never re-runs the
  command, so an agent pane comes back as a bare shell, and the kept record stays `live`, which our re-adoption
  would then attach to as if it were the agent.
- Keep `Interrupted` and rely on the next server start to restore. Rejected: the panes and an emptied workspace
  close now, and nothing restarts them until ac restarts herdr.

### D2. Resume waits for the backend the pane will use

`pane_jobs_ready_probe` becomes a readiness check for the backend a resumed pane will start on: with
`runtime = "core"`, the primary core answers `hello` (starting it per D3 if allowed); otherwise today's pane-jobs
probe. The timing stays pane-launchd-jobs D3's: 5 s settle, 30 s after a burst (a logout delivers
`host_shutdown` to every terminal at once), retry every 10 s, at once on a client attach. Resume itself is
unchanged: `respawn_pane_agent`, else the launch command, else a shell, each through `spawn_preferred`.

A notice names the loss once per burst: "Herdr Core ended N panes; they resume in place". Log lines carry the
core reason.

Alternative: a separate resume path for core panes. Rejected: the suspended state, its timing, its client fact
and its tests already exist; a second path would differ only in the probe.

### D3. herdr-max starts its primary core again after a loss

The rule "start the core only before the first connection" becomes: the server may start its **primary** core
session (D6) when its socket has been missing or refusing for at least 3 s without a break. The window clears a
`hc server restart` (socket missing about 0.15 s, phase 0 Q6) and a `hc update` (0.6 s). The start uses the clean
environment of core-backend D2 and goes through `hc server start`, so it is the same launchd one-shot. A
secondary session (the old `herdr` core, D6) is never started.

A pure `CoreLossGuard` decides, with the clock passed in:

- `Allowed` until three starts after a loss fall within ten minutes;
- then `GaveUp` for the rest of the server's life (a config reload resets it): no more starts, suspended panes
  resume on the PTY (core-backend D1's fallback), and one notice says "Herdr Core keeps ending; panes resume on
  the PTY".

A core-server-only loss (the probe's SIGKILL row) then repairs itself: the new core reconciles the live holder,
the panes' streams reattach (core-backend D5), and the programs keep their pids. No pane is suspended.

This narrows core-backend D2's "never restart" to what it protected: herdr-max still never runs `hc server stop`,
`restart` or `update`, and never touches a core session it does not own as primary.

Alternatives:

- Never start the core again. Rejected: suspended panes would wait forever, and new core panes would fall back to
  the PTY until herdr restarts.
- Run `hc server restart` on a hung core. Rejected: it acts on every terminal in the session, including other
  clients' in the default core (D5).

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

Only a reproduced case becomes a one-screen report (current, expected, shortest repro, impact, version), drafted
for ac on the desk; ac decides whether to submit it. A design wish (holder per terminal) is a GitHub Discussions
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

A terminal is ours when it carries `herdr.session=<label>`; reattach also checks `herdr.pane_id` (core-backend
D10). In the default core, terminals without our label (Herdr.app's, the phone's, an agent's `hc run`) are never
claimed, never listed as orphans and never killed. Every core call herdr-max makes there is scoped by that
label filter or by an address it saved.

Alternatives:

- Rename the label to `herdr-max.session`, away from the `herdr.` prefix the apps use for shared labels
  (herdr-app `docs/labels.md`). Rejected for phase 3: every live terminal carries `herdr.session`, so a rename
  needs a dual-read period for no present conflict. Revisit if the apps ever define `herdr.session`.
- A config switch for the session move. Rejected: re-adoption across sessions (D6) makes the move reversible in
  code without a second configuration to test, and the rollback rule (D10 below) covers older builds.

### D6. One connection per core session, links carry their session

The single `CLIENT` slot becomes a `CoreSessions` registry keyed by session name:

- **primary**: `core_session_name()`; may be started (D3); new panes go here.
- **secondary**: any other session a restored or handed-off pane's link names, plus the legacy `herdr` session
  when the primary is `default` and the legacy socket exists. Never started; connected only while its socket
  answers; dropped when no pane uses it and its orphan scan is empty.

Each `CorePaneIo` stores its session. `CoreTerminalLink.session` comes from the pane, not from
`core_session_name()` (today's `PaneIo::core_terminal` would relabel a legacy pane as `default` on the next
save, and the next restore would look for it in the wrong core). Claims are `(session, address)` pairs, because
addresses are per core: `t7` can exist in both. The orphan scan runs per session with the label filter.

`HC_SOCKET` applies to the primary session only. Secondary sessions use core's path rules.

The persisted field keeps its shape, so no persisted-state migration is needed: phase 2 links already say
`"session": "herdr"`.

Alternative: move every legacy terminal to the default core at once and keep one connection. Rejected:
terminals cannot move between cores, so "moving" means restarting every agent at once, which is what the idle
move (D9) exists to avoid.

### D7. Sharing the default core with other clients

- **Launch context.** The default core may already run, started by Herdr.app (launchd) or by the phone over SSH
  (a direct child of an SSH login, which can lack GUI keychain and privacy context and "may end with its
  launching app"). On connect, the server compares the core's parent: a core whose parent is not launchd (pid 1)
  gets one warning and one notice: "Herdr Core in session default was started over SSH; restart it from a desktop
  terminal". The server never restarts it (D3). The pane's environment is unaffected either way, because the
  keep-list wrapper gives each program exactly its own variables (core-backend M1).
- **The `herdr-app` document.** PR 59 (`openspec/changes/core-spaces-doc/`) writes the whole document, which is
  right for a core only herdr-max uses. In the default core, Herdr.app writes the same document. So herdr-max
  writes only the spaces it marks as its own (a field the app keeps through `rest`, and an id prefix as a second
  mark in case an app release drops unknown fields), keeps every other space as read, re-reads on a CAS conflict,
  and on "off" removes its own spaces instead of writing `null`.
- **The status mirror** (`openspec/changes/core-status-mirror/`) writes to the pane's terminal device, so it works
  in whichever core holds the pane. No change.

### D8. Restored and respawned panes start on core

With `runtime = "core"`, a restored pane with no live saved terminal starts on the primary core, with its saved
history seeded into herdr's parser as today. Core's own screen starts with the fresh program, so the phone shows
no pre-restore history (accepted). A pane adopted by fd in a live handoff stays on the PTY: its program is alive
and cannot change backend. Respawns already go through `spawn_preferred`. `core_v1_a_restored_pane_stays_on_the_pty`
is replaced by a test that states the new rule.

### D9. Moving an idle pane

A new server method, `pane.respawn_if_idle {pane, check_only}`, answers `{moved, refusals}`. A new method name is
required because an older server would ignore an `if_idle` field on `pane.respawn` and respawn a busy agent
(AGENTS.md § Stable client endpoint contract, on method shapes). The CLI is `herdr pane respawn --if-idle
[--check]`.

The server decides in one app-loop turn, from facts it already holds, through a pure `MoveEligibility` function:

| Fact | Refusal |
|---|---|
| the pane runs on the primary core | `already_on_primary` |
| suspended, an overlay or popup pane, or a worktree removal is stopping it | `not_movable` |
| client input to the pane in the last 60 s | `recent_input` |
| agent pane: state not idle for 60 s | `agent_busy` |
| agent pane: `background_activity` or `background_count > 0` | `background_work` |
| agent pane: the manifest's `prompt_box_body` is not empty | `draft_in_box` |
| agent pane: no resume plan | `no_resume_plan` |
| shell pane: not `shell_at_prompt`, a foreground job, or more than the shell in its session | `shell_busy` |

A move is `respawn_pane_agent` (agent) or a shell respawn in the pane's directory (shell), on the primary core.
It covers PTY panes and panes in the legacy `herdr` core alike.

The driver, `scripts/core_move_idle_panes.sh`, is the lever a person reruns:

1. lists panes not on the primary core, minus `--exclude` globs (default `*-hlp`) and the names in
   `~/.config/herdr/core-move-exclude` (the computer-use host panes such as `manager-lab-hlp` and
   `manager-aruba-hlp`: their Screen Recording and Accessibility grants belong to the PTY program, and a core
   pane's privacy prompts name Herdr Core, phase 0 Q9);
2. moves one canary, then verifies it: same agent session id once its first hook report lands, same pane name,
   the new process's argv carries the plan's `--model`, `--effort`, `--permission-mode` and `--settings`, and the
   agent's status line shows the same model before and after;
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

**Rollback rule.** core-backend D10's rule now covers two sessions. An older build (beta 202 or earlier) looks
only in core session `herdr`: it restores a `default` pane fresh and leaves its program running unseen. Before
rolling back, close core panes or list them with `HC_SESSION=default hc ls --label herdr.session=herdr` and
`HC_SESSION=herdr hc ls --label herdr.session=herdr`.

### D11. The CPU gate

Before the rollout, `scripts/core_busy_profile.sh` runs PTY and core alternately, four runs a side, in both
profiles (default, and `V12_ONE_PROCESS=1`), each with `V12_TOP` counters and the load average recorded, release
build, under the m4m heavy lock. Pass: median server CPU on core within 1.3x of the PTY in both profiles. Core and
holder CPU, and the idle cost of a second running core (legacy `herdr` beside `default`), are recorded, not gated.
Results go to `v12-phase3.md` in this change.

Over 1.3x: a conditional PR replaces the per-chunk hop with a reader that hands whole batches to each pane's
parser (core-backend `v12-results.md`, "Not changed"). It must keep one pane's parse off the shared reader and
show that no caller waits on a core reply while holding a terminal lock. Because the hop is per byte batch times
panes, the PR also profiles 1 and 15 panes (AGENTS.md § Multiplicative performance paths).

### D12. Removing the bridge and the old core

On each Mac, in order:

1. The phase 3 beta runs; new panes land in `default`.
2. The idle move (D9) empties the legacy `herdr` core of herdr-max panes, except panes that are never idle; those
   end on their own or wait for ac.
3. The bridge line is removed from chezmoi `dot_zshenv` (relayed to the dotfiles owner, never edited from this
   repo). The app's recorded probe command, run through `zsh -c` with an SSH-like environment and a
   `terminal.list`, then lists the default core's terminals, herdr-max's included. ac checks the phone (V24).
   Panes still in the legacy core stay off the phone until they move or end.
4. With the legacy core holding no live terminal, ac stops it: `HC_SESSION=herdr hc server stop`. herdr-max drops
   the secondary connection when the socket goes.

## Risks

- **Double agents.** A resume while the old program lives duplicates a conversation. D1a requires a core answer and
  a dead pid. V3 and V14 guard it.
- **Restart loops.** A core that keeps dying could be started forever. `CoreLossGuard` (V4, V16).
- **Wrong pane, wrong core.** With two sessions, an address alone can match a terminal in the other core. Claims and
  links are `(session, address)` pairs (V6, V7).
- **Other clients' terminals.** The default core is shared. Every call is label-scoped (V6, V17).
- **Moving a busy agent.** `MoveEligibility` is built from heuristics (detection state, prompt box, background count).
  A wrong "idle" ends work in flight. One canary, per-pane verification, stop at first failure (V8, V20, V25).
- **Rollback duplicates.** An older build cannot see `default` panes (D10 rule, V21).
- **Persisted state and identity.** No new field, but `core_terminal.session` takes a new value, and suspension now
  reaches core panes. Refactor-risk under AGENTS.md: invariants with adversarial identity state (V13), and a
  roundtable before code.
- **Security.** New: the server starts its core again after a loss, and a new method can respawn panes (same reach as
  `pane respawn` today). Two-vendor review at ship.

## Verifier: required tests

"Fake core" is the in-process test server of core-backend (`core_backend/fake.rs`), extended to serve two sessions.
"Live core" runs against a real hc 0.1.21+ in a scratch `HC_SESSION` and scratch XDG directories (`just
core-live-test`); any test that uses session `default` first asserts `XDG_RUNTIME_DIR` and `XDG_STATE_HOME` are
under its own temp folder, so it can never reach ac's default core. "Live server" is a throwaway herdr through
`scripts/core_live_test.sh`. "On the Mac" runs on ac's machines after a beta, by the named actor.

| # | Proves | Where | Answer key | Pass |
|---|---|---|---|---|
| V1 | exit mapping (D1) | unit, `core_backend/pane_io.rs` | D1 table | each reason maps as the table; `killed` with no signal stays `Exited` |
| V2 | a holder loss suspends and resumes (D1, D2) | fake core: CLOSE `holder_lost` on three panes in one tab | pane-launchd-jobs D3 | three panes suspended, layout and workspace kept, one burst notice; after the core answers, each resumes by its plan |
| V3 | no suspension while the program may run (D1a) | unit over a fake process table and a fake core | D1a | core unreachable and pid alive: reattach loop, no suspension; core says `not_found` and pid gone: suspended; pid reused (start time differs): suspended |
| V4 | `CoreLossGuard` (D3) | unit, clock passed in | D3 | no start before 3 s of missing socket; third start in ten minutes, then `GaveUp`; reload resets; a secondary session is never started |
| V5 | label and session names (D5) | unit, `core_backend/mod.rs` | D5 | release default: session `default`, label `herdr`; named: `herdr-<name>` both; debug: `herdrdev[-<name>]` both |
| V6 | claims across two sessions (D6) | fake core with sessions `default` and `herdr`, `t7` in both | D6 | each `t7` claimed by its own pane; an unlabelled terminal in `default` is not claimed, listed or killed; orphans listed per session with the right `HC_SESSION` in the notice |
| V7 | a link keeps its session (D6) | unit, `src/persist/snapshot.rs` | D6 | a pane reattached in `herdr` saves `"session": "herdr"` after the primary became `default` |
| V8 | `MoveEligibility` (D9) | unit, pure function | D9 table | each fact gives its refusal; an idle agent with an empty box, no background work and a plan is movable |
| V9 | the move is atomic and old servers are safe (D9) | unit, `src/app/api.rs`; CLI against a server without the method | D9 | input inside 60 s refuses; `check_only` changes nothing; a server without the method answers unsupported and nothing respawns |
| V10 | restored panes on core (D8) | live core, replaces `core_v1_a_restored_pane_stays_on_the_pty` | D8 | `runtime = "core"`: a restored pane runs on core with its history; `"pty"`: on the PTY; an fd-adopted pane stays PTY |
| V11 | shared `herdr-app` document (D7) | unit, `core_spaces_doc.rs` and `spaces_doc.rs` | D7 | own spaces replaced, others kept byte for byte, conflict re-reads, off removes only own spaces |
| V12 | endpoint compatibility | existing fixture tests | AGENTS.md § Stable client endpoint contract | generation-1 fixtures and digests unchanged; only the new JSON API method is added |
| V13 | identity invariants | unit, `AppState::assert_invariants_for_test` on `test_with_adversarial_identity_state` | AGENTS.md § Testing | invariants hold after suspend, resume and move of core panes |
| V14 | live: holder loss | live core: SIGTERM the scratch core's holder with two shells and an agent stand-in (compiled stub `claude`, reported with `--resume sess-p3`) | D1, D2 | all three suspended, then running on a new holder; the stub shows `args=[--resume sess-p3]`; the workspace kept |
| V15 | live: core server loss | live core: SIGKILL the scratch core server | D3 | herdr starts a core after 3 s or more; panes reattach; program pids unchanged; nothing suspended |
| V16 | live: loss loop | live core: kill the holder three times, each after the resume | D3 | the third resume runs on the PTY; one "keeps ending" notice |
| V17 | live: two sessions | live core, scratch `default` and `herdr`: a pane started under phase 2 naming, then a build with D5 | D5, D6 | the old pane is re-adopted from `herdr`; a new pane goes to `default`; an unlabelled `default` terminal is untouched |
| V18 | live: restore on core | live core: `server stop`, then start from the saved session | D8 | restored panes on core; the agent's resume command typed once |
| V19 | live server: mixed handoff | `scripts/core_live_test.sh`: one PTY pane, one pane in `herdr`, one in `default`; live handoff, restart, stop | core-backend V10 | pids kept across handoff and restart; stop ends both core sessions' panes |
| V20 | live server: idle move | `scripts/core_live_test.sh`: an idle shell, a shell running `sleep 30`, a pane typed into 10 s before, an idle agent stub | D9 | idle shell moved with its directory; the busy and typed-into panes refused with their reasons; the stub moved with identical plan arguments |
| V21 | live server: rollback hazard | `live-handoff --import-exe` beta 202 with a pane in `default` | D10 rule | old build restores it fresh; the `default` terminal keeps running; the next phase 3 build lists it as an orphan |
| V22 | CPU gate | `scripts/core_busy_profile.sh`, D11 protocol | PTY runs of the same series | median server CPU on core at most 1.3x of PTY in both profiles; else the batch reader PR, then this row again |
| V23 | render path, if the batch reader PR lands | `just bench-render-scale`, 1 and 15 panes | `openspec/changes/core-backend/bench-render-phase2.txt` | median within noise; scaling delta reported |
| V24 | on the Mac: the phone (ac) | Herdr Mobile on m4m, then ac-mbm5, bridge removed | #293 row 16 | herdr-max panes listed from `default`; the Inbox shows their states |
| V25 | on the Mac: canary move (herdr, with ac's go) | `scripts/core_move_idle_panes.sh` on m4m, then ac-mbm5 | D9 | the canary keeps its conversation id, name, model, effort and permission mode; then the batch; host panes untouched; no PTY pane left outside the exclusions |
| V26 | on the Mac: quit drill (ac or a computer-use session) | a named throwaway herdr session (own core `herdr-<name>`); Activity Monitor "Quit" on that core's holder pid | D1, D2 | its panes suspended and resumed; ac's own sessions untouched |
| V27 | on the Mac: retire the legacy core (ac) | D12 steps 3 and 4 | D12 | probe command lists `default`; legacy core stopped; herdr logs no error; no orphan notice |
| V28 | upstream evidence (herdr-core) | throwaway core: D4 cases 1 and 2 | D4 | each case recorded as reproduced or not, with versions; a reproduced one drafted for ac in bug-template form |

## Roundtable questions

Refactor-risk under AGENTS.md (restore and handoff, persisted state, workspace and pane identity). The roundtable
runs before code moves and records its findings in `roundtable.md` here.

1. **D1 and D1a, suspension of core panes.** Is "a core that answers plus a dead pid with a matching start time"
   enough to rule out a live program? Which path could still resume an agent whose old program runs (a holder that
   hangs without dying, a core that answers but lags its holder)?
2. **D1, `server_stop` as a suspension.** Someone who ran `hc server stop` on purpose sees herdr start the core
   again and resume every pane. Should `server_stop` close panes instead, or suspend without restarting the core?
3. **D3, starting the core again.** Is a 3 s missing socket a safe sign of a loss, and not a slow update? After a
   logout (`host_shutdown`), can a start land the core in a login context that is ending, and does the 30 s burst
   settle cover it?
4. **D5 and D6, identity across two cores.** Every claim and link is now `(session, address)`. Which code still keys
   core panes by address alone (handoff manifest, orphan list, the spaces document, the status mirror's pin)? Can
   any restore path attach a pane to the other core's `t7`?
5. **D6, persisted links.** `core_terminal.session` changes value from `herdr` to `default` for new panes. What do a
   handoff snapshot and `session.json` written by a phase 3 build do in a phase 2 build, beyond the duplicate D10
   describes?
6. **D7, the shared default core.** Is the parent-pid test a reliable sign of an SSH-started core on macOS? Can
   another same-user client forge `herdr.session=herdr` and `herdr.pane_id` to be adopted, and does that open
   anything phase 2's own session did not?
7. **D9, the idle move.** Which false "idle" readings are possible per agent (Claude's in-process subagents,
   background shells, a queued prompt), and are the server's facts enough, or must the driver's per-agent
   verification also gate the move?
8. **D8, restored panes on core.** Restored history lives only in herdr's parser. Does any client, fleet tool or the
   spaces document assume core's screen and herdr's screen match for a restored pane?
