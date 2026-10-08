## ADDED Requirements

### Requirement: A restored pane keeps its launch flags

herdr SHALL record, for each agent pane, the carry-listed flags the agent
process was launched with, read from the process's exact argv, and SHALL add
every recorded flag the resume command does not already carry when it restores
the pane.

#### Scenario: A GPT Claude pane

- **WHEN** a pane runs `claude --settings /u/gpt.settings.json --model gpt-6-astra`
  and the server restarts
- **THEN** the pane restores with `claude --resume <id> --settings
  /u/gpt.settings.json` and the model, effort and permission mode it had

#### Scenario: A hand-started Codex pane

- **WHEN** a pane runs `codex -m gpt-6-astra -s read-only -c k=v` with session
  `<id>` and the server restarts
- **THEN** it restores with `codex resume <id> -m gpt-6-astra -s read-only -c k=v`

#### Scenario: Prompts are never carried

- **WHEN** an agent was launched with a prompt or a one-shot flag
- **THEN** the restore command does not contain it

### Requirement: A pane never restores as another agent

A pane SHALL restore only with the session of the agent detected in it at
shutdown. A session saved for a different agent SHALL be dropped when the
agent changes, SHALL be left out of the snapshot, and SHALL be replaced by the
running agent's own session report.

#### Scenario: Claude, then Codex in the same pane

- **WHEN** a pane ran claude, claude exited without a shell prompt being seen,
  and codex now runs there
- **THEN** a restore starts codex with codex's session, never claude

#### Scenario: Codex run by Claude's own tools

- **WHEN** claude runs codex through its tools, so claude keeps the terminal's
  foreground
- **THEN** claude's saved session is kept

### Requirement: The restore command can be read before a restart

`pane list` and `pane get` SHALL show, for a pane with an agent session, the
exact command a restore would run now.

#### Scenario: Checking before a reboot

- **WHEN** a script reads `pane list`
- **THEN** each agent pane's `agent_session.restore_argv` equals the command a
  restore would run
