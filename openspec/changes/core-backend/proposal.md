## Why

Phase 1 put one seam, `PaneIo` (`src/pane/runtime_io/`), between a pane and the program it shows
(https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/277, beta 194). Phase 2 adds the second
backend the migration is for: the pane's program runs in a herdr-core terminal, so core owns the
process, keeps it alive across a core update, and later (phases 3 and 5) across our own restarts and
on other machines. The pilot on hc 0.1.21 said go (`openspec/changes/herdr-core-client/design.md`
§ 5a).

Records: https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/280 (this phase),
https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/267 (the migration).

## What Changes

- A core backend behind the seam: `PaneIo::Core`, in `src/pane/runtime_io/core/` (Unix only). A new
  pane runs as a core terminal (`terminal.run`), and its attach stream feeds the same libghostty-vt
  parser, detection and client protocol as a PTY pane.
- Off by default: `[experimental] runtime = "core"` (default `"pty"`) selects it for **new panes**.
  Existing panes, restored panes and panes adopted by live handoff stay on the PTY backend.
- One framed connection per herdr server to a core session of its own (`herdr` for the default
  herdr session, `herdr-<name>` for a named one), started by the server when it is not running.
- Status: a core `status` event (the program's OSC 7501 report) is an authoritative report for the
  pane; screen detection fills in when the program does not report.
- Identity: each terminal carries `herdr.session` and `herdr.pane_id` labels (phase 3 uses them to
  re-adopt terminals). Hooks keep reaching our API unchanged, because the pane's environment, with
  `HERDR_PANE_ID` and the socket, is passed to core explicitly.
- Two decisions carried from phase 1 and phase 0:
  - **Closing a core pane ends its detached processes, as a PTY pane close does** (roundtable F6):
    `terminal.kill`, then the same local session sweep.
  - **Fleet input never uses `hc send`/`terminal.send`**: all input is DATA on the pane's own attach
    stream (phase 0 Q13: `send` pastes onto a person's unfinished draft after 60 s).
- If core is unavailable (no `hc`, too old, cannot connect), the pane starts on the PTY backend and
  the client shows one notice.
- Live handoff with core panes in the session keeps every PTY pane; core panes are not handed over
  by fd (they answer `Unsupported`) and are restored as fresh panes until phase 3.

## Capabilities

### New Capabilities

- `core-pane-backend`: running a pane's program as a herdr-core terminal behind the pane backend
  seam, selected by config, with fallback, status, identity, close and handoff rules.

### Modified Capabilities

None. With `runtime = "pty"` (the default) nothing changes.

## Impact

- Code: new `src/pane/runtime_io/core/` (client, framing, backend); `src/pane/runtime_io/mod.rs` (the
  `Core` variant); `src/pane.rs` spawn selection; `src/config/model.rs` (`runtime`); the live
  handoff path (skip core panes); detection authority (status events); docs in `docs/next`.
- Dependencies: none new (std Unix sockets, `serde_json`, existing threads).
- Hot path: per chunk, one channel hop from the connection's reader to the pane's parse thread.
  Profiled with 1 and 15 panes, and with phase 0's 30-terminal busy test, against the PTY backend.
- Platforms: Unix only; `runtime = "core"` on Windows logs a warning and uses the PTY backend.
- Risk: refactor-risk (pane runtime, handoff, detection authority). Roundtable before code.
