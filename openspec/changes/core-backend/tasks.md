Planned 2026-11-14 (herdr, the lead, after phase 1 shipped in beta 194;
https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/280). Verifier rows V1-V14 are in
design.md § Verifier.

Heavy cargo on m4m only under the shared lock: `lockf -k /tmp/m4m-heavy.lock nice -n 15 env
CARGO_BUILD_JOBS=6 NEXTEST_TEST_THREADS=6 ZIG=/opt/homebrew/opt/zig@0.16/bin/zig <cmd>`. Live core
tests use a scratch `HC_SESSION` and scratch XDG dirs, never the user's default core, and a test
Claude Code only with `--settings ~/.config/claude/direct-noconnectors.settings.json`.

## 1. Before code

- [x] 1.1 Gitea issue 280; worktree `issue/280-core-backend` from master `55815bee`
- [ ] 1.2 Verifier review by herdr (not the implementer); record changes or sign-off in design.md
- [ ] 1.3 Live checks that shape the design: core's first paint starts with a reset or not (D5);
  `env` in `terminal.run` replaces the core's environment fully (D4); a Claude Code hook in a core
  pane reaches our socket (V11 assumption)

## 2. Roundtable (refactor-risk)

- [ ] 2.1 `roundtable.md`: protected behaviour (PTY panes unchanged; detection precedence; handoff
  of PTY panes; client protocol), the exact entry point for status reports (D9), the handoff code
  path for an `Unsupported` pane (D10), findings

## 3. Client and fake core (tests first)

- [ ] 3.1 Framing codec (`src/pane/runtime_io/core/frame.rs`): encode/decode, 16 MiB cap, partial
  reads; unit tests
- [ ] 3.2 Fake core for tests: hello, `terminal.run`, `terminal.attach`, DATA/CREDIT/CLOSE/RESIZE/
  FACTS, `events.subscribe`, scripted exits
- [ ] 3.3 `CoreClient`: connect (socket rules, D2), start the core when absent (clean env), hello,
  version gate, RPC table, reader thread, event routing; V9 against the fake core
- [ ] 3.4 Architecture test V13 (no `terminal.send`)

## 4. Backend

- [ ] 4.1 `PaneIo::Core`: spawn (D4), parse thread with credit after `on_read` (D3), input and
  submission, replies as AUTOMATIC_INPUT, resize and FACTS (D6); V2, V4, V5
- [ ] 4.2 Exit mapping (D7) and exactly-once; V6
- [ ] 4.3 Reattach with reset before repaint (D5); V3
- [ ] 4.4 `terminate_session`: kill, then the local sweep (D8); V7
- [ ] 4.5 Status events as authoritative reports (D9); V8
- [ ] 4.6 Handoff with core panes (D10); V10
- [ ] 4.7 Config `[experimental] runtime`, fallback notice (D1), Windows warning; V1, V9;
  `docs/next` config reference
- [ ] 4.8 `just core-live-test` recipe; V11
- [ ] 4.9 `just check` green; V12 profile, delta recorded

## 5. Ship

- [ ] 5.1 CI green on Linux, macOS, Windows (draft PR for CI)
- [ ] 5.2 herdr merges, cuts the beta, installs; `runtime = "core"` turned on for m4m only (V14)
- [ ] 5.3 A day of use; SHAs, beta and notes on issue 280; close it
- [ ] 5.4 Archive; tick 4.2 in `openspec/changes/herdr-core-client/tasks.md`
