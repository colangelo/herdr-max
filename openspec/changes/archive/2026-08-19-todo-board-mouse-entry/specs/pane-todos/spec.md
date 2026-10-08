# pane-todos

## ADDED Requirements

### Requirement: Tab-bar todo indicator

The TUI tab bar SHALL show a compact todo indicator immediately left of the
notification indicator: the todo glyph, plus the count of outstanding todos
across every pane in the session when that count is nonzero. Clicking the
indicator SHALL toggle the session todo board — the board's first mouse entry.

The indicator SHALL be visible even when nothing is outstanding, as a bare
glyph, exactly as the notification indicator is: the mouse path to the board
must not disappear at the moment it would be used to review or add.

The indicator SHALL take its color from the highest priority among the
outstanding todos it counts, the same rule the per-pane border indicator
already applies, so the two surfaces never disagree about urgency.

The tab bar's two trailing indicators SHALL use the fork's modified-letter
glyph language: `τ` for todos and `и` for notifications. The per-pane border
indicator keeps `▾`; it marks a place on a pane, not an entry point in the
chrome.

#### Scenario: The indicator counts the whole session

- **WHEN** panes across several spaces hold five outstanding todos in total
- **THEN** the tab bar shows the todo glyph with a count of 5
- **AND** its color reflects the highest priority among them

#### Scenario: Clicking toggles the board

- **WHEN** the user clicks the todo indicator
- **THEN** the session todo board opens
- **AND** clicking the indicator again closes it

#### Scenario: Nothing outstanding still shows the entry point

- **WHEN** no pane holds an outstanding todo
- **THEN** the indicator renders as the bare glyph with no count
