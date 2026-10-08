# session-persistence Specification

## Purpose
How the session file is written and read so a session survives crashes and bad
files: writes are durable before they replace the previous file, and an unusable
file is kept rather than overwritten.

## Requirements

### Requirement: An unusable session file is preserved, never overwritten

When the session file exists but cannot be used — unreadable, unparseable, or
written by a newer herdr than the one starting — the server SHALL move it aside
under a timestamped name before continuing, and SHALL log that it did so.

The session SHALL then start empty, as it does today. What SHALL NOT happen is
the empty session being written over the file that could not be read: the
unusable file is the only remaining copy of the user's panes, their working
directories and their todos, and a todo is content that exists nowhere else.

The preserved name SHALL carry a UTC timestamp so repeated failures do not
overwrite one another and the newest is identifiable by name alone. Preserved
files SHALL NOT be deleted automatically.

Moving the file aside SHALL be the same act as preserving it, so no window
exists in which the file is both unusable and still at the path a save would
write to.

#### Scenario: A torn session file survives the startup that could not read it

- **WHEN** the session file is truncated or otherwise unparseable and the server starts
- **THEN** the file is moved aside under a timestamped name
- **AND** the session starts empty
- **AND** the moved-aside file still holds everything it held before the server started

#### Scenario: A newer session file is not destroyed by an older herdr

- **WHEN** the session file records a snapshot version newer than the running herdr supports
- **THEN** it is moved aside under a timestamped name rather than ignored in place
- **AND** upgrading herdr again does not find it overwritten

#### Scenario: Repeated failures do not overwrite each earlier copy

- **WHEN** two unusable session files are preserved in turn
- **THEN** both preserved files exist under distinct names

### Requirement: Session writes are durable before they replace the previous file

The session file SHALL be written to a temporary path, flushed to disk, and only
then renamed over its destination. Without the flush the rename may reach the
filesystem before the data does on an unexpected power loss, which is the very
failure that produces a torn file.

#### Scenario: The written file is flushed before it takes the place of the old one

- **WHEN** a session snapshot is saved
- **THEN** its contents are flushed to disk before the rename that publishes it

### Requirement: The last client size survives a restart

The session file SHALL store the remembered client size as an optional field.
On start, a server SHALL use it as the no-client size until a client attaches.
A missing, zero or unreadable value SHALL fall back to `server.headless_cols/rows`.

#### Scenario: A cold start uses the remembered size

- **WHEN** the session file holds a 310×56 client size and the server starts with no client
- **THEN** restored panes are spawned at 310×56, not 120×40

#### Scenario: An older session file

- **WHEN** the session file has no client size
- **THEN** the server uses `server.headless_cols/rows`, as today

#### Scenario: An older build reads a newer file

- **WHEN** a build without this change reads a session file that holds a client size
- **THEN** it loads the session and ignores the field

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
