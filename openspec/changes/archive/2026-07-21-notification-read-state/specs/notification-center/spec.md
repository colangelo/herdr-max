# notification-center

## MODIFIED Requirements

### Requirement: Server-owned notification log

The server SHALL keep a bounded in-memory notification log: a ring buffer
(capacity 100) of entries carrying a monotonic id, kind, title, context, an
optional workspace/pane target, a timestamp, and a per-entry `read` flag
(false when posted). Every notification that raises a transient toast SHALL
also be appended to the log through a single posting helper, so the log and
the displayed toasts cannot disagree. The unread count SHALL be the number of
entries whose `read` flag is false. v1 sources SHALL be exactly the existing
toast sources: agent needs-attention, agent finished, update installed, and
`notification.show` injections. Transient toast behavior (single toast,
per-kind durations, click target) SHALL be unchanged.

#### Scenario: A toast is also logged

- **WHEN** an agent-finished toast is raised for a pane
- **THEN** the transient toast appears as today
- **AND** a matching entry (kind, title, context, pane target) is appended to
  the notification log with a new monotonic id, unread

#### Scenario: The log is bounded

- **WHEN** more notifications than the capacity have been posted
- **THEN** the oldest entries are evicted and the newest capacity-many remain

#### Scenario: External injections are logged

- **WHEN** a client calls `notification.show`
- **THEN** the resulting notification is appended to the log like any other
  source

#### Scenario: Reading one entry decrements unread

- **WHEN** three unread entries exist and one is marked read
- **THEN** the unread count is 2 and the other entries stay unread

### Requirement: Notification API, event, and CLI

The server SHALL expose `notification.list` (entries newest-first, each
carrying its `read` flag, plus the unread count), `notification.mark_seen`
(with an optional entry `id`: present marks that entry read, absent marks all
entries read; idempotent either way), and `notification.clear` (empty the
log; ids stay monotonic) over the socket API, SHALL emit a
`NotificationPosted` subscription event when an entry is appended, and the
CLI SHALL provide `herdr notification list [--json]` (showing each entry's
read state) and `herdr notification clear [--json]`. The protocol version
SHALL be bumped only if the source protocol is not already greater than the
latest released protocol.

#### Scenario: Listing returns unread count

- **WHEN** three notifications are posted and one has been marked read
- **THEN** `notification.list` returns the entries newest-first, each with its
  `read` flag, and an unread count of 2

#### Scenario: Marking one entry read

- **WHEN** a client calls `notification.mark_seen` with an entry `id`
- **THEN** only that entry becomes read and the unread count decreases by at
  most one

#### Scenario: Marking seen zeroes unread

- **WHEN** a client calls `notification.mark_seen` without an `id` and unread
  entries are present
- **THEN** every entry becomes read and a subsequent `notification.list`
  reports zero unread

#### Scenario: Subscribers see posted notifications

- **WHEN** a client is subscribed to events and a notification is posted
- **THEN** the client receives a `NotificationPosted` event for that entry

#### Scenario: Clearing empties the log

- **WHEN** a client calls `notification.clear` with entries present
- **THEN** the log becomes empty and a subsequent `notification.list` returns no
  entries with zero unread
- **AND** the next posted notification receives an id greater than any prior id

## REMOVED Requirements

### Requirement: Notification panel with keyboard navigation

Replaced by "Notification panel with keyboard navigation and read tracking":
the open-marks-all-seen model is gone and most scenarios changed meaning, so
the requirement is rewritten wholesale rather than scenario-patched.

## ADDED Requirements

### Requirement: Notification panel with keyboard navigation and read tracking

The TUI SHALL provide a notification panel listing the log newest-first,
opened by the indicator or by a configurable `open_notification_center`
keybinding registered in `KeysConfig` and listed in the `prefix+?` help
panel. Opening the panel SHALL NOT change any entry's read state. Unread
entries SHALL render with their kind-colored icon and a bold title; read
entries SHALL render with a blank icon column and a regular-weight title in
the muted (dim) foreground, keeping column alignment. The selected row
SHALL keep its accent highlight band while its icon column and title weight
continue to reflect read state, so a selected row stays distinguishable as
read or unread. Up/Down and `j`/`k` SHALL move the
selection; Enter SHALL mark the selected entry read, jump to its target pane
(focusing its workspace, tab, and pane via the same path as the existing
toast click), and close the panel; clicking a row SHALL do the same for that
row; Esc and `q` SHALL close without changing read state. Entries without a
pane target SHALL not be actionable and SHALL stay unread. `r` SHALL mark
all entries read while keeping the log, leaving the panel open. The panel
footer SHALL use the settings-panel button language — filled boxes with the
shortcut hint inside, right-aligned: `c clear all` and `esc close`, plus
`r mark read` when the panel is wide enough — where `c`/"clear all" empties
the log through the same server operation as `notification.clear` and leaves
the panel open on the empty state.

#### Scenario: Clearing from the panel empties the log

- **WHEN** the user presses `c` or clicks the "clear all" button with entries
  present
- **THEN** the log becomes empty and the unread indicator's pill disappears
- **AND** the panel stays open showing the empty state

#### Scenario: Opening leaves unread intact

- **WHEN** the user opens the panel with three unread notifications
- **THEN** the unread count stays 3 and the indicator's count pill still
  shows 3

#### Scenario: Activating an entry marks it read

- **WHEN** the user presses Enter on an unread agent-finished notification
- **THEN** the target workspace, tab, and pane are focused, the panel closes,
  that entry is read, and the unread count decreases by one
- **AND** reopening the panel shows that entry quiet (no icon, dim regular
  title) while unvisited entries keep their dot and bold title

#### Scenario: Mark all read keeps history

- **WHEN** the user presses `r` with unread entries present
- **THEN** every entry becomes read, the pill disappears, the log still lists
  all entries, and the panel stays open

#### Scenario: Targetless entries are not actionable

- **WHEN** the selection is on an entry without a pane target and the user
  presses Enter
- **THEN** nothing is focused, the panel stays open, and the entry stays
  unread
