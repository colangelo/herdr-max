## ADDED Requirements

### Requirement: Focused main pane and ordered right stacks

The main-vertical preset SHALL preserve focus and pane identity, put the focused pane in the left half, and arrange other panes in current pane_ids order on the right. One through four others SHALL form one equal-height vertical chain. Five or more SHALL form two equal-width columns, with ceil(n/2) panes in the first and the remainder in the second, filled top-down. A single pane SHALL remain unchanged.

#### Scenario: Three panes
- **WHEN** main-vertical is applied with the second pane focused
- **THEN** that pane occupies the left half and the first and third panes stack on the right

#### Scenario: Six panes
- **WHEN** main-vertical is applied with six panes
- **THEN** the five other panes fill a three-pane right column followed by a two-pane right column

### Requirement: Additive endpoint compatibility

layout.set_preset SHALL retain its existing enum and shape. layout.apply_preset SHALL accept all four presets using a separate params type and enum. The client SHALL use the original method for original presets and SHALL cycle even-horizontal, even-vertical, tiled, main-vertical only when the new method is explicitly advertised. Otherwise it SHALL retain the original three-step cycle.

#### Scenario: Older server
- **WHEN** the server does not advertise layout.apply_preset
- **THEN** layout cycling uses exactly the original three presets and methods

#### Scenario: Successful application
- **WHEN** a cycle request succeeds
- **THEN** the client briefly shows the applied layout name using existing feedback

### Requirement: CLI preset names

The CLI SHALL accept --preset main-vertical and main-v, retain --set, and list all four presets for an unknown value.

#### Scenario: Main preset alias
- **WHEN** the user requests main-v
- **THEN** the CLI applies main-vertical through layout.apply_preset
