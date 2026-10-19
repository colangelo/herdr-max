## Why

`pane respawn` restarts a pane's process, but for an agent started from the
shell it leaves an empty prompt: the agent stops and nothing resumes its
conversation. A cold restore does resume every agent, from records Herdr keeps
live for each pane (the agent session, the hook-reported resume command and the
recorded launch flags). There is no way to use them for one pane without
restarting the whole server.

The need came up on 2026-10-19: panes opened before beta 167 still run under the
old unsigned binary's identity for macOS privacy prompts until their shell
restarts (fork issue 236), and the same job recurs for Claude Code update
restarts and stuck agents. Asked by ac; tracked as
https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/238.

## What Changes

- New API method `pane.respawn_agent` (same parameters and refusals as
  `pane.respawn`): the pane gets a fresh shell in its current directory and
  Herdr types the agent's resume command, the one `agent_session.restore_argv`
  previews, through the same deferred-resume path a restore uses.
- `herdr pane respawn <pane> --resume [--force]` sends it.
- A pane with no resumable agent session is refused with `no_agent_session`
  and left untouched.

## Impact

- `src/app/api.rs` (`pane_agent_resume_plan`, `respawn_pane_agent`),
  `src/app/api/panes.rs` (handler), `src/api/schema.rs` (method),
  `src/cli/pane/bindings.rs` (`--resume`), docs in `docs/next`.
- Fork-only: `pane respawn` and the launch-flag records are fork features.
