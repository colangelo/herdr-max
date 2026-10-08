## ADDED Requirements

### Requirement: The move picker names spaces as the sidebar does

The move-pane picker SHALL label every space heading with the same name the
sidebar shows for that space, resolved from the space's live root-pane working
directory. A pane moved to a new space SHALL seed that space's identity from the
pane's live working directory.

#### Scenario: A space made by a move keeps its own name

- **WHEN** a pane whose live directory is `CONTEXT` (stored directory still
  `master`) is moved from the space "master" to a new space, and the picker is
  opened again from "master"
- **THEN** the picker lists "master" and "CONTEXT", not "master" twice

### Requirement: The move picker can be searched

The move-pane picker SHALL offer the kit's list search. A query SHALL match
space names, tab numbers and labels, and the `new tab` and `new space` rows. A
space heading SHALL stay visible, not selectable, while any of its destinations
match; a matching heading SHALL keep all its destinations.

#### Scenario: Find a space by name

- **WHEN** the user presses `/` in the picker and types part of a space's name
- **THEN** only that space's heading and destinations remain, the first
  destination is selected, and Enter moves the pane there

#### Scenario: No match

- **WHEN** the query matches nothing
- **THEN** the picker says so, keeps the query, and Enter does nothing
