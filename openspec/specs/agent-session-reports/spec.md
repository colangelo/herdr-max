# agent-session-reports Specification

## Purpose
How an agent's session report (the hook's `pane.report_agent_session`) becomes
the pane's recorded agent session, so resume, idle maintenance and the API can
name the session the pane is running.

## Requirements

### Requirement: A restarted agent keeps its new session report

When an agent in a pane is replaced by a new process of the same agent (a
restart), the new process's startup session report SHALL become the pane's
agent session, whether the report arrives before or after detection sees the
replacement, and whether or not detection saw the old process exit. A startup
report from the same agent with no replacement process (a nested `claude -p`,
for example) SHALL still not take over the pane's session.

#### Scenario: A Claude restarted with pane run keeps its session

- **WHEN** the Claude in a pane exits and a new Claude is started at once with
  `pane run`, and its SessionStart hook reports a session before detection has
  seen the old Claude exit
- **THEN** the pane's agent session is the new Claude's once detection sees
  the new process

#### Scenario: The replacement is seen first

- **WHEN** detection sees the new Claude process before its SessionStart
  report arrives
- **THEN** that one startup report becomes the pane's agent session, and a
  second conflicting startup report does not

#### Scenario: A nested agent does not take the session

- **WHEN** a Claude inside the pane's Claude reports a startup session and no
  replacement process is seen
- **THEN** the pane keeps its session

#### Scenario: A real exit still clears it

- **WHEN** the Claude that reported the pane's session exits and nothing
  replaces it
- **THEN** the pane's agent session is cleared
