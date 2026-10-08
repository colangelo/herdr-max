## ADDED Requirements

### Requirement: Pinned spaces stay at the top in pin order

A pinned space SHALL be listed above every unpinned space, and pinned spaces
SHALL be listed in the order they were pinned, under every `workspace_sort`
mode. Unpinned spaces SHALL sort below as before. Unpinning SHALL return the
space to the place the sort mode gives it.

#### Scenario: Pin order under priority sort

- **WHEN** space B is pinned, then space D, and the sort mode is `priority`
- **THEN** the list starts B, D whatever the agent states are, followed by the
  other spaces in attention order

#### Scenario: Unpin

- **WHEN** B is unpinned
- **THEN** D stays first and B takes the place the sort mode gives it

### Requirement: Pinned agents stay at the top of the agent panel in pin order

A pinned agent SHALL be listed above every unpinned agent in one block, in pin
order, under every `agent_panel_sort` mode and any agent view sort. Unpinned
agents SHALL sort below as before.

#### Scenario: Grouped mode

- **WHEN** an agent in the third space is pinned in `spaces` mode
- **THEN** it is the first row of the agent panel, above the agents of the
  first space

### Requirement: A pin is saved with the session

The pin of a space and of an agent SHALL be saved in the session snapshot and
SHALL survive a restart and a live handoff. A snapshot written without the
field SHALL load with nothing pinned. A resumed agent SHALL keep its pin.

#### Scenario: Old file

- **WHEN** a snapshot with no pin field is loaded
- **THEN** no space and no agent is pinned and the order is unchanged

### Requirement: Pins are controlled through the API and keys

`workspace.pin`, `workspace.unpin`, `agent.pin` and `agent.unpin` SHALL set and
clear the pin and return the updated entity; `pinned` SHALL appear on workspace,
agent and pane info. The actions `toggle_pin_workspace` and `toggle_pin_agent`
SHALL exist, unbound by default, and appear in the keybinding help.

#### Scenario: CLI

- **WHEN** `herdr workspace pin w2` runs
- **THEN** `workspace get w2` reports `pinned: true` and the space is listed
  after any space pinned earlier

### Requirement: Pinned rows are marked and do not animate

A pinned row SHALL show a one-cell marker before its name. The priority-sort
bubble motion SHALL NOT animate pinned rows.

#### Scenario: Narrow sidebar

- **WHEN** a pinned row is drawn at the narrowest sidebar width
- **THEN** the marker takes one cell and the row does not overflow
