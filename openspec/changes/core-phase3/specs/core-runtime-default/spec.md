## ADDED Requirements

### Requirement: Restored panes follow the runtime setting

With `[experimental] runtime = "core"`, a restored pane without a live saved core terminal SHALL start on the primary
core session, seeded with its saved history. With `runtime` unset or `"pty"` it SHALL start on the PTY. A pane
adopted by fd in a live handoff SHALL stay on the PTY.

#### Scenario: Restore with core selected
- **WHEN** the server restores a saved pane and `runtime = "core"`
- **THEN** the pane's program runs in a core terminal and its earlier history is shown

#### Scenario: Live handoff of a PTY pane
- **WHEN** a PTY pane is handed over by fd
- **THEN** it stays a PTY pane with the same program

### Requirement: An idle pane can be moved onto the primary core

The server SHALL offer a method, `pane.respawn_if_idle`, that moves a pane not running on the primary core onto it
only when the pane is idle, and SHALL decide and act in one turn of its loop. A refused move SHALL change nothing and
SHALL answer every reason it was refused. With `check_only` the server SHALL only answer.

#### Scenario: Idle agent pane
- **WHEN** an idle Claude pane on the PTY with an empty prompt box is moved
- **THEN** it runs on the primary core by its resume plan with the same session id, name and launch flags

#### Scenario: Someone is typing
- **WHEN** a client typed into the pane 10 s ago
- **THEN** the move is refused with `recent_input` and the pane is unchanged

#### Scenario: Older server
- **WHEN** the CLI asks a server that has no `pane.respawn_if_idle`
- **THEN** the CLI reports the method as unsupported and no pane is respawned

### Requirement: What idle means for a move

The server SHALL treat a pane as idle for a move only when no client input reached it in the last 60 s and, for an
agent pane, its state has been idle for 60 s, it reports no background work, its prompt box is empty and it has a
resume plan; for a shell pane, the shell is at its prompt and no other process runs in its session.

#### Scenario: Background work
- **WHEN** an idle agent pane reports one background shell
- **THEN** the move is refused with `background_work`

#### Scenario: Busy shell
- **WHEN** a shell pane runs `sleep 30` in the foreground
- **THEN** the move is refused with `shell_busy`

### Requirement: The runtime default stays per machine

The code default of `[experimental] runtime` SHALL stay `"pty"` until the PTY path is removed. Machines that run
herdr-max on core SHALL select it in their own config.

#### Scenario: No runtime key
- **WHEN** a config has no `runtime` key
- **THEN** new panes start on the PTY and no core connection is opened

### Requirement: Core's CPU cost is gated before the rollout

Before core becomes the runtime of every pane on ac's Macs, the busy profile SHALL show herdr's server on core at
most 1.3 times its CPU on the PTY, by median, in both the default and the one-process profile.

#### Scenario: Over the gate
- **WHEN** the re-measure shows more than 1.3x in either profile
- **THEN** the rollout waits for the batch-handing reader and a new measurement within the gate
