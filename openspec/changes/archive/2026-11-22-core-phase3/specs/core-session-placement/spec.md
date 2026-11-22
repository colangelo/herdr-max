## ADDED Requirements

### Requirement: The default herdr session runs in the default core session

A release build SHALL start the default herdr session's new core panes in core session `default`. A named herdr
session SHALL use core session `herdr-<name>`. A debug build SHALL use `herdrdev` for the default session and
`herdrdev-<name>` for a named one.

#### Scenario: Release default session
- **WHEN** a release build of the default herdr session starts a core pane
- **THEN** the terminal runs in core session `default`

#### Scenario: Named session
- **WHEN** herdr session `trial` starts a core pane
- **THEN** the terminal runs in core session `herdr-trial`

### Requirement: Ownership is the herdr.session label

Every core terminal a herdr session starts SHALL carry `herdr.session` with the session's label (`herdr`,
`herdr-<name>`, `herdrdev`, `herdrdev-<name>`), whichever core session it runs in. The server SHALL claim, list as
an orphan, or kill only terminals that carry its label, and SHALL reattach only a terminal whose `herdr.pane_id`
also matches the pane. Notices that name a command SHALL give the core session and the label separately.

#### Scenario: Another client's terminal in the default core
- **WHEN** the default core holds a terminal without `herdr.session=herdr`
- **THEN** herdr-max neither attaches to it, nor names it in the orphan notice, nor kills it

#### Scenario: Orphan notice in the default core
- **WHEN** an unclaimed terminal labelled `herdr.session=herdr` runs in core session `default`
- **THEN** the notice says `HC_SESSION=default hc ls --label herdr.session=herdr`

### Requirement: Panes are re-adopted from the session their link names

A core pane's saved link SHALL record the core session the pane's terminal runs in, taken from the pane, never from
the primary session's name. On restore and live handoff the server SHALL reattach each pane in the session its link
names, by the pair of session and address, without starting a core that is not its primary. A build that reads
sessions `default` and `herdr` SHALL be installed before any build places panes in `default`.

#### Scenario: A pane from before the move
- **WHEN** a phase 3 build restores a pane whose link says session `herdr`, address `t7`
- **THEN** the pane reattaches to `t7` in core session `herdr`, and its next save still says session `herdr`

#### Scenario: The same address in two cores
- **WHEN** core sessions `default` and `herdr` both hold a terminal `t7` labelled for this herdr session
- **THEN** each is claimed only by the pane whose link names its session

#### Scenario: Rollback after the placement move
- **WHEN** a pane runs in `default` and herdr is rolled back to the build that reads two sessions but places in
  `herdr`
- **THEN** that build re-adopts the pane from `default` and starts nothing fresh

### Requirement: HC_SOCKET names the primary core only

When `HC_SOCKET` is set, the server SHALL use it only for its primary core session. Every secondary session's
socket SHALL follow core's own path rules.

#### Scenario: HC_SOCKET with a legacy session
- **WHEN** `HC_SOCKET` is set and a pane's link names a secondary session
- **THEN** the server reaches that session at its own socket path, not at `HC_SOCKET`

### Requirement: A shared default core is checked, never restarted

When the server connects to core session `default` and finds positive evidence that an SSH login started that core,
the server SHALL log one warning and show one notice naming the problem and the fix, and SHALL NOT restart that
core. A parent pid of 1 alone SHALL NOT count as evidence either way.

#### Scenario: The phone started the core first
- **WHEN** the default core's session leader is an SSH login
- **THEN** one notice says it was started over SSH and should be restarted from a desktop terminal

#### Scenario: A desktop start
- **WHEN** the default core has parent pid 1 and no SSH login in its session
- **THEN** no notice is shown

### Requirement: The herdr-app document names only primary-session terminals

When the server publishes its spaces into the `herdr-app` document, a pane SHALL be a terminal leaf only when its
terminal runs in the primary core session. A pane in another core session SHALL be left out as a non-core pane is.
The document SHALL be written only in the primary session.

#### Scenario: A pane still in the legacy core
- **WHEN** a pane's terminal runs in core session `herdr` and the primary is `default`
- **THEN** the published space leaves that pane out, and its split's other side keeps the slot
