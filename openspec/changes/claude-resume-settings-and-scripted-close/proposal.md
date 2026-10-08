## Why

JOB 13 from herdr-helper, 2026-09-29; ac said "build".

- https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/123: after a reboot,
  restart or crash, herdr restores every Claude pane as plain
  `claude --resume <id>`. The pane comes back on the settings defaults, so it
  loses the permission mode it was in (ac launches with
  `--dangerously-skip-permissions` and cycles bypass/auto), its model and its
  effort. ac: save "all" three.
- https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/120: a `pane.close`
  sent over the API for a pane with open todos opens "Close pane with
  unfinished todos?" on every attached client, and the caller's retry leaves
  an orphan "Close workspace?" modal behind. Scripts (master-helper retiring
  panes) also have no way to close without a confirmation. ac: "there should
  be a command that skips the confirmation".
- https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/121: the Claude
  background-shell rule needs a space after `shells`, so a footer that ends at
  `· 2 shells` reads as idle.

Base: upstream 2552d101 (agents report their own resume command) was ported
first in the same batch (https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/124).

## What Changes

- **Claude reports its own resume command** (#123). The Claude hook reads the
  hook input and the transcript tail and reports
  `resume_argv = claude --resume <id> [--model M] [--effort E]
  [--allow-dangerously-skip-permissions] [--permission-mode P]` with
  `pane.report_agent_session`. It reports on SessionStart, and now also on
  UserPromptSubmit and Stop, so the saved command follows mode, model and
  effort changes; herdr stores only a changed command. Restore runs the
  reported command instead of the built-in one (upstream behaviour).
- **The Claude integration installs three hooks** (#123): the SessionStart
  entry as today, plus UserPromptSubmit and Stop entries running the same
  script with the same `session` action. Uninstall removes all three.
  Integration version: 9 (one bump for the batch, shared with the a5e1c941
  port).
- **API closes never open a modal** (#120). A `pane.close`, `tab.close` or
  `workspace.close` that did not come from the TUI answers
  `confirmation_required` with the reason and leaves every client's screen
  alone. The TUI keeps its modal and token flow.
- **`--force` on `pane close`, `tab close`, `workspace close`** (#120, API
  field `force`). Without it, a close that would drop open todos is refused,
  and the error lists them. With it, the close goes ahead and the result
  carries the dropped todos (`closed_todos`), so nothing is lost silently.
  `force` also answers the worktree-group confirmation for pane and tab close;
  `workspace.close` keeps its explicit `close_group`.
- **No orphan confirm modal** (#120). A ConfirmClose whose target is gone
  (consumed, or the pane closed underneath it) leaves the mode instead of
  falling back to "Close workspace?".
- **Background-shell footer** (#121). `shells?\s*(?:·|$)` in the Claude
  manifest, with tests for both footer shapes.

## Capabilities

### New Capabilities

- `scripted-close`: how a close request from outside the TUI is answered, and
  how `force` works.

### Modified Capabilities

- `agent-session-reports`: a Claude pane's reported resume command keeps its
  permission mode, model and effort.

## Impact

- `src/integration/assets/claude/herdr-agent-state.sh` and `.ps1`,
  `src/integration/claude_settings.rs`, `src/integration/tests.rs`.
- `src/api/schema/{panes,tabs,workspaces,response}.rs`, `src/app/api/{panes,
  tabs,workspaces}.rs`, `src/app/runtime_mutations.rs`, `src/app/actions.rs`,
  `src/cli/{pane,tab,workspace,runtime,spec}.rs`, `docs/next/api/herdr-api.schema.json`.
- `src/detect/manifests/claude.toml` and `website/agent-detection/claude.toml`.
- Docs: `docs/next/website/src/content/docs/{integrations,cli-reference,socket-api}.mdx`.
- API: new optional `force` fields and a new result shape for forced closes;
  old clients that send no `force` get today's answers minus the modal.
