# Design — respawn pane

## Decision 1: generalise the existing respawn, do not add a second one

`respawn_shell_for_launch_pane` already does the hard part: read the terminal's
cwd, take the runtime's current size, rebuild the launch env, spawn a fresh
runtime, swap it into the same `terminal_id`, clear the agent runtime identity,
and schedule a session save. The only thing wrong with it for this feature is
its name and its hardcoded shell.

So it becomes `respawn_pane_runtime(pane_id, RespawnTarget)`, where the target
is either `LaunchArgv` (re-run `launch_argv` when the terminal has one, else
the shell) or `Shell` (what the agent-exit path asks for today). The existing
`RuntimeExitAction::RespawnShell` caller passes `Shell` and keeps its exact
behaviour; the new action passes `LaunchArgv`. One code path, two callers, no
duplicated spawn logic.

`launch_argv` is already populated for command panes (`popup.rs` sets it via
`with_launch_argv`, and the split/run paths do the same), and `None` for plain
shells — which is exactly the fallback condition, so no new bookkeeping.

## Decision 2: liveness is `child_pid()`, not a process probe

The confirmation gate needs "is something still running here". The narrow
accessor `TerminalRuntime::child_pid() -> Option<u32>` answers it: `Some` means
a live child, `None` means it exited. Deliberately not
`probe_foreground_process_from_jobs` or `process-info` — those walk the process
table, and this is a keypress-time decision, not a render-loop one, but the
cheap fact is sufficient and cannot go stale in the way a cached probe can.

"Live work" is therefore: a live child **or** outstanding todos. A pane whose
process already exited respawns with no prompt, which is the common recovery
case and the one where a prompt would be pure friction.

## Decision 3: reuse the confirmation modal and its consumption pattern

`confirm_pane_close_with_todos` established the pattern: the gate stores a
pending token, the modal's accept path re-issues the *same* runtime mutation,
and the second pass through the gate consumes the token and proceeds. That
keeps the whole decision in one place and needs no `force` flag on the wire.

Respawn adds `confirm_respawn_pane: Option<PaneId>` alongside
`confirm_close_pane` and reuses `Mode::ConfirmClose`, because the dialog
already branches on which token is set (pane close vs workspace close); this is
a third branch with its own wording, not a new mode. The two tokens are
mutually exclusive — setting one clears the other — so a close prompt can never
be answered into a respawn.

Consequence to respect: `AppState::assert_invariants_for_test` asserts pane
liveness for `confirm_close_pane`, so the new token gets the same assertion.

## Decision 4: server-owned, per the runtime/client guardrail

Respawning is a runtime fact — a process is replaced, an event fires, agent
detection re-runs — not TUI presentation. So it lands as `Method::PaneRespawn`
on the API with a `herdr pane respawn` CLI, and the keybinding is one client of
it, dispatched through `runtime_pane_respawn` exactly as close is. A sibling
agent recovering a wedged pane is then the same operation the keybinding
performs, not a parallel path.

Adding a method is additive on the wire, so `PROTOCOL_VERSION` stays put per
the project's bump rule.

## Decision 5: what survives, stated explicitly

Kept: pane id, public pane id, terminal id, layout position, size, pane label,
todos, scrollback. Replaced: the child process. Cleared: agent runtime identity
(via the existing `clear_agent_runtime_identity_after_respawn`), so detection
re-identifies whatever the pane becomes rather than reporting the dead agent.

## Decision 6: the respawn cwd sees through a wrapper's nested PTY

Dogfooding found respawn restarting in the wrong directory. The pane's direct
child here is `atuin`, which re-runs the real shell inside a PTY of its own and
never leaves its launch directory, so every cwd source that asks the pane's own
child answers with where the pane started.

`PaneRuntime::interactive_cwd` prefers the shell's own report (OSC 7), then the
foreground job one PTY below (`platform::nested_foreground_job`), then the
direct child. It stays out of the cheap `cwd()` that the sidebar calls per pane
per render: walking the process tree is affordable for a keypress, not for a
frame. The same limitation still affects that cheap path — split inheritance and
`pane.get` — and is tracked separately as
https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/63.

## Decision 7: respawn does not move focus

The refocus lives in the agent-exit caller, not in the shared primitive. Leaving
it shared meant a scripted `pane.respawn` of a background pane pulled the UI to
another workspace, which contradicts "preserve the pane's placement". The
keybinding respawns the focused pane, so it is unaffected either way.

## Alternatives considered

- **Close plus re-split.** What the user does today; loses position and id, and
  cannot be scripted against a stable pane id.
- **Sending the shell a `reset`/`exec`.** Only works when a shell is at the
  prompt — useless for the wedged-process case that motivates this.
- **Always prompting.** Rejected with the user: the common case is a pane whose
  process already died, where a prompt is friction with nothing to protect.
- **Never prompting.** Rejected: `prefix+ctrl+x` sits next to `prefix+x`, and an
  unguarded respawn of a working agent is unrecoverable.
