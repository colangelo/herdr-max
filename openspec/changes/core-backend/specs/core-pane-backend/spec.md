## ADDED Requirements

### Requirement: Core backend is opt-in per new pane

The server SHALL spawn new panes on the herdr-core backend only when `[experimental] runtime` is
`"core"`. With the key unset or `"pty"`, every pane SHALL use the PTY backend and the server SHALL NOT
connect to herdr-core. A restored pane SHALL
reattach to its saved core terminal when it has one; any other restored or fd-adopted pane SHALL use the
PTY backend.

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

### Requirement: Core panes survive live handoff and restart

On a live handoff or `herdr server restart`, the old server SHALL leave core panes' programs running,
and the new server SHALL reattach each pane to its saved terminal when that terminal is live and its
labels match the pane. PTY panes SHALL keep handing over by file descriptor.

#### Scenario: Beta install with a core pane
- **WHEN** a session with one PTY pane and one core pane is live-handed-off
- **THEN** both programs keep their pids and the core pane shows its terminal's current screen

#### Scenario: Terminal gone
- **WHEN** a restored pane's saved terminal is no longer live
- **THEN** the pane starts a fresh program, resuming its agent as a restore does today

### Requirement: Stop ends core terminals, orphans are kept

On `herdr server stop`, the session's core terminals SHALL be killed. Live terminals labelled with the
session that no pane claims SHALL be left running and named in one notice.

#### Scenario: Server stop
- **WHEN** `herdr server stop` runs
- **THEN** the session's core terminals are finished

#### Scenario: Unclaimed terminal
- **WHEN** a live terminal labelled with this session matches no restored pane
- **THEN** it keeps running and one notice names it

### Requirement: The session sweep never reaches herdr-core

Closing a core pane SHALL NOT signal the herdr-core server, its holders or other terminals' programs.

#### Scenario: Close next to another terminal
- **WHEN** one core pane is closed while another core pane runs
- **THEN** the core server, its holders and the other program are still running
