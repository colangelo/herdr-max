## ADDED Requirements

### Requirement: Mirror agent status into herdr-core

With `[experimental] core_status_mirror = true`, the herdr server SHALL write herdr's
agent status for each pane on the core backend into that pane's terminal device as one
OSC 7501 root record whenever the status changes. PTY panes SHALL receive nothing.

#### Scenario: a working agent shows in core

- **WHEN** the flag is on and a core pane's agent `claude` becomes working
- **THEN** core's `terminal.get` for that terminal reports status `working` with app
  `claude`

#### Scenario: the agent goes away

- **WHEN** the agent is released after a mirrored status
- **THEN** core's record for that terminal is cleared

#### Scenario: flag off

- **WHEN** the flag is off
- **THEN** no bytes are written to any terminal device

### Requirement: Status words and the app field

The record SHALL map working to `working`, blocked to `blocked` with `kind=permission`,
done to `done`, idle to `idle`, and unknown, released or no agent to `clear`. The agent
label SHALL be sent as `app` only when it is 1 to 64 characters from `[A-Za-z0-9._+-]`.

#### Scenario: blocked

- **WHEN** a core pane's agent `codex` becomes blocked
- **THEN** the record is `ESC ]7501;state=blocked:kind=permission:app=codex ESC \`

#### Scenario: an unsafe label

- **WHEN** the agent label contains `:`, `;`, ESC, BEL or another control character
- **THEN** the record is sent without `app`

### Requirement: Never split the program's output

The server SHALL write a record only when the pane's core output has been quiet for at
least 100 ms and the stream is outside any escape sequence (ESC, CSI, OSC, DCS, SOS,
PM, APC). Otherwise it SHALL keep only the latest record for that pane and retry.

#### Scenario: output inside an OSC

- **WHEN** the last bytes core sent for the pane open an OSC that is not terminated
- **THEN** no record is written until the OSC ends and the output is quiet again

#### Scenario: a burst of changes

- **WHEN** the status changes several times while the gate is closed
- **THEN** only the latest record is written once the gate opens

### Requirement: The mirror never feeds herdr's status

The echoed record SHALL NOT change herdr's screen, detection text or agent status, and
herdr SHALL NOT read core's status.

#### Scenario: echo in the stream

- **WHEN** a mirrored record arrives back in the pane's output
- **THEN** the pane's visible and detection text are unchanged and no hook report or
  hint is raised

### Requirement: Config switch with reload

`[experimental] core_status_mirror` SHALL default to false and SHALL apply on config
reload. Turning it on SHALL mirror the current status of every core pane; turning it
off SHALL clear the records it mirrored.

#### Scenario: turned on by reload

- **WHEN** the flag goes from false to true on reload with a working core agent pane
- **THEN** core reports that pane `working` without a new status change
