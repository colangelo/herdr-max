# copy-mode-key-repeat

## MODIFIED Requirements

### Requirement: Held escape-coded keys repeat in copy mode

Herdr SHALL re-dispatch `KeyEventKind::Repeat` key events in copy mode, matching
the behavior already provided in terminal mode. The set of modes that honor key
repeat SHALL be defined by a single predicate used consistently across both the
live and headless input paths, at both the `Repeat`-dispatch decision and the
`Press`-time repeat-suppression decision, so that widening the mode set cannot
be defeated by the suppression bookkeeping.

Modes that honor key repeat SHALL be `Terminal`, `Copy`, and the
alternate-screen scroll passthrough mode, each of which exists to sustain a
held motion. All other modes (for example `Prefix`, `Navigate`, and
modal/dialog modes such as `ReleaseNotes` and `ConfirmClose`) SHALL continue to
ignore repeat events, so a held modal confirm/close key cannot fire multiple
times or leak repeats into a pane.

Each honoring mode SHALL hold a context distinct from the others, so that a
transition between two honoring modes changes the context and stops the
repeats, rather than letting a held key carry its repeats across the
transition. The passthrough mode's context SHALL NOT route to the terminal, so
its repeats re-dispatch through the app-level key handler that forwards the
scroll rather than being sent to the pane as raw key presses.

Plain (non-escape-coded) keys are unaffected: they arrive as repeated `Press`
events rather than `Repeat` events, so this requirement introduces no double-fire
for ordinary character keys.

#### Scenario: Held Ctrl-U repeats in copy mode

- **WHEN** the pane is in copy mode and the user holds `Ctrl-U` (half-page up),
  producing a `Press` followed by one or more `Repeat` events for that key
- **THEN** each `Repeat` event is dispatched to the copy-mode handler
- **AND** the viewport scrolls by a half page per event rather than only once

#### Scenario: Held scroll key repeats in the passthrough mode

- **WHEN** the alternate-screen scroll passthrough mode is active and the user
  holds `Ctrl-U`, producing a `Press` followed by two `Repeat` events
- **THEN** the application receives three scroll sends rather than one

#### Scenario: Held modal key does not repeat outside terminal/copy modes

- **WHEN** a modal mode (for example `ReleaseNotes`) is active and the user holds
  a key, producing `Press` then `Repeat` events
- **THEN** the `Repeat` events are ignored
- **AND** no repeat leaks into a terminal pane

#### Scenario: Repeat in copy mode routes to the copy-mode handler

- **WHEN** the headless input path receives a `Repeat` key event while the active
  mode is `Copy`
- **THEN** the event is routed to the non-terminal (copy-mode) key handler, not
  the terminal-passthrough handler
