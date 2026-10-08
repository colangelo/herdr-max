## ADDED Requirements

### Requirement: A close from outside the TUI never opens a modal

A `pane.close`, `tab.close` or `workspace.close` that did not come from the
TUI SHALL NOT change any client's mode or pending confirmation. When the close
needs confirmation it SHALL be refused with `confirmation_required` and a
message that says why.

#### Scenario: An API close of a pane with todos

- **WHEN** a client is attached and `pane.close` arrives over the socket for a
  pane with an open todo
- **THEN** the answer is `confirmation_required` naming the todo, and the
  client's mode is unchanged

#### Scenario: The retry does not leave a modal behind

- **WHEN** the same close is sent again without `force`
- **THEN** it is refused again, and no "Close workspace?" modal appears

### Requirement: force closes and reports what it dropped

`pane close`, `tab close` and `workspace close` SHALL take `--force` (API
field `force`). With it the close SHALL proceed past the todo and worktree
group confirmations, and the result SHALL list the open todos of every closed
pane. `workspace.close` SHALL still need `close_group` to close a group.

#### Scenario: Forced close of a pane with todos

- **WHEN** `herdr pane close <p> --force` runs on a pane with two open todos
- **THEN** the pane closes and the result carries both todos under its id

#### Scenario: Tab close with todos, no force

- **WHEN** `tab.close` targets a tab where one pane has an open todo
- **THEN** it is refused with `confirmation_required` and the tab stays

### Requirement: No orphan close confirmation

A close confirmation whose target no longer exists SHALL leave the
confirmation mode instead of showing a different question.

#### Scenario: The pane closes under its modal

- **WHEN** the TUI shows "Close pane with unfinished todos?" and that pane is
  closed by an API request with `force`
- **THEN** the modal is gone, and "Close workspace?" is not shown
