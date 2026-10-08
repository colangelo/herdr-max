# tui-overlay-kit

## ADDED Requirements

### Requirement: Overlay geometry has one definition

The geometry of an anchored overlay panel SHALL be produced by one shared
resolver taking an anchor, the available screen, a caller-measured content
width, width bounds, a row count and cap, and a footer allowance, and returning
the panel's outer rect, its inner rect, its list rect, and its footer row.

An overlay SHALL NOT compute its own placement, clamping, or inner-rect
arithmetic. What an overlay measures — how wide its own rows are — SHALL remain
the overlay's own.

The resolved geometry SHALL be read by both the renderer and the mouse
hit-test, so what is drawn and what is clickable cannot diverge.

An overlay whose geometry cannot be resolved SHALL render nothing and hit-test
to nothing, so the two go quiet together.

#### Scenario: Two panels share one resolver

- **WHEN** two anchored panels with different anchors and content are laid out
- **THEN** both are placed by the shared resolver and neither carries its own
  placement arithmetic

#### Scenario: What is drawn is what is clickable

- **WHEN** a panel is rendered at a given size
- **THEN** the cells it draws are the cells its hit-test accepts

#### Scenario: A panel that cannot fit goes quiet

- **WHEN** the screen is too small for a panel's minimum geometry
- **THEN** the panel renders nothing and accepts no clicks

### Requirement: Overlay button rows have one definition

An overlay's footer buttons SHALL be described once, as an ordered list of
button, optional key hint, label, and drop priority, from which the rects, the
hit-test, and the row's position are all derived.

When the row is too narrow for every button, buttons SHALL be dropped by
descending drop priority until the row fits; the button that closes or dismisses
the overlay SHALL never be dropped. A click landing on the button row but not on
a button SHALL be inert rather than dismissing the overlay.

#### Scenario: Rects, hit-test, and position agree

- **WHEN** a footer row is laid out
- **THEN** the rect a button is drawn in is the rect its hit-test accepts

#### Scenario: A narrow row drops optional buttons

- **WHEN** the panel is too narrow for every button
- **THEN** the lowest-priority buttons are dropped and the dismiss button remains

#### Scenario: A near-miss does not dismiss

- **WHEN** a click lands on the button row but on no button
- **THEN** nothing happens and the overlay stays open

### Requirement: Overlay lists share one cursor

A list-bearing overlay SHALL hold its selection in the shared list cursor rather
than its own index, and SHALL derive its visible window and its row-to-index
hit-test from that cursor.

The window SHALL keep the selection visible by revealing the nearest edge rather
than recentering. Moving the selection SHALL clamp at both ends rather than wrap.
The row-to-index mapping used by the mouse SHALL be the inverse of the mapping
used to render, from one definition.

#### Scenario: Selection stays visible

- **WHEN** the selection moves past the bottom of the visible window
- **THEN** the window scrolls by the minimum needed to reveal it

#### Scenario: Movement clamps

- **WHEN** the selection is on the last row and is moved down
- **THEN** it stays on the last row

#### Scenario: Clicking selects what was drawn there

- **WHEN** a list row is clicked
- **THEN** the index selected is the index rendered on that row

### Requirement: Overlay lists share one keymap

Every list-bearing overlay SHALL accept the same chords to move its selection:
the arrow keys, the `j` / `k` pair, the `ctrl+j` / `ctrl+k` pair, the
`ctrl+n` / `ctrl+p` pair, half-page movement, and first / last row.

Where an overlay has a focused text input, the chords that are not plain
characters SHALL keep working while that input is focused, and the plain
character chords SHALL be text. No list SHALL require the arrow keys.

Half-page movement is the exception: `ctrl+u` and `ctrl+d` are the shared text
field's kill-to-start and delete-forward, so while a text input has focus they
belong to the field and the list keeps the rest.

#### Scenario: The same chords work in every list

- **WHEN** the move-down chord is pressed in any list-bearing overlay
- **THEN** the selection moves down

#### Scenario: Chords survive a focused search box

- **WHEN** an overlay's search input is focused and a modified move chord is
  pressed
- **THEN** the selection moves and the search text is unchanged
- **WHEN** a plain character chord is pressed instead
- **THEN** it is inserted into the search text
- **WHEN** the half-page chord is pressed instead
- **THEN** the text field takes it, because it is that field's kill

### Requirement: Text inputs share one field

Every overlay text input SHALL be backed by the shared text field, with an
explicit insertion point, the shared editing set, and one definition of word
boundaries.

An overlay SHALL NOT implement its own insertion, deletion, or word-boundary
handling.

#### Scenario: Every input edits the same way

- **WHEN** a readline motion or kill is invoked in any overlay text input
- **THEN** it behaves as it does in every other overlay text input

#### Scenario: One definition of a word

- **WHEN** delete-word-backward is invoked in two different overlays over the
  same text
- **THEN** both delete the same span

### Requirement: An overlay is one value

Overlay state SHALL be held as a single optional value whose variants carry each
overlay's own state, rather than as parallel optional fields paired with a mode
by convention. It SHALL NOT be representable for the active mode to name one
overlay while a different overlay's state is present, nor for two overlays'
states to be present at once.

Behaviour that varies per overlay — whether it wants an ASCII input source,
whether it honours held-key repeats, and which entries it contributes to the
keybinding help panel — SHALL be derived from that value rather than restated as
a separate list per behaviour.

Every overlay SHALL declare what it contributes to the keybinding help panel:
either its entries, or — for a surface reached only by the mouse or by the app
itself — the reason it has no keybinding to document. The declaration SHALL be
exhaustive over the overlays, so an overlay that declares nothing does not
build, and a test SHALL check that what an overlay claims is what the panel
shows.

#### Scenario: Mode and state cannot disagree

- **WHEN** an overlay is open
- **THEN** the active mode and the present overlay state are the same overlay

#### Scenario: Cross-cutting behaviour follows the overlay

- **WHEN** a new overlay is added
- **THEN** its input-source and key-repeat behaviour come from its own definition
  rather than from a separately maintained list

#### Scenario: An overlay that says nothing about the help panel fails the build

- **WHEN** an overlay does not declare what it contributes to the panel
- **THEN** the build fails

#### Scenario: An overlay whose entry is missing from the panel fails the tests

- **WHEN** an overlay declares an entry that the panel does not show
- **THEN** the test suite fails

### Requirement: Overlays are covered by rendered-layout tests

Each overlay SHALL have a test rendering it into a terminal buffer at a fixed
size and asserting its rows, so a change to shared geometry that moves any
overlay is caught by that overlay's own test.

These tests SHALL be written against existing behaviour before shared geometry
is introduced beneath them, so an unintended move is visible as a test diff.

#### Scenario: A geometry change that moves an overlay is caught

- **WHEN** shared geometry changes such that an overlay renders differently
- **THEN** that overlay's rendered-layout test fails

#### Scenario: A refactor that changes nothing passes untouched

- **WHEN** an overlay is moved onto the shared primitives without intending a
  visual change
- **THEN** its rendered-layout test passes without being edited
