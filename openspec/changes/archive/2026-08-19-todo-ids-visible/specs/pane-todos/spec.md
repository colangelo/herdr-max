# pane-todos

## ADDED Requirements

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
