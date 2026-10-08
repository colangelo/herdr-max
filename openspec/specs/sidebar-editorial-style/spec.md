# sidebar-editorial-style Specification

## Purpose
Define the opt-in `ui.sidebar_style = "editorial"` sidebar preset — jump
numbers right-aligned on the entry's name row, thin uppercase section headers,
dimmed meta lines on non-active entries — with `"default"` leaving rendering
unchanged, plus the `[ui.state_colors]` per-state overrides for the state
glyphs and state text across the expanded sidebar, collapsed sidebar, mobile
switcher, and navigator. Both apply on config live-reload, and the state colors
are independent of the style preset.
## Requirements
### Requirement: Editorial sidebar preset
The system SHALL provide `ui.sidebar_style` with values `default` and `editorial` (default `default`). With `default`, sidebar rendering SHALL be unchanged from pre-change behavior. With `editorial`, in both the spaces list and the agents panel: jump numbers SHALL render right-aligned on the entry's first (name) row instead of leading the second row; section headers SHALL render as thin uppercase (no bold, dimmed); and the meta line of non-active entries SHALL render dimmed while the active entry keeps its accent styling.

#### Scenario: Opt-in only
- **WHEN** `ui.sidebar_style` is unset or `default`
- **THEN** sidebar rendering is identical to pre-change output

#### Scenario: Right-aligned number on the name row
- **WHEN** `ui.sidebar_style = "editorial"` and jump numbers are enabled
- **THEN** each entry's jump symbol renders at the right edge of its name row in the configured number color, and long names truncate before reaching the number

#### Scenario: Editorial headers
- **WHEN** `ui.sidebar_style = "editorial"`
- **THEN** the section headers render as uppercase text without bold, and the agents sort-toggle label remains functional

### Requirement: State color overrides
The system SHALL provide a `[ui.state_colors]` table with optional `working`,
`idle`, `done`, `blocked`, and `unknown` colors. Each set value SHALL recolor
the matching state glyph in upstream v0.8.0's static glyph set (static working
mark, idle-seen circle, done mark, blocked mark, unknown mark — there is no
animated spinner) and the matching state text in both sidebar sections and the
collapsed sidebar; unset values SHALL fall back to the theme palette slots
used by upstream's distinct status indicators. The options SHALL apply on
config live-reload and be independent of `ui.sidebar_style`.

#### Scenario: Override applies everywhere
- **WHEN** `[ui.state_colors] working = "#ffc832"` is set and the config reloads
- **THEN** working glyphs and working state text in the spaces list, agents
  panel, and collapsed sidebar all use `#ffc832`

#### Scenario: Theme fallback
- **WHEN** a `[ui.state_colors]` key is unset
- **THEN** that state renders with the same theme palette color as upstream's
  static status indicators

#### Scenario: Working state renders without animation
- **WHEN** an agent is working and no override recolors the working state
- **THEN** the working glyph is upstream's static mark, does not animate, and
  triggers no recurring renders while the agent stays working

### Requirement: Active-row integrity in editorial mode
In editorial mode the right-aligned number SHALL render over the active entry's background band without gaps, and the active left/right border bar, active background, and bubble motion SHALL behave exactly as in default mode.

#### Scenario: Active band under the number
- **WHEN** the active entry renders in editorial mode with an active background configured
- **THEN** the cells between the truncated name and the right-aligned number show the active background, not the default background

