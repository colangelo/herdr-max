## ADDED Requirements

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

- **WHEN** the focused pane has no scrollback above the viewport and the user
  invokes one of the actions
- **THEN** the pane enters copy mode with the viewport unchanged

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
