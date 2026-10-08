## ADDED Requirements

### Requirement: Motion scheduling arms only while motion is pending
Motion ticks SHALL be scheduled from the motion component's own next-due
time rather than a free-running periodic animation timer. While every
motion-managed list is settled (display order equals target order and no
settle delay is pending), the system SHALL schedule no timer wakeups on
behalf of motion. While motion is pending, the wake cadence SHALL follow
the configured settle/step timing, not a fixed frame interval. This SHALL
hold for attached-client and headless server operation alike, and with
`ui.sort_motion = "instant"` motion SHALL contribute no wakeups at all.

#### Scenario: Idle lists cost nothing
- **WHEN** all motion-managed lists are settled and agents are working
- **THEN** no timer wakeups or renders are scheduled on behalf of motion,
  regardless of agent activity

#### Scenario: Reorder wakes only per motion timing
- **WHEN** a priority change diverges a list and bubble motion is enabled
- **THEN** the system wakes at the motion component's next-due instants
  (settle delay, then per-step spacing) until the list settles, then stops
  scheduling motion wakeups

#### Scenario: Instant mode schedules nothing
- **WHEN** `ui.sort_motion = "instant"`
- **THEN** reorders apply immediately and motion never arms a timer deadline
