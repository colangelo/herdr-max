# pane-working-directory

## ADDED Requirements

### Requirement: A pane reports the working directory of its interactive shell

The working directory Herdr reports for a pane SHALL be the directory of the
shell the user is interacting with, including when a wrapper process re-runs
that shell inside a PTY of its own.

Resolution SHALL prefer the directory the shell reported for itself, then the
directory resolved from the pane's process tree, then the directory of the
pane's direct child process.

Resolution SHALL follow at most one nested PTY: a child of the pane's process
that holds a controlling terminal of its own. A child sharing the pane's
terminal SHALL NOT be followed.

#### Scenario: An ordinary pane reports its shell's directory

- **WHEN** a pane's own child is the interactive shell and the shell changes
  directory
- **THEN** the pane reports the new directory

#### Scenario: A wrapped shell's directory is reported, not the wrapper's

- **WHEN** a pane's child is a wrapper that runs the interactive shell in a
  nested PTY, and that shell is in a different directory from the wrapper
- **THEN** the pane reports the shell's directory

#### Scenario: An ordinary child is not followed

- **WHEN** a pane's shell runs a command that shares the pane's terminal and
  that command is in a different directory
- **THEN** the pane still reports the shell's directory

#### Scenario: A shell's own report wins

- **WHEN** a pane's shell reports its working directory to the terminal
- **THEN** the pane reports the directory the shell reported

### Requirement: Directory resolution stays off presentation paths

Resolving a pane's working directory from its process tree SHALL happen on the
pane's own background cadence, not as part of rendering, view computation, or
output parsing. Reading a pane's working directory SHALL NOT inspect the process
tree.

Resolution SHALL refresh on a fixed interval rather than only when the pane's
foreground process group changes, so that a directory change which starts no new
process is still reported.

#### Scenario: Reading the directory does not inspect processes

- **WHEN** a caller reads a pane's working directory
- **THEN** the value is served from already-resolved state

#### Scenario: A directory change that starts no process is still reported

- **WHEN** a pane's shell changes directory without starting a new process
- **THEN** the pane reports the new directory without further input

### Requirement: Directory consumers agree

Every surface derived from a pane's working directory SHALL use the same
resolved value, so that a pane's workspace identity, its automatic name, the
directory inherited by panes created from it, and the directory reported over
the API cannot disagree.

#### Scenario: A workspace in a wrapped shell resolves its repository

- **WHEN** a workspace's identity pane runs a wrapped shell inside a git
  repository, and the wrapper was launched outside that repository
- **THEN** the workspace resolves the repository the shell is in

#### Scenario: A new pane inherits the directory the user is in

- **WHEN** a pane running a wrapped shell is split
- **THEN** the new pane starts in the shell's directory
