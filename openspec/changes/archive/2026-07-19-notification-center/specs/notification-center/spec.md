## ADDED Requirements

### Requirement: Server-owned notification log

The server SHALL keep a bounded in-memory notification log: a ring buffer
(capacity 100) of entries carrying a monotonic id, kind, title, context, an
optional workspace/pane target, and a timestamp, plus a `last_seen_id`
high-water mark. Every notification that raises a transient toast SHALL also be
appended to the log through a single posting helper, so the log and the
displayed toasts cannot disagree. v1 sources SHALL be exactly the existing
toast sources: agent needs-attention, agent finished, update installed, and
`notification.show` injections. Transient toast behavior (single toast,
per-kind durations, click target) SHALL be unchanged.

#### Scenario: A toast is also logged

- **WHEN** an agent-finished toast is raised for a pane
- **THEN** the transient toast appears as today
- **AND** a matching entry (kind, title, context, pane target) is appended to
  the notification log with a new monotonic id

#### Scenario: The log is bounded

- **WHEN** more notifications than the capacity have been posted
- **THEN** the oldest entries are evicted and the newest capacity-many remain

#### Scenario: External injections are logged

- **WHEN** a client calls `notification.show`
- **THEN** the resulting notification is appended to the log like any other
  source

### Requirement: Notification API, event, and CLI

The server SHALL expose `notification.list` (entries newest-first plus the
unread count and the seen marker), `notification.mark_seen` (advance the
marker; idempotent), and `notification.clear` (empty the log; ids stay
monotonic) over the socket API, SHALL emit a `NotificationPosted` subscription
event when an entry is appended, and the CLI SHALL provide `herdr notification
list [--json]` and `herdr notification clear [--json]`. The protocol version
SHALL be bumped only if the source protocol is not already greater than the
latest released protocol.

#### Scenario: Listing returns unread count

- **WHEN** three notifications are posted and the marker is at the first
- **THEN** `notification.list` returns the entries newest-first with an unread
  count of 2

#### Scenario: Marking seen zeroes unread

- **WHEN** a client calls `notification.mark_seen` with unread entries present
- **THEN** the marker advances to the newest entry and a subsequent
  `notification.list` reports zero unread

#### Scenario: Subscribers see posted notifications

- **WHEN** a client is subscribed to events and a notification is posted
- **THEN** the client receives a `NotificationPosted` event for that entry

#### Scenario: Clearing empties the log

- **WHEN** a client calls `notification.clear` with entries present
- **THEN** the log becomes empty and a subsequent `notification.list` returns no
  entries with zero unread
- **AND** the next posted notification receives an id greater than any prior id

### Requirement: Top-right unread indicator

The TUI tab bar SHALL show a compact notification indicator in its trailing
region: an icon that is always visible as a click target, with an unread-count
pill shown only when the unread count is greater than zero. Clicking the
indicator SHALL toggle the notification panel. The indicator SHALL share the
trailing region non-destructively with the existing tab-bar controls so later
status modules can join it.

#### Scenario: Unread count appears

- **WHEN** two notifications arrive while the user is elsewhere
- **THEN** the indicator shows a count pill with 2

#### Scenario: Click toggles the panel

- **WHEN** the user clicks the indicator
- **THEN** the notification panel opens
- **AND** clicking the indicator again closes it

### Requirement: Notification panel with keyboard navigation

The TUI SHALL provide a notification panel listing the log newest-first
(kind-colored icon, title, context, relative time), opened by the indicator or
by a configurable `open_notification_center` keybinding registered in
`KeysConfig` and listed in the `prefix+?` help panel. Opening the panel SHALL
mark all notifications seen. Up/Down and `j`/`k` SHALL move the selection;
Enter SHALL jump to the selected notification's target pane (focusing its
workspace, tab, and pane via the same path as the existing toast click) and
close the panel; Esc and `q` SHALL close without jumping; clicking a row SHALL
jump to that row's target. Entries without a pane target SHALL not be
actionable. The panel SHALL show a footer "Clear all" button (also bound to
`c`) that empties the log through the same server operation as
`notification.clear` and leaves the panel open on the empty state. The panel
SHALL reuse the existing overlay/list UI language rather than introducing a
one-off surface.

#### Scenario: Clearing from the panel empties the log

- **WHEN** the user presses `c` or clicks the "Clear all" button with entries
  present
- **THEN** the log becomes empty and the unread indicator's pill disappears
- **AND** the panel stays open showing the empty state

#### Scenario: Opening marks seen

- **WHEN** the user opens the panel with unread notifications
- **THEN** the unread count becomes zero
- **AND** the indicator's count pill disappears

#### Scenario: Enter jumps to the target pane

- **WHEN** the user selects an agent-finished notification with Up/Down and
  presses Enter
- **THEN** the target workspace, tab, and pane are focused
- **AND** the panel closes

#### Scenario: Targetless entries are not actionable

- **WHEN** the selection is on an entry without a pane target and the user
  presses Enter
- **THEN** nothing is focused and the panel stays open
