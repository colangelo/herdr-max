# pane-todos

## ADDED Requirements

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

## MODIFIED Requirements

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

A live link SHALL be presented leading with the target's public pane identifier,
followed by the captured label, so the link both addresses and names its target.
The identifier SHALL be derived from the live target at presentation time rather
than stored, so it is either correct or absent. A dead link, which resolves to no
identifier, SHALL be presented with its captured label alone.

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
  label and no identifier, and activating it does not change focus

#### Scenario: Ambiguous link targets are rejected

- **WHEN** a link is requested by an agent name matching more than one live agent
- **THEN** the call fails with `todo_link_unresolved` and no todo is created or
  modified

### Requirement: Pane todo panel and editing

Activating the indicator, by click or by a bindable action, SHALL open a panel
anchored to that pane listing its todos in presentation order, following the
existing overlay language. The panel SHALL support moving the selection,
toggling done, removing a todo, clearing done todos, opening a todo for editing,
adding a new todo, following a todo's link, and closing.

The panel SHALL offer the add action even when the pane holds no todos, so
opening a quiet pane's panel is never a dead end.

The panel SHALL list one row per todo regardless of how many lines that todo's
text holds, showing its first line with a marker indicating that more follows.

Opening a todo for editing SHALL present a modal built from the existing dialog
structure allowing its text, priority, link, and done state to be changed, with
explicit save and cancel. Because the modal's text field owns both the panel's
done-toggle key and the key that commits an edit, the modal SHALL offer the
toggle and the commit under distinct bindings that are not part of the text
field's editing set. A todo being composed SHALL NOT offer the done toggle,
since it cannot be already done.

Choosing a link SHALL present the session navigator in a selection mode rather
than cycling through candidates, so the target can be searched and filtered.
Rows that are not panes SHALL be context only and SHALL NOT be selectable as a
target, the todo's own pane SHALL NOT be offered, and the selection SHALL
include an explicit entry that clears the link. Leaving the selection without
choosing SHALL leave the link as it was. Pane rows SHALL show the public
identifier they would stage alongside the name they are listed under.

The selection SHALL be movable from the keyboard without the arrow keys in every
state of the picker, including while its search is focused, using the same chords
in both states. The arrow keys SHALL continue to work.

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
