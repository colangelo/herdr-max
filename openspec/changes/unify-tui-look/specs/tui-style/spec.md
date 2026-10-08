## ADDED Requirements

### Requirement: One frame and title language

A herdr overlay SHALL draw a single-line frame in the palette `accent` on `panel_bg`. Inside it, the
first row SHALL hold a bold `text` title, lowercase, inset one column, with an optional dim count on
the right. The second row MAY hold a dim subtitle. A program drawn inside a herdr popup SHALL draw
no frame of its own. No view SHALL draw a second frame inside the first.

#### Scenario: Asks pane inside a popup

- **WHEN** the asks list is shown in a herdr popup
- **THEN** its first row reads `asks` in bold on the left and `30 open` dim on the right, its
  second row is the dim summary `lab 30 · aruba 0 · owner all · age all`, and it draws no border

#### Scenario: Todos panel preview

- **WHEN** the todos/notes panel shows the selected todo in full
- **THEN** the full text sits below a `surface1` rule with no frame around it

### Requirement: Selection is one accent bar

The selected row of a primary list SHALL be a full-width bar with an `accent` background and bold
`panel_bg` text (256-colour 111 on 234 in curses programs). Reverse video and a pointer glyph such
as `❯` SHALL NOT mark the selection when colours are available. Settings-style lists that pick one
value MAY use a `surface0` bar with bold `text` instead.

#### Scenario: Moving down the asks list

- **WHEN** the cursor moves to the next ask
- **THEN** that whole row is the accent bar and the previous row returns to its normal colours

#### Scenario: Option under the cursor

- **WHEN** the cursor is on an option in the answering view
- **THEN** that option is the accent bar, and the other option labels keep the 256-colour 147
  light blue

#### Scenario: Terminal without colours

- **WHEN** the terminal reports no colours
- **THEN** the selected row falls back to reverse video

### Requirement: Colour carries one meaning each

Green SHALL mean done or saved. Yellow SHALL mean changed, working, or needs a look. Red SHALL mean
urgent, failed, or offline. `accent` SHALL mean the selected thing or a key. `overlay0` SHALL mark
secondary text. A hint that needs no attention, such as a scroll hint, SHALL NOT be yellow. Option
labels and descriptions in the asks pane SHALL stay 256-colour 147.

#### Scenario: A saved answer

- **WHEN** an ask has a saved answer
- **THEN** its `✔` mark is green and its header, title and metadata are dim

#### Scenario: More explanation below

- **WHEN** the explanation in the answering view is longer than its region
- **THEN** the hint `↓ N more lines · pgdn scroll · e expand` is dim

### Requirement: Footer is a key bar or a button row, never both

A browsing or editing view SHALL end with one key bar. The bar SHALL list `key label` pairs inset
one column, each key in bold `accent` and each label dim, with two spaces between pairs. A key that
does nothing in the current state SHALL be drawn all dim or left out. A modal that ends a task SHALL
end with a button row instead: centred chips, the `↵` primary chip in `accent` and the rest in
`surface0`. One row SHALL separate the footer from the body, and a status message MAY use that row.

#### Scenario: Asks list footer

- **WHEN** the asks list is shown
- **THEN** its last row is `enter answer  / text  a owner  g age  h answered  o webpage  q close`,
  with the keys in bold accent and the labels dim

#### Scenario: Todos/notes buttons

- **WHEN** the todos/notes panel is open
- **THEN** `↵ open pane` is the accent primary chip and `spc toggle`, `c clear done`, `esc close`
  are `surface0` chips

### Requirement: A mode keeps its own colour

A mode that has its own colour SHALL keep it. Its bar SHALL show exactly one filled chip, the mode
name, with dark bold text on that colour, and its keys SHALL be bold in the same colour. Every other
label in the bar SHALL be dim text, with no second chip. A mode without a colour of its own, and
everything that is not a mode bar, SHALL use `accent`. A new mode SHALL pick a palette colour that
no other mode uses.

#### Scenario: Today's mode colours

- **WHEN** each mode's bar is drawn
- **THEN** PREFIX, SCROLL, COPY and NAVIGATE use `accent`, PANES uses `red` (`#f38ba8`,
  256-colour 211), RESIZE uses `mauve`, and SYNC uses `SYNC_YELLOW` (`#ffd60a`, 256-colour 220)

#### Scenario: PANES bar

- **WHEN** the display-panes bar is shown
- **THEN** `PANES` is a red chip with dark bold text, `1-3` and `any key` are bold red, and the
  build reads `version 0.8.2-…` with `version` dim and no second chip

#### Scenario: NAVIGATE bar

- **WHEN** navigate mode is on
- **THEN** `NAVIGATE` is an accent chip and its keys are bold accent

#### Scenario: SYNC chip

- **WHEN** a tab is syncing input to two panes
- **THEN** the `SYNC 2 panes` chip keeps its yellow `#ffd60a` and is not recoloured to `accent`

### Requirement: Lost connection is shown in place

A view whose data comes over a connection SHALL show a lost connection as a red label at the start
of the rule under its title. While the connection is down, the keys that would write SHALL be drawn
dim and the status row SHALL say what is read-only.

#### Scenario: Bus down in the asks list

- **WHEN** the asks pane cannot reach the bus
- **THEN** the rule row reads `▲ bus unreachable — read-only, retrying` in red followed by the
  rule, the status row is yellow, and `enter answer` in the key bar is dim

### Requirement: Rules, spacing and truncation

A rule SHALL be `─` in `surface1`, inset one column on both sides. Content SHALL be inset one
column. Long text SHALL be cut with `…`. Metadata (owner, key, age, turn, `#N`) SHALL be dim and
right-hand, and numbers in it SHALL be right-aligned. When a row is too narrow, metadata SHALL be
dropped before the title is cut.

#### Scenario: Narrow asks list

- **WHEN** the asks list is narrower than its columns
- **THEN** the turn, owner and key columns drop in that order, and then the title is cut with `…`

#### Scenario: Empty list

- **WHEN** no ask is open
- **THEN** the body shows one dim line, `no open asks`, inset one column
