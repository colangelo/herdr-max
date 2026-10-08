# pane-todos Specification

## Purpose
Define the per-pane todo list: the store and its limits, display ordering,
persistence with the pane across restart and handoff, links to another pane,
the socket API, event, and CLI that expose the same state to external clients,
and the TUI surfaces — the pane indicator, the panel, and the editor including
its text composition keys.

## Requirements

### Requirement: Pane-scoped todo store

The server SHALL keep an ordered list of todos per pane. Each todo SHALL carry a
per-pane monotonic id, text, a `done` flag, a priority of high, normal, or low,
an optional link to another pane, and created/updated timestamps. The list SHALL
live in server state as pure data, constructible and testable without a PTY.

The server SHALL reject a todo whose text is empty or exceeds 500 characters,
and SHALL reject an addition that would take a pane beyond 50 todos, returning
an explicit error in each case rather than truncating or silently discarding.

Todo ids SHALL NOT be reused within a pane, so a stale id refers to nothing
rather than to a different todo.

#### Scenario: A todo is added to a pane

- **WHEN** a todo is added to a pane with text and no explicit priority
- **THEN** it is appended with a fresh per-pane id, `done` false, and normal
  priority
- **AND** its created and updated timestamps are set

#### Scenario: Text limits are enforced

- **WHEN** a todo is added with empty text
- **THEN** the call fails with `todo_text_empty` and the pane's list is unchanged
- **WHEN** a todo is added with text longer than 500 characters
- **THEN** the call fails with `todo_text_too_long` and the pane's list is
  unchanged

#### Scenario: The per-pane cap is enforced

- **WHEN** a pane already holds 50 todos and another is added
- **THEN** the call fails with `todo_limit_reached` and no todo is evicted

#### Scenario: Ids are not reused

- **WHEN** a todo is removed and a new one is added to the same pane
- **THEN** the new todo receives an id greater than every id previously issued
  for that pane

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

### Requirement: Todos persist with their pane

Todos SHALL be written into the pane's session snapshot alongside its label and
agent name, so they survive server restart and live handoff. The snapshot field
SHALL default to empty when absent, so session files written before this feature
load unchanged, and SHALL be omitted entirely for panes with no todos.

Closing a pane that still has outstanding todos SHALL ask for confirmation
before the pane is destroyed.

#### Scenario: Todos survive a restart

- **WHEN** a pane with todos is saved and the session is restored
- **THEN** the pane's todos are present with their text, done state, priority,
  and ids intact

#### Scenario: Older session files still load

- **WHEN** a session file written without a todos field is restored
- **THEN** restore succeeds and the pane has an empty todo list

#### Scenario: Closing a pane with outstanding todos is confirmed

- **WHEN** a pane with at least one not-done todo is closed
- **THEN** a confirmation is requested before the pane is destroyed
- **WHEN** every todo on the pane is done
- **THEN** the pane closes without additional confirmation

### Requirement: Cross-pane todo links

A todo MAY link to any other pane in the session, including panes in other
workspaces and panes running no agent. The link SHALL store the target's
internal pane identity plus a label captured when the link was created.

The captured label SHALL name the target rather than merely locate it: a
manually labelled pane by its label, an agent pane by its agent, and a pane
running a plain shell by the command it launched. Only a target offering none
of these SHALL fall back to its identifier. Because the label is captured once
and read back long afterwards, it SHALL prefer those stable names over the
pane's live terminal title, which the session navigator leads with but which
drifts underneath a stored link.

Because pane identity is unique across the session while public pane
identifiers are scoped to a workspace, resolving a link target to its public
identifier SHALL locate the target's own workspace rather than assuming the
active one.

On restore, the target SHALL be remapped through the same identifier map that
remaps layout references. A link whose target cannot be resolved SHALL be
preserved as a dead link that retains its label, is presented as inert, and
SHALL NOT resolve to any other pane.

#### Scenario: A link survives restore

- **WHEN** a todo links to another pane and the session is restored
- **THEN** the link resolves to the same pane it referred to before the restart

#### Scenario: A link crosses workspaces

- **WHEN** a todo links to a pane in a different workspace and is saved
- **THEN** the link is stored rather than discarded
- **AND** following it moves focus to that pane in its own workspace
- **AND** it still resolves after the session is restored

#### Scenario: A shell pane is a valid target

- **WHEN** a link targets a pane running no agent
- **THEN** the link is created and its captured label names the launched command
- **WHEN** that same pane also carries a manual label or an agent
- **THEN** those name it instead

#### Scenario: A target with no name keeps its identifier

- **WHEN** a link targets a pane with no manual label, no agent, and no
  launched command on record
- **THEN** its captured label is the pane's own identifier

#### Scenario: A live link is addressed before it is named

- **WHEN** a todo carries a live link to a pane
- **THEN** the link is presented with that pane's public identifier first and its
  captured label after it

#### Scenario: A moved target is addressed by where it is now

- **WHEN** a linked pane's public identifier changes and the todo is presented
  again
- **THEN** the presented identifier is the target's current one

#### Scenario: A link to a closed pane goes dead

- **WHEN** a todo's link target no longer exists
- **THEN** the todo is retained, its link is presented as dead with its captured
  label, and activating it does not change focus

#### Scenario: Ambiguous link targets are rejected

- **WHEN** a link is requested by an agent name matching more than one live agent
- **THEN** the call fails with `todo_link_unresolved` and no todo is created or
  modified

### Requirement: Todo socket API and event

The server SHALL expose `todo.list`, `todo.add`, `todo.update`, `todo.remove`,
and `todo.clear`. `todo.list` SHALL return the todos of one pane when given a
pane, and every pane's todos when not, each entry carrying its owning pane. Each
mutating call SHALL emit a `todo.changed` event carrying the affected pane on the
existing subscription stream, so external consumers can follow the same state.

#### Scenario: Listing every pane's todos

- **WHEN** `todo.list` is called without a pane
- **THEN** todos from all panes are returned, each identifying its owning pane

#### Scenario: Mutations are evented

- **WHEN** a todo is added, updated, removed, or cleared
- **THEN** a `todo.changed` event naming the affected pane is emitted to
  subscribers

#### Scenario: Unknown targets are reported

- **WHEN** a call names a pane that does not exist
- **THEN** it fails with `pane_not_found`
- **WHEN** a call names a todo id that does not exist on the pane
- **THEN** it fails with `todo_not_found`

### Requirement: Todo CLI

The CLI SHALL provide `herdr todo` verbs to add, list, complete, reopen, edit,
remove, and clear todos. The target pane SHALL be resolved from an explicit pane
argument, a current-pane flag, or the calling pane's environment, matching the
resolution the existing pane verbs use. With no target given, commands SHALL act
on the calling pane, so an agent can record its own next steps without knowing
its pane id.

#### Scenario: An agent records its own next steps

- **WHEN** a process running inside a pane runs the add verb with only text
- **THEN** the todo is created on that pane

#### Scenario: Another pane is targeted explicitly

- **WHEN** the add verb is given an explicit pane target
- **THEN** the todo is created on that pane rather than the calling one

#### Scenario: Machine-readable output

- **WHEN** the list verb is run with the JSON flag
- **THEN** it prints the todos as JSON including ids, done state, priority, and
  link target

### Requirement: Pane todo panel and editing

Activating the indicator, by click or by a bindable action, SHALL open a panel
anchored to that pane listing its todos in presentation order, following the
existing overlay language. The panel SHALL support moving the selection,
toggling done, removing a todo, clearing done todos, opening a todo for editing,
adding a new todo, following a todo's link, and closing.

The panel SHALL offer the add action even when the pane holds no todos, so
opening a quiet pane's panel is never a dead end.

Opening a todo for editing SHALL present a modal built from the existing dialog
structure allowing its text, priority, link, and done state to be changed, with
explicit save and cancel. Because the modal's text field owns the panel's
done-toggle key, the modal SHALL offer the toggle under a distinct binding. A
todo being composed SHALL NOT offer the done toggle, since it cannot be already
done.

Choosing a link SHALL present the session navigator in a selection mode rather
than cycling through candidates, so the target can be searched and filtered.
Rows that are not panes SHALL be context only and SHALL NOT be selectable as a
target, the todo's own pane SHALL NOT be offered, and the selection SHALL
include an explicit entry that clears the link. Leaving the selection without
choosing SHALL leave the link as it was.

Following a link SHALL move focus to the linked pane through the same focus path
used when jumping to a notification's pane.

Every action introduced SHALL appear in the keybinding help panel, including any
left unbound by default.

#### Scenario: Opening the panel

- **WHEN** the indicator is clicked or the open action is invoked on a pane with
  todos
- **THEN** a panel anchored to that pane lists its todos in presentation order

#### Scenario: Adding a todo from the panel

- **WHEN** the add action is invoked from the panel
- **THEN** the edit modal opens on a new todo for that pane
- **AND** saving returns to the panel with the new todo listed

#### Scenario: An empty panel can still add

- **WHEN** the panel is opened on a pane with no todos
- **THEN** the add action is offered

#### Scenario: A multi-line todo occupies one panel row

- **WHEN** the panel lists a todo whose text holds more than one line
- **THEN** it occupies a single row showing the first line with a marker

#### Scenario: Editing a todo

- **WHEN** a todo is opened for editing, its text changed, and the change saved
- **THEN** the todo's text and updated timestamp change while its id, done state,
  and creation timestamp are preserved

#### Scenario: Commit and done-toggle stay clear of the editing set

- **WHEN** a key belonging to the text field's editing set is pressed in the edit
  modal
- **THEN** it edits the text and neither commits the edit nor toggles done

#### Scenario: Toggling done from the edit modal

- **WHEN** a todo is opened for editing and the done toggle is activated and the
  change saved
- **THEN** the todo's done state flips while its text, id, and creation timestamp
  are preserved
- **WHEN** the edit is cancelled instead
- **THEN** the todo's done state is unchanged

#### Scenario: Choosing a link target

- **WHEN** the link control is activated in the edit modal
- **THEN** the navigator opens in selection mode listing panes across every
  workspace
- **WHEN** a pane row is chosen
- **THEN** the edit returns with that pane staged as the link target

#### Scenario: Pane rows carry the identifier they stage

- **WHEN** the picker lists a pane row
- **THEN** the row shows that pane's public identifier as well as its name

#### Scenario: The picker moves without arrow keys while searching

- **WHEN** the picker's search is focused and the move-down chord is pressed
- **THEN** the selection moves down and the search text is unchanged
- **WHEN** the same chord is pressed with the search not focused
- **THEN** the selection moves down as well

#### Scenario: Non-pane rows are not targets

- **WHEN** a workspace row is activated in selection mode
- **THEN** it expands or collapses, no link target is chosen, and the
  selection stays open
- **WHEN** a tab row is activated instead
- **THEN** nothing happens, since only workspaces carry expansion state

#### Scenario: Leaving selection keeps the previous link

- **WHEN** selection mode is dismissed without choosing
- **THEN** the todo's link is whatever it was before the selection opened

#### Scenario: Following a link

- **WHEN** a todo with a live link is followed
- **THEN** focus moves to the linked pane

#### Scenario: Actions are discoverable

- **WHEN** the keybinding help panel is shown
- **THEN** every action introduced by this feature is listed, showing `unset` for
  any that has no binding

### Requirement: Todo text composition

Composing a todo's text SHALL use a field with an explicit insertion point.
Typed and pasted text SHALL be inserted at the insertion point rather than
appended, and deletions SHALL act relative to it.

The field SHALL support, at minimum: moving the insertion point by character and
by word in both directions and to the start and end of the text; deleting the
character before and the character after it; killing to the start and to the end
of the text; killing the word before it; yanking the most recent kill back at the
insertion point; and undoing recent edits. Motions and kills SHALL be reachable
without the arrow keys, using the conventional readline chords, while the arrow
keys SHALL continue to work.

Undo SHALL be bound to a chord that a terminal without the enhanced keyboard
protocol can still deliver, and MAY additionally accept an enhanced-protocol-only
chord. No editing capability SHALL be reachable *only* through a chord that
requires the enhanced keyboard protocol.

Todo text MAY contain newlines. Where the field accepts newlines, the key that
inserts a newline SHALL NOT also commit the edit: committing SHALL have its own
distinct key, and cancelling SHALL remain unchanged. Pasted text SHALL retain
its newlines and SHALL continue to drop other control characters.

The field SHALL enforce the store's text limit as text is composed, so the modal
cannot compose a todo the server will reject.

#### Scenario: Text is inserted at the insertion point

- **WHEN** the insertion point is moved to the start of the text and a character
  is typed
- **THEN** the character appears at the start and the rest of the text is
  preserved

#### Scenario: Readline motions and kills

- **WHEN** the insertion point is moved to the end of the text and kill-to-start
  is invoked
- **THEN** the text is emptied
- **WHEN** yank is invoked
- **THEN** the killed text is restored at the insertion point

#### Scenario: Deleting forward and backward

- **WHEN** the insertion point sits between two characters and delete-forward is
  invoked
- **THEN** the character after it is removed and the insertion point does not
  move
- **WHEN** delete-backward is invoked instead
- **THEN** the character before it is removed and the insertion point moves back

#### Scenario: Undo restores the previous text

- **WHEN** text is typed and then killed, and undo is invoked
- **THEN** the text before the kill is restored

#### Scenario: Undo does not require the enhanced keyboard protocol

- **WHEN** the terminal reports keys without the enhanced keyboard protocol
- **THEN** undo is still reachable by a chord that terminal can deliver

#### Scenario: A todo can hold more than one line

- **WHEN** the newline key is pressed while composing and text is typed after it
- **THEN** the todo's text holds both lines
- **AND** the edit is not committed by that keypress
- **WHEN** the commit key is pressed
- **THEN** the todo is saved with both lines and survives a session restore

#### Scenario: Paste keeps newlines and drops other control characters

- **WHEN** text containing a newline and an escape character is pasted into the
  field
- **THEN** the newline is kept and the escape character is dropped

#### Scenario: The store's limit is enforced while composing

- **WHEN** the field already holds the maximum number of characters
- **THEN** further typed characters are ignored rather than composing a todo the
  store would reject

### Requirement: Always-on pane todo indicator

Every pane that draws a top border SHALL show a todo indicator at the far right
of it, whether or not the pane holds todos, so the affordance is in the same
place on every such pane and a pane with no todos can still be opened by mouse.
A pane that draws no top border SHALL show no indicator, since there is no
chrome to carry it; the bindable action remains the path there.

The indicator SHALL distinguish three states: a pane with outstanding todos
carries their count, a pane whose todos are all done carries the glyph without a
count, and a pane with no todos carries the glyph rendered in the dimmest tone
so it reads as an empty affordance rather than as completed work.

The indicator SHALL be colored by the highest outstanding priority when
outstanding todos exist, and SHALL be configurable off, suppressing all three
states. The region used to draw the indicator and the region that accepts a
click SHALL be derived from a single shared definition. When pane width forces a
choice the indicator SHALL win: it is laid out first and the title takes what is
left, dropping itself when that is too narrow. The indicator SHALL be omitted
only when the pane cannot carry the glyph and its enclosing border corners at
all.

#### Scenario: Outstanding count is shown

- **WHEN** a pane holds three not-done todos and one done todo
- **THEN** its indicator shows a count of three

#### Scenario: An empty pane still offers the affordance

- **WHEN** a pane with a top border has no todos
- **THEN** the indicator is drawn without a count in the dimmest tone
- **AND** activating it opens that pane's todo panel

#### Scenario: A pane without a top border carries no indicator

- **WHEN** a pane draws no top border
- **THEN** no indicator is drawn for it regardless of its todos

#### Scenario: The indicator outlives the title in a squeeze

- **WHEN** a pane is too narrow to show both the indicator and its title
- **THEN** the indicator is drawn and the title gives up the space
- **WHEN** the pane cannot carry the glyph and its border corners at all
- **THEN** the indicator is omitted

#### Scenario: Empty is distinguishable from all-done

- **WHEN** one pane has no todos and another pane's todos are all done
- **THEN** both show the glyph without a count, rendered in different tones

#### Scenario: The indicator can be turned off

- **WHEN** the pane todo indicator is configured off
- **THEN** no indicator is drawn for any pane, including panes holding todos

#### Scenario: Click target matches what is drawn

- **WHEN** the indicator is drawn for a pane
- **THEN** the cells that respond to a click are exactly the cells drawn

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

The board's selection, the pointer and the visible window SHALL be three
independent things. Pointer motion SHALL NOT move the selection. The scroll
wheel SHALL move the visible window without moving the selection. A click on a
row that is not selected SHALL select it and do nothing else; a click on the
already-selected row SHALL activate it. A click on a todo's link chip SHALL
follow the link, selected or not, because a chip is an explicit target rather
than a row.

Scrolling driven by the selection SHALL keep the selected todo's group heading
visible whenever the heading and the selection both fit in the window. A
selected row whose heading has scrolled away does not say which pane it belongs
to, which is the one thing the board exists to answer.

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

#### Scenario: A click selects before it acts

- **WHEN** the user clicks a row that is not the selected one
- **THEN** that row becomes selected and the board stays open
- **WHEN** the user clicks the already-selected row
- **THEN** focus moves to the pane that owns it and the board closes

#### Scenario: The pointer and the wheel leave the selection alone

- **WHEN** the pointer moves across the board's rows
- **THEN** the selection does not follow it
- **WHEN** the user scrolls the wheel
- **THEN** the visible window moves and the selection stays where it was

#### Scenario: The first group's heading can be scrolled back to

- **WHEN** the list has been scrolled down and the selection is moved back to the first todo
- **THEN** the heading above it is visible

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

### Requirement: Tab-bar todo indicator

The TUI tab bar SHALL show a compact todo indicator immediately left of the
notification indicator: the todo glyph, plus the count of outstanding todos
across every pane in the session when that count is nonzero. Clicking the
indicator SHALL toggle the session todo board — the board's first mouse entry.

The indicator SHALL be visible even when nothing is outstanding, as a bare
glyph, exactly as the notification indicator is: the mouse path to the board
must not disappear at the moment it would be used to review or add.

The indicator SHALL take its color from the highest priority among the
outstanding todos it counts, the same rule the per-pane border indicator
already applies, so the two surfaces never disagree about urgency.

The tab bar's two trailing indicators SHALL use the fork's modified-letter
glyph language: `τ` for todos and `и` for notifications. The per-pane border
indicator keeps `▾`; it marks a place on a pane, not an entry point in the
chrome.

#### Scenario: The indicator counts the whole session

- **WHEN** panes across several spaces hold five outstanding todos in total
- **THEN** the tab bar shows the todo glyph with a count of 5
- **AND** its color reflects the highest priority among them

#### Scenario: Clicking toggles the board

- **WHEN** the user clicks the todo indicator
- **THEN** the session todo board opens
- **AND** clicking the indicator again closes it

#### Scenario: Nothing outstanding still shows the entry point

- **WHEN** no pane holds an outstanding todo
- **THEN** the indicator renders as the bare glyph with no count

### Requirement: Todo editor readability

The todo compose/edit overlay SHALL be sized for reading and writing a full
todo: wide enough that prose does not immediately leave the visible area, and
with a text block tall enough that a todo near the length cap is mostly visible
at once, clamped by the screen.

The text block SHALL soft-wrap its content at word boundaries to the block's
width instead of scrolling horizontally. Explicit newlines SHALL be preserved
as hard breaks. A word longer than the block's width SHALL break mid-word
rather than disappear off the edge. The caret SHALL remain visible through
wrapping: the block scrolls vertically, in wrapped visual rows, by the least
amount that keeps the caret's row on screen.

A mouse click on the text SHALL place the caret at the clicked character,
resolved through the same wrap layout the renderer used, so the caret lands
where the pointer is.

Wrapping SHALL be presentation only: the stored todo text is unchanged, and no
soft break introduces a character into it.

#### Scenario: Long prose wraps instead of escaping sideways

- **WHEN** the todo's text is wider than the text block
- **THEN** it wraps at word boundaries onto following rows
- **AND** no horizontal scrolling occurs and no text is cut off at the right edge

#### Scenario: Hard newlines survive wrapping

- **WHEN** the todo's text contains explicit newlines
- **THEN** each newline starts a new row exactly as typed
- **AND** saving returns the text with only the author's own newlines in it

#### Scenario: The caret stays visible while typing past a wrap

- **WHEN** typing carries the caret past the block's width or below its last row
- **THEN** the caret continues on the next wrapped row, scrolling the block vertically when needed

#### Scenario: Clicking wrapped text places the caret at the clicked character

- **WHEN** the user clicks a character on any wrapped row
- **THEN** the caret moves to that character in the underlying text

### Requirement: Visible todo identity

Every rendered todo row — on the pane todo panel and on the session board —
SHALL show the todo's id, dim and right-aligned as `#<id>`, so a todo can be
named in conversation: by the user to an agent, and by one agent to another.
The id SHALL be the same id the CLI and the socket API already use, so what is
read on screen is the address `herdr todo done/edit/rm` accepts.

The edit overlay SHALL show the id of the todo being edited in its title.
Composing a new todo SHALL show no id, because none exists until it is saved.

A todo's full address across panes is its owning pane plus its id. The board's
group heading names the pane; the row's id completes the address.

#### Scenario: A row shows the id the CLI accepts

- **WHEN** a todo with id 12 is rendered on the panel or the board
- **THEN** the row shows `#12`, dim, at its right edge
- **AND** `herdr todo done 12` (with that pane) acts on exactly that todo

#### Scenario: The editor names what it is editing

- **WHEN** the edit overlay opens on todo 12
- **THEN** its title reads `edit todo/note #12`
- **AND** composing a new todo shows the plain `new todo/note` title

#### Scenario: The id survives beside a link chip

- **WHEN** a todo carries a link chip and an id
- **THEN** the id sits at the row's right edge with the chip beside it
- **AND** the todo's own text truncates before either is lost

### Requirement: The selected todo's full text has its own box

The pane todo panel and the todo board SHALL show the selected todo's full text
in the kit's detail box whenever its row hides text, in every space, and SHALL
keep the panel's height steady while the selection moves among the visible
todos.

#### Scenario: Same todos, same detail, any space

- **WHEN** two panes in different spaces hold the same todos and the same todo
  is selected in each
- **THEN** both panels show the same detail box

#### Scenario: The panel does not jump

- **WHEN** the selection moves from a multi-line todo to a one-line todo
- **THEN** the panel keeps its height

### Requirement: The todo board can be searched

The session todo board SHALL offer the kit's list search. A query SHALL match
todo text, `#id` and the pane heading. A pane heading SHALL stay while any of
its todos match, and a matching heading SHALL keep all its todos. The board's
letter commands SHALL act only while the list, not the search, has focus.

#### Scenario: Filter the board

- **WHEN** the user presses `/` on the board and types a word from one todo
- **THEN** only that todo and its pane heading remain, and Enter opens the
  owning pane

#### Scenario: Letters type while searching

- **WHEN** the search has focus and the user types `d`
- **THEN** `d` goes into the query and no todo is removed
