# display-panes Specification

## Purpose
A tmux-style display-panes overlay: one key labels every pane of the current tab
with its number, address, name and size in characters, and shows the window size,
so a user can read pane sizes and jump to a pane by number without the API.

## Requirements

### Requirement: A key shows every pane's address and size

The `keys.display_panes` action (default `prefix+i`) SHALL show, over each pane
of the current tab, an index, the pane's public address, its name and its
terminal size in characters, and SHALL show the window size and the pane area in the mode
bar. It SHALL be listed in the keybinding help panel.

#### Scenario: Two panes are labelled

- **WHEN** a tab holds two panes and the user presses `prefix+i`
- **THEN** each pane shows `N  <address> · <name>  <cols>x<rows>` with N = 1 and 2, and
  the mode bar shows the window and pane-area sizes

#### Scenario: The action is discoverable

- **WHEN** the user opens the keybinding help
- **THEN** it lists the display panes action with its binding

### Requirement: The labels go away on their own or on any key

The labels SHALL close after 3 seconds, or on the next key or mouse press,
whichever comes first. A digit naming a shown pane SHALL focus that pane before
closing. Any other key SHALL close the labels and not reach the pane.

#### Scenario: Timeout

- **WHEN** 3 seconds pass with no input
- **THEN** the labels close and the app is back in terminal mode

#### Scenario: Focus by number

- **WHEN** the labels are shown and the user presses `2`
- **THEN** pane 2 is focused and the labels close

#### Scenario: Any other key

- **WHEN** the labels are shown and the user presses `x`
- **THEN** the labels close, focus does not change, and `x` is not sent to a pane
