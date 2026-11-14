## ADDED Requirements

### Requirement: Core backend is opt-in per new pane

The server SHALL spawn new panes on the herdr-core backend only when `[experimental] runtime` is
`"core"`. With the key unset or `"pty"`, every pane SHALL use the PTY backend and the server SHALL NOT
connect to herdr-core. Panes restored from `session.json` or adopted by live handoff SHALL use the PTY
backend in this phase.

#### Scenario: Default configuration
- **WHEN** `runtime` is unset and a pane is created
- **THEN** the pane runs on the PTY backend and no herdr-core connection is opened

#### Scenario: Core selected
- **WHEN** `runtime = "core"` and herdr-core is usable
- **THEN** a new pane's program runs in a herdr-core terminal labelled with the herdr session and pane id

### Requirement: Fallback when herdr-core is unusable

When `runtime = "core"` but herdr-core is missing, older than 0.1.21, speaks another protocol
generation, refuses the connection, or fails to run the program, the server SHALL start the pane on
the PTY backend and SHALL show one notice per server lifetime naming the reason.

#### Scenario: hc not installed
- **WHEN** `runtime = "core"` and no `hc` is found
- **THEN** the pane starts on the PTY backend and one notice says herdr-core is unavailable

### Requirement: A core pane behaves like a PTY pane

A core pane's program SHALL receive the argv, cwd and environment it was given; its output SHALL reach
the terminal parser in order; input SHALL arrive in order with queued submissions sent as text then
Enter; a resize SHALL reach the program; terminal queries SHALL be answered for the current size.
Fleet input SHALL be written to the pane's attach stream and SHALL NOT use herdr-core's
`terminal.send`.

#### Scenario: Environment reaches the program
- **WHEN** a core pane starts a program
- **THEN** the program sees `HERDR_PANE_ID` and the other variables a PTY pane's program would see

#### Scenario: Repaint after a core restart
- **WHEN** herdr-core restarts while a core pane is open
- **THEN** the pane reattaches and its screen matches the terminal's current screen

### Requirement: Exit is reported once with a mapped reason

A core pane SHALL report its program's end exactly once, after marking the wait complete, mapping a
normal exit to `Exited`, a signal death or a core-side stop to `Interrupted`, and a terminal that is gone
with no record to `WaitFailed`.

#### Scenario: Normal exit
- **WHEN** the program exits with code 3
- **THEN** one `PaneDied` with reason `Exited` is sent

### Requirement: Closing a core pane ends its detached processes

Closing a core pane SHALL kill its herdr-core terminal and then end every remaining process in the
program's session with hangup, terminate and kill, as closing a PTY pane does.

#### Scenario: Detached job
- **WHEN** a core pane whose program started a detached background job is closed
- **THEN** the detached job is no longer running

### Requirement: Program status reports are authoritative

A herdr-core status report (OSC 7501) for a core pane's terminal SHALL set that pane's agent state as an
authoritative report. When no report has arrived, screen detection SHALL decide as for a PTY pane.

#### Scenario: A reporting agent
- **WHEN** the program reports `working` and then `blocked` with kind `permission`
- **THEN** the pane reads working, then blocked with reason permission

#### Scenario: A silent program
- **WHEN** a plain shell runs in a core pane and reports nothing
- **THEN** the pane's state comes from screen detection

### Requirement: Live handoff keeps PTY panes in a mixed session

In a live handoff, PTY panes SHALL be handed over by file descriptor as before. Core panes SHALL NOT be
counted as handed over; the old server SHALL end their terminals and the new server SHALL restore them
as fresh panes.

#### Scenario: Mixed session
- **WHEN** a session with one PTY pane and one core pane is live-handed-off
- **THEN** the PTY pane's program keeps its pid and the core pane is present again with a new program
