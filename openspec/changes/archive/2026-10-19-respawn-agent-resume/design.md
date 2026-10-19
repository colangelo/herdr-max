## Context

A restore resumes an agent by giving its terminal a pending resume plan; the
app then spawns the pane's shell and types the plan's command
(`start_pending_agent_resume`), with retries, the Codex app-server naming and the
session name. `TerminalState::restore_plan_preview` builds the same plan from a
live terminal for `agent_session.restore_argv` (fork issue 201); the API adds a
Claude pane's live footer (model, effort) on top (fork issue 144).

## Decisions

- **Reuse the restore path, not a new launcher.** `respawn_pane_agent` stops
  the current runtime, sets the terminal's pending plan and calls
  `start_pending_agent_resume_for_terminal`, so the command, retries and naming
  are exactly a restore's. When it cannot start at once (no screen size), the
  pending-resume loop starts it later, as after a restore.
- **The plan is the preview.** `pane_agent_resume_plan` is
  `restore_plan_preview` plus the live footer, so what `--resume` types is what
  `pane get` shows as `restore_argv`.
- **Keep the agent's records.** Unlike `pane.respawn`, the agent runtime
  identity is not cleared: the session, name, reported resume and launch
  record stay, so a later restore resumes the same session. The pane shows the
  agent as idle at once, as a restored pane does.
- **A new method, not a field.** An older server would ignore a `resume` field
  on `pane.respawn` and run a plain respawn, stopping the agent. A separate
  method makes an older server answer "unknown method". It is not advertised to
  the client shell; the TUI does not use it.
- **Refuse before asking.** A pane with no resumable session is refused with
  `no_agent_session` before the live-work check, so `--force` never stops a
  process it cannot bring back.

## Risks

- The typed command runs in the user's shell, as on restore; an agent missing
  from PATH fails the same way a restore would, with the restore retry logic.
