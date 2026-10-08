# collapsed-sidebar-parity Specification

## Purpose
Define the parity the collapsed sidebar keeps with the expanded one: the active
agent gets the same active background band and a bold jump symbol, space and
agent rows are labelled with the jump symbol their chord actually resolves
(digits then letters, always one cell so the state icon column cannot shift),
non-active jump symbols honour `ui.workspace_number_color` /
`ui.agent_number_color`, and `ui.sidebar_active_border = "left"`/`"right"`
reserves an edge column and draws the accent bar on the active space and agent
rows.
## Requirements
### Requirement: Collapsed active-agent highlight
The collapsed sidebar SHALL highlight the active agent row with the same active background band (`sidebar_active_band_bg`) used by collapsed space rows, and the active row's jump symbol SHALL render bold in the palette `text` color. Non-active agent rows SHALL keep their current un-banded rendering.

#### Scenario: Active agent is visible
- **WHEN** the sidebar is collapsed and a pane is the active pane
- **THEN** that agent's collapsed row shows the active background band and a bold jump symbol, and no other agent row does

### Requirement: Collapsed jump-symbol labels
Collapsed space and agent rows SHALL be labelled with `jump_symbol` of their visible list position — digits 1-9 then letters a-z, blank beyond — matching what `switch_workspace` / `focus_agent` chords actually resolve. Labels SHALL always occupy one cell so the state icon column never shifts or collides.

#### Scenario: Tenth entry shows a letter
- **WHEN** the sidebar is collapsed and a list has ten or more entries
- **THEN** the tenth row is labelled `a` (not `10`), with the state icon in its usual column separated by a gap

### Requirement: Collapsed number-color parity
Collapsed space and agent rows SHALL colour their non-active, non-selected jump symbols with `ui.workspace_number_color` / `ui.agent_number_color` respectively, falling back to the same palette slot as today when unset.

#### Scenario: Override applies collapsed
- **WHEN** `ui.agent_number_color` is set and the sidebar is collapsed
- **THEN** non-active agent jump symbols render in that color

### Requirement: Collapsed active-border parity
When `ui.sidebar_active_border` is `left` or `right`, the collapsed sidebar SHALL reserve one extra edge column on that side and draw the active-border bar (same symbol/color resolution as expanded: `pane_border_active_style`, `pane_border_active_color`) across the active space row and the active agent row. When `sidebar_active_border` is `off`, `above`, `below`, or `both`, collapsed rendering SHALL not draw a bar and SHALL not reserve the extra column.

#### Scenario: Left bar in collapsed mode
- **WHEN** `ui.sidebar_active_border = "left"` and the sidebar is collapsed
- **THEN** the active space row and active agent row each show the accent bar in the leftmost column, row content shifts right by one cell, and the state icons remain fully visible

#### Scenario: No bar mode keeps current width
- **WHEN** `ui.sidebar_active_border` is `off` (default)
- **THEN** the collapsed sidebar renders at its current width with no bar column

