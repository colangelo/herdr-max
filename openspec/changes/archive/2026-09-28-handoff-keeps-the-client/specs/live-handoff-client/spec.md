## ADDED Requirements

### Requirement: Panes keep their size through a live handoff

A live handoff SHALL carry the exporting server's effective size, and the
importing server SHALL use it as its size while no client is attached, until a
client attaches.

#### Scenario: A handoff with a client attached keeps pane widths

- **WHEN** a 173×59 client is attached and the server hands off
- **THEN** after the handoff, before any client reattaches, panes keep the size they had

#### Scenario: A manifest without a size falls back to the headless default

- **WHEN** the manifest carries no size
- **THEN** the importing server uses `server.headless_cols`/`headless_rows` as today

### Requirement: The app client reattaches after a live handoff

An app client that receives the live-handoff shutdown SHALL wait up to 30 s for
the new server and reattach at its current terminal size. Other shutdowns,
detaches, and direct terminal attaches SHALL exit as before.

#### Scenario: A client survives a handoff

- **WHEN** a client is attached and the server hands off
- **THEN** the client reconnects to the new server without anyone relaunching it

#### Scenario: A stopped server still ends the client

- **WHEN** the server stops normally
- **THEN** the client exits with its shutdown message, as today
