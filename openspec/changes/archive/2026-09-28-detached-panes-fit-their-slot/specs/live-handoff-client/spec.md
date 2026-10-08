## ADDED Requirements

### Requirement: Panes created with no client attached get the no-client size

While no client is attached, a new pane SHALL be spawned at the no-client size
(`server.headless_cols/rows`, or a size a live handoff carried), and a split SHALL
divide the target pane's size by its ratio, resizing the target to its share.

#### Scenario: A split made while detached divides the target

- **WHEN** no client is attached and a new 40×120 pane is split down with ratio 0.65
- **THEN** the panes' PTYs are 26×120 and 14×120

#### Scenario: A pane's size no longer depends on the displayed tab

- **WHEN** the displayed tab's first pane is 39×46 and a workspace is created with no client attached
- **THEN** the new pane is spawned at the no-client size, not 39×46
