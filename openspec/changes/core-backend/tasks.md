Planned 2026-11-14 (herdr, the lead, after phase 1 shipped in beta 194;
https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/280). Verifier rows V1-V14 are in
design.md § Verifier.

Heavy cargo on m4m only under the shared lock: `lockf -k /tmp/m4m-heavy.lock nice -n 15 env
CARGO_BUILD_JOBS=6 NEXTEST_TEST_THREADS=6 ZIG=/opt/homebrew/opt/zig@0.16/bin/zig <cmd>`. Live core
tests use a scratch `HC_SESSION` and scratch XDG dirs, never the user's default core, and a test
Claude Code only with `--settings ~/.config/claude/direct-noconnectors.settings.json`.

## 1. Before code

- [x] 1.1 Gitea issue 280; worktree `issue/280-core-backend` from master `55815bee`
- [x] 1.2 Verifier review by herdr: approved with changes (2026-11-14), folded into design.md
- [x] 1.3 Live checks that shape the design: the first paint starts with CAN + soft reset, so no
  extra reset (D5); `env` merges into the core's environment and cannot unset, so programs start
  under `/usr/bin/env -u ...` (D4); the hook-to-our-socket check moves to V11 (it needs a throwaway
  herdr, not ac's)

## 2. Roundtable (refactor-risk)

- [x] 2.1 `roundtable.md` (draft; R1-R6 folded into D7, D9, D10): protected behaviour (PTY panes unchanged; detection precedence; handoff
  of PTY panes; client protocol; the new `core_terminal` field in persisted state), the exact entry
  point for status reports (D9), the handoff code path for a core pane (D10), findings

## 3. Client and fake core (tests first)

- [x] 3.1 Framing codec (`src/pane/runtime_io/core_backend/frame.rs`): encode/decode, 16 MiB cap, partial
  reads; unit tests
- [x] 3.2 Fake core for tests: hello, `terminal.run`, `terminal.attach`, DATA/CREDIT/CLOSE/RESIZE/
  FACTS, `events.subscribe`, scripted exits
- [x] 3.3 `CoreClient`: connect (socket rules, D2), start the core when absent (clean env), hello,
  version gate, RPC table, reader thread, event routing; V9 against the fake core. Checked live
  against the pilot core (`live_client_runs_attaches_and_reads_a_real_core`, ignored unless
  `HERDR_TEST_HC_SOCKET`): it found that the API takes `labels` as a map, which the fake now enforces
- [x] 3.4 Architecture test V13 (no `terminal.send`): `tests/architecture_core_send.rs`; bites

## 4. Backend

- [x] 4.1 One shared strip list for the PTY builder and the core `env -u` wrapper; `PaneIo::Core`: spawn (D4), parse thread with credit after `on_read` (D3), input and
  submission, replies as AUTOMATIC_INPUT, resize and FACTS (D6); V2, V4, V5 live
  (`src/pane/core_backend_live_tests.rs`, `HERDR_TEST_HC=1`). The first attach asks for the whole
  history: without it, output printed before the attach was lost (V3 live caught it)
- [x] 4.2 Exit mapping (D7) and exactly-once; V6 (live `core_v6_exit_is_reported_once`). A program that
  ends before the attach (core answers `finished`) still opens: its last screen and exit are queued
- [x] 4.3 Reattach (D5; no extra reset, core's first paint resets itself); V3 live `core_v3_reattaches_after_a_core_restart`; a core is started only before the first connection (a reconnect once started a second core)
- [x] 4.4 `terminate_session`: list the session first, kill, then sweep the list (D8; listing after
  the kill found nothing, the session leader was gone); V7 live
- [x] 4.5 Status events as `HookStateReported` + `AgentHintReported`, `seq = max(prev + 1, now_ns)`
  (D9, R2, R4); test: after a handoff or reattach the next report is accepted; V8
- [x] 4.6 Re-adoption (D10): core panes left out of the handoff fd list before any dup (R1);
  `core_terminal` in the pane snapshot, detach on handoff and `stop_for_restart` (R3),
  label-checked reattach on restore with no launch line (R6), kill on stop, orphan notice; V10
  (pane half): fake-core tests `a_reattach_*`, `a_kept_core_pane_*`, the snapshot field test, and
  live `core_v10_a_kept_pane_is_reattached_by_the_next_server` (same pid, earlier screen, nothing
  typed, input works, orphan listed, close ends it). Also: a handoff import keeps the core
  programs it attached to until `assume_handoff_ownership`, so a failed import cannot end them
- [x] 4.6b V10 (server half) through a throwaway herdr with `runtime = "core"`: a mixed live
  handoff completes, then `herdr server restart`, then `herdr server stop`
  (`scripts/core_live_test.sh`, in `just core-live-test`). It found that a pane restored fresh went
  to core, against D1: restored panes now always start on the PTY (live
  `core_v1_a_restored_pane_stays_on_the_pty`); the restart warning now says core panes keep their
  programs
- [x] 4.6a Core lifetime and the sweep guard; V15, V16. V15 in `scripts/core_live_test.sh`: the core
  our server started keeps one pid through live handoff, restart and stop, is not our child or in
  our session, and hands its programs none of `HERDR_PANE_ID`, `HERDR_TAB_ID`,
  `HERDR_WORKSPACE_ID`, `CLAUDE_CODE_*`, `ANTHROPIC_*`, `CLAUDECODE` (control: a core started from
  an agent's shell hands on 11 of them). D2's warning for a core in our own session is in
  `connect_session`. V16: `the_sweep_guard_refuses_a_session_shared_with_core`
- [x] 4.7 Config `[experimental] runtime`, fallback notice (D1), Windows warning; V1, V9;
  `docs/next` config reference with the rollback rule. The notice is `AppEvent::CoreUnavailable`,
  sent the first time a spawn falls back, shown as a toast and kept in the notification log; V9:
  `an_unusable_core_falls_back_to_the_pty_with_one_notice` (too old, then refused) and
  `an_unavailable_core_raises_a_notice_naming_the_reason`. It found a leak: a client refused for its
  version kept its reader thread and connection, so each later spawn waited 5 s on a hello; the
  last client handle now closes the connection (`dropping_the_last_client_closes_the_connection`)
- [x] 4.8 `just core-live-test` recipe: the live tests and `scripts/core_live_test.sh`; V11 there
  (`herdr pane report-agent` run inside the core pane lands on that pane)
- [x] 4.9 `just check` green; V12 profile, delta recorded in `v12-results.md`: render bench
  unchanged (median 0.93); the busy profile is about 1.9x the PTY, most of it core's own
  per-terminal screen (Shape B parses twice). herdr restated V12 (our server within 1.3x, core cost
  recorded) and ships phase 2 opt-in; our server's +10-30% is issue 282, gated before phase 3

## 5. Ship

- [ ] 5.1 Two-vendor security review of D2-D4 (ask the security session); fix or answer findings
- [x] 5.1a CI green on Linux, macOS, Windows (draft PR https://github.com/colangelo/herdr-max/pull/46,
  head 96113389). It found a unix-only `iter()`, status types dead on Windows, and the D2 warning
  comparing sessions (a launchd-started herdr shares launchd's); now a parent check
- [ ] 5.2 herdr merges, cuts the beta, installs; `runtime = "core"` only in the throwaway session
  `herdr-core-trial` on m4m for a day of herdr-core's agent work (V14)
- [ ] 5.2a Then an ask on the desk: does ac want `runtime = "core"` in his own session
- [ ] 5.3 A day of use; SHAs, beta and notes on issue 280; close it
- [ ] 5.4 Archive; tick 4.2 in `openspec/changes/herdr-core-client/tasks.md`
