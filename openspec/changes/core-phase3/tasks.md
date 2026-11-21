Planned 2026-11-21 (herdr-core, for herdr; https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/295),
revised the same day after the roundtable (design.md § Roundtable 2026-11-21). Verifier rows V1-V46 are in design.md
§ Verifier. Each PR below is shippable on its own and ends in a check.

Heavy cargo on m4m only under the shared lock: `lockf -k /tmp/m4m-heavy.lock nice -n 15 env
CARGO_BUILD_JOBS=6 NEXTEST_TEST_THREADS=6 ZIG=/opt/homebrew/opt/zig@0.16/bin/zig <cmd>`. Live core tests use a
scratch `HC_SESSION` and scratch XDG directories, never ac's `herdr` or `default` core; any test of session
`default` asserts its XDG directories are under its own temp folder first. A test Claude Code runs only with
`--settings ~/.config/claude/direct-noconnectors.settings.json`; agent resumes are tested with a compiled stub.

Order (design D14): A1, A2, S, B, then R0 (the PR B beta on both Macs), then C, D, M (E only if M fails), F, G,
rollout R.

## 0. Before code

- [x] 0.1 Roundtable on the first draft; findings folded into design.md § Roundtable 2026-11-21
- [ ] 0.2 Verifier review by herdr (the lead); changes folded into design.md
- [ ] 0.3 Gitea sub-issues for PR A1, A2, S, C and G (`[fork]` titles), each linking #295

## PR A1. Loss outcomes, identity in the link, no fresh start on a guess (#287, herdr-max side)

- [x] A1.1 `exit_reason` per D1 with `SuspendCause`; `ReattachOutcome` in `pane_io.rs` with the limit on `Shared`,
  `core_unreachable_since` fact; V1, V3, V34
- [x] A1.2 `PaneCoreTerminalSnapshot.pid` and `pid_started` (optional), written from the attach identity; V31
- [x] A1.3 `RestoreReattach` table in `reattach_core_pane`, the `CoreWaiting` pane state and its 10 s retry; V30
- [x] A1.4 Known-session search before any fresh start; V45; live V29 in `scripts/core_live_test.sh`
- [x] A1.5 Dead-program check before every resume (all causes, pid 0 may run, agent pid); V35
- [x] A1.6 V13 invariants with suspended and waiting core panes; V2, V14 in `just core-live-test`
- [ ] A1.7 Check: `just check` 0, `just core-live-test` 0, CI green on Linux, macOS and Windows (both checks 0 on
  m4m, 2026-11-21; CI waits for the push)

A1 as built (47219db8, 6dd307f6):

- `ReattachOutcome::Gone` carries the `ChildExitReason` of D1, not a bare `SuspendCause`: a finished record whose
  program exited by itself (`exited`, a code) still closes the pane once the program is gone.
- `RestoreReattach` is `after_failed_attach` in `src/persist/restore.rs` (`Wait` or `Search`) plus `CoreRestore`.
  A live address labelled for another pane is treated as gone for this pane, except that a recorded program that
  still runs makes the pane wait (D1a: never a fresh start while it runs).
- The pane state is `TerminalState::hold: Option<PaneHold>` (`Suspended(Suspension)` or `CoreWaiting`), replacing
  the `suspended` flag. Waiting and suspended panes keep their link in the saved session, so the next restore checks
  it again. The API pane fact gains the optional `core_waiting` and `core_unreachable_since`.
- Secondary sessions are reached at their own socket, never started and never through `HC_SOCKET`
  (`connect_secondary`); the full `CoreSessions` registry stays PR B.
- V29 (b), a secondary session down, is covered in unit tests (`core_restore_tests`), not live: a live secondary
  needs the PR C placement.
- A `server_stop` pane resumes like the other causes until A2 adds D3b's stop record; with the core still down it
  resumes on the PTY.
- Two terminals labelled for one pane show the conflict on the pane; their separate notice comes with A2's notices
  (the orphan notice already names them).

## PR A2. Core start, loss guard, resume off the loop

- [x] A2.1 Resume worker: specs built on the loop, batches of 8 on `resume-worker`, `AppEvent::PaneResumed`; V43
  with the 1 and 16 pane numbers in the PR body (fake core answering `terminal.run` after 2 s: the loop's longest
  turn 0.11 ms with 1 pane, 0.55 ms with 16; at most 8 runs at once)
- [x] A2.2 `connect_session`: refused counts as missing, primary only, exit 5 polled again, the 3 s rule replacing
  `EVER_CONNECTED`; V38; V15 live (herdr started the killed core after 3.2-3.3 s, every pid kept)
- [x] A2.3 `CoreLossGuard` over loss bursts, `core: gave_up` in `herdr server status`, reset on a new server; V4,
  V36; V16 live
- [x] A2.4 `server_stop` as `CoreStopped` with the stop record (D3b); V37 live
- [x] A2.5 Notices per cause (D2 table) and the optional `suspend_cause` fact; V12 fixtures unchanged
- [x] A2.6 `docs/next`: core loss behaviour, the waiting state, the guard reset on handoff, the "Running in
  Background" rule; the same line relayed to the owner of CONTEXT `SKILLS/herdr` (this repo's
  `skills/herdr/SKILL.md` changes only at a stable release)
- [ ] A2.7 Check: `just check` 0, `just core-live-test` 0, CI green (both checks 0 on m4m, 2026-11-21; CI waits for
  the push)

A2 as built:

- The worker runs each pane of a batch on a thread of its own, so a batch takes as long as its slowest pane, and
  the next batch leaves when every answer of the last one has landed. Respawn and agent resume are split into
  prepare (on the loop), spawn and install (on the loop), so `pane respawn` and the restore path share the code.
- The 3 s rule applies to every primary connection after the first, the never-start connector included: a pane a
  restore reattached, a waiting pane's probe and the spaces-doc publisher can each start the primary after 3 s
  down. Only the server connector starts a core at once before the first connection. V29a now waits for herdr's
  own start instead of a start by hand.
- While the guard has given up, new panes start on the PTY too, not only resumed ones (`uses_core` gates
  `spawn_preferred`); PR S's `RuntimeChoice::Pty(GaveUp)` replaces the gate.
- A `server_stop` suspension is a person's act and not a loss event for the guard.
- The guard's notice reaches the app through an event sender the app registers with the core backend: a start
  that trips the guard happens on a pane's thread.
- Restored panes still start on the PTY until PR F, so in V37's second run (a herdr restart) the panes come back on
  the PTY and the core starts with the next new core pane.
- A waiting pane whose core answered and whose program is gone still starts fresh on the loop
  (`attach_waiting_pane`): one pane per probe answer, not a mass resume.
- The "stopped by a person" record (D3b) lives in the server's memory only. A herdr restart, or a live
  handoff, after `hc server stop` forgets it, so the new server's first connection starts the core again.

## PR S. `core_pty_panes`

- [x] S.1 `[experimental] core_pty_panes` parse and reload; `excluded_from_core` over the stable name (D13)
- [x] S.2 `RuntimeChoice` replacing `pty_only`; every caller of `spawn_preferred` passes it (new pane, restore,
  resume, respawn); V32 naming `manager-lab-hlp` and `manager-aruba-hlp`
- [x] S.3 V33 live in `scripts/core_live_test.sh` (restore and resume parts; the move part lands with G)
- [x] S.4 `docs/next` config reference for the key
- [ ] S.5 Check: `just check` 0, `just core-live-test` 0, CI green

S as built:

- `RuntimeChoice` and `PtyReason` live in `src/pane/runtime_io/choice.rs`. The reasons are `Excluded`, `GaveUp`
  and `Restored`. `FdAdopted` is not a variant: an fd adopted in a live handoff never reaches
  `spawn_preferred`. `Restored` marks a restored pane's fresh program until PR F starts it on core (D8); F
  removes it. The guard's give-up moved from inside `spawn_preferred` into `RuntimeChoice::for_pane`, so a
  pane's choice is made once, when its spawn is built.
- The pattern list is process state set at startup and on reload, beside `[experimental] runtime`.
  `excluded_from_core` is the only matcher (`*`, `?`, case-sensitive).
- A new pane matches only when it is started with a command that names its agent (`claude --name x`, read
  with the agent's carry list). A new pane that starts as a shell has no name yet, so it follows `runtime`.
  This is how the computer-use host panes are made today (`pane split`, then `claude --name` typed into the
  shell), so once `runtime = "core"` is on, a new host pane must be named and respawned (the docs say so).
- A resume with core on asks the pane-jobs probe when any suspended pane will resume on the PTY, an excluded
  one included.
- An explicit agent name is cleared by a plain respawn (`clear_agent_runtime_identity_after_respawn`), so a
  pane matched only by that name and respawned to a shell no longer matches. A launch name survives it.
- V32 is unit tests in `choice.rs`, `launch_flags.rs`, `terminal/metadata.rs` (a title is not a name) and
  `app/pane_resume.rs` (resume, respawn and new pane against a fake core). The restore call site is covered by
  `RuntimeChoice::restored` and V33: until PR F every restored program runs on the PTY anyway.
- V33 is section 5 of `scripts/core_live_test.sh`, macOS only (pane jobs): a compiled `claude` stand-in started
  as `claude --name test-hlp` in a PTY pane, then `runtime = "core"`: `pane respawn --resume`, a stopped pane
  job, and a herdr restart each leave it on the PTY while a new unnamed pane goes to core; with the pattern
  removed, the same respawn puts it on core.

## PR B. One connection per core session (placement unchanged)

- [x] B.1 Characterization first: today's reattach, orphan and handoff tests named as the protected behaviour
- [x] B.2 `CoreSessions` registry (D6): primary, and secondaries including `herdr` and `default`; never started;
  `HC_SOCKET` primary only; V42
- [x] B.3 Every call site of D5's table moved to label or session; `CorePaneIo` stores its session;
  `CoreTerminalLink` reads it; claims as `(session, address)`; V7
- [x] B.4 Orphan scan per session with the label/session notice; V6 against a two-session fake core
- [ ] B.5 Check: `just check` 0, `just core-live-test` 0 (core-backend V10 unchanged), V19 with sessions `herdr` and
  `herdr-<name>`, CI green (both checks 0 on m4m, 2026-11-21; CI waits for the push)

B as built:

- Protected behaviour (B.1): `core_restore_tests` in `src/persist/restore.rs` (V30, V45), the reattach tests in
  `core_backend/pane_io.rs`, the reattach and orphan half of `core_backend_live_tests`, and the handoff and restart
  sections of `scripts/core_live_test.sh`; two new pins, the orphan scan against a fake core and V7.
- The registry is `CoreSessions` in `core_backend/sessions.rs`, with the session names and connectors moved there
  from `core_backend/mod.rs`. Each secondary has its own slot lock, so a core that hangs on hello holds up only its
  own session. A secondary is released at the end of an orphan scan that found no claim and no orphan in it.
- `core_session_name()` still returns `herdr_session_label()`; PR C changes only the former.
- The known-session search and the orphan scan look in the same list: the primary, the sessions panes' links name,
  every connected secondary, then `herdr` and `default` for the release default herdr session.
- The orphan scan counts a waiting pane's link as a claim, so a core that answers again between the restore and the
  scan does not show a waiting pane's program as an orphan.
- Most of D5's table was already in place after A1 and A2: `CorePaneIo` stored its session, the link read it,
  `PaneIo::reattach_core` reached the link's session, and the sweep guard and the spaces publisher used the pane's
  own and the primary's connection. B adds the label, the per-session orphan scan and notice, and the registry.
- V19 runs with the scratch sessions `herdrdev-v10` and `herdrdev-v10-b`, not `herdr` and `herdr-<name>`: the live
  test is a debug build. With placement unchanged, the only way to get a pane into a second session is a link that
  names it, so the test kills the server (core programs live on), points pane G's saved link at a program labelled
  for G in the second session, and starts the server again.
- V6 runs in a debug unit test, where `herdr` and `default` are known only through panes' links, so the test claims
  a terminal in each; the release-only rule for `herdr` and `default` has its own unit test.

## R0. The PR B beta on both Macs (no code)

- [ ] R0.1 Beta with A1, A2, S, B, installed on m4m and ac-mbm5 in the same step by live handoff, panes kept;
  `core_pty_panes = ["*-hlp"]` added to both configs; V44 recorded on #295 with the beta number

## PR C. The default herdr session moves to core session `default` (#293 1b)

- [x] C.1 `herdr_session_label()` and the new `core_session_name()` (D5); V5
- [x] C.2 SSH-start notice on positive evidence (D7); unit over a fake process table with an `sshd-session`
  session leader and a ppid-1 desktop start
- [x] C.3 V17 live; V19 with `herdr`, `default` and a PTY pane; V21 rollback to the R0 beta and to beta 202
- [x] C.4 `docs/next`: the default session, the rollback rule (D10)
- [ ] C.5 Check: `just check` 0, `just core-live-test` 0, CI green; two-vendor security review of A1, A2 and C
  together (core restart after loss, the shared default core, forged labels: open question 3)

C as built:

- `core_session_name()` is `core_session_for` in `core_backend/sessions.rs`: the release default herdr session gives
  `default`, every other session its label's name. The primary connector, the orphan scan and the known-session
  search needed no change: B's registry already read `herdr` and `default` for that session.
- The SSH check is `core_backend/ssh_start.rs`: `started_over_ssh` over a `ProcessTable` (`getsid`, then the
  session leader and its parents, looking for `sshd` or `sshd-session`). It runs once per core pid on each new
  primary connection, a core herdr started included, and raises `AppEvent::CoreStartedOverSsh`, one warning and
  one notice. Limit, measured on m4m with hc 0.1.21: the core server is its own session leader with parent
  launchd, both after a start from a herdr pane with `SSH_CONNECTION` set and in each core running there, so for an
  hc-started core the test finds no SSH login and the notice does not fire. It fires only for a core that stays in
  its starter's session. The core's environment was not used instead: any core started by a process that
  inherited the SSH variables carries them (inferred: the variables are inherited, as the probe's core showed). The
  replacement D7 names, core's `installed_by` and launch facts in `server.status`, is the real fix.
- V5 is a unit test in `sessions.rs`; V17, V19 and V21 are `scripts/core_default_session_live_test.sh`, which needs
  a release build (only a release build uses `default`) and which `just core-live-test` runs after
  `cargo build --release`. It asserts its XDG directories are under its own temp folder before any hc call.
  V17 runs twice: a saved link pointed at a labelled program in scratch `herdr` (as B's V19), and a pane that
  beta 202 itself started in `herdr` before the handoff back.
- V21 runs only with `HERDR_R0_BIN` and `HERDR_202_BIN`. Run on 2026-11-21 with beta 204
  (`0.9.5-ac-beta.204-lichtsteiner`, the published beta asset, built at 785e60a0, PR B's head) as the R0 beta,
  and a local release build of 1a590805 (beta 202's commit, #293) as beta 202: not the published 202 binary, and
  without its beta version string.
- Until PR D, the spaces publisher writes to the primary, now `default`, with panes still in `herdr` as leaves
  whose addresses mean nothing there (D7): C and D ship in the same beta (R.1).

## PR D. The `herdr-app` leaf filter (after PR 59)

- [x] D.1 `spaces_document` leaf filter by the primary session; publisher on the primary only; V11
- [x] D.2 Check: `just check` 0, `just core-live-test` 0 (core-spaces-doc rows 10, 11, 13 unchanged), CI green (both 0 on m4m,
  2026-11-21; CI waits for the push)

D as built:

- The leaf filter is the `core_address` closure `spaces_document` already took: `primary_address` keeps a link's
  address only when its session is `core_session_name()`. `spaces_document` itself did not change. V11 is the unit
  test `only_primary_session_panes_are_leaves` in `core_spaces_doc.rs`, with a `herdr` link beside `default` links.
- The publisher already wrote through `existing_core_connector()`, the primary's connection, so D.1's "primary
  only" needed no change there.
- `core_session_name()` in `runtime_io/mod.rs` was test-only; it is now Unix-wide, with an empty-string twin for
  non-Unix (no pane has a link there). Rows 10, 11 and 13 are the live `core_spaces_doc` tests in the core live
  test, unchanged and passing.
- Limit: a primary pane's leaf is only checked against the link's session; the live test does not put a legacy
  pane beside a primary one in a published document.

## M. CPU re-measure (no code; after PR C, on its release build)

- [ ] M.1 D11 protocol on m4m: alternating PTY and core, eight runs a side, default and `V12_ONE_PROCESS=1`,
  `V12_TOP` and load recorded, the noise band computed; idle cost of a second core recorded; results in
  `v12-phase3.md`; V22
- [ ] M.2 Check: both gates met in both profiles, or PR E is opened before PR F and G roll out

## PR E. Batch-handing reader (only if M fails)

- [ ] E.1 The reader hands whole batches to each pane's parser; no pane's parse on the shared reader; proof that no
  caller waits on a core reply under a terminal lock (test or architecture check)
- [ ] E.2 Check: V22 again within the gates; V23 with 1 and 15 panes; `just check` 0, CI green

## PR F. Restored panes start on core (D8)

- [ ] F.1 Restored panes consult `runtime` and `RuntimeChoice`; fd-adopted panes `Pty(FdAdopted)`; V10 replaces
  `core_v1_a_restored_pane_stays_on_the_pty`; V18
- [ ] F.2 `docs/next` config reference: what `runtime = "core"` now covers
- [ ] F.3 Check: `just check` 0, `just core-live-test` 0, CI green

## PR G. Move idle panes onto the primary core (D9)

- [ ] G.1 `MoveEligibility` pure function with the full D9 table; input stamps on the queued Submit and
  `agent.message` note sends; manifest move facts; V8, V40 (unit part), V41
- [ ] G.2 `KillOutcome` from `CorePaneIo::kill`, the confirmed-kill move; `pane.respawn_if_idle` method, CLI
  `herdr pane respawn --if-idle [--check]`, API schema entry; V9, V12, V39
- [ ] G.3 `scripts/core_move_idle_panes.sh` with `--exclude` adding to the server's list, the agent-bell queue
  check, canary, per-pane verification, stop on first failure; maintenance script test for its argument parsing
- [ ] G.4 V20 and the move part of V33 in `scripts/core_live_test.sh`
- [ ] G.5 `docs/next`: what a move keeps and loses (a shell's process state)
- [ ] G.6 Check: `just check` 0, `just core-live-test` 0, CI green; security read of the new method

## R. Rollout on both Macs (no code)

- [ ] R.1 Beta with C, D, F, G (and E when merged), installed on m4m and ac-mbm5 in the same step by live handoff;
  panes kept; `runtime = "core"` and `core_pty_panes` confirmed in both configs
- [ ] R.2 V26 quit drill in a named throwaway session on m4m
- [ ] R.3 V40 live part (real Claude Code, no-connector settings) in a throwaway session
- [ ] R.4 V25 canary move on m4m by mac-m4, announced first, then the batch; then on ac-mbm5 by mac-m5, announced
  first (no fresh go from ac: he asked for step 3 on 2026-11-21, decide-from-acs-principles-v2)
- [ ] R.5 D12: per-host bridge removal relayed to the dotfiles owner; V24 phone check by ac; V27 legacy core stopped
  by mac-m4 on m4m and mac-m5 on ac-mbm5, each announced first
- [ ] R.6 V28 upstream evidence (any time after PR A1); a reproduced case drafted in bug-template form and sent as an
  `ASK:` to manager-lab's desk; ac submits it
- [ ] R.7 V46 when ac next updates hc
- [ ] R.8 SHAs, beta number and notes on #295, #287 and #293; close #287's herdr-max half and #293 1b after the
  checks; tick phase 3 in `openspec/changes/herdr-core-client/design.md` § 6.4
- [ ] R.9 Archive this change
