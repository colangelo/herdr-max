## ADDED Requirements

### Requirement: A resolved Codex thread becomes the pane's agent session

When a naming job resolves the thread of a Codex pane, herdr SHALL record it as
the pane's agent session with source `herdr:codex`, agent `codex` and kind `id`,
unless the pane already has a hook-reported session or no longer runs Codex.

#### Scenario: A started daemon pane carries its thread id

- **WHEN** `agent start --kind codex` launches on the daemon and its thread is named
- **THEN** the pane's `agent_session` is `{source: herdr:codex, agent: codex, kind: id, value: <thread id>}`
