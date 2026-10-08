## ADDED Requirements

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

#### Scenario: Priority orders the list

- **WHEN** a pane holds a normal todo added first and a high todo added second
- **THEN** the high todo is presented first

#### Scenario: Done todos sink

- **WHEN** a high-priority todo is marked done and a normal-priority todo is not
- **THEN** the not-done normal todo is presented before the done high one

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

A todo MAY link to another pane. The link SHALL store the target's internal pane
identity plus a label captured when the link was created. On restore, the target
SHALL be remapped through the same identifier map that remaps layout references.

A link whose target cannot be resolved SHALL be preserved as a dead link that
retains its label, is presented as inert, and SHALL NOT resolve to any other
pane.

#### Scenario: A link survives restore

- **WHEN** a todo links to another pane and the session is restored
- **THEN** the link resolves to the same pane it referred to before the restart

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

### Requirement: Pane todo indicator

A pane with at least one todo SHALL show an indicator at the far right of its top
border carrying the count of outstanding todos. A pane whose todos are all done
SHALL show the indicator without a count. A pane with no todos SHALL show no
indicator and SHALL render exactly as it does without this feature.

The indicator SHALL be colored by the highest outstanding priority, and SHALL be
configurable off. The region used to draw the indicator and the region that
accepts a click SHALL be derived from a single shared definition. When pane width
forces a choice, the indicator SHALL be laid out before the pane title so the
title truncates instead of the control disappearing.

#### Scenario: Outstanding count is shown

- **WHEN** a pane holds three not-done todos and one done todo
- **THEN** its indicator shows a count of three

#### Scenario: A quiet pane is unchanged

- **WHEN** a pane has no todos
- **THEN** no indicator is drawn and the border renders identically to a build
  without this feature

#### Scenario: Click target matches what is drawn

- **WHEN** the indicator is drawn for a pane
- **THEN** the cells that respond to a click are exactly the cells drawn

### Requirement: Pane todo panel and editing

Activating the indicator, by click or by a bindable action, SHALL open a panel
anchored to that pane listing its todos in presentation order, following the
existing overlay language. The panel SHALL support moving the selection,
toggling done, removing a todo, clearing done todos, opening a todo for editing,
following a todo's link, and closing.

Opening a todo for editing SHALL present a modal built from the existing dialog
structure allowing its text, priority, link, and done state to be changed, with
explicit save and cancel. Because the modal's text field owns the panel's
done-toggle key, the modal SHALL offer the toggle under a distinct binding. A
todo being composed SHALL NOT offer the done toggle, since it cannot be already
done.

Following a link SHALL move focus to the linked pane through the same focus path
used when jumping to a notification's pane.

Every action introduced SHALL appear in the keybinding help panel, including any
left unbound by default.

#### Scenario: Opening the panel

- **WHEN** the indicator is clicked or the open action is invoked on a pane with
  todos
- **THEN** a panel anchored to that pane lists its todos in presentation order

#### Scenario: Editing a todo

- **WHEN** a todo is opened for editing, its text changed, and the change saved
- **THEN** the todo's text and updated timestamp change while its id, done state,
  and creation timestamp are preserved

#### Scenario: Toggling done from the edit modal

- **WHEN** a todo is opened for editing and the done toggle is activated and the
  change saved
- **THEN** the todo's done state flips while its text, id, and creation timestamp
  are preserved
- **WHEN** the edit is cancelled instead
- **THEN** the todo's done state is unchanged

#### Scenario: Following a link

- **WHEN** a todo with a live link is followed
- **THEN** focus moves to the linked pane

#### Scenario: Actions are discoverable

- **WHEN** the keybinding help panel is shown
- **THEN** every action introduced by this feature is listed, showing `unset` for
  any that has no binding
