## ADDED Requirements

### Requirement: A frame never shows a pane mid synchronized update

herdr SHALL NOT present a client frame that shows a pane whose synchronized
update (DEC mode 2026) is open and younger than `SYNC_HOLD_MAX` (200 ms). The
client SHALL keep its last complete frame, and herdr SHALL send the held frame
when the block ends.

#### Scenario: A key arrives while an app is mid-frame

- **WHEN** an app in the visible tab has begun a synchronized update and has
  drawn half its frame, and a key arrives that would make herdr render
- **THEN** no frame is sent to the client until the app ends the block, and the
  frame then sent shows the app's complete new frame

#### Scenario: A hidden pane never holds the screen

- **WHEN** a pane in another tab or workspace is mid-block
- **THEN** the visible tab renders as usual

### Requirement: A stuck synchronized update cannot freeze the screen

A block open longer than `SYNC_HOLD_MAX` SHALL stop holding frames, and herdr
SHALL draw the pane as it is. herdr SHALL render at that deadline without
needing input.

#### Scenario: An app never ends its block

- **WHEN** an app begins a synchronized update and never ends it
- **THEN** within 200 ms plus one render, the client shows the pane's current
  contents

### Requirement: A block that starts while a frame is built voids that frame

If a pane shown in a frame begins or ends a synchronized update while the
frame is being built, herdr SHALL drop that frame and render again.

#### Scenario: Begin during the build

- **WHEN** a pane's epoch changes between the pre-build check and the end of the
  build
- **THEN** the frame is not sent and a new render is requested

### Requirement: Partial updates follow the same rule

The retained dirty-patch path SHALL NOT send rows of a pane that is mid-block;
it SHALL fall back to the full render, which holds.

#### Scenario: Dirty rows of a mid-block pane

- **WHEN** only a mid-block pane has dirty rows
- **THEN** no patch is sent for it until the block ends or times out
