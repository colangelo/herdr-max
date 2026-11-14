# Core backend: design

Phase 2 of the herdr-core migration (`openspec/changes/herdr-core-client/design.md` § 6.4).
Issue: https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/280

## Context

- **The seam** (phase 1, `55815bee`): `PaneIo` in `src/pane/runtime_io/mod.rs`, with `Actor` (the
  PTY actor) and a test channel. `PaneIo::spawn(pane_id, SpawnSpec, PaneIoSink, &child_pid,
  child_wait_completed)` starts a program; `PaneIoSink { on_read, on_exit }` carries the unchanged
  parser callback and the exit report; `terminate_session` ends the session on close;
  `adopt_handoff_fd` takes over a handed-off fd; a backend without a fd answers
  `duplicate_handoff_fd` with `ErrorKind::Unsupported`.
- **Core 0.1.21 facts** (phase 0, `hc schema`, core `schema/README.md`, `docs/security.md`):
  - `terminal.run {argv | line, cwd, env, size {cols, rows}, display, labels, keep, no_parent,
    no_notify, rm, ...}` returns the `Terminal` record, with `address` and `pid`.
  - Framed mode after `hello {framed: true}`: `[channel u32][kind u8][len u32][payload]`; DATA (0),
    CREDIT (1), CLOSE (2), RESIZE (3, cells only), FOCUS (4), FACTS (5, display facts including the
    cell size), AUTOMATIC_INPUT (6, terminal replies that are not human activity, only when both
    `hello.features` and the attach reply say `automatic_input`).
  - `terminal.attach` returns a channel; the core sends a first paint (screen, optional history,
    status, title), then raw output. A stream starts with 256 KiB credit; more than 4 MiB behind
    gets a repaint instead of the backlog.
  - Exit: attach CLOSE `{reason, exit_code, signal}` and a `finished` event `{reason, exit_code,
    signal, finished_ms}`. Reasons: `exited`, `killed`, `server_stop`, `host_shutdown`,
    `holder_lost`, `stopped_by_restart`.
  - Events: `events.subscribe` (label filters), `status` events carry the 7501 report (`state`
    idle, working, done, blocked, error; `kind`, `msg`, `app`); `seq` restarts at 0 on a new core.
  - A core restart or update closes streams; reattach works in 0.29-0.36 s and the first paint is the
    current screen, not the missed bytes (Q6).
  - Socket: `HC_SOCKET`, else `$XDG_RUNTIME_DIR/herdr-core/<session>.sock`, else
    `/run/user/<uid>/herdr-core/`, else the state dir `~/.local/state/herdr-core/<session>/`.
  - Labels: free-form keys, values with spaces and 5000+ characters accepted, `key=` removes; a
    finished terminal's labels are dropped from `ls --all` (pilot, 2026-11-14).
  - Q2 (answered 2026-11-14): a program in a core terminal sees `HC_TERMINAL_ID`,
    `HC_TERMINAL_SOCKET`, `HC_SESSION`, and `hc label $HC_TERMINAL_ID k=v` works from inside.
- **`CommandBuilder`** (`vendor/portable-pty/src/cmdbuilder.rs`) exposes `get_argv`, `get_cwd`,
  `iter_full_env_as_str`, `is_default_prog` and `get_shell`, so runtime-seam D5 resolves without
  changing the builders.

## Goals and non-goals

Goals: a new pane can run as a core terminal with the same rendering, input, detection, API and
fleet behaviour as a PTY pane; the switch is off by default and per new pane; a failed core never
costs a pane; the PTY path is untouched.

Non-goals (later phases): re-adopting core terminals after our own restart or live handoff (phase 3,
uses the labels added here); layout in a core document (phase 4); remote machines (phase 5); Windows.

## Decisions

### D1. Selection and fallback

`[experimental] runtime = "pty" | "core"`, default `"pty"`, read at each spawn (a config reload
applies to the next new pane). Only new panes, splits, API spawns, agent starts and respawns
consult it; restore from `session.json` and handoff adoption stay PTY in phase 2.

When `"core"` is selected and the core client is not usable (no `hc` on `PATH`, `hello` fails, the
protocol generation is not 1, `server.status.version` is below 0.1.21, or `terminal.run` fails), the
pane is spawned on the PTY backend and the server raises one client notice per server lifetime
("herdr-core unavailable: <reason>; new panes use the PTY"). A missing optional feature never
rejects the pane (AGENTS.md § Stable client endpoint contract, by analogy).

`runtime = "core"` on Windows: a warning at config load, PTY used.

### D2. One core session per herdr session, started by our server

The core session name is `herdr` for the default herdr session and `herdr-<name>` for a named one,
so a test or throwaway herdr never shares a core with ac's sessions and never touches the user's
default core. The socket path follows core's endpoint rules (Context). When the socket is missing,
the server runs `hc server start` with `HC_SESSION` set and its own environment, minus inherited
agent-session variables (`CLAUDE_CODE_*`, `ANTHROPIC_*`, `HERDR_PANE_ID`, `HERDR_TAB_ID`,
`HERDR_WORKSPACE_ID`), because core keeps the environment of whoever started or updated it (phase 0
side finding). Our server never runs `hc update`, `hc server stop` or `hc server restart`.

### D3. One framed connection per server, one parse thread per pane

`CoreClient` owns one Unix-socket connection in framed mode:

- a writer behind a mutex (frames are small and written whole);
- one reader thread that reads frames and routes them: channel 0 to the RPC table (request id →
  one-shot reply) and to the event handler; a stream channel to its pane's bounded queue;
- per core pane, one parse thread that pops chunks, calls the unchanged `on_read`, writes the
  returned terminal replies, then sends CREDIT for the bytes it consumed.

Returning credit only after `on_read` keeps today's backpressure: a pane whose parser falls behind
stops getting bytes, and core repaints it after 4 MiB instead of queueing without bound. One slow
pane never blocks the reader for others, because each pane has its own queue (bounded at the
stream's credit) and thread, as each PTY pane has its own actor thread today.

Alternative kept: a connection per pane (simpler failure isolation, 30 sockets for 30 panes, and
one `hc connect` process each if we ever go over stdio). Rejected for now because reconnect after a
core update is then N reconnects instead of one, and phase 5's remote machines arrive through one
local core anyway.

### D4. Spawn

`PaneIo::spawn` keeps its signature. For the core backend it:

1. turns the `CommandBuilder` into `argv` (or the builder's default shell via `get_shell` when
   `is_default_prog`), `cwd` and the full environment (`iter_full_env_as_str`), so the program sees
   exactly what a PTY pane's program would (including `HERDR_PANE_ID`, `HERDR_SOCKET_PATH` and the
   shell integration variables);
2. calls `terminal.run` with `size {cols, rows}`, `display` (our host theme colors and cell size),
   `labels {herdr.session, herdr.pane_id}`, `no_parent: true`, `no_notify: true`, `keep: false`,
   blocking for at most 5 s (the PTY spawn is blocking too; the macOS anchor can take longer);
3. publishes `Terminal.pid` into `child_pid`, then calls `terminal.attach` and starts the parse
   thread. The pid is in hand before any stream exists, so runtime-seam's F1 order is structural
   here, not a matter of thread timing.

### D5. Output, reattach and repaint

Stream DATA, including the first paint, goes through `on_read` like PTY bytes. When the stream
closes without an exit (connection loss, core restart or update, CLOSE with no exit reason), the
backend reconnects and reattaches with backoff (50 ms, doubling to 2 s, while the terminal is
`live` in `terminal.get`). Before the new first paint it feeds the parser a full reset (`ESC c`)
so the repaint is authoritative, as Q6 showed the first paint is the screen, not the missed bytes.
Scrollback printed during the gap is lost; requesting `history` on reattach is a later improvement.
Whether core's first paint already starts with a reset is checked live (task 3.2) and the extra
reset kept or dropped accordingly.

### D6. Input, replies, resize

- `try_send_bytes` and `queue_user_input_submission` write DATA on the pane's channel; the
  submission keeps its text, delay, Enter order on the pane's writer path (V5 of runtime-seam).
- Terminal replies from `on_read` go out as AUTOMATIC_INPUT when negotiated, else DATA, so core does
  not count our automatic answers as a person typing.
- `resize(rows, cols, cell_w, cell_h, replies)` sends RESIZE (cells), and FACTS with the cell size
  when it changed; replies follow.
- `validate_input_submission` answers `Ok` (no tty fd to check; same as Windows today).
- **Never** `terminal.send`: core's quiet and draft rules apply to its own `send`, not to our stream.

### D7. Exit report

The first of attach CLOSE with an exit reason, or a `finished` event, for the terminal ends it; the
second is ignored. The backend sets `child_wait_completed`, then calls `on_exit` exactly once:

| core | `ChildExitReason` |
|---|---|
| `exited` with an exit code | `Exited` |
| `exited` or `killed` with a signal | `Interrupted` |
| `killed` with no signal (our close) | `Exited` |
| `server_stop`, `host_shutdown`, `holder_lost`, `stopped_by_restart` | `Interrupted` (checkpoint, as a PTY signal death) |
| connection lost and the terminal is gone from `terminal.get` | `WaitFailed` |

### D8. Close ends detached processes (runtime-seam roundtable F6)

`terminate_session` for a core pane: `terminal.kill`, then wait up to 250 ms for the exit report,
then run the same local sweep as the PTY backend over the program's session
(`platform::session_processes(pid)`, HUP → TERM → KILL, 250 ms each). Core's kill hangs up the
terminal and ends its own program and foreground job but does not sweep deliberately detached
processes (core skill); our sweep ends them, so a pane close means the same thing on both backends.
This is local only: the pid lives on this host. Phase 5 (remote) must decide again.

Alternative: accept core's narrower kill and document that a detached job survives a pane close.
Rejected: it is user-visible (a dev server keeps its port) and differs between backends.

### D9. Status and identity

- The client subscribes to `events.subscribe` with the label filter `herdr.session=<name>` on the same
  connection. A `status` event for a pane's terminal becomes an authoritative agent report for that
  pane, through the same path hook reports take today (exact entry point chosen in the roundtable:
  `src/app/api/panes.rs` report handling and `PaneRuntime`'s `self_reported_agent_active` /
  `full_lifecycle_authority_active` flags). `state` maps: idle → idle, working → working, done →
  done, blocked → blocked with `kind` as `blocked_reason`, error → idle with the message logged.
  `status.app` maps to our agent kind (`claude-code` → claude, `codex`, `pi`, `opencode`; unknown →
  detection decides).
- When no `status` has arrived, screen detection runs as today (the Claude `●` busy line counts since
  `08e69f76`).
- Labels `herdr.session` and `herdr.pane_id` (the public pane id) are written at `terminal.run`.
  Hooks need no change in phase 2: they reach our socket through the environment we pass.

### D10. Live handoff and restart with core panes

A core pane has no fd. In a live handoff the old server sends only PTY panes by fd; for a core pane
`duplicate_handoff_fd` answers `Unsupported` and the handoff code treats it as not handed over: the
old server kills its core terminal at shutdown (D8 semantics), and the new server restores it from
`session.json` as a fresh pane, as a restart does today. The other `PaneIo` handoff steps
(`begin_handoff`, `set_handoff_paused`, `release_after_commit`) answer `Ok(())` as no-ops for the
core variant, and the handoff code must not count a core pane as handed over (runtime-seam D4's
open point, decided here). Phase 3 replaces "kill and restore" with re-adoption by label.

### D11. Process facts

`Terminal.pid` is the program's pid on this host, so pid-based facts (foreground job, cwd, argv,
`shell_at_prompt`, nested job) work unchanged. `foreground_process_group_id` answers `None`;
`follow_cwd` and `foreground_cwd` already fall back (runtime-seam roundtable F6). The pane anchor
does not apply: macOS privacy prompts name Herdr Core (Q9).

## Risks

- **Detection authority.** Routing `status` events as authoritative reports changes who decides a
  core pane's state. The roundtable must name the exact entry point and the tests that hold today's
  precedence for PTY panes.
- **Handoff with mixed panes.** A bug in D10 could drop PTY panes in a handoff. `tests/live_handoff.rs`
  plus a mixed-session test guard it.
- **Hot path.** One extra channel hop per chunk per core pane. Profiled (V12).
- **Core lifecycle.** A core we start keeps running after our server stops (by design: it owns the
  programs). In phase 2 we kill our core terminals on shutdown, so nothing is left running.
- **Logout (Q8, open).** Off by default; must be answered before phase 3 defaults it on.

## Roundtable

Refactor-risk (pane runtime, handoff, detection authority). `roundtable.md` lists protected
behaviour (PTY panes unchanged; detection precedence; handoff of PTY panes in a mixed session; the
client protocol) and records findings before code moves. Task 2.

## Verifier: required tests

"Fake core" is an in-process test server speaking the framed protocol subset we use (hello, run,
attach, DATA, CREDIT, CLOSE, RESIZE, FACTS, events), so CI covers the client without `hc`. "Live" runs
against a real hc 0.1.21+ in a scratch `HC_SESSION` and scratch XDG dirs; those tests are
`#[ignore]` unless `HERDR_TEST_HC=1`, and `just core-live-test` runs them locally.

| # | Proves | Where it meets the system | Answer key | Pass |
|---|---|---|---|---|
| V1 | default unchanged: `runtime` unset or `"pty"` spawns PTY panes only | config + spawn unit test; the whole existing suite | today's behaviour | no core connection opened; suite green |
| V2 | spawn contract on core: argv, cwd and env (incl. `HERDR_PANE_ID`) reach the program | live: runtime-seam V1's program, on the core backend | the values passed in | equal |
| V3 | output order and repaint: 20 000 numbered lines in order; after a core restart mid-run, the pane's screen equals `hc read` | live; plus fake core for the reset-before-repaint order | runtime-seam V5; `hc read --lines 0` | in order; screens equal after reattach |
| V4 | input order: plain writes then a queued submission arrive text, then Enter | live: runtime-seam V14's program | the bytes | equal |
| V5 | resize and replies: `stty size` sees the new size; `CSI 6n` after `999;999H` answers the new size; replies go as AUTOMATIC_INPUT when negotiated | live; fake core asserts the frame kind | runtime-seam V6 | equal; kind 6 seen |
| V6 | exit mapping per D7, exactly one `PaneDied`, flag set first | fake core sends each CLOSE/`finished` case; live for `exit 3` and a signal | D7 table | each case maps as the table; one event |
| V7 | close ends detached processes (D8) | live: the program starts `nohup sleep 300 &` and a trap logger; close the pane | the PTY backend's result for the same program | the detached pid is gone on both backends; signal logs match |
| V8 | status events are authoritative and detection fills in | fake core sends `status` working/idle/blocked; live Claude Code with `--settings ~/.config/claude/direct-noconnectors.settings.json` | D9 mapping; detection for a shell with no reports | pane state follows the events; a plain shell still reads idle by detection |
| V9 | fallback: no `hc`, old version, refused connection, failed `run` → PTY pane plus one notice | fake core / no socket | D1 | pane alive on PTY; exactly one notice |
| V10 | mixed live handoff: PTY panes survive by fd, core panes restore fresh, nothing orphaned | `tests/live_handoff.rs` style test with one PTY and one core pane (live) | D10 | PTY pid unchanged; core pane present with a new pid; old core terminal finished |
| V11 | hooks still report through our API from a core pane | live: `herdr pane report-agent` from inside a core pane | today's report path | the report lands on the right pane |
| V12 | hot path cost | `just bench-render-scale` plus phase 0's 30-terminal busy test, PTY vs core backend | the PTY numbers | core within 1.5× of PTY server CPU; scaling delta reported |
| V13 | no `terminal.send` anywhere | architecture test over `src/` | none allowed | no match |
| V14 | live use | a beta with `runtime = "core"` on m4m for a day (herdr installs), agents started in core panes | ac's normal day | no lost pane, no wrong state seen |

The table was drafted by the implementer; herdr (the lead) reviews it before code (task 1.2).
