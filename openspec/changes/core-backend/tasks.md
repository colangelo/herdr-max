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
- [ ] 4.5 Status events as `HookStateReported` + `AgentHintReported`, `seq = max(prev + 1, now_ns)`
  (D9, R2, R4); test: after a handoff or reattach the next report is accepted; V8
- [ ] 4.6 Re-adoption (D10): core panes left out of the handoff fd list before any dup (R1);
  `core_terminal` in the pane snapshot, detach on handoff and `stop_for_restart` (R3),
  label-checked reattach on restore with no launch line (R6), kill on stop, orphan notice; V10
- [ ] 4.6a Core lifetime and the sweep guard; V15, V16
- [ ] 4.7 Config `[experimental] runtime`, fallback notice (D1), Windows warning; V1, V9;
  `docs/next` config reference with the rollback rule
- [ ] 4.8 `just core-live-test` recipe; V11
- [ ] 4.9 `just check` green; V12 profile, delta recorded

## 5. Ship

- [ ] 5.1 Two-vendor security review of D2-D4 (ask the security session); fix or answer findings
- [ ] 5.1a CI green on Linux, macOS, Windows (draft PR for CI)
- [ ] 5.2 herdr merges, cuts the beta, installs; `runtime = "core"` only in the throwaway session
  `herdr-core-trial` on m4m for a day of herdr-core's agent work (V14)
- [ ] 5.2a Then an ask on the desk: does ac want `runtime = "core"` in his own session
- [ ] 5.3 A day of use; SHAs, beta and notes on issue 280; close it
- [ ] 5.4 Archive; tick 4.2 in `openspec/changes/herdr-core-client/tasks.md`
