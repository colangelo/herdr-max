# pane-respawn Specification

## Purpose
Define the respawn action that replaces a pane's process without removing the
pane: what survives it (pane id, public pane id, terminal id, position, size,
label, todos, scrollback) and what does not (the child process, the agent
runtime identity), which command the replacement runs, the confirmation that
guards live work, and the API, CLI, and keybinding surfaces that reach it.

## Requirements

### Requirement: Respawn a pane in place

Herdr SHALL provide a respawn action that replaces the focused pane's process
without removing the pane. Respawning SHALL preserve the pane's identity and
placement — its internal pane id, its public pane id, its terminal id, its
position and size in the layout, its label, and its todos — and SHALL replace
only the running process.

The respawned process SHALL be the pane's recorded launch command when the pane
has one, and the configured shell otherwise. Respawning SHALL reuse the pane's
current working directory, its current size, and its launch environment.

Respawning SHALL clear the pane's agent runtime identity, so agent detection
re-identifies the pane from what actually comes back rather than continuing to
report the process that was replaced.

The action SHALL be a configurable `KeysConfig` entry (rebindable and
unbindable) defaulting to `prefix+ctrl+x`, SHALL appear in the `prefix+?` help
panel, and SHALL be listed as a commented entry in the generated config
template.

#### Scenario: A command pane re-runs its command

- **WHEN** a pane was launched with a command and the user respawns it
- **THEN** the same command runs again in that pane
- **AND** the pane keeps its id, position, size, label, and todos

#### Scenario: A shell pane gets a fresh shell

- **WHEN** a pane was launched as a plain shell with no recorded command and
  the user respawns it
- **THEN** a new shell runs in that pane

#### Scenario: Agent identity does not survive the respawn

- **WHEN** a pane running an identified agent is respawned
- **THEN** the pane no longer reports the previous agent's runtime identity
- **AND** detection identifies the pane from the new process

#### Scenario: A pane whose process exited respawns

- **WHEN** a pane's process has already exited and the user respawns the pane
- **THEN** a new process starts in that pane

### Requirement: Respawn confirms before killing live work

Respawning SHALL prompt for confirmation when the pane would lose live work,
and SHALL proceed without prompting otherwise. A pane counts as holding live
work when its child process is still running, or when it has outstanding
todos.

Confirming SHALL respawn the pane. Cancelling SHALL leave the pane and its
process untouched. A pending confirmation for a pane SHALL be consumed by the
confirming retry, so that confirmation requires no additional parameter on the
respawn request.

A pending respawn confirmation and a pending close confirmation SHALL be
mutually exclusive, so that answering one cannot perform the other.

#### Scenario: A live process prompts first

- **WHEN** the user respawns a pane whose child process is still running
- **THEN** a confirmation prompt is shown
- **AND** the process is not replaced until the user confirms

#### Scenario: Confirming respawns the pane

- **WHEN** a respawn confirmation is shown and the user confirms
- **THEN** the pane is respawned

#### Scenario: Cancelling leaves the pane alone

- **WHEN** a respawn confirmation is shown and the user cancels
- **THEN** the pane keeps its running process

#### Scenario: An exited pane with no todos does not prompt

- **WHEN** the user respawns a pane whose process has exited and which has no
  outstanding todos
- **THEN** the pane is respawned with no confirmation prompt

#### Scenario: Outstanding todos prompt even when the process exited

- **WHEN** the user respawns a pane that has outstanding todos
- **THEN** a confirmation prompt is shown

### Requirement: Respawn is available over the API and CLI

Respawning SHALL be exposed as a runtime method that names a target pane, and
as a CLI command over that method, so a client other than the TUI can recover a
pane. The method SHALL apply the same confirmation semantics as the keybinding
and SHALL report whether the pane was respawned.

Requesting a respawn for an unknown pane SHALL fail with an error rather than
respawning another pane.

#### Scenario: Respawn over the API

- **WHEN** a client requests a respawn for a pane id
- **THEN** that pane's process is replaced under the same rules as the
  keybinding

#### Scenario: Unknown pane is an error

- **WHEN** a client requests a respawn for a pane id that does not exist
- **THEN** the request fails and no pane is respawned
