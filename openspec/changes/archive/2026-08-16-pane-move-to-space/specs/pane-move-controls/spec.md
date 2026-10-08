# pane-move-controls

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

The picker SHALL list only valid destinations: it SHALL exclude the pane's
current tab and SHALL exclude tabs that cannot receive the pane. The action SHALL
NOT open a picker that offers no destination.

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
- **AND** no space heading can be selected or submitted

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
