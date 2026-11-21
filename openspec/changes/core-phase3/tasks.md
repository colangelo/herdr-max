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

## PR S. `core_pty_panes`

- [ ] S.1 `[experimental] core_pty_panes` parse and reload; `excluded_from_core` over the stable name (D13)
- [ ] S.2 `RuntimeChoice` replacing `pty_only`; every caller of `spawn_preferred` passes it (new pane, restore,
  resume, respawn); V32 naming `manager-lab-hlp` and `manager-aruba-hlp`
- [ ] S.3 V33 live in `scripts/core_live_test.sh` (restore and resume parts; the move part lands with G)
- [ ] S.4 `docs/next` config reference for the key
- [ ] S.5 Check: `just check` 0, `just core-live-test` 0, CI green

## PR B. One connection per core session (placement unchanged)

- [ ] B.1 Characterization first: today's reattach, orphan and handoff tests named as the protected behaviour
- [ ] B.2 `CoreSessions` registry (D6): primary, and secondaries including `herdr` and `default`; never started;
  `HC_SOCKET` primary only; V42
- [ ] B.3 Every call site of D5's table moved to label or session; `CorePaneIo` stores its session;
  `CoreTerminalLink` reads it; claims as `(session, address)`; V7
- [ ] B.4 Orphan scan per session with the label/session notice; V6 against a two-session fake core
- [ ] B.5 Check: `just check` 0, `just core-live-test` 0 (core-backend V10 unchanged), V19 with sessions `herdr` and
  `herdr-<name>`, CI green

## R0. The PR B beta on both Macs (no code)

- [ ] R0.1 Beta with A1, A2, S, B, installed on m4m and ac-mbm5 in the same step by live handoff, panes kept;
  `core_pty_panes = ["*-hlp"]` added to both configs; V44 recorded on #295 with the beta number

## PR C. The default herdr session moves to core session `default` (#293 1b)

- [ ] C.1 `herdr_session_label()` and the new `core_session_name()` (D5); V5
- [ ] C.2 SSH-start notice on positive evidence (D7); unit over a fake process table with an `sshd-session`
  session leader and a ppid-1 desktop start
- [ ] C.3 V17 live; V19 with `herdr`, `default` and a PTY pane; V21 rollback to the R0 beta and to beta 202
- [ ] C.4 `docs/next`: the default session, the rollback rule (D10)
- [ ] C.5 Check: `just check` 0, `just core-live-test` 0, CI green; two-vendor security review of A1, A2 and C
  together (core restart after loss, the shared default core, forged labels: open question 3)

## PR D. The `herdr-app` leaf filter (after PR 59)

- [ ] D.1 `spaces_document` leaf filter by the primary session; publisher on the primary only; V11
- [ ] D.2 Check: `just check` 0, `just core-live-test` 0 (core-spaces-doc rows 10, 11, 13 unchanged), CI green

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
