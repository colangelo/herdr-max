# sidebar-sort-motion

## ADDED Requirements

### Requirement: Configurable step easing
The system SHALL provide `ui.sort_motion_easing` with values `linear` and `bubble` (default `linear`). With `linear`, every motion step SHALL be spaced by `ui.sort_motion_step_ms`. With `bubble`, the spacing SHALL vary across a reshuffle — slowest as the burst begins and ends, quickest mid-burst — bounded to a fixed range around the configured step. The option SHALL apply on config live-reload and to every motion consumer alike.

#### Scenario: Linear keeps a constant cadence
- **WHEN** `ui.sort_motion_easing` is `linear`
- **THEN** every gap between motion steps equals `ui.sort_motion_step_ms`, whatever the length of the reshuffle

#### Scenario: Bubble accelerates and decelerates
- **WHEN** `ui.sort_motion_easing` is `bubble` and a reshuffle spans several steps
- **THEN** the first and last gaps are longer than the quickest mid-burst gap

#### Scenario: Cadence stays within bounds
- **WHEN** any burst progress is reached, including its first and final step
- **THEN** the computed gap stays within the easing curve's configured multiples of the reference step, with no rounding excursion beyond them

#### Scenario: Long travels visibly hesitate
- **WHEN** a row must travel many positions
- **THEN** the gaps near the ends of the burst exceed the reference step, so the motion reads as hesitating before accelerating
