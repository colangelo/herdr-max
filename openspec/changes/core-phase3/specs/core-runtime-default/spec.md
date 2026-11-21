## ADDED Requirements

### Requirement: Restored panes follow the runtime setting

With `[experimental] runtime = "core"`, a restored pane without a live saved core terminal, whose program is known
to be gone, SHALL start on the primary core session, seeded with its saved history, unless the server's config keeps
it on the PTY. With `runtime` unset or `"pty"` it SHALL start on the PTY. A pane adopted by fd in a live handoff
SHALL stay on the PTY.

#### Scenario: Restore with core selected
- **WHEN** the server restores a saved pane and `runtime = "core"`
- **THEN** the pane's program runs in a core terminal and its earlier history is shown

#### Scenario: Live handoff of a PTY pane
- **WHEN** a PTY pane is handed over by fd
- **THEN** it stays a PTY pane with the same program

### Requirement: Some panes stay on the PTY by server config

The server SHALL read `[experimental] core_pty_panes`, a reloadable list of glob patterns, default empty. A pane
SHALL match by its stable name: the `--name` value in its saved agent launch flags, else its explicitly set herdr
agent name; never its terminal title or a name the agent reported about itself.

#### Scenario: A title is not a name
- **WHEN** a pane's terminal title is `manager-lab-hlp` but it has no such launch name or explicit agent name
- **THEN** it does not match

### Requirement: An excluded pane stays on the PTY on every path

A pane matching `core_pty_panes` SHALL start on the PTY for a new pane, a restore, a suspended-pane resume and a
respawn, and a move SHALL be refused with `excluded`. A move driver's own exclusions SHALL only add to the list.

#### Scenario: The computer-use host panes
- **WHEN** `core_pty_panes = ["*-hlp"]` and panes run `claude --name manager-lab-hlp` and `claude --name
  manager-aruba-hlp`
- **THEN** both restore, resume and respawn on the PTY, and a move of either is refused with `excluded`

### Requirement: An idle pane can be moved onto the primary core

The server SHALL offer a method, `pane.respawn_if_idle`, that moves a pane not running on the primary core onto it
only when the pane is idle, and SHALL decide in one turn of its loop. A refused move SHALL change nothing and SHALL
answer every reason it was refused. With `check_only` the server SHALL only answer.

#### Scenario: Idle agent pane
- **WHEN** an idle Claude pane on the PTY with an empty prompt box is moved
- **THEN** it runs on the primary core by its resume plan with the same session id, name and launch flags

#### Scenario: Older server
- **WHEN** the CLI asks a server that has no `pane.respawn_if_idle`
- **THEN** the CLI reports the method as unsupported and no pane is respawned

### Requirement: A move starts the replacement only after a confirmed kill

A move SHALL end the old program first and SHALL start the replacement only when the runtime reports the terminal
finished or not found and the program's pid no longer runs. Otherwise it SHALL refuse with `core_unreachable`.

#### Scenario: The kill is not confirmed
- **WHEN** the legacy core does not answer the kill of a pane's terminal
- **THEN** the move is refused with `core_unreachable`, no replacement starts, and the pane is unchanged

### Requirement: Input on every path is recent input

A move SHALL be refused with `recent_input` when input reached the pane in the last 60 s by any path: client keys
and paste, the API's send-text, send-keys and send-input, a queued submission (delivered or still queued), and an
agent message in either mode.

#### Scenario: Someone is typing
- **WHEN** a client typed into the pane 10 s ago
- **THEN** the move is refused with `recent_input` and the pane is unchanged

#### Scenario: A queued message
- **WHEN** an agent message was queued for the pane 20 s ago and is not yet delivered
- **THEN** the move is refused with `recent_input`

### Requirement: What idle means for an agent pane

An agent pane SHALL be idle for a move only when its manifest provides the background and prompt-box facts, its
state has been idle for 60 s, it reports no background work, its prompt box is empty, it has a resume plan, and no
process outside the agent's tree runs in its session.

#### Scenario: Background work
- **WHEN** an idle agent pane reports one background shell
- **THEN** the move is refused with `background_work`

#### Scenario: An agent without the move facts
- **WHEN** a Codex pane is moved
- **THEN** the move is refused with `unsupported_agent`

### Requirement: What idle means for a shell pane

A shell pane SHALL be idle for a move only when the shell is at its prompt, no other process runs in its session,
and no input reached it in the last 10 minutes.

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

Before core becomes the runtime of every pane on ac's Macs, the busy profile, eight runs a side, SHALL show herdr's
server on core at most 1.3 times its CPU on the PTY by median, and core's total CPU (herdr's server, the core server
and its holder) at most 1.2 times phase 2's measured total, in both the default and the one-process profile. A
result within the measured noise band of the gate SHALL be decided by eight more runs a side.

#### Scenario: Over the gate
- **WHEN** the re-measure shows more than 1.3x for the server, or more than 1.2x of phase 2's total, in either profile
- **THEN** the rollout waits for the batch-handing reader and a new measurement within the gates
