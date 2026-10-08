## MODIFIED Requirements

### Requirement: Codex threads carry the pane's agent name

When `agents.codex.name_threads` and `agents.codex.app_server` are both true,
herdr SHALL name the pane's daemon thread after the pane's agent name with
`thread/name/set`, off the app loop. It SHALL skip ephemeral threads and
sub-agent threads without error. For a new start it SHALL name the single
unnamed top-level thread in the pane's cwd created since the launch, and SHALL
name none when that is ambiguous. Renaming the agent SHALL rename the thread.
When herdr knows neither the thread id nor a thread carrying the old name, a
rename SHALL find the thread from the pane's Codex process: the id given after
`resume`, else the single unnamed top-level thread in the pane's cwd created
within 30 seconds of that process starting, naming none when ambiguous.
With `name_threads` false, layer A SHALL behave exactly as without it.

#### Scenario: A new Codex pane's thread is named

- **WHEN** both switches are on and agent `worker` is started as Codex in `/repo`
- **THEN** the daemon's thread for that pane is named `worker`

#### Scenario: Two new threads in the same cwd are not guessed

- **WHEN** two unnamed threads in the pane's cwd appeared since the launch
- **THEN** herdr names neither until the session id is reported

#### Scenario: A hand-launched Codex pane is named on rename

- **WHEN** Codex was typed into a pane by hand and the pane's agent is renamed `worker`
- **THEN** the thread that Codex process created is named `worker`
