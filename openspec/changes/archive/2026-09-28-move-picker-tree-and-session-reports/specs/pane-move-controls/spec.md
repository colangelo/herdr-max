## MODIFIED Requirements

### Requirement: Move focused pane to another tab via a picker

The TUI SHALL provide an action that opens a modal picker listing the places the
focused pane can be moved to, and on selection moves the pane there via the
existing `pane.move` operation. The picker SHALL reuse the existing
modal/picker UI language rather than introducing a bespoke screen. The action
SHALL be bound by default to `prefix+m` and SHALL be a configurable `KeysConfig`
entry.

The picker SHALL offer, as destinations:

- the other tabs of the pane's own space
- the tabs of every other space
- a new tab, in any listed space
- a new space, created by the move

Destinations SHALL be grouped by space, with the pane's own space first and the
remaining spaces in the order the sidebar lists them. Within a space, tabs SHALL
appear in tab order followed by that space's new-tab destination. The new-space
destination SHALL appear last.

Space headings SHALL NOT be selectable; selection SHALL move between
destinations only.

The picker SHALL list only valid destinations: the pane's current tab SHALL NOT
be a destination, and tabs that cannot receive the pane SHALL be excluded. The
pane's current tab SHALL still be shown in its place, greyed and marked as where
the pane is now, as a row that cannot be selected, submitted or counted as a
destination. The action SHALL NOT open a picker that offers no destination.

Selecting a destination SHALL preserve the pane's running process and terminal
contents, and SHALL focus the moved pane at its destination, switching the active
space when the destination lies in another space.

#### Scenario: Move a pane into a selected tab

- **WHEN** the workspace has at least one other tab and the user triggers the move-to-tab action
- **THEN** a modal picker opens listing the destinations grouped by space
- **AND** selecting a tab moves the focused pane into that tab with a default split next to the target tab's focused pane
- **AND** the pane's running process and terminal contents are preserved

#### Scenario: Move a pane to a tab in another space

- **WHEN** the session has more than one space and the user selects a tab belonging to a space other than the pane's own
- **THEN** the pane is moved into that tab
- **AND** the active space becomes the destination's space with the moved pane focused
- **AND** the source space keeps its remaining panes re-laid out

#### Scenario: Move a pane to a new tab in another space

- **WHEN** the user selects the new-tab destination listed under a space other than the pane's own
- **THEN** a tab is created in that space holding the moved pane
- **AND** the active space becomes that space with the moved pane focused

#### Scenario: Space headings are not destinations

- **WHEN** the picker is open and the user moves the selection through the list
- **THEN** the selection moves from destination to destination
- **AND** no space heading, and not the pane's current tab, can be selected or submitted

#### Scenario: Cancel the picker leaves layout unchanged

- **WHEN** the move-to-tab picker is open and the user dismisses it without selecting
- **THEN** no pane is moved and the layout is unchanged

#### Scenario: No other tabs disables the action

Narrowed by this change: having no other tab no longer disables the action on its
own, because the new-space destination is always offerable. The picker is
suppressed only when the pane has nowhere at all to go.

- **WHEN** the session holds exactly one space with one tab and the focused pane is the only pane in it
- **THEN** the picker does not open
- **AND** the user is shown a non-blocking indication that there is nowhere to move the pane

#### Scenario: A lone tab still opens the picker

- **WHEN** the current space has only one tab and that tab holds more than one pane
- **THEN** the picker opens offering the new-tab and new-space destinations, and any tabs in other spaces
- **AND** it does not report that there is no other tab to move into

#### Scenario: Picker is rejected while the source tab is zoomed

- **WHEN** the focused tab is zoomed and the user triggers the move-to-tab action
- **THEN** the picker does not open and the pane is not moved
- **AND** the user is shown a non-blocking indication that the action is unavailable while zoomed

## ADDED Requirements

### Requirement: The move picker is drawn as a tree

The move-pane picker SHALL draw its destinations as a tree in the session
navigator's visual language: each space heading with its state icon, name and
pane count in the navigator's heading style; its tabs and its new-tab action as
branches under it; a blank line between spaces; and the navigator's full-row
selection. A tab row SHALL set its number and name apart from the word `tab`,
and SHALL show the names of the panes it holds. The new-tab and new-space rows
SHALL be drawn as actions (a `+` mark, dimmer text), not as places. The picker
SHALL name the pane being moved and where it is, and SHALL show, for the
selected row, a one-line detail of what the move would do. Its width SHALL be
measured from these rows, not from the filtered ones.

#### Scenario: Space names stand out

- **WHEN** the picker opens in a session with several spaces
- **THEN** each space heading is drawn in the navigator's heading style with
  its pane count, and its tabs hang under it as branches

#### Scenario: The pane's own tab is shown but not offered

- **WHEN** the picker opens for a pane in tab 1 of the space "herdr"
- **THEN** tab 1 of "herdr" is listed greyed and marked as where the pane is
- **AND** moving the selection never lands on it, and it is not counted in the
  destination count

#### Scenario: A tab shows what is in it

- **WHEN** a tab holds panes named `keyboard-shortcuts` and `install tuicr`
- **THEN** its row shows those names, and the detail line lists every pane in
  it when the row is selected
- **AND** a search for `keyboard` keeps that tab and its space heading

#### Scenario: The tree stays whole while searching

- **WHEN** a query leaves only the second of a space's tabs
- **THEN** that tab is drawn as the space's last branch
