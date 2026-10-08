## ADDED Requirements

### Requirement: Process info sees behind a recognised wrapper

`pane process-info` SHALL report, beside the pane's own foreground job, the
foreground job of the nested PTY when the pane's foreground is a recognised PTY
wrapper, and SHALL report `shell_at_prompt`, true only when the pane's shell,
directly or behind the wrapper, is alone at the front of its terminal.

#### Scenario: A job behind atuin is visible

- **WHEN** a pane behind `atuin pty-proxy` runs `sleep 600`
- **THEN** process info lists `sleep` among the nested foreground processes and `shell_at_prompt` is false

#### Scenario: An idle wrapped shell is at its prompt

- **WHEN** the shell behind the wrapper waits at its prompt
- **THEN** `shell_at_prompt` is true

### Requirement: Panes record their last input time

A pane SHALL record the time input last reached its PTY from a user or a caller,
expose it as `last_input_at_unix` on pane info, and keep it across session save
and restore. Terminal query replies, the history harvest that serves reads, and
restore or resume launches SHALL NOT update it. A pane with no recorded input
SHALL omit the field.

#### Scenario: Sending text stamps the pane

- **WHEN** `pane send-text` writes to a pane
- **THEN** that pane's `last_input_at_unix` is at least the time of the write

#### Scenario: Reading a pane does not stamp it

- **WHEN** a pane is only read, including an alternate-screen history harvest
- **THEN** its `last_input_at_unix` is unchanged

#### Scenario: The time survives a restart

- **WHEN** the session is saved and restored
- **THEN** each pane reports the `last_input_at_unix` it had
