# live-handoff-client Specification

## Purpose
What a live handoff and a detached server do to the attached client and to pane
sizes: panes keep their size through a handoff, the app client reattaches on its
own, and panes created with no client attached get a sensible size.

## Requirements

### Requirement: Panes keep their size through a live handoff

A live handoff SHALL carry the exporting server's effective size, and the
importing server SHALL use it as its size while no client is attached, until a
client attaches.

#### Scenario: A handoff with a client attached keeps pane widths

- **WHEN** a 173×59 client is attached and the server hands off
- **THEN** after the handoff, before any client reattaches, panes keep the size they had

#### Scenario: A manifest without a size falls back to the headless default

- **WHEN** the manifest carries no size
- **THEN** the importing server uses `server.headless_cols`/`headless_rows` as today

### Requirement: The app client reattaches after a live handoff

An app client that receives the live-handoff shutdown SHALL wait up to 30 s for
the new server and reattach at its current terminal size. Other shutdowns,
detaches, and direct terminal attaches SHALL exit as before.

#### Scenario: A client survives a handoff

- **WHEN** a client is attached and the server hands off
- **THEN** the client reconnects to the new server without anyone relaunching it

#### Scenario: A stopped server still ends the client

- **WHEN** the server stops normally
- **THEN** the client exits with its shutdown message, as today

### Requirement: Panes created with no client attached get the no-client size

While no client is attached, a new pane SHALL be spawned at the no-client size,
and a split SHALL divide the target pane's size by its ratio, resizing the
target to its share. The no-client size SHALL be, in order: a size a live
handoff carried; the last remembered client size; `server.headless_cols/rows`.

#### Scenario: A split made while detached divides the target

- **WHEN** no client is attached and a new 40×120 pane is split down with ratio 0.65
- **THEN** the panes' PTYs are 26×120 and 14×120

#### Scenario: A pane's size no longer depends on the displayed tab

- **WHEN** the displayed tab's first pane is 39×46 and a workspace is created with no client attached
- **THEN** the new pane is spawned at the no-client size, not 39×46

### Requirement: Detaching keeps the last real client size

The server SHALL remember the foreground client's terminal size, on attach, on
every resize, and when the foreground client changes, unless either dimension
is below 80×24. When no client is attached, it SHALL use that size as the
no-client size, unless a live handoff carried a size.
`server.remember_client_size = false` SHALL restore the previous behaviour.

#### Scenario: Detaching does not shrink panes

- **WHEN** a 310×56 client detaches
- **THEN** panes keep their size, and no pane is resized to 120×40

#### Scenario: The last resize wins

- **WHEN** a client attaches at 200×50, resizes to 310×56, and detaches
- **THEN** the no-client size is 310×56

#### Scenario: A tiny client is not remembered

- **WHEN** a 310×56 client detaches, then a 60×20 client attaches and detaches
- **THEN** the no-client size is 310×56

#### Scenario: Only the foreground client counts

- **WHEN** a 310×56 foreground client and a 100×30 background client are attached, and both detach
- **THEN** the no-client size is 310×56

#### Scenario: Opting out

- **WHEN** `server.remember_client_size = false` and a 310×56 client detaches
- **THEN** the no-client size is `server.headless_cols/rows`
