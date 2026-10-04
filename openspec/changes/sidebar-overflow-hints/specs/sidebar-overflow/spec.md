## ADDED Requirements

### Requirement: Edge rows say what is hidden

With `sidebar_overflow` set to `rows` or `both`, the spaces list and the agent
panel SHALL show a dim summary row at an edge that has entries hidden past it.
The top row SHALL read `↑ N pinned · M more`, with one `↑` per hidden pin
(at most four) in that pin's marker colour; `N pinned` is omitted when none are
pinned and `M more` when all hidden entries are pinned. The bottom row SHALL read
`↓ N more`. When a hidden entry is blocked or finished and unseen, the row SHALL
end with `● K state`, the dot in the colour of the most urgent such state. A side
with nothing hidden SHALL have no row and SHALL take no space.

#### Scenario: Pins above

- **WHEN** two pinned spaces and three others are scrolled out of the top
- **THEN** the first row of the list reads `↑↑ 2 pinned · 3 more`

#### Scenario: A blocked agent below

- **WHEN** five agents are hidden below and one is blocked
- **THEN** the last row reads `↓ 5 more · ● 1 blocked`, the dot in the blocked colour

#### Scenario: Everything fits

- **WHEN** the list fits its body
- **THEN** there is no edge row and the first entry is on the first row

### Requirement: An edge row scrolls a page

A click on an edge row SHALL scroll its list one page toward that edge and SHALL
NOT focus or select anything.

#### Scenario: Click below

- **WHEN** the bottom row of the spaces list is clicked
- **THEN** the list scrolls down by the number of entries it shows

### Requirement: Fog lifts the background next to a hidden edge

With `sidebar_overflow` set to `fog` or `both`, the two visible entries nearest
an edge with hidden entries SHALL get a lighter background, the nearest by
about 9% and the next by about 4% from the sidebar background toward the text
colour. Text colours SHALL NOT change. When a hidden entry is blocked or
finished and unseen, the lift SHALL take a faint tint of that state's colour.
The active, selected and dragged entries SHALL NOT be fogged.

#### Scenario: Plain fog

- **WHEN** a list is scrolled to the middle on a `#0a0a0a` background with `#d2d2d2` text
- **THEN** the entries next to both edges have backgrounds `#1c1c1c` and `#121212`

#### Scenario: Selection wins

- **WHEN** the active entry is the one nearest the top edge
- **THEN** it keeps the active-row background

### Requirement: One setting, live

`ui.sidebar_overflow` SHALL accept `both` (the default), `rows`, `fog` and `off`,
SHALL apply to both lists, SHALL be read at startup and on `reload-config`, and
SHALL have an entry in the `--default-config` template.

#### Scenario: Off

- **WHEN** the setting is `off`
- **THEN** both lists lay out as before: no edge rows, no fog
