Planned 2026-11-14 (ac: "start planning for the migration / split to herdr core and our herdr
max"; https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/277). Phase 0 gate: go
(`openspec/changes/herdr-core-client/design.md` § 5a, `1652ffa7`). Verifier rows V1-V15 are in
design.md § Verifier.

Heavy cargo on m4m only under the shared lock: `lockf -k /tmp/m4m-heavy.lock nice -n 15 env
CARGO_BUILD_JOBS=6 NEXTEST_TEST_THREADS=6 ZIG=/opt/homebrew/opt/zig@0.16/bin/zig <cmd>`.

## 1. Before any code moves

- [x] 1.1 Task worktree and branch `issue/277-runtime-seam` (wt), from current `master`
- [x] 1.2 Baseline: `just bench-render-scale` (1 and 15 panes), numbers saved in this change (V10)
- [x] 1.3 Verifier review: herdr approved with additions (V13-V15; V1, V2, V3, V6, Risks)

## 2. Roundtable (refactor-risk)

- [x] 2.1 `roundtable.md`: each protected behaviour, where it lives at the base commit, which test
  holds it (existing or V1-V8), and what the review found; adjust design.md if it finds a gap

## 3. Characterization tests, green on the unmodified code

- [x] 3.1 V1 spawn contract (argv, cwd, env), `#[cfg(unix)]` module in `src/pane.rs` tests
- [x] 3.2 V2 exit report (`exit 3` → `Exited`; signal → `Interrupted`; one event; flag first)
- [x] 3.3 V3 and V4 termination escalation and preserve-on-drop (trap script logs signals)
- [x] 3.4 V5 output order (20 000 numbered lines; scrollback limit set so all fit, roundtable F5)
- [x] 3.5 V6 resize reaches the program (`stty size`) and a `CSI 6n` reply keeps its order
- [x] 3.6 V8 process facts through the `pane.process_info` handler. The F1 claim (first output
  already sees the shell pid) has no test that can fail: see design.md D2
- [x] 3.6a V13 is the existing anchor test (named in design.md); V14 and V15 written
- [x] 3.6b F2 is named by https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/279, not a test:
  forcing `PtyIoActor::spawn` to fail needs a failure hook in production code, which the fix will
  add with its own test. F4 count: `PaneRuntime::test_with_actor` (`src/pane.rs:4643`) via
  `TerminalRuntime::test_with_actor` (3 callers in `src/platform/macos/input_tests.rs`), and 4
  `PaneRuntimeIo::TestChannel` literals in `src/pane.rs` tests (6292, 6352, 6406, 6451); 29
  dispatch arms in production code
- [x] 3.7 V9 architecture test, with today's offenders listed as the allow-list to shrink
- [x] 3.8 Commit: `test(pane): characterize the pty runtime before the backend seam`

## 4. The seam (no behaviour change)

- [x] 4.1 `src/pane/runtime_io/mod.rs`: move `PaneRuntimeIo` as `PaneIo` with its methods, unchanged;
  `TestChannel` moves with it. Commit
- [x] 4.2 `src/pane/runtime_io/pty.rs`: `PaneIo::Actor` keeps wrapping `PtyIoActorHandle`;
  `PaneIo::spawn(pane_id, SpawnSpec, PaneIoSink, &child_pid, child_wait_completed)` calls
  `spawn_with_portable_pty` and `PtyIoActor::spawn`; the child wait moves in
  and calls `on_exit` once after setting `child_wait_completed`. Commit
- [x] 4.3 `shutdown_pane_processes` and helpers move to `pty.rs`; `PaneIo::terminate_session`;
  `Drop` keeps its order. Commit
- [x] 4.4 `PaneIo::adopt_handoff_fd`; `from_handoff_fd` calls it. Commit
- [x] 4.5 Handoff methods answer `ErrorKind::Unsupported` for a variant without a descriptor (a
  unit test with `TestChannel` configured that way). Commit
- [x] 4.6 `src/pane.rs` no longer imports `crate::pty` or `portable_pty` (except the command
  builders, D5); V9 allow-list shrinks to its final form. Commit. Done: the
  PTY mentions left in `src/pane.rs` and `src/terminal/runtime.rs` are the command builder and
  test-only constructors
- [x] 4.7 `just check` green at `6a09802a` (5457 tests); `just bench-render-scale` against 1.2: 95
  timing rows, median after/before 0.977, 10th-90th percentile 0.94-1.01, worst row +8% at 311 us
  (noise); `bench-after.txt`. The read callback is the same boxed closure as before, so no new
  per-chunk work (herdr's D3 note)

## 5. Ship

- [x] 5.1 Push the branch; CI green on Linux, macOS and Windows (V11): draft
  https://github.com/colangelo/herdr-max/pull/45 at `d7a8212f`, all five jobs pass. The first
  `Windows ConPTY package` run died starting `powershell.exe` (0xC0000142, before any herdr code);
  the rerun passed, and the job passes on master
- [ ] 5.2 `just beta-local`, installed by live handoff on both Macs at once (V12); report the beta
  by build number
- [ ] 5.3 A day of use with no regression; comment SHAs, beta and notes on issue 277; close it
- [ ] 5.4 Archive this change; tick 4.1 in `openspec/changes/herdr-core-client/tasks.md`

## 6. Review follow-ups (herdr, 2026-11-14, review of `d7a8212f`, merged as `19c36b2a`)

- [x] 6.1 V2 flag order proved inside `on_exit` (`v2_on_exit_runs_after_the_wait_is_marked_complete`);
  bites: swapping the store and the callback fails it
- [x] 6.2 F1: a seam-level test was written and dropped, because it passed with the order reversed
  (5 of 5); recorded in design.md D2 instead
- [x] 6.3 V9: test helpers moved into `src/pane/runtime_io/`; `src/pane.rs` off the `crate::pty`
  list; the scan catches `use crate::pty;` and `pty::actor::`; bites: a `use crate::pty;` probe in
  `src/sound.rs` fails it
- [x] 6.4 design.md D1, D2, D4 and task 4.2 match the code; task 2.1 ticked
- [x] 6.5 D4: the fd-less answers to quiesce, pause and release are a phase 3 decision (design.md D4)
- [x] 6.6 V2 table row says `kill -KILL`, as the test does
