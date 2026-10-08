# pane-move-controls Specification

## Purpose
Define the keyboard actions that relocate the focused pane — breaking it out to
a new tab, moving it to a chosen tab or space through a picker, quick-moving it
to an adjacent tab, and moving it to a new space — including which destinations
the picker offers and the requirement that every action delegates to the
existing runtime move rather than reimplementing it.

## Requirements

### Requirement: Break focused pane to a new tab

The TUI SHALL provide an action that moves the focused pane out of its current tab into a newly created tab within the same workspace, delegating to the existing `pane.move` operation with a `new_tab` destination. The action SHALL be bound by default to `prefix+!` and SHALL be exposed as a configurable entry in `KeysConfig` so users can rebind or unbind it.

The action SHALL preserve the pane's running process and terminal. After the move, the newly created tab SHALL become the active tab and the moved pane SHALL be focused within it, unless the pane is the only pane in its source tab.

#### Scenario: Break a pane from a multi-pane tab

- **WHEN** the focused tab contains more than one pane and the user triggers the break-pane action
- **THEN** the focused pane is removed from its current tab and placed as the sole pane of a new tab in the same workspace
- **AND** the running process and terminal contents of that pane are preserved
- **AND** the new tab becomes active with the moved pane focused
- **AND** the source tab remains with its remaining panes re-laid out

#### Scenario: Break the only pane in a tab is a no-op

- **WHEN** the focused tab contains exactly one pane and the user triggers the break-pane action
- **THEN** no new tab is created and the pane stays in place
- **AND** the user is shown a non-blocking indication that there is nothing to break out

#### Scenario: Break is rejected while the tab is zoomed

- **WHEN** the focused tab is zoomed and the user triggers the break-pane action
- **THEN** the pane is not moved
- **AND** the user is shown a non-blocking indication that the action is unavailable while zoomed

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

### Requirement: Quick move focused pane to the adjacent tab

The TUI SHALL provide two actions that move the focused pane into the next tab and the previous tab within the current workspace, each using the existing `pane.move` operation with a `tab` destination and a default split, without any intermediate picker. Both actions SHALL be configurable `KeysConfig` entries with default bindings.

Adjacency SHALL follow the workspace's current tab order. When there is no adjacent tab in the requested direction, the action SHALL be a no-op with a non-blocking indication rather than wrapping around or creating a tab.

#### Scenario: Move a pane to the next tab

- **WHEN** a tab exists after the current tab and the user triggers the move-to-next-tab action
- **THEN** the focused pane is moved into that next tab with a default split
- **AND** the pane's running process and terminal contents are preserved

#### Scenario: Move a pane to the previous tab

- **WHEN** a tab exists before the current tab and the user triggers the move-to-previous-tab action
- **THEN** the focused pane is moved into that previous tab with a default split

#### Scenario: No adjacent tab is a no-op

- **WHEN** there is no tab in the requested direction and the user triggers the quick-move action
- **THEN** no pane is moved and no tab is created
- **AND** the user is shown a non-blocking indication that there is no adjacent tab

### Requirement: Pane-move actions delegate to the existing runtime operation

The new pane-move actions SHALL be implemented entirely in the TUI/client input layer and SHALL invoke the existing `pane.move` API path. This change SHALL NOT add a new socket message, SHALL NOT change the `pane.move` request or response schema, and SHALL NOT bump the protocol version.

Failures returned by `pane.move` (for example a rejected move because a source or destination tab is zoomed) SHALL be surfaced to the user as a non-blocking indication, and the layout SHALL remain in its pre-action state.

#### Scenario: No protocol surface is added

- **WHEN** the change is implemented
- **THEN** no new `Method`/`Request` variant is introduced for pane movement
- **AND** the protocol version is unchanged

#### Scenario: A rejected move is surfaced without corrupting layout

- **WHEN** a pane-move action invokes `pane.move` and the operation returns a rejection
- **THEN** the pane remains where it was
- **AND** the user is shown a non-blocking indication describing why the move did not happen

### Requirement: Move focused pane to a new space

The move picker SHALL offer a destination that creates a new space and moves the
focused pane into it, delegating to the existing `pane.move` operation with a
`new_workspace` destination.

The new space SHALL be created with the server's default naming, the same as a
space created by any other route; the move SHALL NOT prompt for a name. The moved
pane SHALL be the sole pane of the new space's first tab, SHALL keep its running
process and terminal contents, and SHALL be focused with the new space active.

This destination SHALL be available whenever the focused pane can be moved,
including when it is the only pane of its tab — moving the last pane of a tab to
a new space SHALL be permitted, and SHALL leave no empty tab behind.

#### Scenario: Move a pane to a new space

- **WHEN** the user selects the new-space destination in the move picker
- **THEN** a new space is created holding the moved pane as the sole pane of its first tab
- **AND** the pane's running process and terminal contents are preserved
- **AND** the new space becomes active with the moved pane focused

#### Scenario: Moving the last pane of a tab to a new space closes the source tab

- **WHEN** the focused pane is the only pane in its tab and the user selects the new-space destination
- **THEN** the pane is moved into the new space
- **AND** the source tab does not remain as an empty tab

#### Scenario: The new space is unnamed and renameable

- **WHEN** a pane has been moved to a new space
- **THEN** that space carries the same default name a space created by any other route would carry
- **AND** it can be renamed afterwards through the existing rename action

### Requirement: The move picker names spaces as the sidebar does

The move-pane picker SHALL label every space heading with the same name the
sidebar shows for that space, resolved from the space's live root-pane working
directory. A pane moved to a new space SHALL seed that space's identity from the
pane's live working directory.

#### Scenario: A space made by a move keeps its own name

- **WHEN** a pane whose live directory is `CONTEXT` (stored directory still
  `master`) is moved from the space "master" to a new space, and the picker is
  opened again from "master"
- **THEN** the picker lists "master" and "CONTEXT", not "master" twice

### Requirement: The move picker can be searched

The move-pane picker SHALL offer the kit's list search. A query SHALL match
space names, tab numbers and labels, and the `new tab` and `new space` rows. A
space heading SHALL stay visible, not selectable, while any of its destinations
match; a matching heading SHALL keep all its destinations.

#### Scenario: Find a space by name

- **WHEN** the user presses `/` in the picker and types part of a space's name
- **THEN** only that space's heading and destinations remain, the first
  destination is selected, and Enter moves the pane there

#### Scenario: No match

- **WHEN** the query matches nothing
- **THEN** the picker says so, keeps the query, and Enter does nothing

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
