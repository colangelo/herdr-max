# pane-clear-scrollback Specification

## Purpose
Define on-demand purging of a pane's saved scrollback with tmux
`clear-history` semantics: the visible screen, terminal modes, and the running
process are untouched, and the purge is delivered by injecting `CSI 3J` through
the existing pane write path rather than a full terminal reset. Covers the
keybinding, the socket API method, and the CLI that reach it, and the rule that
a Herdr-originated clear is not dropped by passthrough filtering.
## Requirements
### Requirement: Clear a pane's saved scrollback

Herdr SHALL provide the ability to purge a pane's saved scrollback buffer on demand while leaving the pane's visible screen contents and its running process untouched. The semantics SHALL match tmux `clear-history` (scrollback-only purge), not a full terminal reset.

The purge SHALL be implemented by injecting the `CSI 3J` (`ESC[3J`) sequence into the pane's terminal through the existing pane write path, so it uses the same mechanism the embedded emulator already honors from programs. The implementation SHALL NOT use the full terminal reset (RIS / `ghostty_terminal_reset`), which would also clear screen contents, modes, and the scroll region.

#### Scenario: Scrollback is purged, visible screen preserved

- **WHEN** a pane has saved scrollback and a clear-scrollback action is invoked for that pane
- **THEN** the pane's saved scrollback is emptied
- **AND** the pane's currently visible screen contents are unchanged
- **AND** the running process in the pane is not signaled or restarted

#### Scenario: Clear on an empty scrollback is a safe no-op

- **WHEN** a pane has no saved scrollback and a clear-scrollback action is invoked
- **THEN** the action completes successfully with no visible change and no error

### Requirement: Clear scrollback via keybinding

The TUI SHALL expose a `ClearScrollback` navigation action targeting the focused pane, registered as a configurable entry in `KeysConfig` so users can bind, rebind, or unbind it. Whether it ships with a default binding SHALL be resolved in design; if bound by default, the chosen chord SHALL be documented in the keyboard reference.

#### Scenario: Clearing the focused pane from the keyboard

- **WHEN** the user triggers the clear-scrollback action
- **THEN** the focused pane's saved scrollback is purged
- **AND** the visible screen and running process are preserved

### Requirement: Clear scrollback via socket API

Herdr SHALL expose a `pane.clear` socket method that purges the saved scrollback of a target pane identified by pane id. This is an additive protocol method; the protocol version SHALL be bumped only if the source protocol is not already ahead of the latest released tag.

#### Scenario: Clearing a pane over the socket

- **WHEN** a client sends a `pane.clear` request for a valid pane id
- **THEN** that pane's saved scrollback is purged
- **AND** a success response is returned

#### Scenario: Clearing an unknown pane returns an error

- **WHEN** a client sends a `pane.clear` request for a pane id that does not exist
- **THEN** the request fails with a not-found error and no other pane is affected

### Requirement: Clear scrollback via CLI

The CLI SHALL provide a `herdr pane clear` subcommand that clears the scrollback of a target pane. It SHALL accept an explicit pane id and SHALL support selecting the current pane (for example via `--current`), consistent with other `herdr pane` subcommands. It SHALL send the `pane.clear` request over the session socket.

#### Scenario: Clearing a pane by id from the CLI

- **WHEN** the user runs `herdr pane clear <pane_id>`
- **THEN** the CLI sends a `pane.clear` request for that pane and reports success

#### Scenario: Clearing the current pane from the CLI

- **WHEN** the user runs `herdr pane clear --current` from inside a pane
- **THEN** the CLI resolves the current pane and clears its scrollback

### Requirement: Herdr-originated clear is not suppressed by passthrough filtering

Herdr strips `CSI 3J` emitted by the `droid` foreground job on the primary screen (an existing compatibility behavior). The clear-scrollback action is an explicit herdr-originated clear and SHALL take effect regardless of that passthrough filter, so the action is not silently dropped when `droid` is the foreground job.

#### Scenario: Clear works even when droid is foreground

- **WHEN** a pane's foreground job is `droid` on the primary screen and the clear-scrollback action is invoked for that pane
- **THEN** the pane's saved scrollback is purged
- **AND** the droid-specific passthrough filter does not suppress the herdr-originated clear

