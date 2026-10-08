## ADDED Requirements

### Requirement: Held escape-coded keys repeat in copy mode

Herdr SHALL re-dispatch `KeyEventKind::Repeat` key events in copy mode, matching
the behavior already provided in terminal mode. The set of modes that honor key
repeat SHALL be defined by a single predicate (`Mode::honors_key_repeat()`) used
consistently across both the live and headless input paths, at both the
`Repeat`-dispatch decision and the `Press`-time repeat-suppression decision, so
that widening the mode set cannot be defeated by the suppression bookkeeping.

Modes that honor key repeat SHALL be `Terminal` and `Copy`. All other modes
(for example `Prefix`, `Navigate`, and modal/dialog modes such as `ReleaseNotes`
and `ConfirmClose`) SHALL continue to ignore repeat events, so a held modal
confirm/close key cannot fire multiple times or leak repeats into a pane.

Plain (non-escape-coded) keys are unaffected: they arrive as repeated `Press`
events rather than `Repeat` events, so this requirement introduces no double-fire
for ordinary character keys.

#### Scenario: Held Ctrl-U repeats in copy mode

- **WHEN** the pane is in copy mode and the user holds `Ctrl-U` (half-page up),
  producing a `Press` followed by one or more `Repeat` events for that key
- **THEN** each `Repeat` event is dispatched to the copy-mode handler
- **AND** the viewport scrolls by a half page per event rather than only once

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

### Requirement: Ctrl-K / Ctrl-J viewport scroll in copy mode

Copy mode SHALL provide line-wise viewport scrolling bound to `Ctrl-K` and
`Ctrl-J`, modeled on vim `Ctrl-Y` / `Ctrl-E`. `Ctrl-K` SHALL scroll the window up
through the scrollback (revealing older lines), and `Ctrl-J` SHALL scroll the
window back toward the live bottom. Both SHALL keep the cursor anchored to the
same buffer text (the cursor SHALL NOT move relative to content) while the cursor
remains within the viewport; if a scroll would move the cursor off-screen, the
cursor SHALL stick to the viewport edge. Both motions SHALL be repeatable while
held, relying on the copy-mode key-repeat behavior above.

#### Scenario: Ctrl-K reveals older lines without moving the cursor

- **WHEN** the pane is in copy mode with scrollback available above the viewport
  and the user presses `Ctrl-K`
- **THEN** the viewport scrolls up by one line, revealing one older line at the top
- **AND** the cursor stays on the same buffer text (its buffer row is unchanged)
  as long as it remains within the viewport

#### Scenario: Ctrl-J scrolls back toward the bottom

- **WHEN** the pane is in copy mode scrolled up into history and the user presses
  `Ctrl-J`
- **THEN** the viewport scrolls down by one line, toward the live bottom
- **AND** the cursor stays on the same buffer text while it remains on screen

#### Scenario: Ctrl-J at the bottom is a no-op

- **WHEN** the pane is in copy mode with the viewport already at the live bottom
  and the user presses `Ctrl-J`
- **THEN** nothing scrolls and the cursor does not move

#### Scenario: Ctrl-K at the top of history is a no-op

- **WHEN** the pane is in copy mode with the viewport already at the oldest
  available scrollback and the user presses `Ctrl-K`
- **THEN** nothing scrolls and the cursor does not move
