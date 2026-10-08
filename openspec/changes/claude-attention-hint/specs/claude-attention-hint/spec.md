## ADDED Requirements

### Requirement: A Claude dialog hook marks the pane as needing attention

When Claude reports a shown permission dialog from the main session (not a
subagent), herdr SHALL record an attention hint with reason `question` for
AskUserQuestion and `permission` otherwise, and SHALL report the pane
`blocked` with that reason while the screen's own answer is only a weak idle.

#### Scenario: A question the screen misses

- **WHEN** the hook reports AskUserQuestion and the screen reads idle only from the title
- **THEN** the pane is `blocked` with `blocked_reason: "question"`

#### Scenario: A subagent asks

- **WHEN** the report carries a subagent id
- **THEN** no hint is recorded

### Requirement: The screen and the lifecycle clear the hint

The hint SHALL clear on the session's next prompt submit, stop or session
start, on process exit, when a strong screen rule holds for about a second,
or after 30 minutes. A strong screen rule SHALL always win over the hint.

#### Scenario: Esc on the question

- **WHEN** the user presses Esc and the idle prompt box returns
- **THEN** the hint clears within about a second and the pane reads idle

#### Scenario: A late report after a clear

- **WHEN** an attention report older than the last clearing report arrives
- **THEN** it is dropped
