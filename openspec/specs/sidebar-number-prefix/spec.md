# sidebar-number-prefix Specification

## Purpose
Define `ui.workspace_number_prefix` and `ui.agent_number_prefix`: optional
leader glyphs rendered immediately before the right-aligned jump number, in the
number's color, to hint the jump chord. They apply only under
`ui.sidebar_style = "editorial"`, an empty prefix renders the bare number
exactly as before, and both apply on config live-reload.
## Requirements
### Requirement: Optional jump-number prefix labels
The system SHALL provide `ui.workspace_number_prefix` and `ui.agent_number_prefix` string options (default `""`). When `ui.sidebar_style` is `editorial` and a prefix is non-empty, the corresponding list SHALL render the prefix immediately before the right-aligned jump number, in the number's color, as one right-aligned label. Empty prefixes SHALL render the bare number exactly as before. The options SHALL apply on config live-reload.

#### Scenario: Workspace prefix renders before the number
- **WHEN** `ui.sidebar_style = "editorial"` and `ui.workspace_number_prefix = "₽"`
- **THEN** a workspace jumped by `prefix+5` shows `₽5` right-aligned on its name row in the workspace number color

#### Scenario: Agent prefix renders before the number
- **WHEN** `ui.sidebar_style = "editorial"` and `ui.agent_number_prefix = "₽⌥"`
- **THEN** an agent jumped by `prefix+alt+2` shows `₽⌥2` right-aligned on its name row in the agent number color

#### Scenario: Empty prefix is unchanged
- **WHEN** a number prefix is unset or empty
- **THEN** the jump number renders exactly as without this change

#### Scenario: Label width reserved so names do not collide
- **WHEN** a prefixed number renders on a narrow sidebar
- **THEN** the name truncates before the full prefix+number label, which always renders in full, and the active background band shows behind the label without gaps

