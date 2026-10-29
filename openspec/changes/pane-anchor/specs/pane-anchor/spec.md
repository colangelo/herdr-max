## ADDED Requirements

### Requirement: Pane processes outlive the server as their responsible process

On macOS the server SHALL start pane processes through a per-session pane anchor
whose responsibility is disclaimed, so each pane process's responsible process is
the anchor and not the server.

#### Scenario: A new pane is attributed to the anchor

- **WHEN** the server creates a pane on macOS with the anchor available
- **THEN** the pane's shell has the anchor as its responsible process

#### Scenario: The anchor survives a live handoff

- **WHEN** the server is replaced by a live handoff
- **THEN** the anchor keeps running with the same pid
- **AND** panes started before the handoff keep the anchor as their responsible process
- **AND** panes created after the handoff are started by the same anchor

### Requirement: Pane behaviour is unchanged

A pane started through the anchor SHALL behave as one the server starts itself:
same environment, working directory, controlling terminal and session, and its
exit status SHALL reach the server that started it.

#### Scenario: Exit status reaches the server

- **WHEN** a pane's process started through the anchor exits with a code
- **THEN** the server classifies that exit from the code, as for a pane it spawned itself

### Requirement: The server falls back to spawning panes itself

When the anchor cannot be started, cannot be reached, speaks another protocol
version, or is disabled with `HERDR_PANE_ANCHOR=off`, the server SHALL spawn
panes itself as before and log why once.

#### Scenario: Anchor disabled

- **WHEN** `HERDR_PANE_ANCHOR=off` is set in the server's environment
- **THEN** panes are spawned by the server and no anchor process starts

### Requirement: The anchor does not linger

The anchor SHALL exit when it has had no live child and no server connection for
10 seconds.

#### Scenario: Session stopped

- **WHEN** the session's server stops and its panes have exited
- **THEN** the anchor exits within about 10 seconds
