## ADDED Requirements

### Requirement: Handoff replacement runs in its own coalition on macOS
On macOS, a live handoff SHALL start the replacement server so that it does not share the replaced server's
resource coalition, using the same one-shot launchd job as a server the client starts, and SHALL fall back to the
existing detached spawn when launchd cannot be used.

#### Scenario: Handoff from a server in a shared coalition
- **WHEN** a server whose coalition also holds unrelated processes hands off to a replacement
- **THEN** the replacement and the panes it spawns afterwards are in a coalition of their own

#### Scenario: App launched from a pane after the handoff
- **WHEN** a GUI app is started from a shell in a pane spawned after the handoff and exits
- **THEN** LaunchServices keeps no entry for it that could end the server's panes

### Requirement: Handoff rollback signals only its replacement
When the replacement was started through launchd, the handoff SHALL identify it by the handoff socket's peer and
SHALL NOT signal any process it cannot identify as that replacement.

#### Scenario: Rollback with an unknown replacement pid
- **WHEN** a handoff fails and the replacement's pid is unknown
- **THEN** no process is signalled and one warning is logged
