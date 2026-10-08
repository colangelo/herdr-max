# Respawn a pane in place

## Why

When a pane's process wedges — an agent stops responding, a dev server locks
up — the only recovery today is to close the pane and split a new one. That
loses the pane's position in the layout, its pane id, and its place in the
sidebar, and forces a re-layout of every sibling. The process is the thing that
failed; the pane is fine.

tmux solves this with `respawn-pane`: kill what is running and start the
command again in the same pane. Herdr already has most of the machinery.
`App::respawn_shell_for_launch_pane` tears down a runtime and spawns a
replacement into the same `terminal_id`, preserving cwd, size, and launch env —
but it is reachable only from `RuntimeExitAction::RespawnShell`, when a launch
command exits on its own, and it always spawns a bare shell. There is no way to
ask for it, and no way to get the original command back.

## What Changes

- **A `respawn_pane` action**, default `prefix+ctrl+x`, deliberately one
  modifier from `prefix+x` (close pane): the same destructive family, and the
  weaker of the two. It appears in the `prefix+?` help panel like every other
  binding.
- **It restarts the pane's recorded launch command**, falling back to a shell.
  `TerminalState.launch_argv` already stores the argv a pane was launched with,
  so a pane started as `claude` re-runs `claude` and a pane started as a plain
  shell gets a fresh shell. No new persisted state.
- **It confirms only when it would kill live work.** A pane whose child process
  is still alive, or which holds outstanding todos, prompts first; a pane whose
  process already exited respawns immediately. This reuses the existing
  confirmation modal and the "a pending confirmation is the user's answer"
  consumption pattern that `confirm_pane_close_with_todos` established.
- **The pane keeps its identity**: same pane id, public pane id, terminal id,
  position, size, label, and todos. Only the process is replaced, and the
  agent runtime identity is cleared so detection re-identifies whatever comes
  back.
- **Exposed as a runtime capability, not a TUI trick**: a `pane.respawn` API
  method and a `herdr pane respawn <pane_id>` CLI command, so scripts and
  sibling agents can recover a wedged pane too.

## Impact

- Affected specs: `pane-respawn` added.
- Affected code: `src/api/schema.rs` (`Method::PaneRespawn`), the runtime
  mutation dispatch, `src/app/api.rs` (generalise
  `respawn_shell_for_launch_pane` to respawn `launch_argv`),
  `src/app/actions.rs` (the confirmation gate), `src/config/` (the binding),
  `src/app/input/navigate.rs` (the action), `src/ui/keybind_help.rs`,
  `src/ui/dialogs.rs` (modal wording), `src/cli/pane.rs`, and docs.
- **Protocol**: adds a method, which is additive rather than an incompatible
  wire change, so `PROTOCOL_VERSION` does not move.
- Fork tracking: https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/62.
  Designed to be upstream-PR-able: neutral naming, no fork-specific behaviour.

## Non-goals

- Resuming an agent's conversation. Respawn restarts a process; it does not
  pass `--continue` or reattach a session. `agent_resume` already owns that,
  and conflating them would make a recovery action silently rewrite history.
- Respawning a whole tab or workspace at once.
- Restoring the dead process's scrollback into the new one. The pane's
  scrollback is kept as-is and the new process writes after it, matching what
  the existing agent-exit respawn already does.
- A `--force` flag on the wire. The confirmation is consumed through the same
  pending-token pattern as close, so the retry needs no extra parameter.
