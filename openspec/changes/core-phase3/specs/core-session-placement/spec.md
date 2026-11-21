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
also matches the pane.

#### Scenario: Another client's terminal in the default core
- **WHEN** the default core holds a terminal without `herdr.session=herdr`
- **THEN** herdr-max neither attaches to it, nor names it in the orphan notice, nor kills it

### Requirement: Panes are re-adopted from the session their link names

A core pane's saved link SHALL record the core session the pane's terminal runs in. On restore and live handoff the
server SHALL reattach each pane in the session its link names, by the pair of session and address, without starting
a core that is not its primary. When the primary session is `default` and the legacy session `herdr` is running,
the server SHALL also list unclaimed terminals labelled for it there.

#### Scenario: A pane from before the move
- **WHEN** a phase 3 build restores a pane whose link says session `herdr`, address `t7`
- **THEN** the pane reattaches to `t7` in core session `herdr`, and its next save still says session `herdr`

#### Scenario: The same address in two cores
- **WHEN** core sessions `default` and `herdr` both hold a terminal `t7` labelled for this herdr session
- **THEN** each is claimed only by the pane whose link names its session

### Requirement: A shared default core is checked, never restarted

When the server connects to core session `default` and that core was not started by launchd, the server SHALL log
one warning and show one notice naming the problem and the fix, and SHALL NOT restart that core.

#### Scenario: The phone started the core first
- **WHEN** the default core is a direct child of an SSH login
- **THEN** one notice says it was started over SSH and should be restarted from a desktop terminal

### Requirement: The herdr-app document keeps other clients' spaces

In a core session other clients also use, the server SHALL write to the `herdr-app` document only the spaces it
marks as its own, SHALL keep every other space unchanged, and when its publishing is turned off SHALL remove only its
own spaces.

#### Scenario: Herdr.app has spaces in the document
- **WHEN** the document holds a Herdr.app space and herdr-max publishes its layout
- **THEN** the Herdr.app space is unchanged and herdr-max's spaces are replaced
