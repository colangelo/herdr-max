## Context

Upstream 2552d101 (ported in this batch) lets a reporter attach `resume_argv`
to `pane.report_agent` / `pane.report_agent_session`. Herdr keeps it only when
the carrying session report was accepted and the reporter holds the pane (for
a recognized agent: it is the detected process and has not exited), persists
it, and on restore types it into the pane's shell instead of the built-in
`claude --resume <id>`.

The Claude hook today runs only on SessionStart and reports the session id.

## Live findings (Claude Code 2.1.284, throwaway tmux on m4m, no prompts sent)

- `--permission-mode bypassPermissions` alone starts in bypass, and bypass
  stays in the shift+tab cycle.
- `--permission-mode auto` alone: the cycle is auto → manual → accept edits →
  plan, with no bypass.
- `--allow-dangerously-skip-permissions --permission-mode auto`: starts in
  auto, and bypass is back in the cycle.
- `--dangerously-skip-permissions --permission-mode auto` starts in **bypass**:
  the dangerous flag wins. So "auto, with bypass reachable" must use the
  `--allow-…` flag, never `--dangerously-skip-permissions`.
- `--permission-mode default` is accepted and shows "manual mode"; `dontAsk`
  is accepted too.
- `--model claude-sonnet-5-5 --effort low` shows "Sonnet 5.5 with low effort".
- SessionStart hook input: `session_id`, `transcript_path`, `cwd`,
  `hook_event_name`, `source`, `model`. No permission mode, no effort.
- Transcripts: user prompt lines carry `permissionMode` (seen:
  bypassPermissions, auto, default, plan); assistant lines carry
  `message.model` and a top-level `effort`.

## Decisions

### Where each value comes from

The hook builds the command from, in order of trust:

- **Permission mode:** the hook input's `permission_mode` (the live value, on
  events that carry it) → the last user line's `permissionMode` in the
  transcript tail → the Claude process's own launch flags
  (`--dangerously-skip-permissions` means bypassPermissions,
  `--permission-mode X` means X). Only the seven known values are passed on;
  anything else is left out.
- **Model:** the hook input's `model` (SessionStart has it) → the last
  assistant line's `message.model`, skipping `<synthetic>`.
- **Effort:** the last assistant line's `effort`, if it is one of low, medium,
  high, xhigh, max.
- **Bypass reachable:** the mode is bypassPermissions, or the launch flags hold
  `--dangerously-skip-permissions`, `--allow-dangerously-skip-permissions` or
  `--permission-mode bypassPermissions`, or a user line in the transcript tail
  was in bypassPermissions. Then the command carries
  `--allow-dangerously-skip-permissions`. A pane restored this way has that
  flag in its own launch flags, so the fact survives any number of restarts.

A value the hook cannot find is left out, and Claude uses its settings default
for it, which is today's behaviour.

### Reading the launch flags

The hook walks up from its parent process (at most 4 steps, `ps -o
ppid=,args=`) to the first process whose command is `claude` or a Claude
versions binary, and reads its flags. This is the agent's own integration
reading its own process, not herdr parsing process invocations (which upstream
declined in herdrdev/herdr#965). The PowerShell hook skips this step; there
bypass is detected from the mode and the transcript only (limit, noted in the
docs).

### Reading the transcript

Only the last 512 KiB are read, from the end, so a long session costs the same
as a short one. Lines are JSON; broken ones are skipped.

### When the hook reports

SessionStart (as today), UserPromptSubmit and Stop. UserPromptSubmit carries
the live mode at the moment of the prompt, and Stop sees the reply's model
and effort in the transcript. A mode changed with shift+tab and followed by a
crash before the next prompt is not seen (limit). Every report sends the full
command; herdr stores it only when it changed, so an unchanged report writes
nothing.

A report on UserPromptSubmit or Stop has no `session_start_source`, so a
nested `claude -p` inside the pane cannot move the pane's session: its report
is refused as a conflicting session, and the resume command that came with it
is dropped with it (2552d101 ties the command to an accepted report).

### The installer

`claude_settings.rs` is built around one canonical SessionStart entry. It
becomes a list of canonical entries — SessionStart with its source matcher,
UserPromptSubmit and Stop without a matcher — each kept in place when already
canonical, appended otherwise, and all removed by uninstall. The removal table
gains the `session` action for UserPromptSubmit and Stop so uninstall finds
them. Byte-exact no-op on a canonical file stays a tested property.

### API closes (#120)

A request dispatched by the TUI (`dispatch_runtime_mutation`) is marked while
it runs; everything else is an API request. For an API request the close gates
are pure checks that never touch `mode` or the confirm tokens:

- worktree group (pane/tab close that would close a group): refused with
  `confirmation_required` unless `force`.
- open todos in the pane, tab or workspace being closed: refused with
  `confirmation_required` and a message that lists them, unless `force`.
- With `force`, the close runs and the success result is
  `closed_todos: [{pane_id, todos}]` for the panes that had open todos.

The TUI keeps its modal and token flow; its workspace and tab closes skip the
todo check (they never asked about todos) because they are TUI requests, and
pass no `force`, so their answers are unchanged.

A modal whose target is gone is also dropped in `compute_view`, because a
pane can vanish under its modal without any close request (its process exits).

`ConfirmClose` with no live target leaves the mode: the token-consuming paths
reset the mode when nothing is left, and closing a pane clears a pending
confirmation that named it.

### Protocol

`force` is an optional field (default false, omitted when false) on new
`PaneCloseParams` / `TabCloseParams` and on `WorkspaceCloseParams`; the old
JSON still parses. No TUI wire-protocol change.

## Risks

- UserPromptSubmit now runs a short Python hook before each prompt (one
  transcript tail read and one socket write). Measured cost goes on the issue.
- A model or effort set only in the launch flags and never used for a reply is
  not seen until the first reply.
