# sidebar-sort-motion

## ADDED Requirements

### Requirement: Reusable list-motion primitive with settle and stepped movement
The system SHALL provide a reusable list-motion component that maintains a persisted display order of stable keys and follows a live target order by (a) holding a diverged entry's position for a configurable settle delay, (b) cancelling the pending motion if the entry re-converges before the delay expires, and (c) then moving the entry one position per configurable step interval until it reaches its target slot. The same rules SHALL apply to upward and downward moves. New keys SHALL appear at their target position immediately and removed keys SHALL disappear immediately, without animation.

#### Scenario: Downward bubble after viewing a finished agent
- **WHEN** a done-tier entry's priority drops because the user views its pane
- **THEN** the entry keeps its current position for the settle delay, then moves down one row per step interval until it reaches its sorted slot

#### Scenario: Upward bubble on state change
- **WHEN** an existing entry's priority rises (its agent starts working, finishes, or blocks)
- **THEN** the entry keeps its current position for the settle delay, then moves up one row per step interval until it reaches its sorted slot

#### Scenario: Re-convergence cancels pending motion
- **WHEN** an entry's target position returns to its display position before the settle delay expires
- **THEN** no motion occurs and the settle clock for that entry is cleared

#### Scenario: Insertions and removals are instant
- **WHEN** a new entry joins the list or an existing entry leaves it
- **THEN** the display order reflects the change immediately without settle or stepping

### Requirement: Coherent display order between ticks
The display order SHALL mutate only inside an explicit animation tick driven by the application's scheduled-task path. Rendering, workspace jump numbers, and mouse hit-testing SHALL all derive from the same display order, so the order they observe cannot change between ticks.

#### Scenario: Follow-up click lands on the same row
- **WHEN** the user clicks a sidebar row and clicks the same coordinates again before the settle delay expires
- **THEN** both clicks resolve to the same entry

#### Scenario: Agent-panel click-time sort is stable
- **WHEN** the agents panel resolves a click by recomputing its entries between two animation ticks
- **THEN** the resolved order is identical to the order used by the most recent render

### Requirement: Sidebar lists adopt motion only under priority sort
The spaces list and the agents panel SHALL route their priority-sorted order through the motion component when `ui.sort_motion` is `bubble`. Manual workspace sort and spaces-grouped agent-panel sort SHALL be unaffected, and state icons, colors, and status glyphs SHALL always render from live state regardless of motion.

#### Scenario: Live state, delayed position
- **WHEN** an agent becomes blocked while motion is pending for its row
- **THEN** the row shows the blocked glyph and color immediately while its position changes only per the motion rules

#### Scenario: Manual sort unaffected
- **WHEN** `ui.workspace_sort` is `manual`
- **THEN** workspace rows never move due to motion and drag-reordering behaves as today

### Requirement: Single configuration applied to every consumer
The system SHALL expose `ui.sort_motion` (`"bubble"` default, or `"instant"`), `ui.sort_motion_settle_ms` (default 2000), and `ui.sort_motion_step_ms` (default 150). All motion consumers SHALL read the same options, the options SHALL apply on config live-reload, and `"instant"` SHALL restore pre-change behavior exactly.

#### Scenario: Instant opt-out
- **WHEN** `ui.sort_motion` is set to `instant` and the config is reloaded
- **THEN** priority-sorted lists reorder immediately on state change, matching pre-change behavior

#### Scenario: Tuning applies everywhere
- **WHEN** the user changes `ui.sort_motion_settle_ms` or `ui.sort_motion_step_ms` and reloads config
- **THEN** both the spaces list and the agents panel use the new timing
