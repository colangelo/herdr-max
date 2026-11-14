# Runtime seam roundtable: protected behaviour before code moves

AGENTS.md classes this change as refactor-risk: it touches the pane runtime, session restore and live
handoff. This record is the roundtable it asks for, held before any code moves (tasks.md 2.1). Base:
`91ce8175` on `issue/277-runtime-seam` (master `15d30cc3` plus the plan docs).

Status: **reviewed by herdr 2026-11-14 (approved at `919a954c`).** Drafted by the implementer
(herdr-core); herdr's verifier review is folded in as V13-V15.

## Protected behaviour, where it lives, what holds it

| Behaviour | Where it lives now | Characterization |
|---|---|---|
| One spawn path for every pane program | `PaneRuntime::spawn_command_builder` (`src/pane.rs:3226`), the only caller of `spawn_with_portable_pty` | **Missing.** V9 architecture test |
| argv, cwd, env reach the program | command builders (`src/pane.rs:105,210,2654`; `platform::pane_custom_command_pty_builder`; `src/integration/env.rs`) | **Missing.** V1 |
| Shell pid published before the first output chunk is parsed | `child_pid.store` at `src/pane.rs:3286`, before `PtyIoActor::spawn` at `:3378`; `on_read` loads it per chunk (`:3333`) | **Missing.** Covered indirectly by V8; see finding F1 |
| Exit reported once, after the wait is marked complete | `spawn_blocking` child wait (`src/pane.rs:3290-3311`): `child_wait_completed.store` then `PaneDied` | **Missing.** V2 |
| Exit classification | `platform::classify_child_exit` (`src/platform/unix_common.rs:54`): signal → `Interrupted`, else `Exited` | **Missing.** V2 |
| Handoff pane reports `Handoff` on reader exit | `on_reader_exit` in `from_handoff_fd` (`src/pane.rs:3159`) | `from_handoff_fd` test (`src/pane.rs:8264`); `tests/live_handoff.rs` |
| Termination: HUP → TERM → KILL, 250 ms each, stop early when gone | `shutdown_pane_processes` (`src/pane.rs:2260`) | Liveness only: `shutdown_liveness_*` (`src/pane.rs:5371-5390`). Escalation **missing**: V3 |
| No signal when processes are preserved for handoff | `preserve_processes_on_drop` (`src/pane.rs:2211,2745,2766`) | `tests/live_handoff.rs` (indirect). **Missing** unit: V4 |
| Drop order: detection, compression, actor, then termination | `impl Drop for PaneRuntime` (`src/pane.rs:2202`) | **Missing.** V3 checks the end state, not the order; see finding F3 |
| Output order into the parser, `content_seq` on every chunk | `on_read` (`src/pane.rs:3327`), actor `read_once` (`src/pty/actor/unix.rs:823`) | `actor_reads_output_while_input_is_backpressured` (actor level). Pane level **missing**: V5 |
| Bells aggregated from the reader | `publish_terminal_bells` in `on_read` | `spawned_pty_reader_aggregates_terminal_bells` (`src/pane.rs:8014`) |
| Resize reaches the program; latest request wins under a full queue | `PaneRuntime::resize` (`src/pane.rs:3970`), actor `resize` (`unix.rs:202`) | `resize_and_nudge_keep_latest_request_when_command_queue_is_full` (actor). Pane level **missing**: V6 |
| Reflow seen by the program | `spawned_pane_process_sees_fish_handle_reflow` (`src/pane.rs:7963`) | that test |
| Handoff quiesce, dup, rollback, release | actor `begin_handoff`, `duplicate_for_handoff`, `rollback_handoff`, `release_after_commit` (`unix.rs:250-358`) | `begin_handoff_*`, `duplicate_for_handoff_requires_quiesced_actor`, `release_after_commit_prevents_further_io` (`unix.rs:1550-1900`); `tests/live_handoff.rs` (V7) |
| Process facts (fg job, cwd, `shell_at_prompt`, nested job) | pid-based: `src/detect/mod.rs:496`, `src/app/api/panes.rs:528-576`, `src/pane.rs:4556-4620` | `foreground_job_cwd_*` (`src/pane.rs:8321-8345`). API shape **missing**: V8 |
| macOS pane anchor answers privacy prompts | `pane_anchor_spawn` inside `spawn_with_portable_pty` | `tests/live_handoff.rs:2345` (anchor persistence); pane-anchor change tests |

## What the roundtable found

### F1. The pid must be published before reads start

`on_read` passes `child_pid.load()` to `process_pty_bytes` for every chunk. Today the pid is stored
before the actor starts reading, so the first chunk already has it. If the seam starts the actor and
then publishes the pid (an easy reorder when the wait moves into the backend), the first chunks are
parsed with pid 0. **Design change:** D2 states the order: `PaneIo::spawn` stores the pid into the
shared atomic before it spawns the actor. V8 gains a check that a program's first output already
sees the pid (a shell integration that reports at startup is the natural probe).

### F2. A failed actor start today leaves a running child with no owner

In `spawn_command_builder` the child-wait task starts before `PtyIoActor::spawn`. If the actor fails
(`?` at `:3387`), no `PaneRuntime` exists, so its `Drop` never terminates the session; the child sees
its master close and usually dies of SIGHUP, but nothing guarantees it, and its `PaneDied` is sent
for a pane that was never created. **Phase 1 keeps this behaviour** (no behaviour change), records it
here, and adds a test that names it so the move cannot make it worse. A fix (start the wait only
after the actor is up, or terminate on the error path) lands after phase 1:
https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/279.

### F3. Drop order is load-bearing but untested

Termination reads `child_wait_completed` to skip a reaped pid, and must run after the actor stops
reading. The seam's `terminate_session` must be called after `io.shutdown()`, as today. V3 should
also assert that no output is parsed after termination begins (the actor is stopped first).

### F4. Tests that build a `PaneRuntime` from a raw actor

`src/terminal/runtime.rs::test_with_actor` and the `openpty` tests in `src/pane.rs` (`:5392`, `:8231`)
construct the runtime from PTY types directly. They move to a `PaneIo`-level test constructor
(`test_with_pane_io`), which is why V9's allow-list names them. Count them before 4.1 so none is
silently dropped.

### F5. V5 needs a large enough scrollback

20 000 numbered lines must fit the test pane's `scrollback_limit_bytes`, or the order check reads a
truncated history and passes or fails for the wrong reason. The test sets the limit and checks the
first and last numbers are both present.

### F6. Phase 2 fit (not phase 1 work, checked so the seam does not need reshaping)

| Seam method | PTY backend | Core backend (phase 2), from phase 0 | Fit |
|---|---|---|---|
| spawn | `spawn_with_portable_pty` | `terminal.run` with `size` and `env`, then attach | ok, needs argv/cwd/env out of `SpawnSpec` (D5) |
| output | actor `read_once` → `on_read` | attach DATA → same `on_read`; first paint after reattach resets the parser | ok |
| input, submission | actor writes | attach DATA; `AUTOMATIC_INPUT` for terminal replies | ok |
| resize | `TIOCSWINSZ` with pixel sizes | RESIZE (cells only) plus FACTS `cell` | ok, pixels move to FACTS |
| exit | `child.wait()` + `classify_child_exit` | CLOSE `{reason, exit_code, signal}` | ok, maps onto `ChildExitReason` |
| shell pid | `child.process_id()` | terminal `pid` | ok (same host) |
| foreground pgid | `tcgetpgrp` on the fd | no fd; core reports `foreground.pid` | ok: `None` already degrades (`follow_cwd` falls back to `cwd()`, `foreground_cwd` to `platform::foreground_process_group_id(pid)`, `src/pane.rs:4578-4605`), and core's `foreground.pid` can fill it |
| terminate | HUP/TERM/KILL over the session | `terminal.kill`; core does not sweep detached processes | **difference, user-visible:** a backgrounded job survives a pane close. Phase 2 requirement to decide: kill the process group through core, or accept and document |
| handoff | fd | none (core keeps the program) | ok: `Unsupported` (D4) |

## Outcome

- D2 gains the pid-before-reads order (F1) and the drop order (F3).
- Tasks gain: a named test for F2's behaviour; a count of raw-actor test constructors (F4); the
  scrollback setting in V5 (F5).
- F2 is filed as https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/279; its fix lands after phase 1.
- F6's terminate difference is a phase 2 requirement to decide (recorded in
  `openspec/changes/herdr-core-client/design.md` § 6.4, phase 2).
