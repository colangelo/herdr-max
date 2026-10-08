## MODIFIED Requirements

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

## ADDED Requirements

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
