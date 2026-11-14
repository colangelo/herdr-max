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

- [ ] 2.1 `roundtable.md`: each protected behaviour, where it lives at the base commit, which test
  holds it (existing or V1-V8), and what the review found; adjust design.md if it finds a gap

## 3. Characterization tests, green on the unmodified code

- [x] 3.1 V1 spawn contract (argv, cwd, env), `#[cfg(unix)]` module in `src/pane.rs` tests
- [x] 3.2 V2 exit report (`exit 3` → `Exited`; signal → `Interrupted`; one event; flag first)
- [x] 3.3 V3 and V4 termination escalation and preserve-on-drop (trap script logs signals)
- [x] 3.4 V5 output order (20 000 numbered lines; scrollback limit set so all fit, roundtable F5)
- [x] 3.5 V6 resize reaches the program (`stty size`) and a `CSI 6n` reply keeps its order
- [x] 3.6 V8 process facts through the `pane.process_info` handler; the first output already sees the
  shell pid (roundtable F1)
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

- [ ] 4.1 `src/pane/io/mod.rs`: move `PaneRuntimeIo` as `PaneIo` with its methods, unchanged;
  `TestChannel` moves with it. Commit
- [ ] 4.2 `src/pane/io/pty.rs`: `PtyPaneIo` wraps `PtyIoActorHandle`; `PaneIo::spawn(SpawnSpec,
  PaneIoSink)` calls `spawn_with_portable_pty` and `PtyIoActor::spawn`; the child wait moves in
  and calls `on_exit` once after setting `child_wait_completed`. Commit
- [ ] 4.3 `shutdown_pane_processes` and helpers move to `pty.rs`; `PaneIo::terminate_session`;
  `Drop` keeps its order. Commit
- [ ] 4.4 `PaneIo::adopt_handoff_fd`; `from_handoff_fd` calls it. Commit
- [ ] 4.5 Handoff methods answer `ErrorKind::Unsupported` for a variant without a descriptor (a
  unit test with `TestChannel` configured that way). Commit
- [ ] 4.6 `src/pane.rs` no longer imports `crate::pty` or `portable_pty` (except the command
  builders, D5); V9 allow-list shrinks to its final form. Commit
- [ ] 4.7 `just check` green (V11); `just bench-render-scale` against 1.2 (V10), delta recorded

## 5. Ship

- [ ] 5.1 Push the branch; CI green on Linux, macOS and Windows (V11)
- [ ] 5.2 `just beta-local`, installed by live handoff on both Macs at once (V12); report the beta
  by build number
- [ ] 5.3 A day of use with no regression; comment SHAs, beta and notes on issue 277; close it
- [ ] 5.4 Archive this change; tick 4.1 in `openspec/changes/herdr-core-client/tasks.md`
