## ADDED Requirements

### Requirement: Every herdr turn moves core's turn

With `[experimental] core_status_mirror = true`, when herdr offered a `working` record for a core
pane that was never written, the next record written for that pane SHALL be preceded by a `working`
record in the same write. A record equal to the last one written SHALL still be written while such a
`working` is owed.

#### Scenario: a short turn while the pane prints

- **WHEN** the last record written for a core pane is `idle`, and herdr offers `working` and then
  `done` before the pane's output has been quiet for 100 ms
- **THEN** the mirror writes `working` and then `done` in one write, and core's `turn` for that
  terminal rises by exactly one

#### Scenario: a second unread turn

- **WHEN** the last record written is `done`, and herdr offers `working` and then `done` before the
  gate opens
- **THEN** the mirror writes `working` and then `done`, and core's `turn` rises by one

#### Scenario: no turn to owe

- **WHEN** the last record written is `idle` and herdr offers `done` with no `working` in between
- **THEN** the mirror writes `done` alone

### Requirement: Read state reaches core's status

With the mirror on, when herdr marks a core pane seen or unseen, the server SHALL offer that pane's
current record again, so a `done` pane that ac reads turns `idle` in core without an agent state
change.

#### Scenario: ac reads a done pane in herdr

- **WHEN** a core pane is `done` in herdr and core, and ac shows its tab in a focused client
- **THEN** core's status for that terminal becomes `idle` within the mirror's gate
