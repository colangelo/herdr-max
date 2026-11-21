## ADDED Requirements

### Requirement: Publish the layout as the herdr-app document

With `[experimental] core_spaces_doc = true`, the herdr server SHALL write its
workspaces, tabs and split trees into this herdr session's core session as the document
`herdr-app`. The document SHALL use the shape the Herdr apps decode: `{"spaces": [...]}`,
with each space as `{id, name, folder, tabs}`, each tab as `{id, name, root}`, and
`root` as a layout node.

#### Scenario: a workspace with a split tab

- **WHEN** the flag is on and workspace `w1` has tab `w1:t1` split side by side into two
  core panes at ratio 0.5
- **THEN** `doc.get {name: "herdr-app"}` returns a space `w1` whose tab `w1:t1` has a
  `row` root with two `block` children, each with weight 0.5 and the pane's core address
  in `content.terminal`

#### Scenario: flag off

- **WHEN** the flag is off
- **THEN** the server makes no `doc.set` call

### Requirement: Only core panes become terminal blocks

A pane that does not run in herdr-core SHALL be left out of the document. Its split
SHALL collapse to the other side, which keeps the slot's size. A tab with no core pane
SHALL have an empty `row` root.

#### Scenario: a PTY pane beside a core pane

- **WHEN** a tab splits a core pane and a PTY pane
- **THEN** the tab's root holds only the core pane's block

### Requirement: Latest-state compare-and-set writer

The server SHALL write from a background thread, with at most one write in flight. Only
the latest document SHALL be kept. A document equal to the last one written SHALL NOT
be written again. Each write SHALL be `doc.set` with the last known version. On
`conflict` the writer SHALL read the version and write again. When core is unavailable,
the writer SHALL keep the document and retry.

#### Scenario: another writer changed the document

- **WHEN** another client wrote `herdr-app` after herdr's last write
- **THEN** herdr's next publish gets `conflict`, reads the new version, and its layout
  replaces the other write

#### Scenario: a burst of changes

- **WHEN** the layout changes several times within 500 ms
- **THEN** the server builds the document once, after 500 ms

### Requirement: Startup, handoff and reload

The server SHALL publish once after startup and after a handoff when the flag is on.
Config reload SHALL apply the flag: turning it on SHALL publish, and turning it off
SHALL write `null` to `herdr-app`.

#### Scenario: turned off on reload

- **WHEN** the flag goes from true to false on config reload
- **THEN** `doc.get {name: "herdr-app"}` returns `value: null`
