## ADDED Requirements

### Requirement: An unnamed agent is named from a name-like terminal title

An agent with no explicit name SHALL report its stripped terminal title as its
`name`, with `name_source: "title"`, when that title is one word of 1-64
characters from `[A-Za-z0-9._-]` starting with a letter or digit and is not an
agent kind or shell name, and no other agent has that name or the same
fallback.

#### Scenario: Claude session name as title

- **WHEN** an unnamed Claude pane has the terminal title "jev-astra"
- **THEN** `agent list` shows `name: "jev-astra"`, `name_source: "title"`, and
  `agent read jev-astra` reads that pane

#### Scenario: Titles that are not names

- **WHEN** the title is "Claude Code", empty, a spinner glyph, or "fix the flaky test"
- **THEN** the agent has no name

#### Scenario: Duplicates

- **WHEN** two unnamed agents have the title "worker"
- **THEN** neither gets the fallback

### Requirement: An explicit name always wins

An explicit name SHALL never be replaced by a fallback, and SHALL be reported
without `name_source`. A fallback SHALL follow the terminal title.

#### Scenario: Rename in Claude

- **WHEN** an unnamed agent's title changes from "alpha" to "beta"
- **THEN** its name is "beta"

### Requirement: Notifications name the agent

Agent notifications SHALL be titled with the agent's name (explicit or
fallback) followed by the event, and with the agent kind when it has no name,
in every delivery.

#### Scenario: Named agent finishes

- **WHEN** the agent named "direction-builder" finishes
- **THEN** the notification title is "direction-builder finished"

#### Scenario: Unnamed agent

- **WHEN** an agent with no name and no name-like title finishes
- **THEN** the notification title is "claude finished"
