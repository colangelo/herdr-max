## ADDED Requirements

### Requirement: Break focused pane to a new tab

The TUI SHALL provide an action that moves the focused pane out of its current tab into a newly created tab within the same workspace, delegating to the existing `pane.move` operation with a `new_tab` destination. The action SHALL be bound by default to `prefix+!` and SHALL be exposed as a configurable entry in `KeysConfig` so users can rebind or unbind it.

The action SHALL preserve the pane's running process and terminal. After the move, the newly created tab SHALL become the active tab and the moved pane SHALL be focused within it, unless the pane is the only pane in its source tab.

#### Scenario: Break a pane from a multi-pane tab

- **WHEN** the focused tab contains more than one pane and the user triggers the break-pane action
- **THEN** the focused pane is removed from its current tab and placed as the sole pane of a new tab in the same workspace
- **AND** the running process and terminal contents of that pane are preserved
- **AND** the new tab becomes active with the moved pane focused
- **AND** the source tab remains with its remaining panes re-laid out

#### Scenario: Break the only pane in a tab is a no-op

- **WHEN** the focused tab contains exactly one pane and the user triggers the break-pane action
- **THEN** no new tab is created and the pane stays in place
- **AND** the user is shown a non-blocking indication that there is nothing to break out

#### Scenario: Break is rejected while the tab is zoomed

- **WHEN** the focused tab is zoomed and the user triggers the break-pane action
- **THEN** the pane is not moved
- **AND** the user is shown a non-blocking indication that the action is unavailable while zoomed

### Requirement: Move focused pane to another tab via a picker

The TUI SHALL provide an action that opens a modal picker listing the other tabs in the current workspace, and on selection moves the focused pane into the chosen tab via the existing `pane.move` operation with a `tab` destination and a default split direction. The picker SHALL reuse the existing modal/picker UI language rather than introducing a bespoke screen. The action SHALL be bound by default to `prefix+m` and SHALL be a configurable `KeysConfig` entry.

The picker SHALL list only valid destination tabs: it SHALL exclude the pane's current tab and SHALL exclude tabs that cannot receive the pane. If there are no valid destination tabs, the action SHALL not open an empty picker.

#### Scenario: Move a pane into a selected tab

- **WHEN** the workspace has at least one other tab and the user triggers the move-to-tab action
- **THEN** a modal picker opens listing the other tabs in the workspace
- **AND** selecting a tab moves the focused pane into that tab with a default split next to the target tab's focused pane
- **AND** the pane's running process and terminal contents are preserved

#### Scenario: Cancel the picker leaves layout unchanged

- **WHEN** the move-to-tab picker is open and the user dismisses it without selecting
- **THEN** no pane is moved and the layout is unchanged

#### Scenario: No other tabs disables the action

- **WHEN** the current workspace has only one tab and the user triggers the move-to-tab action
- **THEN** the picker does not open
- **AND** the user is shown a non-blocking indication that there is no other tab to move into

### Requirement: Quick move focused pane to the adjacent tab

The TUI SHALL provide two actions that move the focused pane into the next tab and the previous tab within the current workspace, each using the existing `pane.move` operation with a `tab` destination and a default split, without any intermediate picker. Both actions SHALL be configurable `KeysConfig` entries with default bindings.

Adjacency SHALL follow the workspace's current tab order. When there is no adjacent tab in the requested direction, the action SHALL be a no-op with a non-blocking indication rather than wrapping around or creating a tab.

#### Scenario: Move a pane to the next tab

- **WHEN** a tab exists after the current tab and the user triggers the move-to-next-tab action
- **THEN** the focused pane is moved into that next tab with a default split
- **AND** the pane's running process and terminal contents are preserved

#### Scenario: Move a pane to the previous tab

- **WHEN** a tab exists before the current tab and the user triggers the move-to-previous-tab action
- **THEN** the focused pane is moved into that previous tab with a default split

#### Scenario: No adjacent tab is a no-op

- **WHEN** there is no tab in the requested direction and the user triggers the quick-move action
- **THEN** no pane is moved and no tab is created
- **AND** the user is shown a non-blocking indication that there is no adjacent tab

### Requirement: Pane-move actions delegate to the existing runtime operation

The new pane-move actions SHALL be implemented entirely in the TUI/client input layer and SHALL invoke the existing `pane.move` API path. This change SHALL NOT add a new socket message, SHALL NOT change the `pane.move` request or response schema, and SHALL NOT bump the protocol version.

Failures returned by `pane.move` (for example a rejected move because a source or destination tab is zoomed) SHALL be surfaced to the user as a non-blocking indication, and the layout SHALL remain in its pre-action state.

#### Scenario: No protocol surface is added

- **WHEN** the change is implemented
- **THEN** no new `Method`/`Request` variant is introduced for pane movement
- **AND** the protocol version is unchanged

#### Scenario: A rejected move is surfaced without corrupting layout

- **WHEN** a pane-move action invokes `pane.move` and the operation returns a rejection
- **THEN** the pane remains where it was
- **AND** the user is shown a non-blocking indication describing why the move did not happen
