## MODIFIED Requirements

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
