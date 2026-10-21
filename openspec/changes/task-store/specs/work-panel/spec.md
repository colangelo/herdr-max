## ADDED Requirements

### Requirement: Read-only work panel

The client shell SHALL open a work panel popup as the `WorkPanel` overlay, from a
new keybinding action `open_work_panel` that is listed in the help panel. The
panel SHALL have four views: *This repo*, *Needs me*, *In flight* and *My tasks*.
Rows SHALL show the forge's canonical fields (title, state, labels, assignee,
last activity) from `issue.list`, with `last_sync` shown apart. The panel SHALL
get its data only from server methods. The client SHALL NOT hold NATS
credentials.

#### Scenario: rows match the forge

- **WHEN** the panel lists `uzi-sandbox` issues
- **THEN** each row's fields equal those the forge API returns for that issue,
  and the header shows the last sync time

#### Scenario: a server without the methods

- **WHEN** the panel opens on a server that does not advertise `issue.list`
- **THEN** the panel says the server lacks the work methods, and other servers
  stay connected

#### Scenario: under --remote

- **WHEN** ac opens the panel in a TUI attached with `--remote` to ac-mbm5
- **THEN** the rows come from ac-mbm5's server, and the m4m client reads nothing
  from the bus

### Requirement: My tasks joins tasks and asks rows

The *My tasks* view SHALL show agents' tasks from `task.list`, grouped by owner
and pane. It SHALL show ac's own task rows from `ask.list`. The server SHALL read
those rows from `ASKS`. Each task SHALL show its derived status. A task owned by a
host with no connection from this client SHALL show its actions as unavailable,
with the reason.

#### Scenario: the owner Mac sleeps

- **WHEN** ac selects an ac-mbm5 task and the client has no ac-mbm5 connection
- **THEN** the done and drop keys show "owner ac-mbm5 not reachable", and nothing
  is queued

### Requirement: Action previews

Each panel action (`c`, `s`, `a`, `x`, `u`, then `h`, `t` and `n`) SHALL be enabled
on its own. Each SHALL show a preview of the exact write, its destination and the
forge account it will appear as, before any confirmation.

#### Scenario: a disabled action

- **WHEN** `s` is not yet enabled and ac presses it
- **THEN** the panel says the action is not enabled, and no preview opens

### Requirement: The server never signs

The client SHALL hand the intent's fields to the signing helper on its own Mac.
The helper SHALL build the canonical envelope, show it, and sign only after a
Touch ID or password check. The client SHALL send `{envelope, signature}` to
`intent.submit`. The server SHALL verify the signature, SHALL NOT sign, and SHALL
report each dispatch state through `intent.changed`.

#### Scenario: ac declines the dialog

- **WHEN** ac cancels the presence dialog for a comment
- **THEN** no intent is submitted, and the panel shows "not confirmed"

#### Scenario: a platform without the helper

- **WHEN** the panel runs on Linux or Windows
- **THEN** every write action shows "confirmations off", and the rows stay
  readable

#### Scenario: an unknown outcome is not retried

- **WHEN** an intent reaches the `unknown` state
- **THEN** the panel shows "maybe executed", and only a new confirmation can
  replace it, naming it in `supersedes`

### Requirement: The task conflict overlay

The client shell SHALL show an outbox conflict in the `TaskConflict` overlay. The
overlay SHALL open from a task row marked `conflict`, or from its notification.
It SHALL show only the fields that differ, as three columns: base, mine (the
outbox entry) and theirs (the stored version), with who changed each and when.

#### Scenario: one field differs

- **WHEN** the outbox entry and the stored version differ only in `title`
- **THEN** the overlay shows one row, `title`, with its base, mine and theirs

### Requirement: Task conflict keys

In the `TaskConflict` overlay, `m` SHALL keep mine, `t` SHALL take theirs, `e`
SHALL open the task editor on a merge, and `Esc` SHALL close the overlay and leave
the conflict in place. Each choice SHALL call `task.resolve_conflict`. The keys
SHALL be listed in the overlay's help lines.

#### Scenario: taking theirs

- **WHEN** ac presses `t` on a conflict that has a queued follow-up edit
- **THEN** the entry is dropped, the follow-up comes back as its own conflict,
  and the row shows the stored version

#### Scenario: another client resolved it first

- **WHEN** ac presses `m` after another client has resolved the conflict
- **THEN** the overlay shows "already resolved" and refreshes

### Requirement: Work counts in the tab bar

The tab bar SHALL show a work count built from the `pane_tasks` resource fact. A
client whose server sends no `pane_tasks` SHALL show no work count.

#### Scenario: an older server

- **WHEN** the server sends `pane_todos` but no `pane_tasks`
- **THEN** the todo indicator works as before, and no work count appears
