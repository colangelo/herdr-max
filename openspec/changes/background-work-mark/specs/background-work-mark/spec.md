## ADDED Requirements

### Requirement: The background mark style is configurable

`[ui] background_mark` SHALL select how an agent row that is Working only
because of background work is drawn. The values are `frames` (default) and
`braille`. An unknown value is a config error, as for every other `[ui]` enum. A config reload SHALL apply a change without a restart.

#### Scenario: Default is unchanged

- **WHEN** `background_mark` is unset
- **THEN** the row alternates `state_symbols.background` and `background_alt` exactly as before

#### Scenario: Unknown value

- **WHEN** `background_mark = "dots"`
- **THEN** the config is rejected with a parse error and the previous config stays

### Requirement: Braille mode counts background items

In `braille` mode the row icon SHALL alternate, on the existing spinner phase,
between the background dot (`state_symbols.background`, default `·`) and one
braille cell with as many dots as there are background items: `⠁ ⠃ ⠇ ⡇ ⡏ ⡟ ⡿ ⣿`
for 1 to 8, and `⣿` for more than 8. The icon SHALL stay one terminal cell wide
and use `state_colors.background`.

#### Scenario: Three subagents

- **WHEN** the screen reads "Waiting for 3 background agents to finish"
- **THEN** the braille phase shows `⠇`

#### Scenario: Background work with no number

- **WHEN** a background rule matches and the line carries no count
- **THEN** the braille phase shows `⠁`

#### Scenario: Nine or more

- **WHEN** the count is 9
- **THEN** the braille phase shows `⣿`

### Requirement: Speed follows the spinner

The alternation SHALL use the existing phase `(spinner_frame / 8) % 2` and so
follow `ui.status_spinner_ms`. With `status_spinner = "off"` the row SHALL show
the static background glyph and no braille cell.

#### Scenario: Spinner off

- **WHEN** `status_spinner = "off"` and `background_mark = "braille"`
- **THEN** the row shows only the background dot

### Requirement: The count travels as an optional fact

The server SHALL publish `background_count` per pane in the optional JSON
resource facts, only for panes that are Working on background work. A client
that receives `background_activity` without `background_count` SHALL treat the
count as 1.

#### Scenario: Old server

- **WHEN** the facts carry `background_activity: true` and no `background_count`
- **THEN** braille mode shows `⠁`

### Requirement: Background work does not change the status

A Claude pane waiting at its prompt SHALL read idle (or done) while background shells, monitors or
agents run, exactly as upstream; the screen rules SHALL NOT report it as working because of the
footer's shell or monitor count. The count SHALL still be recorded for the mark.

#### Scenario: Idle prompt with shells

- **WHEN** the screen shows an empty prompt box and the footer `· 2 shells · ← 1 agent`
- **THEN** the pane is idle and its background count is 2

#### Scenario: Idle row in braille mode

- **WHEN** `background_mark = "braille"` and an idle row has a count of 3
- **THEN** the row icon alternates the dot and `⠇` in the background colour
