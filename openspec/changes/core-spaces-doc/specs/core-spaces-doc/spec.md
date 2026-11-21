## ADDED Requirements

### Requirement: Publish the layout as the herdr-app document

With `[experimental] core_spaces_doc = true`, the herdr server SHALL write its
workspaces, tabs and split trees into this herdr session's core session as the document
`herdr-app`. The document SHALL use the shape the Herdr apps decode: `{"spaces": [...]}`,
with each space as `{id, name, folder, tabs, herdr_max: 1}`, each tab as
`{id, name, root}`, and `root` as a layout node.

#### Scenario: a workspace with a split tab

- **WHEN** the flag is on and workspace `w1` has tab `w1:t1` split side by side into two
  core panes at ratio 0.5
- **THEN** `doc.get {name: "herdr-app"}` returns a space `w1` whose tab `w1:t1` has a
  `row` root with two `block` children, each with weight 0.5 and the pane's core address
  in `content.terminal`

#### Scenario: flag off

- **WHEN** the flag is off
- **THEN** the server makes no `doc.set` call

### Requirement: herdr-max writes only its own spaces

herdr SHALL treat a space with a `herdr_max` field as its own and every other space as
foreign. Each write SHALL keep every foreign space unchanged and in its order, and the
document's other top-level fields. herdr's spaces SHALL replace its marked spaces at the
place of the first marked space, or follow the foreign spaces when none is marked. herdr
SHALL NOT overwrite a value that is not `null` or an object whose `spaces` is absent or
a list of objects.

#### Scenario: the phone added a space

- **WHEN** core's document holds a foreign space `s9` and then herdr's marked space
- **THEN** herdr's next publish writes `s9` unchanged and then herdr's current spaces

#### Scenario: an undecodable document

- **WHEN** core's `herdr-app` value is a string
- **THEN** herdr makes no `doc.set` call, logs a warning once, and tries again later

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
be written again. Each write SHALL read the document, merge herdr's spaces into it, and
`doc.set` the result with the version it read. On `conflict` the writer SHALL read and
merge again. When core is unavailable, the writer SHALL keep the document and retry.

#### Scenario: another writer changed the document

- **WHEN** another client writes `herdr-app` between herdr's read and herdr's write
- **THEN** herdr's write gets `conflict`, and herdr reads the new value and merges its
  spaces into it, keeping the other client's spaces

#### Scenario: a burst of changes

- **WHEN** the layout changes several times within 500 ms
- **THEN** the server builds the document once, after 500 ms

### Requirement: Startup, handoff and reload

The server SHALL publish once after startup and after a handoff when the flag is on.
Config reload SHALL apply the flag: turning it on SHALL publish, and turning it off
SHALL remove herdr's marked spaces. The document SHALL become `null` only when nothing
foreign remains.

#### Scenario: turned off on reload

- **WHEN** the flag goes from true to false on config reload and core's document holds
  a foreign space `s9` and herdr's spaces
- **THEN** `doc.get {name: "herdr-app"}` returns only `s9`

#### Scenario: turned off with nothing foreign

- **WHEN** the flag goes from true to false and the document holds only herdr's spaces
- **THEN** `doc.get {name: "herdr-app"}` returns `value: null`
