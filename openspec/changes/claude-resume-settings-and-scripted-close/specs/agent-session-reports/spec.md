## ADDED Requirements

### Requirement: A restored Claude pane keeps its mode, model and effort

The Claude integration SHALL report, with the pane's session, the command
that resumes that session in its current permission mode, model and effort,
and SHALL report it again on each prompt and each finished turn, so the
command herdr keeps is the latest one. A pane that could reach bypass
permissions SHALL still reach it after restore, whatever mode it was in.

#### Scenario: A pane in auto mode, launched with bypass

- **WHEN** a Claude pane was launched with `--dangerously-skip-permissions`,
  is in auto mode on Sonnet with low effort, and herdr restarts
- **THEN** herdr types `claude --resume <id> --model claude-sonnet-5-5
  --effort low --allow-dangerously-skip-permissions --permission-mode auto`

#### Scenario: A pane in bypass

- **WHEN** a Claude pane is in bypassPermissions and herdr restarts
- **THEN** the command carries `--allow-dangerously-skip-permissions
  --permission-mode bypassPermissions`

#### Scenario: Nothing known yet

- **WHEN** the hook finds no mode, model or effort
- **THEN** the command is `claude --resume <id>` with no other flags

#### Scenario: A nested Claude does not move the command

- **WHEN** a `claude -p` inside the pane reports another session on a prompt
- **THEN** the pane keeps its session and its resume command
