## ADDED Requirements

### Requirement: A moved pane's former ids survive a restart and a handoff

When a pane moves to another space, the public id it had before the move SHALL
keep resolving to it, in the running server, after a restart that restores the
session, and after a live handoff. An id that names a live pane SHALL resolve to
that pane, even if it is also a former id of another pane. A pane's former ids
SHALL be forgotten when the pane closes.

#### Scenario: A report under the old id after a handoff

- **WHEN** a pane `w2:p1X` is moved to space `wV` (becoming `wV:p1`), the
  server hands off to a new binary, and a hook then reports an agent session
  for `w2:p1X`
- **THEN** the session is recorded on the moved pane

#### Scenario: The same after a restart

- **WHEN** the same pane's session is saved, the server restarts and restores
  it, and a hook reports for `w2:p1X`
- **THEN** the report lands on the moved pane

#### Scenario: A live id wins

- **WHEN** an id that is a former id of one pane is also the current id of
  another, live pane
- **THEN** the id resolves to the live pane
