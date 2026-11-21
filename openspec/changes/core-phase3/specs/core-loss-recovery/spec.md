## ADDED Requirements

### Requirement: A runtime loss suspends a core pane

When a core pane's terminal finishes with reason `holder_lost`, `host_shutdown`, `server_stop` or
`stopped_by_restart`, the server SHALL keep the pane in its layout as suspended, with its agent's resume plan, and
SHALL NOT close the pane or its workspace. A terminal that finishes as `exited` with a code, or `killed` with no
signal, SHALL end the pane as it does today.

#### Scenario: The holder is quit
- **WHEN** the core's holder process ends and every terminal it held finishes with `holder_lost`
- **THEN** each of those panes stays in its place as suspended and the workspace stays open

#### Scenario: A person closes a terminal from another client
- **WHEN** a core pane's terminal finishes as `killed` with no signal
- **THEN** the pane closes as an exited pane

### Requirement: No resume while the old program may run

The server SHALL suspend a core pane whose connection was lost only after the core answers that the terminal is
not found or finished and the program's pid, checked with the start time recorded at attach, no longer runs. While
the core does not answer, the pane SHALL stay attached-in-waiting and SHALL NOT be resumed.

#### Scenario: Core unreachable, program alive
- **WHEN** the core's socket is gone and the pane's program still runs under its holder
- **THEN** the pane is not suspended and no second copy of its program starts

#### Scenario: Core back, terminal gone
- **WHEN** the core answers `not_found` for the pane's terminal and its pid no longer runs
- **THEN** the pane is suspended and later resumes by its plan

### Requirement: Suspended core panes resume on the backend they will use

The server SHALL resume suspended panes after the settle time of pane suspension (5 s, or 30 s after a burst of
three losses within 2 s), or at once on a client attach, once the backend the pane will start on is ready. An agent
pane SHALL resume by its saved plan; any other pane SHALL re-run its launch command or a shell. One notice per burst
SHALL say how many panes the loss ended.

#### Scenario: Agent pane after a holder loss
- **WHEN** a suspended Claude pane's core is ready again
- **THEN** the pane starts in place and runs its resume plan with the same session id

### Requirement: The server starts its primary core again after a loss

The server SHALL start its primary core session again, with the clean environment of the first start, when that
session's socket has been missing or refusing for at least 3 s without a break. It SHALL NOT start any other core
session, and SHALL NOT run `hc server stop`, `hc server restart` or `hc update`. After three such starts within ten
minutes it SHALL stop starting the core for the server's lifetime (until a config reload), resume suspended panes on
the PTY, and show one notice.

#### Scenario: Only the core server died
- **WHEN** the core server process is killed and its holder keeps running
- **THEN** the server starts a core after the socket has been missing for 3 s, the panes reattach, and every program
  keeps its pid

#### Scenario: The core keeps ending
- **WHEN** the core is lost a third time within ten minutes
- **THEN** no further core start is attempted, the suspended panes resume on the PTY, and one notice says so

#### Scenario: A core restart in progress
- **WHEN** `hc server restart` leaves the socket missing for under 3 s
- **THEN** the server does not start a core and the panes reattach to the restarted one
