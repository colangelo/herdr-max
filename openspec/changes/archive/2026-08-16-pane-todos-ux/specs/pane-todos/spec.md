# pane-todos

## REMOVED Requirements

### Requirement: Pane todo indicator

Replaced by "Always-on pane todo indicator": the indicator was conditional on a pane holding
todos, and this change makes it unconditional on every pane that draws a top
border. The scenario "A quiet pane is unchanged" asserted that an empty pane
draws no indicator, which is exactly the behaviour being replaced, so the
requirement is rewritten wholesale rather than scenario-patched.

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

## ADDED Requirements

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
