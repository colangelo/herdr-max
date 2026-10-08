## ADDED Requirements

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
