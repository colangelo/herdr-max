# session-persistence

## ADDED Requirements

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
