## ADDED Requirements

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
