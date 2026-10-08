## Decisions

**Process info behind a wrapper.** Reuse `detect::wrapped_shell_job` with the
owner-aware nested lookup. The nested job is reported beside the pane's own, not
instead of it, so existing readers are unchanged. `shell_at_prompt` is the same
answer `agent start` uses (`platform::available_pane_shell`), so the two can
never disagree.

**Last input time.** The stamp lives on the pane runtime as an atomic (unix ms,
0 = unknown), set by the runtime's input methods: `send_bytes`,
`try_send_bytes`, `send_bytes_after` and the paste methods. That covers every
user and caller path without touching ~30 call sites. Automatic writers use new
`*_untracked` variants: the alternate-screen history harvest (a read), and
restore/resume launches. Terminal query replies already go through
`write_terminal_response`, which is not an input method. The session save reads
the stamp into the pane snapshot, and restore seeds it back. Seconds are exposed;
the save does not force extra writes, so the value is as fresh as the last
session save (which happens on shutdown and handoff).

**Codex thread as agent session.** `spawn_name_job` takes an optional reply:
event sender plus pane. On a resolved thread it sends
`AppEvent::CodexThreadResolved`. The app records
`PersistedAgentSession {source: "herdr:codex", agent: "codex", Id}` only when
the pane still runs Codex and no hook session exists. The official source keeps
restore's resume path working unchanged.

## Risks

- A stamp from a focus-report or mouse-report write counts as input. That is
  user activity, so it is acceptable.
- `last_input_at_unix` is unknown for panes saved before this change. Callers
  must treat unknown as active.
