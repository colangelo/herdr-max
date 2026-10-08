## ADDED Requirements

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
