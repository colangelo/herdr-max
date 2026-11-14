## Why

herdr-max is moving its runtime onto upstream's herdr-core: core will own the processes, their
survival across updates, and remote machines, while our server keeps parsing, detection, the API
and the TUI (Shape B, `openspec/changes/herdr-core-client/design.md` § 6). Phase 0 (the live pilot
on hc 0.1.21, design.md § 5a) says go.

Today nothing can plug in under a pane. `PaneRuntime` (`src/pane.rs`) spawns through
`crate::pty::backend::spawn_with_portable_pty`, owns the `portable_pty::Child`, runs the child
wait itself, signals the process session on drop through `shutdown_pane_processes`, and talks to the
PTY through `PaneRuntimeIo`, an enum whose only real variant is the PTY actor. A second runtime
would have to edit all of those places at once.

Phase 1 puts one seam in front of the PTY layer and moves today's code behind it with no change in
behaviour. Phase 2 then adds a core backend behind `[experimental] runtime = "core"`.

Records: https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/277 (this phase),
https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/267 (the migration).

## What Changes

- A new module, `src/pane/io/`, owns the seam: `PaneIo` (today's `PaneRuntimeIo`, widened),
  `SpawnSpec` and `PaneIoSink`. The exit report reuses the existing neutral
  `platform::ChildExitReason`.
- Spawn, the child wait, the exit report, session termination and the shell pid move from
  `PaneRuntime` into the PTY implementation behind `PaneIo`. `PaneRuntime` only calls the seam.
- Live handoff keeps working as it does. Its fd operations stay on `PaneIo`, and a backend without
  a master fd answers them with an error.
- Characterization tests land first, before any code moves, for the behaviour the move protects.
- An architecture test keeps `portable_pty` and `crate::pty` out of `src/pane.rs` and its callers.
- No user-visible change: no config, CLI, API, wire or persisted-state change.

## Capabilities

### New Capabilities

- `pane-runtime-backend`: how a pane starts, feeds, resizes, observes and ends the terminal program
  it shows, through one backend seam.

### Modified Capabilities

None. `session-persistence`, `live-handoff-client` and `pane-activity-and-process` keep their
requirements; this change must not alter what they specify.

## Impact

- Code: `src/pane.rs` (`PaneRuntime`, `PaneRuntimeIo`, `spawn_command_builder`, `from_handoff_fd`,
  `shutdown_pane_processes`), new `src/pane/io/` (`mod.rs`, `pty.rs`), `src/pty/` (unchanged
  behaviour, now private to the seam), `src/terminal/runtime.rs` and `src/persist/restore.rs`
  (call sites only).
- Risk: refactor-risk under AGENTS.md § Testing (pane runtime, restore and handoff). A roundtable
  record is required before code moves.
- Hot path: the PTY read callback (`on_read`) is per 8 KiB chunk × panes. The seam must not add a
  lock, allocation or dynamic dispatch there; `just bench-render-scale` before and after.
- Platforms: Unix and Windows actors both sit behind the seam. Windows has no handoff.
