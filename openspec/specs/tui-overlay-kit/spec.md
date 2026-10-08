# tui-overlay-kit Specification

## Purpose
Define the shared TUI overlay kit every modal, panel, picker, and menu is
built on, so overlay behaviour has one definition rather than one copy per
surface: the anchored-panel geometry resolver both the renderer and the mouse
hit-test read, the footer button rows derived from one definition, the list
cursor and the list keymap every list-bearing overlay shares, the text field
and its readline editing, the single `Overlay` value that makes two open
overlays unrepresentable and derives input source and help entries from the
variant, and the rendered-layout tests that hold every overlay's appearance.

## Requirements

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

### Requirement: Titled overlays reserve a header block

An overlay that draws a title SHALL reserve a header block of that title plus
one blank row beneath it, defined once in the kit as the mirror of the footer
block, so a title is never drawn flush against the overlay's first content row.

An overlay whose header is more than one line SHALL extend the block by those
lines rather than redefining it, and the extended value SHALL be named once and
read by both that overlay's renderer and its hit-test, so what is drawn and what
is clickable cannot disagree about where the content starts.

An overlay that draws no title reserves no header block.

#### Scenario: A title is not drawn against its content

- **WHEN** a titled overlay is rendered
- **THEN** a blank row separates its title from its first content row

#### Scenario: Render and hit-test agree on where rows begin

- **WHEN** a titled overlay's row is clicked
- **THEN** the row acted on is the row drawn at that cell

#### Scenario: An untitled overlay is unaffected

- **WHEN** an overlay with no title is rendered
- **THEN** its content begins at its first inner row

### Requirement: Overlay lists share one search

Every overlay list that can be searched SHALL use one kit search piece for its
query field, focus, matching, key handling and search row. Matching SHALL
require every whitespace-separated word of the query to appear in a row's search
text, ignoring case. `/` SHALL focus the search from the list. While the search
is focused, typing SHALL edit the query and select the first match, the shared
list chords SHALL still move the selection, Enter SHALL accept the selected row,
and Esc SHALL leave the search and keep the query. From the list, Esc SHALL
clear a set query before it closes the overlay. The session navigator, the
move-pane picker and the todo board SHALL use this piece.

#### Scenario: The same search works in all three dialogs

- **WHEN** the user presses `/` and types `ctx` in the navigator, the move-pane
  picker or the todo board
- **THEN** each shows only the rows whose text contains `ctx`, with the first
  match selected, and draws the search row the same way

#### Scenario: Leaving the search keeps the filter

- **WHEN** a query is typed and Esc is pressed once
- **THEN** the search loses focus and the list stays filtered
- **AND** a second Esc clears the query, and a third closes the overlay

#### Scenario: Moving the navigator onto the piece changes nothing visible

- **WHEN** the navigator is drawn after the extraction
- **THEN** its rendered-layout test passes unedited

### Requirement: Centred list dialogs share one placement and width rule

The session navigator, the move-pane picker and the todo board SHALL be placed
by the kit's panel geometry, centred on the screen, with a width measured from
their own content, bounded below per dialog and above by one shared maximum of
120 columns, and never wider than the screen.

#### Scenario: The navigator in a wide window

- **WHEN** the navigator opens in a 310x56 window
- **THEN** it is at most 120 columns wide and centred

#### Scenario: A small window still fits

- **WHEN** a dialog opens in a window narrower than its measured width
- **THEN** it takes the window's width and stays usable

### Requirement: A list's detail box has one definition

An overlay that shows the full text of its selected row SHALL show it in an inner
bordered box between the list and the footer, drawn by one kit helper. The box
SHALL appear whenever the selected row hides text (more than one line, or a
first line cut to fit), SHALL keep one height while the selection moves within
the visible rows, and SHALL be omitted when fewer than three rows are free.

#### Scenario: The rule is the text, not the place

- **WHEN** the selected todo is a single line that fits its row
- **THEN** no text appears in the detail box, in any space

#### Scenario: A cut line opens the detail

- **WHEN** the selected todo's single line is wider than its row
- **THEN** the box shows the whole text, wrapped

#### Scenario: No room

- **WHEN** fewer than three rows are free between the list and the footer
- **THEN** the detail box is not drawn

### Requirement: A list dialog's status column is measured, not stepped

A centred list dialog with a right-hand status column (the session navigator,
the move-pane picker) SHALL measure that column from its rows, counting the
longest state word a row can show, up to a per-dialog cap, and SHALL draw the
column at the width it measured. When the dialog is narrower than its measured
width, the label column SHALL give way first, down to a floor, before any
status is cut.

#### Scenario: A status fits the box it sized

- **WHEN** the navigator opens in a 310x56 window on rows whose widest status
  is `keyboard-shortcuts · idle`
- **THEN** every status is drawn whole, and the box is no wider than its
  labels, that status column and its border need (never under its footer
  floor, never over 120)

#### Scenario: A state change does not overflow the column

- **WHEN** a pane's status changes from `idle` to `working` while the
  navigator is open
- **THEN** the new status still fits the column, and the box does not change
  width

#### Scenario: A small window cuts labels before statuses

- **WHEN** the navigator opens in a window narrower than its measured width
- **THEN** long labels are cut first, and a status is cut only once the label
  column is at its floor
