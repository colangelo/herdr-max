## ADDED Requirements

### Requirement: macOS panes run in their own launchd job
On macOS with a GUI session, each new pane program SHALL be started by a per-pane keeper that runs as its own
gui-domain launchd job and stays that job's process for the pane's life, so the pane has a resource coalition of its own.

#### Scenario: App launched from a pane exits
- **WHEN** a GUI app started from a pane's shell exits
- **THEN** no LaunchServices entry remains whose stop could end the server or another pane

#### Scenario: One pane's coalition is ended
- **WHEN** the system ends one pane's launchd job
- **THEN** only that pane's program ends, and the server and other panes keep running

### Requirement: Panes ended by the session come back by resume
A pane whose keeper ended without reporting its program's exit SHALL be kept in the layout as suspended with its
resume plan and SHALL be respawned and resumed on the next herdr start or client attach with a GUI session.

#### Scenario: Logout and login
- **WHEN** ac logs out and back in and starts herdr
- **THEN** every pane that ran an agent is back in its place, resumed from its saved plan
