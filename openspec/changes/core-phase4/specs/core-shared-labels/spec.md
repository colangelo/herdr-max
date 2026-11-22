## ADDED Requirements

### Requirement: Follow turns and labels without reading status

With `[experimental] core_label_sync = true`, the herdr server SHALL subscribe to its own core
session's events for the terminals labelled with this herdr session, and SHALL track each terminal's
`turn`, `herdr.seen` and `herdr.pinned`. It SHALL NOT read core's `status` records. With the flag off it
SHALL NOT subscribe or write any `herdr.seen` or `herdr.pinned` label.

#### Scenario: a status event arrives

- **WHEN** core sends a `status` event for one of herdr's terminals
- **THEN** herdr's agent state for that pane does not change

#### Scenario: the core restarts

- **WHEN** the core connection is lost and comes back
- **THEN** herdr takes a fresh snapshot and applies only the connect rules

### Requirement: Write herdr.seen when a person looks

A core pane is on screen when an attached shell client whose outer terminal reported focus-in shows
the pane's tab, and the tab is not zoomed on another pane. While a pane is on screen, the server SHALL
decide once per core turn: when `herdr.seen` is missing or below the turn, it SHALL write
`herdr.seen=<turn>`. It SHALL NOT lower `herdr.seen`. A pane that leaves the screen and comes back
SHALL be decided again.

#### Scenario: ac reads a finished agent

- **WHEN** a core pane's turn is 3, its `herdr.seen` is 2, and ac focuses a client that shows its tab
- **THEN** the server writes `herdr.seen=3` once

#### Scenario: another client marks it unread while it stays on screen

- **WHEN** the pane stays on screen at turn 3 and another client writes `herdr.seen=2`
- **THEN** the server writes nothing until the pane leaves the screen and returns, or turn 4 arrives

#### Scenario: an unfocused client

- **WHEN** the only client showing the tab reported focus-out, or never reported focus
- **THEN** the server writes no `herdr.seen`

#### Scenario: an explicit mark read

- **WHEN** `herdr pane mark-seen` marks two done core panes read
- **THEN** the server writes `herdr.seen=<turn>` on both terminals

#### Scenario: a new core pane

- **WHEN** herdr starts a core terminal for a new pane
- **THEN** the terminal's labels include `herdr.seen=0`

### Requirement: Read herdr.seen from other clients

When `herdr.seen` on a terminal rises to its current turn or above, the server SHALL mark the herdr
pane seen. When it falls from at least the turn to below it, the server SHALL mark the pane unseen
if the pane is an idle agent. A turn change alone SHALL NOT mark a pane unseen.

#### Scenario: ac reads the agent on the phone

- **WHEN** a pane is `done` in herdr at core turn 4 and the phone writes `herdr.seen=4`
- **THEN** herdr shows the pane `idle` in its sidebar, agent list and API

#### Scenario: the phone marks it unread

- **WHEN** an idle, seen agent pane at turn 4 has `herdr.seen` changed from 4 to 3
- **THEN** herdr shows the pane `done`

#### Scenario: a stale raise

- **WHEN** a pane is `done` at turn 4 and another client writes `herdr.seen=3`
- **THEN** the pane stays `done`

### Requirement: Share agent pins as herdr.pinned

The server SHALL write `herdr.pinned=1` when herdr pins an agent in a core pane and `herdr.pinned=`
when it unpins one. A `herdr.pinned` change from another client SHALL pin the agent at the end of
herdr's pin order, or unpin it. At connect, an agent pinned on either side SHALL end pinned on both.

#### Scenario: pin in herdr

- **WHEN** ac runs `herdr agent pin` on a core pane
- **THEN** its terminal carries `herdr.pinned=1`

#### Scenario: unpin on the phone

- **WHEN** another client removes `herdr.pinned` from a pinned agent's terminal
- **THEN** herdr's agent list shows it unpinned

#### Scenario: pinned only in herdr at connect

- **WHEN** herdr connects and an agent is pinned in herdr but its terminal has no `herdr.pinned`
- **THEN** the server writes `herdr.pinned=1`
