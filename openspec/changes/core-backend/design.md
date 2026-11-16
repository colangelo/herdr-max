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
consult it. A restored pane with a saved `core_terminal` reattaches to it (D10); any other restored
pane, and a pane adopted by fd, stays PTY.

When `"core"` is selected and the core client is not usable (no `hc` on `PATH`, `hello` fails, the
protocol generation is not 1, `server.status.version` is below 0.1.21, or `terminal.run` fails), the
pane is spawned on the PTY backend and the server raises one client notice per server lifetime
("herdr-core unavailable: <reason>; new panes use the PTY"). A missing optional feature never
rejects the pane (AGENTS.md § Stable client endpoint contract, by analogy).

`runtime = "core"` on Windows: a warning at config load, PTY used.

### D2. One core session per herdr session, started by our server

The core session name is `herdr` for the default herdr session and `herdr-<name>` for a named one,
so a test or throwaway herdr never shares a core with ac's sessions and never touches the user's
default core. A debug build uses `herdrdev` and `herdrdev-<name>`, which no release name can equal
(a release session named `dev` is `herdr-dev`). `HC_SOCKET` overrides the socket for every session;
the server logs once, at info, that it is in effect. The socket path follows core's endpoint rules
(Context). When the socket is missing, the server runs `hc server start` with `HC_SESSION` set and its own environment, minus inherited
agent-session variables (`CLAUDE_CODE_*`, `ANTHROPIC_*`, `HERDR_PANE_ID`, `HERDR_TAB_ID`,
`HERDR_WORKSPACE_ID`), because core keeps the environment of whoever started or updated it (phase 0
side finding). The core is never started with a launching pane's environment. Our server never
runs `hc update`, `hc server stop` or `hc server restart`. `hc` is found on `PATH` once, at its
first use, and run by that absolute path afterwards; the server logs the path and `hc --version` at
info.

**Core lifetime.** A core our server starts outlives our server: on macOS `hc server start` goes
through core's one-shot launchd job (the pilot core runs with parent pid 1, in its own session, as
does its holder), so neither our server's exit, our live handoff, our session sweep (D8) nor closing
the pane that caused the start reaches it. A start that falls back to a direct child (core logs the
reason in `core.log`) would not have that guarantee; the server detects it (`server.status` pid's
parent is our pid) and logs a warning. V15 checks the lifetime.

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
   `labels {"herdr.session": ..., "herdr.pane_id": ...}` (a JSON map; the API refuses the CLI's
   `k=v` list), `no_parent: true`, `no_notify: true`, `keep: false`,
   blocking for at most 5 s (the PTY spawn is blocking too; the macOS anchor can take longer);
3. calls `terminal.attach` with `history` (all of it: output printed between `run` and the attach
   has already scrolled off the first paint's screen; found by V3 live), publishes `Terminal.pid`
   into `child_pid`, and starts the parse thread. A program that ended before the attach (core
   answers `finished`) still gets a pane: its last screen (`terminal.read`) and its exit record
   are queued to the parser. The pid is in hand before any stream exists, so runtime-seam's F1 order is structural
   here, not a matter of thread timing.

**Stripped variables stay stripped.** Checked live (task 1.3, 2026-11-14): `terminal.run`'s `env`
is merged into the core's own environment (a probe program saw our one variable plus the ~100 the
core started with), and its values are strings only, so it cannot unset anything. herdr strips
variables from every pane (outer agent identity such as `CLAUDECODE` and
`CLAUDE_CODE_CHILD_SESSION`, multiplexer markers such as `TMUX` and `ZELLIJ_*`, stale SSH and PTY
proxy variables, and `HERDR_PANE_ID` for a pane that omits it; `src/pane.rs` around 138-255). A core
started from such an environment would hand them back (phase 0's 401 leak). So the backend runs every
program as `/usr/bin/env -u K1 -u K2 ... -- <argv>`, with one `-u` per key herdr strips (one shared
list, used by the PTY path too, so the two cannot drift), and D2 still starts the core with them
removed. `env` replaces itself with the program (same pid), so pid-based facts are unaffected;
core's `argv` for the terminal starts with `/usr/bin/env` (cosmetic, `hc ls` only). V2 checks a
variable present in the core's environment and stripped by herdr is absent in the program.

### D5. Output, reattach and repaint

Stream DATA, including the first paint, goes through `on_read` like PTY bytes. When the stream
closes without an exit (connection loss, core restart or update, CLOSE with no exit reason), the
backend reconnects and reattaches with backoff (50 ms, doubling to 2 s, while the terminal is
`live` in `terminal.get`). The first paint is authoritative on its own: checked live (task 1.3), it
starts with CAN (`0x18`, cancels any half-parsed sequence), a soft reset (`CSI ! p`) and mode
resets, then repaints the whole screen. So the backend adds no reset of its own (a full `ESC c`
could also clear the pane's scrollback). Scrollback printed during the gap is lost; requesting
`history` on reattach is a later improvement.

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
| (status `done`) | not an exit: maps to `Idle` in D9 (roundtable R4) |
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

**The sweep never reaches core.** Checked live on hc 0.1.21 (2026-11-14, `getsid`): a core
terminal's program is its own session leader (sid = its pid), the holder and the core server each
lead their own sessions, and a `nohup ... &` child of the program stays in the program's session. So
`session_processes(pid)` holds the program and its detached jobs only. As a guard against a future
core that changes this, the core backend refuses to sweep (and logs at warn) when the program's
session id equals the core's or any holder's (`server.status` pids), and the sweep list drops those
pids if present. V7 asserts the core and the core's other terminals survive the close.

**Narrowed after the security review (M2, 2026-11-15): a program that already exited.** When the
pane's program has exited before its pane closes, the close signals nothing, so the program's
detached jobs keep running (`sweep_targets` in `src/pane/runtime_io/pty.rs`; the PTY backend shares
the rule). The reason is pid reuse: once the program has exited, its pid is free, and a process that
holds it now is a stranger whose session the sweep would end. A close while the program runs is
unchanged. Sweeping the remaining members of the old session is possible later: they still carry
its session id, and on macOS and Linux the kernel does not reuse a pid that a live session still
uses as its id, so the sweep could list processes by that session id, never signal the old pid
itself, and stay safe. That kernel rule must be checked on each platform before relying on it.

Alternative: accept core's narrower kill and document that a detached job survives a pane close.
Rejected: it is user-visible (a dev server keeps its port) and differs between backends.

### D9. Status and identity

- **Changed after the security review (M3, 2026-11-15):** herdr does not use core's `status`. In
  hc 0.1.21 core learns status only from OSC 7501 in a terminal's output, so a file, page or log
  printed in an agent pane could set that agent's state and raise blocked hints, a route from
  outside content to a control signal that PTY panes do not have. The client does not subscribe to
  `events.subscribe`; screen detection and hooks decide, as for a PTY pane. Using core status again
  needs core to mark status a process set through its API, apart from status parsed from output; it
  could then come in as before (a `HookStateReported` from `herdr:core` with
  `seq = max(previous + 1, wall-clock ns)`, R2 and R4), below hook reports and screen detection.
  The first version of this decision, which made core status authoritative, is in git history at
  `56847c12`.
- Labels `herdr.session` and `herdr.pane_id` (the public pane id) are written at `terminal.run`.
  Hooks need no change in phase 2: they reach our socket through the environment we pass.

### D10. Live handoff and restart keep core panes running (minimal re-adoption)

Chosen per herdr's review: (a) re-adopt in phase 2, not (b) kill and restore. Every beta install is
a live handoff; with (b), everyone running `runtime = "core"` would lose every agent conversation in
a core pane on each install, and keeping programs alive across our own replacement is the point of
the migration.

- **Persisted link.** A core pane's record in `session.json` (and in the handoff snapshot) gains an
  optional field `core_terminal: {session, address}`. Older binaries ignore it (the persist structs
  do not deny unknown fields); a PTY pane omits it.
- **Leaving.** On live handoff and on `herdr server restart` (which sends `server.stop_for_restart`,
  roundtable R3), the old server detaches from core panes (closes their attach channels and the
  connection) without killing them: a core pane counts as preserved, like a PTY pane handed over by
  fd. On `server.stop`, on pane close and on a pane respawn it kills them (D8), as today's semantics
  for those actions.
- **Not in the fd list.** Today one pane failing `duplicate_handoff_fd` aborts the whole handoff
  (`src/server/headless/lifecycle.rs:157-174`, roundtable R1). Core panes are left out of the
  handoff entries, the manifest's pane list and the fd list before any fd is duplicated; they ride in
  the snapshot's `core_terminal` instead.
- **Arriving.** While restoring, a pane with `core_terminal` calls `terminal.get` on that address and
  checks `state: live` and the labels `herdr.session` and `herdr.pane_id` match. If they do, the pane
  attaches (D5: the first paint repaints) and publishes the terminal's pid; detection and status
  resume; no restore launch line is sent into the running program (roundtable R6). If not (terminal gone, labels differ, core unreachable), the pane is restored as today (a
  fresh program, agent resume through `restore_argv`).
- **Handoff steps.** For a core pane, `duplicate_handoff_fd` stays `Unsupported` (it is not sent by
  fd); `begin_handoff`, `set_handoff_paused` and `release_after_commit` stop and resume the pane's
  stream locally and return `Ok(())`. The handoff code must count a core pane as preserved through
  `core_terminal`, not through an fd (runtime-seam D4's open point, decided here).
- **Orphans.** After restore, live terminals labelled `herdr.session=<ours>` that no pane claimed are
  left running (never killed in phase 2: they may hold an agent's work), logged at warn one per
  terminal, and listed in one notice with the command to see them (`hc ls --label
  herdr.session=<name>`).
- **Rollback to an older build.** An older herdr (beta 194, a stable release) ignores
  `core_terminal`, restores the pane as a fresh PTY pane, and leaves the core terminal running unseen:
  a duplicate agent. Old builds cannot be fixed, so the rule is written in the docs and the proposal:
  before rolling back with core panes open, close them, or list them with `hc ls --label
  herdr.session=<name>` and kill them. A phase 2 build that later finds such a terminal reports it
  as an orphan (above).
- **Mixed sessions.** PTY panes keep handing over by fd, unchanged.

### D11. Process facts

`Terminal.pid` is the program's pid on this host, so pid-based facts (foreground job, cwd, argv,
`shell_at_prompt`, nested job) work unchanged. `foreground_process_group_id` answers `None`;
`follow_cwd` and `foreground_cwd` already fall back (runtime-seam roundtable F6). The pane anchor
does not apply: macOS privacy prompts name Herdr Core (Q9).

## Risks

- **Detection authority.** Routing `status` events as authoritative reports changes who decides a
  core pane's state. The roundtable must name the exact entry point and the tests that hold today's
  precedence for PTY panes.
- **Handoff with mixed panes.** A bug in D10 could drop PTY panes in a handoff, or attach a pane to
  the wrong terminal. `tests/live_handoff.rs`, the label check, and V10 guard it.
- **Persisted state.** `core_terminal` is a new optional field in `session.json`: refactor-risk
  under AGENTS.md (persisted state). Roundtable item.
- **Hot path.** One extra channel hop per chunk per core pane. Profiled (V12).
- **Core lifecycle.** A core we start keeps running after our server stops (by design: it owns the
  programs). `herdr server stop` kills our core terminals; a handoff or restart leaves them for the
  next server (D10).
- **Security.** A new socket client that starts programs and passes full environments (D2-D4).
  Two-vendor security review before the beta (house rule `security-review-two-vendors`), task 5.1.
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
| V2 | spawn contract on core: argv, cwd and env (incl. `HERDR_PANE_ID`) reach the program, and a variable herdr strips stays absent even when the core's environment has it | live: runtime-seam V1's program on the core backend, with the scratch core started with `CLAUDECODE=1` and `TMUX=x` | the values passed in; the strip list | equal; stripped keys absent |
| V3 | output order and repaint: 20 000 numbered lines in order; after a core restart mid-run, the pane's screen equals `hc read` | live; plus fake core for the reset-before-repaint order | runtime-seam V5; `hc read --lines 0` | in order; screens equal after reattach |
| V4 | input order: plain writes then a queued submission arrive text, then Enter | live: runtime-seam V14's program | the bytes | equal |
| V5 | resize and replies: `stty size` sees the new size; `CSI 6n` after `999;999H` answers the new size; replies go as AUTOMATIC_INPUT when negotiated | live; fake core asserts the frame kind | runtime-seam V6 | equal; kind 6 seen |
| V6 | exit mapping per D7, exactly one `PaneDied`, flag set first | fake core sends each CLOSE/`finished` case; live for `exit 3` and a signal | D7 table | each case maps as the table; one event |
| V7 | close ends detached processes (D8) and never reaches core | live: the program starts `nohup sleep 300 &` and a trap logger; a second core terminal runs alongside; close the pane | the PTY backend's result for the same program | the detached pid is gone on both backends; signal logs match; the core server, its holder and the second terminal are still alive |
| V8 | printed status is never an agent report (D9 as changed by security review M3) | live: `cat` of a file holding forged OSC 7501 idle and blocked in a core pane | detection and hooks only | no `HookStateReported` and no `AgentHintReported` from it |
| V9 | fallback: no `hc`, old version, refused connection, failed `run` → PTY pane plus one notice | fake core / no socket | D1 | pane alive on PTY; exactly one notice |
| V10 | mixed live handoff and restart keep programs (D10); a handoff with a core pane completes, never rolls back (R1); a reattached pane gets no launch line (R6) | `tests/live_handoff.rs` style test with one PTY and one core pane (live), then `herdr server restart`, then `herdr server stop` | D10 | after handoff and restart: both programs keep their pids and the core pane's screen equals `hc read`; after stop: the core terminal is finished; a pane whose saved terminal is gone or finished restores fresh; an unclaimed labelled terminal survives, is logged at warn and named in one notice |
| V11 | hooks still report through our API from a core pane | live: `herdr pane report-agent` from inside a core pane | today's report path | the report lands on the right pane |
| V12 | hot path cost | `just bench-render-scale` plus phase 0's 30-terminal busy test (30 terminals printing a 90-byte line twice a second, 30 s), PTY vs core backend | PTY baseline: herdr server 0.83 s CPU per 30 s busy and 0.23 s idle with 30 panes, server+anchor 20.4 MB footprint (phase 0 Q4); render bench in `openspec/changes/runtime-seam/bench-after.txt` | our server within 1.3× of the PTY on the same busy profile; core and holder CPU (busy, idle) and footprint recorded, not gated (Shape B parses every pane twice by design; restated by herdr 2026-11-15, was "server + core + holder within 1.5×"); render bench median within noise; scaling 1 vs 15 panes reported |
| V13 | no `terminal.send` anywhere | architecture test over `src/` | none allowed | no match |
| V15 | core lifetime (D2) | live: our server starts the core, then exits, live-hands-off and restarts; a pane that triggered the start is closed | none of those | the core pid is unchanged throughout and is not in our server's session or process group; the core's environment has no `HERDR_PANE_ID` or `CLAUDE_CODE_*` |
| V16 | the sweep guard (D8) | unit: a fake process table where the program shares the holder's session | D8 guard | no signal sent; one warn logged |
| V14 | live use, in a throwaway session first | a beta with `runtime = "core"` in a named throwaway herdr session `herdr-core-trial` on m4m, with herdr-core's own agent work for a day; only then an ask on the desk for ac to opt his own session in | that day | no lost pane, no wrong state, programs kept across one beta install |

The table was drafted by the implementer and reviewed by herdr (the lead) on 2026-11-14 at
`99f21b7c`: approved with changes, now the D2 lifetime, D8 guard, D10 re-adoption, the error log, V7
and V10 widened, V12 with baseline numbers, V14 in a throwaway session, and V15-V16.
