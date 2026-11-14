## ADDED Requirements

### Requirement: One backend seam for every pane program

Every pane program SHALL be started, fed, resized, observed and ended through the pane backend seam
(`PaneIo`). This covers new panes, splits, API spawns, agent start and respawn, popups, custom
commands, session restore and live-handoff adoption. Code outside the seam's allow-list SHALL NOT
name `crate::pty` or `portable_pty`.

#### Scenario: A new spawn path cannot bypass the seam
- **WHEN** code outside `src/pane/io/`, `src/pty/`, `src/platform/` and the command builders refers to `crate::pty::` or `portable_pty::`
- **THEN** the architecture test fails and names the file

#### Scenario: Restore and handoff adoption use the seam
- **WHEN** a session is restored from `session.json`, or a replacement server adopts a pane by fd
- **THEN** the pane's program is attached through `PaneIo`, the same as a new pane

### Requirement: Behaviour unchanged behind the PTY backend

With the PTY backend, a pane SHALL behave as it did before the seam: the program gets the argv, cwd
and environment it was given; output reaches the terminal parser in order; a resize reaches the
program; the shell pid and process facts are reported as before.

#### Scenario: Spawn contract
- **WHEN** a pane starts a program with a given argv, cwd and environment variable
- **THEN** the program sees exactly those values

#### Scenario: Output order
- **WHEN** the program prints 20 000 numbered lines
- **THEN** the pane's history holds them in order with no gap

#### Scenario: Resize
- **WHEN** the pane is resized and the program then asks for its size
- **THEN** it gets the new rows and columns

### Requirement: Exit is reported once, after the wait completes

When a pane's program ends, the backend SHALL report the exit exactly once, as `Exited` for a normal
exit and `Interrupted` for a signal death, and only after it has marked the wait complete. A pane
adopted by handoff has no wait handle and SHALL report `Handoff` when its reader ends.

#### Scenario: Normal exit
- **WHEN** the program runs `exit 3`
- **THEN** one `PaneDied` event with reason `Exited` is sent, after the wait is marked complete

#### Scenario: Signal death
- **WHEN** the program is killed by a signal
- **THEN** one `PaneDied` event with reason `Interrupted` is sent

### Requirement: Session termination policy unchanged

When a pane is dropped and its processes are not preserved for a handoff, the backend SHALL signal
the pane's process session with hangup, then terminate, then kill, waiting up to 250 ms after each
and stopping as soon as every process is gone. When the processes are preserved, it SHALL send no
signal.

#### Scenario: A program that ignores hangup and terminate
- **WHEN** a pane is dropped whose program traps hangup and terminate
- **THEN** it receives hangup, then terminate, then kill, in that order, within one second

#### Scenario: A program that exits on hangup
- **WHEN** a pane is dropped whose program exits on hangup
- **THEN** it receives only hangup

#### Scenario: Preserved for handoff
- **WHEN** a pane marked to preserve its processes is dropped
- **THEN** its program is still running afterwards

### Requirement: Handoff by file descriptor is a backend capability

Live handoff by master file descriptor SHALL remain available for the PTY backend on Unix, with the
same quiesce, duplicate, roll back and release steps. A backend without a master file descriptor
SHALL answer those steps with an `Unsupported` error rather than panic.

#### Scenario: PTY backend handoff
- **WHEN** the live-handoff suite runs
- **THEN** every test passes without edits

#### Scenario: Backend without a descriptor
- **WHEN** a handoff step is asked of a backend that has no master file descriptor
- **THEN** it returns an error of kind `Unsupported`
