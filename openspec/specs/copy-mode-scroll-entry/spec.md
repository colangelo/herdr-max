# copy-mode-scroll-entry Specification

## Purpose
Define the one-gesture scroll actions — `copy_mode_page_up`,
`copy_mode_half_page_up`, `copy_mode_line_up`, which enter copy mode on the
focused pane already scrolled up (tmux `copy-mode -u`), and their downward
mirrors `copy_mode_page_down`, `copy_mode_half_page_down`,
`copy_mode_line_down` — their repeat-gesture continuation and exit-restore
semantics, the alternate-screen scroll
passthrough mode those gestures divert to when the focused pane's application
owns the alternate screen and has no scrollback to enter, and the preserved
send-prefix precedence that keeps `prefix+prefix` as literal passthrough.

## Requirements

### Requirement: One-gesture copy-mode entry with upward scroll

Herdr SHALL provide three prefix-level actions — `copy_mode_page_up` (default
`prefix+pageup`), `copy_mode_half_page_up` (default `prefix+ctrl+u`), and
`copy_mode_line_up` (default `prefix+ctrl+k`) — that, in a single gesture,
enter copy mode on the focused pane and scroll the viewport up by a full page,
half page, or one line respectively (tmux `copy-mode -u` semantics). The
actions SHALL be configurable `KeysConfig` entries (rebindable and unbindable),
SHALL appear in the `prefix+?` help panel, and SHALL be listed as commented
entries in the generated config template.

The actions SHALL delegate to the existing copy-mode entry and scroll
primitives rather than introducing new scroll machinery, and SHALL work
identically in the live and headless input paths.

When the focused pane's terminal is in the alternate screen, the actions SHALL
NOT enter copy mode and SHALL instead enter the alternate-screen scroll
passthrough mode on that pane (see `Alternate-screen scroll passthrough mode`).
Copy-mode entry semantics below apply to panes on the primary screen. If the
pane's terminal state cannot be resolved, the actions SHALL fall back to
copy-mode entry.

#### Scenario: Page up from terminal mode

- **WHEN** the user is in terminal mode with scrollback available and presses
  the prefix followed by `PageUp`
- **THEN** the focused pane enters copy mode
- **AND** the viewport is scrolled up by one page in the same gesture

#### Scenario: Half-page and line variants

- **WHEN** the user presses the prefix followed by `Ctrl-U` (or `Ctrl-K`)
- **THEN** the focused pane enters copy mode scrolled up by half a page (or one
  line)

#### Scenario: No scrollback still enters copy mode

- **WHEN** the focused pane is on the primary screen, has no scrollback above
  the viewport, and the user invokes one of the actions
- **THEN** the pane enters copy mode with the viewport unchanged

#### Scenario: Alternate screen diverts to passthrough

- **WHEN** the focused pane's application has the alternate screen active and
  the user invokes one of the actions
- **THEN** the pane does not enter copy mode
- **AND** the alternate-screen scroll passthrough mode becomes active on that
  pane

#### Scenario: No focused pane is a safe no-op

- **WHEN** no pane can enter copy mode (no focused pane, or a zero-size pane)
  and the user invokes one of the actions
- **THEN** nothing changes and no error is raised

### Requirement: Repeat gesture continues scrolling without re-entry

When copy mode is already active on the focused pane, invoking any of the three
actions SHALL scroll further without re-entering copy mode: the copy-mode state
— in particular the entry scroll anchor (`entry_offset_from_bottom`) used to
restore the viewport on exit — SHALL be preserved. The actions SHALL be members
of the set of prefix actions that do not cancel active copy mode.

Exit semantics SHALL be unchanged: leaving copy mode without copying restores
the scroll position captured when copy mode was first entered.

#### Scenario: Two page-up gestures accumulate

- **WHEN** the user performs `prefix+PageUp` twice in a row on a pane with
  ample scrollback
- **THEN** after the second gesture the viewport is two pages up
- **AND** the copy-mode entry scroll anchor still reflects the position before
  the first gesture

#### Scenario: Exit restores the pre-gesture position

- **WHEN** the user enters via `prefix+PageUp` (one or more times) and then
  exits copy mode without copying
- **THEN** the viewport returns to the position it had before the first
  gesture

### Requirement: Send-prefix precedence is preserved

The `prefix+prefix` send-prefix behavior (pressing the prefix chord again in
prefix mode passes the literal prefix key through to the focused pane) SHALL
take precedence over these bindings and remain unchanged. Consequently
`prefix+ctrl+b` SHALL NOT be a default binding for any of the new actions
(with the default `ctrl+b` prefix it is shadowed by send-prefix); the config
template comment SHALL note that users with a different prefix may bind it
manually.

#### Scenario: Double prefix still passes through

- **WHEN** the user presses the prefix chord twice with the default `ctrl+b`
  prefix
- **THEN** a literal `Ctrl-B` is sent to the focused pane
- **AND** no copy-mode scroll action fires

### Requirement: Alternate-screen scroll passthrough mode

When a copy-mode scroll entry action fires on a pane whose terminal is in the
alternate screen, Herdr SHALL enter a scroll passthrough mode pinned to that
pane. On entry, the page and half-page actions SHALL forward one `PageUp` key
to the pane's application; the line action SHALL forward one wheel-up tick as
specified below.

While the mode is active, Herdr SHALL forward scroll intents to the pinned
pane's application: `Ctrl-U` and `PageUp` SHALL send `PageUp`; `Ctrl-D` and
`PageDown` SHALL send `PageDown`; `g` and `Home` SHALL send `Home`; `Shift-G`
and `End` SHALL send `End`. Forwarded keys SHALL be encoded through the pane's
terminal input state (the same encoding a physical key press would receive).

For line-granular scrolling, `Ctrl-K`, `k`, and `Up` SHALL send one wheel-up
tick, and `Ctrl-J`, `j`, and `Down` one wheel-down tick. A wheel tick SHALL be
encoded exactly as a physical wheel event over the pane would be: a mouse
report at the pane's centre when the application captures the mouse,
alternate-scroll arrow keys when the pane is in alternate-scroll mode
(DECSET 1007), and dropped without effect when the pane supports neither.
Arrow keys SHALL NOT be forwarded as key presses.

`Esc`, `q`, and `Enter` SHALL exit the mode back to terminal input without
forwarding anything. The prefix chord SHALL enter prefix mode, as it does from
copy mode. All other keys SHALL be swallowed without reaching the application.

The mode SHALL NOT scroll the pane's own viewport or create copy-mode state,
and exiting SHALL NOT move the viewport. If the pinned pane stops being the
focused pane, or its runtime can no longer be resolved, the next key SHALL exit
the mode instead of forwarding. While the mode is active the focused pane SHALL
visibly indicate it, naming the exit key.

Plain copy-mode entry (`copy_mode`) SHALL remain unchanged on alternate-screen
panes, entering ordinary copy mode over the visible screen.

#### Scenario: Half-page gesture pages an alt-screen application

- **WHEN** the focused pane's application has the alternate screen active and
  the user presses the prefix followed by `Ctrl-U`
- **THEN** the application receives a `PageUp` key press
- **AND** the passthrough mode is active on that pane
- **AND** no copy-mode state exists

#### Scenario: Repeat scrolling stays on the home row

- **WHEN** the passthrough mode is active and the user presses `Ctrl-U` twice
  and then `Ctrl-D`
- **THEN** the application receives `PageUp`, `PageUp`, then `PageDown`
- **AND** the mode remains active throughout

#### Scenario: Line keys scroll a mouse-capturing application

- **WHEN** the passthrough mode is active on a pane whose application captures
  the mouse and the user presses `Ctrl-K` and then `Ctrl-J`
- **THEN** the application receives a wheel-up and then a wheel-down mouse
  report positioned inside the pane

#### Scenario: Line keys respect alternate-scroll mode

- **WHEN** the passthrough mode is active on a pane in alternate-scroll mode
  (DECSET 1007) without mouse capture and the user presses `Ctrl-K`
- **THEN** the application receives the alternate-scroll arrow encoding for one
  wheel-up tick

#### Scenario: Line keys drop where no wheel path exists

- **WHEN** the passthrough mode is active on a pane with mouse capture off and
  alternate-scroll disabled and the user presses `Ctrl-K`
- **THEN** the application receives nothing and the mode remains active

#### Scenario: Exit returns keys to the application

- **WHEN** the passthrough mode is active and the user presses `Esc`
- **THEN** the mode ends and no key is forwarded for the `Esc`
- **AND** subsequent typed keys reach the application as ordinary terminal
  input

#### Scenario: Other keys are swallowed

- **WHEN** the passthrough mode is active and the user presses a key outside
  the scroll and exit vocabulary
- **THEN** the application receives nothing and the mode remains active

#### Scenario: Primary-screen panes are unaffected

- **WHEN** the focused pane is on the primary screen and the user invokes a
  copy-mode scroll entry action
- **THEN** copy mode is entered exactly as specified by `One-gesture copy-mode
  entry with upward scroll`

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
