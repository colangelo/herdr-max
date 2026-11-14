# Runtime seam: design

Phase 1 of the herdr-core migration (`openspec/changes/herdr-core-client/design.md` § 6.4).
Issue: https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/277

## Context: the boundary today

Read on 2026-11-14 at `1652ffa7`. Line numbers are for that commit.

- **One spawn function.** `spawn_with_portable_pty` (`src/pty/backend/unix.rs:12`; Windows
  `src/pty/backend.rs:17`) opens the PTY, tries the macOS pane anchor
  (`platform::pane_anchor_spawn`), and returns `SpawnedPty { master_fd | master, child }`.
- **One caller.** `PaneRuntime::spawn_command_builder` (`src/pane.rs:3226`). Every spawn reaches
  it: new panes and splits (`src/workspace/tab.rs`), API spawns (`src/app/api.rs`,
  `src/app/api/panes.rs`), agent start and respawn (`src/app/agent_resume.rs`), popups, custom
  commands, session restore (`src/persist/restore.rs:885,904`), snapshots and notifications. The
  wrappers are in `src/terminal/runtime.rs`.
- **Handoff adoption is not a spawn.** `PaneRuntime::from_handoff_fd` (`src/pane.rs:3014`) builds
  the actor from an imported fd (`src/persist/restore.rs:872`, `src/terminal/runtime.rs:84`). It
  has no child handle, so it reports exit as `ChildExitReason::Handoff`.
- **What `PaneRuntime` does itself today**, outside `PaneRuntimeIo`:
  - stores the shell pid from `child.process_id()` in `child_pid` (`src/pane.rs:3286`);
  - runs `child.wait()` on `spawn_blocking`, classifies it (`platform::classify_child_exit`),
    sets `child_wait_completed`, and sends `AppEvent::PaneDied` (`src/pane.rs:3290-3311`);
  - builds the `on_read` closure that feeds libghostty-vt (`src/pane.rs:3327`), then hands it to
    `PtyIoActor::spawn`;
  - on drop, unless `preserve_processes_on_drop`, signals the whole process session HUP → TERM →
    KILL with 250 ms between each (`shutdown_pane_processes`, `src/pane.rs:2260`).
- **What already goes through the enum** `PaneRuntimeIo` (`src/pane.rs:2024`): user input, queued
  submissions, terminal replies, resize, the post-handoff redraw nudge, shutdown, the foreground
  process group (`tcgetpgrp`), and the handoff steps (quiesce, dup the fd, roll back, release). Its
  variants are `Actor(PtyIoActorHandle)` and, under `cfg(test)`, `TestChannel`.
- **Process facts that are pid-based**, not fd-based: foreground job, argv, cwd, `shell_at_prompt`,
  nested job (`src/detect/mod.rs:496`, `platform::process_cwd`, `src/app/api/panes.rs:528-576`).
  They work for any backend whose programs run on this host, which a local core does.

## Goals and non-goals

Goals:

- Every way a pane gets a running program goes through one seam, and `PaneRuntime` never names
  `portable_pty` or `crate::pty`.
- Today's PTY code is the first implementation behind it, with no change in behaviour on any
  platform.
- Phase 2 can add a core backend by adding one variant and one module, without editing the
  callers in `src/workspace`, `src/app`, `src/persist` or `src/server`.

Non-goals:

- No core backend, no config switch, no new dependency (phase 2).
- No opaque handoff token and no re-adoption of terminals by address (phase 3).
- No change to pid-based process facts (they stay valid for a local core; remote is phase 5).
- No change to the wire, the API, `session.json` or the handoff manifest.

## Decisions

### D1. Widen the existing enum; no trait object

`PaneRuntimeIo` becomes `PaneIo` in `src/pane/io/mod.rs`, still an enum:

```rust
pub(crate) enum PaneIo {
    Pty(pty::PtyPaneIo),          // today's actor, Unix and Windows
    #[cfg(test)]
    TestChannel(test_channel::TestChannelIo),
    // phase 2: Core(core::CorePaneIo)
}
```

Why an enum: the set of backends is closed and in this repo; the existing code already dispatches
this way, with `TestChannel` as a second variant; no `dyn` and no boxing; the compiler lists every
method a new variant must answer. A trait object would add an indirection with no second
implementer today.

Alternative kept open: if phase 2 shows the core variant needs its own async machinery that does
not fit a match arm, introduce a trait then, with two real implementers to shape it.

### D2. The seam owns spawn, exit, termination and the shell pid

```rust
pub(crate) struct SpawnSpec {
    pub rows: u16,
    pub cols: u16,
    pub command: portable_pty::CommandBuilder,   // see D5
}

pub(crate) struct PaneIoSink {
    pub on_read: Box<dyn FnMut(&[u8]) -> PtyReadResult + Send>,   // unchanged closure
    pub on_exit: Box<dyn FnOnce(ChildExitReason) + Send>,         // sends PaneDied
}

impl PaneIo {
    pub(crate) fn spawn(pane_id: PaneId, spec: SpawnSpec, sink: PaneIoSink) -> io::Result<Self>;
    #[cfg(unix)]
    pub(crate) fn adopt_handoff_fd(pane_id: PaneId, fd: OwnedFd, child_pid: u32,
                                   sink: PaneIoSink) -> io::Result<Self>;

    pub(crate) fn shell_pid(&self) -> u32;                       // 0 when unknown
    pub(crate) fn child_wait_completed(&self) -> bool;
    pub(crate) fn terminate_session(&self);                      // today's shutdown_pane_processes
    // ...plus every method PaneRuntimeIo already has, unchanged
}
```

- The child wait moves into `PtyPaneIo::spawn`; it calls `sink.on_exit` exactly once, after it has
  set `child_wait_completed`, as today. `PaneRuntime` turns that into `AppEvent::PaneDied`.
- `shutdown_pane_processes` and its helpers move to `src/pane/io/pty.rs` unchanged. `PaneRuntime`'s
  `Drop` calls `io.shutdown()`, then `io.terminate_session()` unless it preserves processes.
- `child_pid: Arc<AtomicU32>` stays where the parser and detection read it; the backend publishes
  into it (the PTY backend stores `child.process_id()` at spawn, the handoff path stores the
  manifest pid). The read path keeps its single atomic load.

### D3. The read path does not change

`on_read` is built by `PaneRuntime` exactly as now and handed to the backend inside `PaneIoSink`.
The PTY backend passes it to `PtyIoActor::spawn` as it does today. No lock, allocation, channel or
dynamic dispatch is added per chunk. `just bench-render-scale` runs before and after.

### D4. Handoff stays fd-based and Unix-only on the PTY variant

The handoff methods (`begin_handoff`, `duplicate_handoff_fd`, `set_handoff_paused`,
`release_after_commit`, `nudge_child_redraw_after_handoff`, `foreground_process_group_id`) stay on
`PaneIo`, `#[cfg(unix)]` as now. The PTY variant forwards them; `TestChannel` keeps today's answers.
A future variant without a master fd must return `io::ErrorKind::Unsupported`, and the server's
handoff code must treat that pane as "not handed off by fd". That last part is phase 3's job; phase
1 only fixes the error kind so phase 3 has one thing to match.

### D5. `SpawnSpec` carries the `CommandBuilder` for now

The command is built by platform helpers that return `portable_pty::CommandBuilder`
(`pane_shell_command_builder`, `platform::pane_custom_command_pty_builder`, env helpers at
`src/pane.rs:105,210`). Phase 1 carries it inside `SpawnSpec` so no helper changes. The type is
named only inside `src/pane/io/` and the builders; the architecture test (V9) allows exactly those
files.

Phase 2 needs argv, cwd and env as plain data for `terminal.run`. It will add `SpawnSpec` accessors
that read them out of the builder (the core backend's only use), or replace the field with a neutral
struct; that choice needs the builder's public API, which phase 1 does not depend on.

### D6. Platform gating

`PaneIo` itself compiles on every target. `src/pane/io/pty.rs` keeps the Unix/Windows split it
inherits from `src/pty/actor.rs` and `src/pty/backend.rs`. Characterization tests that spawn real
programs are `#[cfg(unix)]` at module level, not per function (two red Windows clippy runs on
2026-10-10 came from Unix-only helpers left ungated). The Windows variant keeps the `deadline`
argument of `queue_user_input_submission`.

## Risks

- **Exit ordering.** Today `child_wait_completed` is set before `PaneDied` is sent, and
  `shutdown_pane_processes` reads it to skip a reaped pid (`process_alive_for_shutdown`). Moving the
  wait must keep that order. Covered by V2 and the existing `shutdown_liveness_*` tests.
- **Drop order.** Today `Drop` aborts detection, aborts compression, shuts the actor down, then
  terminates the session. Keep it. Covered by V3.
- **Handoff.** `tests/live_handoff.rs` is the guard; it must pass unchanged (V7).
- **Hidden callers.** A spawn that bypasses `spawn_command_builder` would escape the seam. The
  architecture test (V9) fails on any `crate::pty::` or `portable_pty::` use outside the allowed
  files.

## Roundtable

AGENTS.md classes this as refactor-risk (pane runtime, restore, handoff). Before code moves, a
roundtable record (`roundtable.md` in this change, the 2026-08-16 overlay kit's is the model) lists
each protected behaviour, where it lives, and which test holds it, and records what the review
found. Task 2.

## Verifier: required tests

Each row must pass before the change is done. "Today" means the test is written and green on the
unmodified code first (characterization), then stays green after the move.

| # | Proves | Where it meets the system | Answer key | Pass |
|---|---|---|---|---|
| V1 | spawn contract: argv, cwd and an env var reach the program | `PaneRuntime::spawn_*` with a real PTY; the program writes them to the screen (`#[cfg(unix)]`) | the values passed in | equal, today and after |
| V2 | exit report: one `PaneDied` per pane, reason `Exited` for `exit 3` and `Interrupted` for a signal death, `child_wait_completed` set first | real spawn of `sh -c 'exit 3'` and `sh -c 'kill -TERM $$'`, events channel | today's classification | same reason, exactly one event, flag set before the event |
| V3 | termination escalation: HUP, then TERM, then KILL, 250 ms apart; a HUP-honouring program gets only HUP | drop a `PaneRuntime` whose program traps signals and logs them to a file | today's order and timing | same signals in order; total under 1 s |
| V4 | no termination when processes are preserved for handoff | drop with `preserve_processes_on_drop = true` | program still alive | pid alive after drop |
| V5 | output order: bytes reach the parser in order, `content_seq` moves on every chunk | program prints 20 000 numbered lines; read the history | the numbers 1..20000 | in order, no gap |
| V6 | resize reaches the program and replies keep their order | resize, then the program runs `stty size` | the new size | equal |
| V7 | live handoff unchanged | `tests/live_handoff.rs`, all of it | today's suite | green, no test edited |
| V8 | process facts unchanged: `shell_pid` is the spawned shell, `shell_at_prompt` true at an idle shell, foreground job seen while `sleep` runs | `pane.process_info` handler against a real spawn | today's answers | equal |
| V9 | no code outside the seam names the PTY layer | architecture test over `src/` (text scan by file) | allow-list: `src/pane/io/`, `src/pty/`, `src/platform/`, the command builders (`src/integration/env.rs`, the builder functions in `src/pane.rs`, D5), and today's test-only users (`src/detect/mod.rs` tests, `src/pane/terminal/migration_tests.rs`, `src/server/headless/tests/mod.rs`, `src/terminal/runtime.rs::test_with_actor`, which becomes `test_with_pane_io`) | no other file matches `crate::pty::` or `portable_pty` |
| V10 | the read path costs the same | `just bench-render-scale`, 1 and 15 panes, before and after | before numbers | within run-to-run noise; delta reported in the PR |
| V11 | the whole suite and every platform build | `just check`; CI on Linux, macOS and Windows (Windows lint is skipped on macOS by design) | green | green |
| V12 | a real session still works | a beta (`just beta-local`) installed by live handoff on both Macs; ac's panes keep running; new panes, splits, an agent respawn and a server restart behave as before | ac's running herdr | no regression seen in a day of use |

The table was drafted by the implementing session; rule `spec-with-verifier-table` wants it
written or reviewed by someone else. Task 1.3 asks for that review.
