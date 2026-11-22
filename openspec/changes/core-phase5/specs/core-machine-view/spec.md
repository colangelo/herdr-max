## ADDED Requirements

### Requirement: Another Mac's herdr panes are shown by a view server

`herdr server --view <machine>` SHALL build its layout from the far side's `herdr-app` document, spaces marked
`herdr_max: 1` only, with one pane per terminal leaf and the leaf's block id as the pane id. It SHALL re-read the
document on its `doc` event, on `missed` and on wake. The TUI SHALL reach it as an endpoint, and
`herdr --remote core:<machine>` SHALL open a client on it.

#### Scenario: The far side adds a tab
- **WHEN** ac-mbm5's herdr creates a tab while m4m views ac-mbm5
- **THEN** the view shows the tab after the document's `doc` event

### Requirement: A view server owns nothing on the far side

A view server SHALL keep no persistence, run no orphan scan, publish no document, open no JSON API socket, and take
status from the terminal's 7501 record. Closing a pane in it SHALL detach only.

#### Scenario: Close in the view
- **WHEN** a person closes a pane in the view
- **THEN** the view detaches; the far side's program and pane stay

### Requirement: The view types without taking a terminal's size

The view SHALL attach with `resumed: true` at the terminal's current size, SHALL send input only as
AUTOMATIC_INPUT frames, and SHALL never send DATA or FOCUS frames. When the attach result does not offer automatic
input, the view SHALL attach read-only and say so.

#### Scenario: Typing in the view
- **WHEN** a person types, pastes, clicks and focuses the view's pane while it is larger than the terminal
- **THEN** every key reaches the program and the far side's terminal keeps its size

### Requirement: The view follows the far size without sending its own

The view SHALL learn the size from the terminal's `size` events and SHALL send a RESIZE only of a far size
different from the last one it sent. An idle view SHALL send no frame other than CREDIT. It SHALL draw the remote
grid at its own pane size, cropped or padded.

#### Scenario: The far herdr restarts
- **WHEN** the far herdr's server detaches from the terminal while the view stays attached
- **THEN** the terminal keeps the far herdr's last size

### Requirement: A bad far-side document does not break the view

The view SHALL parse the document at the boundary with size, depth and id limits, ignore unknown fields, and keep
its last good layout with one warning when a document is invalid.

#### Scenario: Oversized document
- **WHEN** the far side's document exceeds the limits
- **THEN** the view keeps its previous layout and logs one warning

### Requirement: Actions and the fleet CLI use the far side's own herdr

Structural actions in a view and `herdr --machine <name>` for a name that is only a core machine SHALL run the far
side's herdr CLI through `machine.exec` with an argv vector, never a shell line. A request SHALL NOT be retried once
sent. The output SHALL be the far side's own, with its pane ids.

#### Scenario: Fleet CLI parity
- **WHEN** `herdr --machine ac-mbm5 pane list --json` runs once through a saved herdr machine and once through the
  core machine
- **THEN** both outputs are the same

#### Scenario: A name with shell signs
- **WHEN** a person renames a viewed workspace to `$(touch x); w`
- **THEN** the far side's herdr receives the name as one argument and no shell runs it

### Requirement: A cut or timed-out reply is an error

A `machine.exec` reply marked `truncated` or `timed_out` SHALL make the CLI fail with a message and no partial
output, whatever its exit code. A request over 8 MiB SHALL be refused before it is sent.

#### Scenario: A cut reply
- **WHEN** the far side's reply exceeds 8 MiB and `machine.exec` returns `truncated: true` with exit code 0
- **THEN** the CLI exits 1 with a message and prints no partial output

### Requirement: The fleet CLI dials a dormant machine only when asked

On a machine that is not connected, `--machine` SHALL refuse with exit 75 and a message naming the ssh dial,
unless `--dial` is given. With `--dial` it SHALL refuse a closed gate, else write its lease, wake the machine
through the dialer, and then send.

#### Scenario: A dormant machine
- **WHEN** an agent runs `herdr --machine ac-mbm5 pane list --json` while ac-mbm5 is dormant
- **THEN** the CLI exits 75 with a message naming the ssh dial and `--dial`, and no ssh starts

#### Scenario: Dial on request
- **WHEN** the same command runs with `--dial` and the gate is open
- **THEN** one dial runs, the far side's JSON is printed, and the CLI's lease is removed after the reply

### Requirement: The old remote stays the fallback

`herdr --remote <host>`, saved herdr machines and `--machine` to them SHALL behave as before this change.
`[remote] prefer` SHALL default to `"ssh"` and SHALL decide only a name that is both a saved herdr machine and a
core machine. When both reach the same far-side core, the TUI SHALL show one notice per run.

#### Scenario: Plain --remote
- **WHEN** a person runs `herdr --remote ac-mbm5`
- **THEN** herdr uses its own ssh path and makes no core call
