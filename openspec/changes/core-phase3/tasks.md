Planned 2026-11-21 (herdr-core, for herdr; https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/295).
Verifier rows V1-V28 are in design.md § Verifier. Each PR below is shippable on its own and ends in a check.

Heavy cargo on m4m only under the shared lock: `lockf -k /tmp/m4m-heavy.lock nice -n 15 env
CARGO_BUILD_JOBS=6 NEXTEST_TEST_THREADS=6 ZIG=/opt/homebrew/opt/zig@0.16/bin/zig <cmd>`. Live core tests use a
scratch `HC_SESSION` and scratch XDG directories, never ac's `herdr` or `default` core; any test of session
`default` asserts its XDG directories are under its own temp folder first. A test Claude Code runs only with
`--settings ~/.config/claude/direct-noconnectors.settings.json`; agent resumes are tested with a compiled stub.

Order: PR A first (the #287 gate before any default). PR B, then C, then D. Measurement M after C. PR F and G after
A. Rollout R last. PR E only if M fails.

## 0. Before code

- [ ] 0.1 Roundtable on design.md § Roundtable questions (refactor-risk: restore and handoff, persisted state,
  identity); findings in `roundtable.md`, folded into the decisions
- [ ] 0.2 Verifier review by herdr (the lead); changes folded into design.md
- [ ] 0.3 Gitea sub-issues for PR A, C and G (`[fork]` titles), each linking #295

## PR A. A core loss suspends panes and the core comes back (#287, herdr-max side)

- [ ] A.1 `exit_reason` per D1; V1
- [ ] A.2 D1a: the dead-program check (pid and start time recorded at attach) and the "Herdr Core unreachable"
  in-place state; V3
- [ ] A.3 Backend-aware resume readiness (D2), one notice per burst; V2 against the fake core
- [ ] A.4 `CoreLossGuard` and the 3 s start rule for the primary session (D3), replacing `EVER_CONNECTED`; V4
- [ ] A.5 V13 invariants; V14, V15, V16 in `just core-live-test`
- [ ] A.6 `docs/next`: core loss behaviour and the "Running in Background" rule; the same line relayed to the owner
  of CONTEXT `SKILLS/herdr` (this repo's `skills/herdr/SKILL.md` changes only at a stable release)
- [ ] A.7 Check: `just check` 0, `just core-live-test` 0, CI green on Linux, macOS and Windows

## PR B. One connection per core session (behaviour unchanged)

- [ ] B.1 Characterization first: today's reattach, orphan and handoff tests named as the protected behaviour
- [ ] B.2 `CoreSessions` registry (D6): primary and secondary, secondary never started; `HC_SOCKET` primary only
- [ ] B.3 `CorePaneIo` stores its session; `CoreTerminalLink` reads it; claims as `(session, address)`; V7
- [ ] B.4 Orphan scan per session; V6 against a two-session fake core
- [ ] B.5 Check: `just check` 0, `just core-live-test` 0 (core-backend V10 unchanged), V19 with sessions `herdr` and
  `herdr-<name>`, CI green

## PR C. The default herdr session moves to core session `default` (#293 1b)

- [ ] C.1 `herdr_session_label()` and the new `core_session_name()` (D5); V5
- [ ] C.2 Legacy `herdr` session as a secondary when the primary is `default` (D6)
- [ ] C.3 SSH-started default core warning (D7); unit over a fake `server.status` and parent pid
- [ ] C.4 V17 live; V19 with `herdr`, `default` and a PTY pane; V21 rollback characterization
- [ ] C.5 `docs/next`: the default session, the two-session rollback rule (D10)
- [ ] C.6 Check: `just check` 0, `just core-live-test` 0, CI green; two-vendor security review of PR A and C
  together (core restart after loss, the shared default core)

## PR D. The `herdr-app` document in a shared core (after PR 59 merges)

- [ ] D.1 Own-space marker (a kept field and an id prefix), merge with other spaces, off removes own only (D7); V11
- [ ] D.2 Live row: a scratch core whose document holds a foreign space keeps it through publish, conflict and off
- [ ] D.3 Check: `just check` 0, `just core-live-test` 0, CI green

## M. CPU re-measure (no code; after PR C, on its release build)

- [ ] M.1 D11 protocol on m4m: alternating PTY and core, four runs a side, default and `V12_ONE_PROCESS=1`, `V12_TOP`
  and load recorded; idle cost of a second core recorded; results in `v12-phase3.md`; V22
- [ ] M.2 Check: medians within 1.3x in both profiles, or PR E is opened before PR F and G roll out

## PR E. Batch-handing reader (only if M fails)

- [ ] E.1 The reader hands whole batches to each pane's parser; no pane's parse on the shared reader; proof that no
  caller waits on a core reply under a terminal lock (test or architecture check)
- [ ] E.2 Check: V22 again within 1.3x; V23 with 1 and 15 panes; `just check` 0, CI green

## PR F. Restored panes start on core (D8)

- [ ] F.1 Restored panes consult `runtime`; fd-adopted panes stay PTY; V10 replaces
  `core_v1_a_restored_pane_stays_on_the_pty`; V18
- [ ] F.2 `docs/next` config reference: what `runtime = "core"` now covers
- [ ] F.3 Check: `just check` 0, `just core-live-test` 0, CI green

## PR G. Move idle panes onto the primary core (D9)

- [ ] G.1 `MoveEligibility` pure function; V8
- [ ] G.2 `pane.respawn_if_idle` method, CLI `herdr pane respawn --if-idle [--check]`, API schema entry; V9, V12
- [ ] G.3 `scripts/core_move_idle_panes.sh` with the exclusion list, canary, per-pane verification, stop on first
  failure; maintenance script test for its argument parsing and exclusions
- [ ] G.4 V20 in `scripts/core_live_test.sh`
- [ ] G.5 Check: `just check` 0, `just core-live-test` 0, CI green; security read of the new method

## R. Rollout on both Macs (no code)

- [ ] R.1 Beta with A, B, C, F, G (and D, E when merged), installed on m4m and ac-mbm5 in the same step by live
  handoff; panes kept; `runtime = "core"` confirmed in both configs
- [ ] R.2 V26 quit drill in a named throwaway session on m4m
- [ ] R.3 V25 canary move on m4m with ac's go on the desk, then the batch; then ac-mbm5
- [ ] R.4 D12: bridge removal relayed to the dotfiles owner; V24 phone check by ac; V27 legacy core stopped by ac
- [ ] R.5 V28 upstream evidence (any time after PR A); a reproduced case drafted for ac in bug-template form
- [ ] R.6 SHAs, beta number and notes on #295, #287 and #293; close #287's herdr-max half and #293 1b after the
  checks; tick phase 3 in `openspec/changes/herdr-core-client/design.md` § 6.4
- [ ] R.7 Archive this change
