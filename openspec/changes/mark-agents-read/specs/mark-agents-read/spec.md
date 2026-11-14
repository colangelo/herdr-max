## ADDED Requirements

### Requirement: Mark panes read through the API

Herdr SHALL provide `pane.mark_seen` with an optional `workspace_id`. Without it,
every unread pane in the session SHALL become read; with it, only that space's panes.
The response SHALL report `changed`, the number of panes that were unread. A pane made
read SHALL report `agent_status` `idle` instead of `done` to every client.

#### Scenario: clear the whole session

- **WHEN** two panes in different spaces are unread and `pane.mark_seen` is sent with
  no params
- **THEN** both panes are read, the response is `changed: 2`, and their agents report
  `idle`

#### Scenario: clear one space

- **WHEN** space A and space B each have one unread pane and `pane.mark_seen` names A
- **THEN** only A's pane is read and the response is `changed: 1`

#### Scenario: nothing to clear

- **WHEN** no pane is unread
- **THEN** the response is `changed: 0` and no state update is published

#### Scenario: unknown space

- **WHEN** `pane.mark_seen` names a `workspace_id` that does not exist
- **THEN** the response is `workspace_not_found` and no pane changes

### Requirement: Mark panes read from the CLI

The CLI SHALL expose `pane.mark_seen` as `herdr pane mark-seen [--workspace <id>]`.

#### Scenario: CLI with a space

- **WHEN** the user runs `herdr pane mark-seen --workspace w2`
- **THEN** the CLI sends `pane.mark_seen` with `workspace_id` `w2` and prints `changed`

### Requirement: Right-click to mark agents read

The client SHALL offer "Mark space read" on a space's right-click menu when that space
has an unread pane, and "Mark all read" on a space's and on an agent row's right-click
menu when any pane is unread. Choosing one SHALL send `pane.mark_seen` with the
matching scope. The items SHALL be hidden when the connected server does not advertise
`pane.mark_seen`.

#### Scenario: menu items follow unread state

- **WHEN** the user right-clicks a space with an unread pane
- **THEN** the menu shows "Mark space read" and "Mark all read"

#### Scenario: older server

- **WHEN** the client is connected to a server whose method list lacks `pane.mark_seen`
- **THEN** neither item is shown and the rest of the menu is unchanged
