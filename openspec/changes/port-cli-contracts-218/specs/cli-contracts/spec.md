## ADDED Requirements

### Requirement: Fork commands survive the contract refactor

Every CLI command, alias and flag the fork offered before the port (measured on fork master
`f3fb2099` against upstream `212b710b`) SHALL still parse and SHALL send the same API method with
the same params JSON after the port.

#### Scenario: A fork-only command on a leaf

- **WHEN** a user runs `herdr pane clear --pane w1:p2`
- **THEN** the CLI sends `pane.clear_scrollback` with `{"pane_id": "w1:p2"}`, as before the port

#### Scenario: A fork flag on an upstream command

- **WHEN** a user runs `herdr tab close --tab w1:t2 --force`
- **THEN** the CLI sends `tab.close` with `force: true`

### Requirement: Every fork method is in the operations table

Each fork-only API method SHALL be declared in `define_operations!` with `client_shell` true
exactly when it was in the fork's client-shell method list, and `changes_ui` true exactly when the
server marked the UI dirty after it before the port.

#### Scenario: Pins stay advertised to the client shell

- **WHEN** a client shell reads the advertised method names
- **THEN** `agent.pin`, `agent.unpin`, `workspace.pin` and `workspace.unpin` are among them

### Requirement: The wire format does not change

The port SHALL NOT change any method name, params field or response shape, and the frozen v1
contract tests SHALL pass without edits to their fixtures.

#### Scenario: Frozen close shapes

- **WHEN** `advertised_client_shell_method_shapes_stay_at_the_v1_contract` runs
- **THEN** it passes with the fork's optional `force` on the close methods
