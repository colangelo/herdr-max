# Core backend roundtable: protected behaviour before code moves

AGENTS.md classes this change as refactor-risk: pane runtime, live handoff and restore, agent
detection authority, and a new optional field in persisted state. This record is the roundtable it
asks for (tasks.md 2.1), held before code. Base: `01611fa9` on `issue/280-core-backend` (master
`55815bee` plus the plan docs). Code read on 2026-11-14 (a read-only map of the status and handoff
paths, checked against the tree).

Status: **draft by the implementer (herdr-core); waiting for herdr's read.**

## Protected behaviour, where it lives, what holds it

| Behaviour | Where it lives | Held by |
|---|---|---|
| PTY panes unchanged with `runtime` unset or `"pty"` | `PaneIo::spawn` (`src/pane/runtime_io/pty.rs`), spawn selection to be added in `src/pane.rs` | runtime-seam characterization tests (`src/pane/runtime_characterization_tests.rs`), `tests/architecture_pty_seam.rs`, the full suite; V1 |
| Hook report path and precedence over screen detection | `handle_pane_report_agent` (`src/app/api/panes.rs:1632`) → `AppEvent::HookStateReported` (`src/events.rs:139`) → `src/app/actions.rs:1579` → `set_hook_authority_at` (`src/terminal/state.rs:986`) → `recompute_effective_state` (`:2975`) | `hook_authority_overrides_fallback_for_same_agent` (`state.rs:3267`), `visible_blocker_overrides_non_blocked_hook_for_same_agent` (`:4449`), `hook_blocked_wins_over_visible_blocker` (`:4531`), `fallback_idle_does_not_override_hook_working` (`:4579`), `stale_hook_report_sequence_is_ignored_for_same_source` (`:5246`), `process_exit_clears_matching_hook_authority_before_reporting_idle` (`:5005`), and `src/app/actions.rs:4325`, `src/app/api.rs:2580` |
| Blocked reason from a hint, not from a hook | `AppEvent::AgentHintReported` (`src/events.rs:190`) → `set_agent_hint_at` (`src/terminal/hint.rs:68`) | hint tests in `src/terminal/hint.rs` |
| Full-lifecycle sources ignore detected state | allowlist `full_lifecycle_hook_authority` (`src/detect/mod.rs:468`); `live_full_lifecycle_hook_authority` (`state.rs:2651`); detector atomics (`src/app/api.rs:520-539`) | `visible_blocker_does_not_override_full_lifecycle_hook_authority` (`state.rs:4474`), `detected_working_fallback_is_ignored_under_full_lifecycle_hook_authority` (`:4710`), `src/pane.rs:7607` |
| Live handoff of PTY panes | `src/server/headless/lifecycle.rs:67-264` (pause readers, capture, entries from runtimes, dup every fd, send, commit, then `preserve_for_handoff`); import zips manifest panes with fds by position (`src/server/headless/bootstrap.rs:193-201`); restore looks imports up by old pane id (`src/persist/restore.rs:729,793,872`) | `tests/live_handoff.rs` (`:772`, `:1006`, `:2012`, `:2117`, `:2208`, `:2345`), `src/server/headless/tests/mod.rs:226`, `src/persist/restore.rs:3231,3375` |
| Restart is a stop plus a fresh start; stop ends programs | `herdr server restart` → `server.stop_for_restart` (`src/session.rs:273`), then a new daemon; `herdr server stop` → `server.stop` (`src/cli/server.rs:50,73`) | `tests/client_mode.rs` restore tests; `tests/session_restore.rs` |
| Persisted pane record loads across versions | `PaneSnapshot` (`src/persist/snapshot.rs:117-164`), `SNAPSHOT_VERSION = 3` (`:12`), only newer versions refused (`:661`), no `deny_unknown_fields`; the handoff manifest carries the same `SessionSnapshot` (`src/server/handoff.rs:59`) | snapshot round-trip tests in `src/persist/snapshot.rs`; `an_older_server_imports_a_manifest_with_an_announced_host_shutdown` (`src/server/handoff.rs`) |
| Client wire protocol and API shapes | `src/protocol/`, `tests/fixtures/endpoint-method-shapes-v1.json` | frozen fixtures and digests (AGENTS.md § Stable client endpoint contract). Phase 2 adds no client-protocol or API field |

## What the roundtable found

### R1. A pane that cannot hand over an fd aborts the whole handoff today

`lifecycle.rs:157-174` collects `duplicate_handoff_fd()?` for every runtime in `handoff_entries`; any
error closes the fds already duplicated, rolls back and returns the error. There is no per-pane skip.
So D10's "a core pane answers `Unsupported`" would make every live handoff fail as soon as one core
pane exists, and with it every beta install. **Design change:** core panes are left out of
`handoff_entries` (and so out of `manifest.panes` and the fd list, whose counts must match) before
any fd is duplicated, by asking the runtime whether it has a master fd. They travel in the snapshot
instead (`PaneSnapshot.core_terminal`), which both handoff and restart carry. `Unsupported` stays as
the answer if anything ever asks a core pane for an fd, as a guard. V10 gains a handoff with a core
pane that must complete, not roll back.

### R2. Hook sequence numbers would drop every report after a core restart

`accept_hook_report` drops a report whose `seq` is at or below the last one for that source, and
ignores unsequenced reports once a source has used `seq` (`state.rs:7166`). Core's event `seq` starts
again at 0 on every new core (phase 0 Q6). Passing it through would silence the pane after any core
update. **Design change:** the backend keeps its own per-pane counter, increasing for every status
it forwards, and never forwards core's `seq`.

### R3. Restart and stop are separate server methods

`herdr server restart` sends `server.stop_for_restart`, not `server.stop`, and does not use live
handoff (`src/session.rs:273`). That is the hook D10 needs: on `stop_for_restart` and on a live
handoff, core panes detach and stay; on `server.stop`, pane close and respawn, they are killed.

### R4. Status enters as a hook report, and "done" is not a state

The entry point for D9 is the same event a hook uses: `AppEvent::HookStateReported { pane_id,
source, agent_label, state, message, seq, session_ref: None }` on the pane's event channel, handled at
`src/app/actions.rs:1579`. `AgentState` has `Working`, `Idle`, `Blocked`, `Unknown`; there is no
"done": done is the unseen mark a `Working` to `Idle` transition leaves. So core's `done` maps to
`Idle` (D7 and D9 change), and a blocked `kind` goes as a separate `AgentHintReported` with
`Some(BlockedReason::...)`, because a hook report carries no reason. Source string: `herdr:core`,
a normal (not full-lifecycle) source, so a visible screen blocker can still override a non-blocked
report for the same agent, as for Claude's own hooks today. Making it full-lifecycle (adding it to
`detect/mod.rs:468`) is not done in phase 2: that list pairs a source with one agent, and core
reports for several.

### R5. Detection still sees the program

The detector's process probing is gated on `pid > 0` (`src/pane.rs:1457-1465`). A core pane publishes
`Terminal.pid` into `child_pid` before the reader starts (D4), and the program runs on this host, so
process identification, `shell_at_prompt` and the full-lifecycle checks that compare the detected
agent with a hook's agent keep working. A core pane without a pid (a failed `run` reply) never
starts; it falls back to the PTY (D1).

### R6. Reattach must not be confused with a fresh spawn

A pane restored from `core_terminal` must publish the terminal's existing pid, keep its agent resume
facts, and must not send the restore launch line (`restore_argv`) into a program that is already
running. The restore path decides spawn vs attach before any launch line is queued; V10 checks the
reattached pane receives no input on reattach.

## Outcome

- D7, D9 and D10 change as R1-R4 say; the tasks gain R1's skip-before-dup and R6's no-launch-line
  check.
- No change to the client protocol, API fixtures or `SNAPSHOT_VERSION` (an optional field, loaded by
  older builds).
