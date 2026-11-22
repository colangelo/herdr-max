## ADDED Requirements

### Requirement: Publish herdr-max's extras as the herdr-max document

With `[experimental] core_extras_doc = true`, the herdr server SHALL write the core document
`herdr-max` in its own core session, with `format: 1`, `writer` (herdr session name and server
version), and `spaces` in herdr's session order. With the flag off the server SHALL write `null`.

#### Scenario: flag off

- **WHEN** the flag is turned off on reload
- **THEN** `doc.get {name: "herdr-max"}` returns `null`

### Requirement: Spaces, tabs and panes carry the extras

Each space SHALL carry its public id, pin place, custom name and unexpired metadata tokens with
wall-clock `expires_ms`. Each tab SHALL carry its public id and custom name. Each core pane SHALL
carry its public id, core address, label, agent name and open todo references as `{id, priority}`.
Ids SHALL equal those of the `herdr-app` document. Panes without a core terminal SHALL be left out.

#### Scenario: a pinned space with a token

- **WHEN** workspace `w1` is pinned first, named `herdr` by the user, and has the token `asks=3` with a
  60 s TTL
- **THEN** `doc.get {name: "herdr-max"}` returns space `w1` with `pin: 1`, `custom_name: "herdr"` and
  `tokens.asks` with `value: "3"` and an `expires_ms` about 60 s ahead

#### Scenario: todos

- **WHEN** a core pane has two open todos and one done todo
- **THEN** the pane lists two `todos` references and no todo text

#### Scenario: join with herdr-app

- **WHEN** both documents are published
- **THEN** every pane `id` and `terminal` in `herdr-max` names a block id and `content.terminal` in
  `herdr-app`

### Requirement: One-way and owned whole

herdr-max SHALL own the whole `herdr-max` value and write it with compare-and-set on the version it
read. It SHALL NOT read the document back into its own state. It SHALL NOT write a value over 1 MiB.
`session.json` SHALL remain the only source herdr-max restores from.

#### Scenario: another value is there

- **WHEN** core's `herdr-max` value was written by another herdr session or by hand
- **THEN** herdr's next publish replaces it whole and logs a warning

#### Scenario: core's document is lost

- **WHEN** core's state is wiped and `herdr-max` reads `null`
- **THEN** herdr-max's spaces, pins, names and todos are unchanged and the next publish writes them
  again
