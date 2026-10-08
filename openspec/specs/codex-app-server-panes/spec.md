# codex-app-server-panes Specification

## Purpose
Codex panes that run on the shared Codex app-server daemon: how they launch, how
their threads are named after the pane's agent, and how a resolved thread becomes
the pane's resumable agent session.

## Requirements

### Requirement: Codex panes can be launched on the shared app-server daemon

When `agents.codex.app_server` is true, herdr SHALL launch `agent start --kind codex`
panes and resume Codex panes on restore with `--remote unix://<socket>` and
`-C <pane cwd>`, where `<socket>` is `agents.codex.app_server_socket` with `~`
expanded. Arguments already given by the caller SHALL take precedence. When the
switch is false, launches SHALL be unchanged.

#### Scenario: A started Codex pane gets the daemon and its own cwd

- **WHEN** `agents.codex.app_server` is true and a Codex agent is started in a pane at `/repo`
- **THEN** the launch argv contains `--remote unix://<socket>` and `-C /repo`

#### Scenario: A resumed Codex pane gets the daemon and its own cwd

- **WHEN** `agents.codex.app_server` is true and a Codex pane is restored with session `S` in `/repo`
- **THEN** the resume argv is `codex resume S --remote unix://<socket> -C /repo`

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

### Requirement: A resolved Codex thread becomes the pane's agent session

When a naming job resolves the thread of a Codex pane, herdr SHALL record it as
the pane's agent session with source `herdr:codex`, agent `codex` and kind `id`,
unless the pane already has a hook-reported session or no longer runs Codex.

#### Scenario: A started daemon pane carries its thread id

- **WHEN** `agent start --kind codex` launches on the daemon and its thread is named
- **THEN** the pane's `agent_session` is `{source: herdr:codex, agent: codex, kind: id, value: <thread id>}`
