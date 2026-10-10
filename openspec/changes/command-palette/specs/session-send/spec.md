## ADDED Requirements

### Requirement: Sending is a server method

herdr SHALL provide an `agent.send` API method that delivers text to a target session either typed
into its pane (`mode: typed`) or as a peer note through the configured note command
(`mode: note`). Every client (the palette, the CLI, a later phone client) SHALL send through it.

#### Scenario: The CLI and the palette share the path

- **WHEN** `herdr agent send planner "hello"` runs
- **THEN** the server types the text as the palette's `as me` would, and returns the same receipt

### Requirement: Typed delivery arrives as typed text

A typed send SHALL be split into pieces of about 300 bytes on character boundaries, sent with a
gap between pieces, followed by Enter as its own key, so the agent sees typed text and not a
collapsed paste.

#### Scenario: A long message

- **WHEN** a 1,000-byte message is sent typed to a Claude Code pane
- **THEN** it arrives in four pieces, then one Enter, and the receipt says `4 pieces, enter pressed`

### Requirement: The target's state is shown before sending

Before the text is sent the bar SHALL show the target's `agent_status`, and for a blocked target
its `blocked_reason`.

#### Scenario: A blocked target

- **WHEN** the picked target is blocked on a permission prompt
- **THEN** the bar shows `blocked: permission` and the prompt's options

### Requirement: A blocked target gets its options, not free text

For a target blocked with reason `question`, `permission` or `form` whose options the server can
read, the bar SHALL offer those options and send the chosen option's key. Free text SHALL NOT be
typed into a blocked target from the bar. With reason `other`, or no options found, the bar SHALL
offer only to open the pane.

#### Scenario: A hermes credential prompt

- **WHEN** the target is a hermes pane blocked on its credential prompt
- **THEN** the bar offers only `open pane` and types nothing

### Requirement: A busy target queues unless told to interrupt

For a `working` target, `enter` SHALL type the text now and let the agent take it at its next
pause. `alt+enter` SHALL press `esc`, wait up to 3 seconds for the target to leave `working`, then
type; if it does not leave `working`, nothing SHALL be typed and the bar says so.

#### Scenario: Interrupt that does not take

- **WHEN** the user interrupts and the target is still `working` after 3 seconds
- **THEN** no text is typed and the bar says the interrupt did not take

### Requirement: Notes keep agent-bell's contract

A note SHALL be sent with the configured note command as a peer message from `herdr-palette`. The
bar SHALL show its receipt (`queued`, `held` with its note, or `injected`). A `held` receipt SHALL
NOT trigger any other delivery.

#### Scenario: A held receiver

- **WHEN** a note goes to a session on HOLD
- **THEN** the bar shows `held` and agent-bell's note, and sends nothing else

### Requirement: Receipts say what they do not mean

Every receipt SHALL end with a line saying it means the text arrived, not that it was read or done.

#### Scenario: A typed send

- **WHEN** a typed send completes
- **THEN** the receipt shows the pieces and Enter, then the target's next state, then that line

### Requirement: Typed sends stay on attached servers

`as me` SHALL be offered only for panes of servers this client is attached to, and `agent.send`
SHALL accept calls only on the local API socket until a device identity exists for remote callers.

#### Scenario: A session only agent-bell knows

- **WHEN** the target exists only in `agent-bell who`
- **THEN** the bar offers `as a note` only
