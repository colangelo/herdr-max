## ADDED Requirements

### Requirement: Clipboard feedback can show in the source pane

With `[ui.toast.clipboard] position = "pane"`, the copied-to-clipboard box
SHALL be centered in the inner area of the pane the copied text came from,
both for herdr's own copies and for OSC 52 writes by an app in a pane.

#### Scenario: Mouse copy

- **WHEN** the user selects text in a pane with `position = "pane"`
- **THEN** the box is centered in that pane

#### Scenario: OSC 52 from an app

- **WHEN** an app in a visible pane writes the clipboard with OSC 52
- **THEN** the box is centered in that pane

### Requirement: Pane-action notes can show in the pane acted on

With `[ui.toast.herdr] pane_feedback = "pane"`, a note set by a pane action
SHALL be centered in the pane that was focused when the note was set. Agent
notifications SHALL keep the `[ui.toast.herdr] position`.

#### Scenario: Pane move unavailable

- **WHEN** a pane move fails with `pane_feedback = "pane"`
- **THEN** the note is centered in the focused pane

### Requirement: Pane placement falls back to the configured spot

When the feedback has no pane, or the pane is not shown in the visible tab,
or its inner area is smaller than the box, the box SHALL be placed as it is
without pane placement.

#### Scenario: A small pane

- **WHEN** the source pane is narrower than the box
- **THEN** the clipboard box shows at the bottom center

### Requirement: Defaults are unchanged

With no configuration, the clipboard box SHALL show at the bottom center and
pane-action notes in the herdr toast corner.

#### Scenario: Default config

- **WHEN** a user copies text with no toast configuration
- **THEN** the box shows at the bottom center, as before
