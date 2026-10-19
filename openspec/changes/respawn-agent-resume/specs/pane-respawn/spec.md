## ADDED Requirements

### Requirement: Respawn a pane into its agent's resume

Herdr SHALL provide `pane.respawn_agent`, with the same parameters, live-work
refusal and `force` result as `pane.respawn`. It SHALL replace the pane's
process with a fresh shell in the pane's current directory and type into it the
command the pane's `agent_session.restore_argv` shows, through the same path a
restore uses. The pane SHALL keep its ids, label, todos and agent records.

#### Scenario: an agent pane is respawned into its resume command

- **WHEN** `pane.respawn_agent` targets a pane whose agent reported the resume
  command `claude --resume S`
- **THEN** the pane runs a fresh shell that was given `claude --resume S` (plus
  the recorded launch flags), and keeps its agent session, name, label and todos

#### Scenario: an older server

- **WHEN** a client sends `pane.respawn_agent` to a server without the method
- **THEN** the server answers with an unknown-method error and does not respawn

### Requirement: Refuse a respawn into a missing agent

`pane.respawn_agent` SHALL refuse a pane with no agent session Herdr can resume
with `no_agent_session`, leaving it unchanged, whether or not `force` is set.
The CLI SHALL expose the method as `herdr pane respawn <pane_id> --resume
[--force]`.

#### Scenario: a pane without an agent session is refused

- **WHEN** `pane.respawn_agent` with `force` targets a plain shell pane
- **THEN** the response is `no_agent_session` and the pane's process is untouched
