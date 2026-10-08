# copy-mode-scroll-entry

## ADDED Requirements

### Requirement: One-gesture downward scroll

Herdr SHALL provide three prefix-level actions mirroring the upward entry
actions — `copy_mode_page_down` (default `prefix+pagedown`),
`copy_mode_half_page_down` (default `prefix+ctrl+d`), and `copy_mode_line_down`
(default `prefix+ctrl+j`) — that scroll the focused pane down by a full page,
half page, or one line respectively. The actions SHALL be configurable
`KeysConfig` entries (rebindable and unbindable), SHALL appear in the
`prefix+?` help panel, and SHALL be listed as commented entries in the
generated config template.

The actions SHALL share the upward actions' entry path rather than introducing
a second one, and SHALL work identically in the live and headless input paths.

When copy mode is already active on the focused pane, the actions SHALL scroll
its viewport down without re-entering copy mode, preserving the entry scroll
anchor exactly as the upward actions do.

When the focused pane's terminal is in the alternate screen, the actions SHALL
enter the alternate-screen scroll passthrough mode on that pane, forwarding one
`PageDown` key for the page and half-page actions and one wheel-down tick for
the line action, under the same encoding rules the upward entry uses.

When the focused pane is on the primary screen and copy mode is not active on
it, the actions SHALL do nothing: a live viewport has nothing below it, so
entering copy mode to scroll down would leave the user in a mode they did not
request.

#### Scenario: Down re-enters the passthrough on an alternate-screen pane

- **WHEN** the focused pane's application has the alternate screen active, no
  scroll mode is active, and the user presses the prefix followed by `Ctrl-D`
- **THEN** the application receives a `PageDown` key press
- **AND** the passthrough mode becomes active on that pane
- **AND** no copy-mode state exists

#### Scenario: The line variant sends a wheel-down tick

- **WHEN** the focused pane's application has the alternate screen active and
  the user presses the prefix followed by `Ctrl-J`
- **THEN** the application receives one wheel-down tick under the same encoding
  rules as an upward tick
- **AND** the passthrough mode becomes active on that pane

#### Scenario: Returning to a scrolled-up application goes down directly

- **WHEN** the passthrough mode has ended while the pane's application is still
  scrolled up, and the user invokes one of the downward actions
- **THEN** the application scrolls down
- **AND** the user does not have to scroll up first

#### Scenario: Down scrolls an active copy mode

- **WHEN** copy mode is active on the focused pane, scrolled up, and the user
  invokes one of the downward actions
- **THEN** the copy-mode viewport scrolls down by the requested amount
- **AND** the entry scroll anchor is unchanged

#### Scenario: Down does nothing on an unscrolled primary-screen pane

- **WHEN** the focused pane is on the primary screen, copy mode is not active on
  it, and the user invokes one of the downward actions
- **THEN** copy mode is not entered and nothing changes

#### Scenario: No focused pane is a safe no-op

- **WHEN** no pane is focused and the user invokes one of the downward actions
- **THEN** nothing changes and no error is raised
