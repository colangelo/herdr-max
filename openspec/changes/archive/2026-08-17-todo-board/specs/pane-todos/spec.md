# pane-todos

## ADDED Requirements

### Requirement: Session-wide todo board

The TUI SHALL provide a bindable action that opens a board listing the todos of
every pane in the session, built from the existing overlay language rather than a
bespoke screen. The action SHALL ship unbound and SHALL appear in the keybinding
help panel while unbound, so it is discoverable before it is bound.

The board SHALL group todos by their owning pane, with panes in the order the
session presents them — space, then tab, then pane. Within a pane, todos SHALL
follow the existing presentation order. Panes holding no todos SHALL NOT appear.

Group headings SHALL identify the owning pane by its addressable id followed by
its label, SHALL NOT be selectable, and selection SHALL step over them.

The board SHALL support moving the selection, toggling done, opening a todo for
editing, following a todo's link, removing a todo, clearing done todos, and
closing. Where an action exists on the pane todo panel, the board SHALL use the
same key for it.

Activating the selected row SHALL move focus to the pane that owns that todo,
through the same focus path used when following a todo's link, and SHALL close
the board. Following a link SHALL continue to target the *linked* pane, so a
linked todo's two destinations stay distinct.

The board SHALL open even when no pane holds a todo, showing an empty state
rather than refusing to open.

The board SHALL NOT replace the pane todo panel; both SHALL remain available and
SHALL read the same store, so a todo presents identically in either.

#### Scenario: The board lists every pane's todos grouped by pane

- **WHEN** todos exist on panes in more than one space and the user opens the board
- **THEN** every one of those todos is listed
- **AND** they are grouped under their owning pane, with panes in space, tab, then pane order
- **AND** each pane's todos follow the existing presentation order

#### Scenario: Panes without todos are omitted

- **WHEN** the session holds panes with no todos and the user opens the board
- **THEN** those panes contribute no heading and no rows

#### Scenario: Headings are not selectable

- **WHEN** the board is open and the user moves the selection through the list
- **THEN** the selection moves from todo to todo
- **AND** no group heading can be selected or activated

#### Scenario: Activating a row focuses the owning pane

- **WHEN** the user activates the selected todo
- **THEN** focus moves to the pane that owns it, switching space and tab as needed
- **AND** the board closes

#### Scenario: Following a link targets the linked pane, not the owner

- **WHEN** the selected todo carries a live link and the user follows it
- **THEN** focus moves to the linked pane rather than to the todo's owning pane

#### Scenario: The board opens with nothing to show

- **WHEN** no pane in the session holds a todo and the user opens the board
- **THEN** the board opens and shows an empty state

#### Scenario: Editing from the board writes through to the pane

- **WHEN** a todo is edited or toggled from the board
- **THEN** the change is stored against its owning pane
- **AND** the pane's own todo panel shows the same state

## MODIFIED Requirements

### Requirement: Todo display ordering

Stored order SHALL be insertion order. Presentation order SHALL be not-done
before done, then priority descending, then creation order. Changing a todo's
priority SHALL NOT change its id or its stored position.

Presentation order is defined within a pane. Where todos from more than one pane
are shown together, they SHALL be grouped by owning pane and ordered within each
group by that same presentation order; they SHALL NOT be interleaved into a
single priority-ordered list, so a pane's todos stay contiguous.

#### Scenario: Priority orders the list

- **WHEN** a pane holds a normal todo added first and a high todo added second
- **THEN** the high todo is presented first

#### Scenario: Done todos sink

- **WHEN** a high-priority todo is marked done and a normal-priority todo is not
- **THEN** the not-done normal todo is presented before the done high one

#### Scenario: Todos from different panes are not interleaved

- **WHEN** a low-priority todo on one pane and a high-priority todo on another are shown together
- **THEN** each appears under its own pane's group
- **AND** the high-priority todo does not move ahead of the other pane's group
