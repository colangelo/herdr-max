# pane-todos

## MODIFIED Requirements

### Requirement: Session-wide todo board

The TUI SHALL provide a bindable action that opens a board listing the todos of
every pane in the session, built from the existing overlay language rather than a
bespoke screen. The action SHALL ship unbound and SHALL appear in the keybinding
help panel while unbound, so it is discoverable before it is bound.

The board SHALL group todos by their owning pane, with panes in the order the
session presents them — space, then tab, then pane. Within a pane, todos SHALL
follow the existing presentation order. Panes holding no todos SHALL NOT appear.

A group SHALL be separated from the group above it by one blank row, and a
group's todos SHALL be indented relative to its heading. The blank row SHALL be
a row of the list rather than an offset the renderer adds, so that one list row
answers to exactly one item for scrolling, selection and hit-testing alike. It
SHALL NOT be selectable, SHALL NOT respond to a click, and SHALL NOT appear
above the first group, where the header block already leaves a blank row.

Indentation SHALL be applied by narrowing the row a todo is drawn into, not by a
second row renderer, so a todo reads identically on the board and on the pane
todo panel.

Group headings SHALL identify the owning pane by its space, then the pane's
label — the space first because the board is read across spaces and where the
work lives is what is being decided between. Where a heading lacks a space name
or a label, it SHALL omit that part rather than render an empty one. Headings
SHALL NOT be selectable, and selection SHALL step over them.

Headings SHALL NOT carry the pane's addressable identifier. That identifier is a
creation counter encoded in a 32-character alphabet, so it names neither a
position on the board nor anything else the reader can see, and its space
component states a second time what the heading already says in words. A
heading's job is recognising the pane, which the space and the label do, and
activating a row travels to that pane without anyone having to read an address.
A todo link chip SHALL continue to carry its identifier, because a chip is a
destination the user may want to address rather than a group being read.

The board SHALL be titled `todos/notes`, and the overlay that composes or edits
an entry SHALL name it the same way. What a pane records is a next step as often
as it is a task, and a title naming only one of them tells the user the other
does not belong there.

One requirement owns both titles deliberately, even though the overlay belongs
to the editing surface: they are one decision about what the feature is called,
and stating them apart is how they drift into naming it a note while it is
composed and a todo the moment it is reopened.

The board SHALL support moving the selection, toggling done, opening a todo for
editing, following a todo's link, removing a todo, clearing done todos, and
closing. Where an action exists on the pane todo panel, the board SHALL use the
same key for it.

Clearing done todos SHALL act on every pane the board is showing rather than on
the selected todo's pane alone. The board is the session's view and the action
names no scope, so scoping it to wherever the selection happens to be leaves it
doing nothing, and saying nothing, whenever the selection is not on a pane with
completed todos.

Activating the selected row SHALL move focus to the pane that owns that todo,
through the same focus path used when following a todo's link, and SHALL close
the board. Following a link SHALL continue to target the *linked* pane, so a
linked todo's two destinations stay distinct.

The board SHALL size itself to its content in both directions, clamped by the
screen. Its width SHALL have a floor wide enough to read a heading and a todo
without wrapping the eye back, and a cap short of a very wide terminal, so the
footer buttons stay near the rows they act on.

The board SHALL open even when no pane holds a todo, showing an empty state
rather than refusing to open.

The board SHALL NOT replace the pane todo panel; both SHALL remain available and
SHALL read the same store, so a todo presents identically in either.

#### Scenario: The board lists every pane's todos grouped by pane

- **WHEN** todos exist on panes in more than one space and the user opens the board
- **THEN** every one of those todos is listed
- **AND** they are grouped under their owning pane, with panes in space, tab, then pane order
- **AND** each pane's todos follow the existing presentation order

#### Scenario: Clearing done reaches panes the selection is not on

- **WHEN** completed todos exist under more than one pane and the user clears done todos with the selection on one of them
- **THEN** the completed todos under every pane the board is showing are cleared
- **AND** no outstanding todo is removed

#### Scenario: A heading names the space before the pane

- **WHEN** a group heading is rendered for a pane in a named space
- **THEN** it reads the space name, then the pane's label
- **AND** it shows no addressable identifier

#### Scenario: A heading omits a part the pane does not have

- **WHEN** a pane carries no label of its own
- **THEN** its heading names the space alone rather than rendering an empty part

#### Scenario: The compose and edit overlay is named like the board

- **WHEN** the overlay opens to compose a new entry, or to edit an existing one
- **THEN** its title names both a todo and a note, as the board's does

#### Scenario: Groups are separated and their todos indented

- **WHEN** the board shows todos under more than one pane
- **THEN** a blank row sits between each group and the one above it
- **AND** no blank row sits above the first group
- **AND** each group's todos are indented relative to its heading

#### Scenario: The blank row between groups is inert

- **WHEN** the user moves the selection through the list or clicks a blank row
- **THEN** the selection moves from todo to todo without landing on it
- **AND** the click changes nothing

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
