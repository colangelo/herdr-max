## ADDED Requirements

### Requirement: Sync mode types into every pane of the tab

While sync mode is on for a tab, text typed into the focused pane of that tab
SHALL also be sent to every other pane of that tab that is in the synced set
(an explicit set of members). Turning the mode on for the whole tab (the key,
`tab.sync on`, or "Sync input" on the focused pane) SHALL make every pane of
the tab a member, and a pane created in the tab while that group is on SHALL
join it. "Sync input" in the menu of a pane other than the focused one SHALL
start a group of exactly those two panes; a pane created while such a group is
on SHALL NOT join it.

#### Scenario: Three panes

- **WHEN** a tab has panes A, B and C, sync is on, and `ls` Enter is typed into A
- **THEN** B and C receive `ls` Enter as well

#### Scenario: Another tab is untouched

- **WHEN** sync is on in tab 1 and a pane in tab 2 is focused and typed into
- **THEN** only that pane receives the input

### Requirement: A pair can be started from the pane menu

The pane menu SHALL offer "Sync input". On the focused pane it SHALL start a
whole-tab group; on any other pane it SHALL start a group of the focused pane
and that pane only.

#### Scenario: Two panes of three

- **WHEN** panes A, B and C exist, A is focused, and "Sync input" is chosen in
  the menu of B
- **THEN** the group is {A, B}, typing into A reaches B and not C, and C has a
  gray border

#### Scenario: Whole tab from the focused pane

- **WHEN** "Sync input" is chosen in the menu of the focused pane A
- **THEN** the group is every pane of the tab

### Requirement: A pane can be taken out of the set

While sync is on, a right click on a pane SHALL take it out of the set, or put
it in when it is out, whichever pane holds the cursor. A group of one pane SHALL
stay in sync mode.

#### Scenario: Opt out

- **WHEN** sync is on and pane B is right-clicked
- **THEN** typing into A reaches C but not B, and B's border is no longer yellow

#### Scenario: Typing into an excluded pane

- **WHEN** B is excluded and is focused
- **THEN** typing reaches B alone

### Requirement: Keys are encoded per target pane

Each synced pane SHALL receive input encoded for its own keyboard and paste
state. herdr's own keys, mouse events, scrolling and focus reports SHALL NOT be
sent to the other panes.

#### Scenario: Prefix key

- **WHEN** the prefix chord is typed with sync on
- **THEN** herdr handles it and no pane receives it

### Requirement: An empty group ends sync after a grace

When the last member is taken out, the tab SHALL stay in sync mode for about
three seconds, show `SYNC ending…`, and then return to normal mode. A right
click on a pane during the grace SHALL put it back and cancel the countdown.
After the grace, a right click SHALL open the pane menu again.

#### Scenario: Grace

- **WHEN** the last member is right-clicked out and three seconds pass
- **THEN** the tab is not syncing and a right click opens the pane menu

#### Scenario: Re-add

- **WHEN** the last member is right-clicked out and, one second later, a pane
  is right-clicked
- **THEN** that pane is the group again and the tab stays in sync mode

### Requirement: Sync is visible

While sync is on, the borders of the synced panes and the bottom bar SHALL be
bright yellow (`#FFD60A`), and a pane outside the group SHALL have a gray
border, distinct from a normal border.

#### Scenario: Hint

- **WHEN** sync turns on for the tab on screen
- **THEN** every synced pane border and the bottom bar are `#FFD60A`, and any
  pane outside the group has a gray border

### Requirement: Sync is shared, runtime-only state

Sync mode SHALL be server state shared by every attached client, available
through the JSON API and CLI, and SHALL NOT be saved in the session snapshot or
carried through a live handoff.

#### Scenario: Restart

- **WHEN** the server restarts or hands off with sync on
- **THEN** no tab is syncing afterwards
