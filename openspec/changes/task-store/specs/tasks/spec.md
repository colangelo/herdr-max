## ADDED Requirements

### Requirement: Task store backends

The server SHALL keep tasks behind a `TaskStore` trait. The `file` backend SHALL be
the default, and SHALL store tasks under the session's data dir. The `kv` backend
(NATS JetStream KV bucket `TASKS`) SHALL compile only with the cargo feature
`tasks-kv`, which is off by default.

#### Scenario: a default build has no bus code

- **WHEN** herdr is built without the `tasks-kv` feature
- **THEN** the binary links no NATS client, and `task.add` stores the task in the
  session's data dir

### Requirement: Task record

A task SHALL be stored under `task.<owner_host>.<id>`. Its `title` SHALL be 1 to
200 characters, and its `body` at most 4000. Its stored `status` SHALL be one of
`open`, `running`, `done` or `dropped`. Its `owner` SHALL be
`agent {name, session_ref}` or `unassigned`, never ac. `runs` SHALL keep the last
20 run ids. A task SHALL never be deleted: `dropped` is its end. At most 50 open
tasks SHALL be bound to one pane.

#### Scenario: a title that is too long is refused

- **WHEN** `task.add` carries a 201-character title
- **THEN** the server answers `task_title_too_long` and stores nothing

### Requirement: The server never waits on the store

The server SHALL answer every `task.*` call from an in-memory cache, fed by a
watch of the store. Every store call SHALL run in a background store task. A
write SHALL answer `pending` with its `write_id` at once. Its outcome SHALL be
published later as `task.changed {write_id, outcome}`. Render, detection and
frame fanout SHALL NOT call the store.

#### Scenario: the bus is slow

- **WHEN** the `kv` backend takes 10 seconds to answer a write
- **THEN** `task.update` returns `pending` at once, panes keep rendering, and
  `task.changed` reports the outcome when it arrives

#### Scenario: the CLI waits for the outcome

- **WHEN** `herdr-beta task done <id>` gets no outcome within its timeout
- **THEN** it prints "pending sync" and exits 0

### Requirement: One writer per task

Only the server whose session is named in `[tasks] bus_session` SHALL run the `kv`
backend and read the host's NATS credential. A server SHALL refuse to write a task
whose `owner_host` is not its own `[tasks] host_id`, with
`task_owned_elsewhere {owner_host}`, and SHALL NOT queue that write. A server that
is not the bus session SHALL refuse task writes with
`task_not_bus_session {bus_session}`, unless it runs its own `file` backend.

#### Scenario: an edit to the other Mac's task

- **WHEN** `task.done` on m4m targets `task.ac-mbm5.tk-1`
- **THEN** the answer is `task_owned_elsewhere {owner_host: "ac-mbm5"}`, and the
  outbox is unchanged

#### Scenario: a throwaway session

- **WHEN** a server started with `--session throwaway` receives `task.add`, and
  `bus_session` names the default session
- **THEN** it answers `task_not_bus_session` and reads no NATS credential

### Requirement: Stable pane keys and pane bindings

Every terminal SHALL have a `pane_key` that is made once and survives restore,
move and live handoff. It SHALL be saved in `PaneSnapshot` and exposed as an
optional `PaneInfo` field. A task's `pane` binding SHALL be
`{host, session, pane_key, label}` on the owner's host and bus session. A
`related_pane` binding MAY name another host, and then only shows its label.

#### Scenario: a binding survives a restart

- **WHEN** a task is bound to a pane and the server restarts
- **THEN** the pane gets a new raw id, and the task still resolves to it by
  `pane_key`

#### Scenario: a pane on another host is refused

- **WHEN** `task.add --pane` names a pane on another host
- **THEN** the answer is `task_pane_elsewhere`, and no task is created

### Requirement: Compare-and-set writes

Every write SHALL carry a `write_id`. The record SHALL keep the last 8 applied ids,
plus any id that a pending entry depends on. A user edit SHALL carry `expected_rev`
and SHALL fail with `conflict` and both versions when the revision has moved. Only
these SHALL rebase on their own: appending a run id, a status move whose
precondition still holds, and refreshing `session_ref`.

#### Scenario: a stale edit

- **WHEN** `task.update` carries an `expected_rev` older than the stored one
- **THEN** the outcome is `conflict` with both versions, and nothing is applied

### Requirement: Ordered outbox and receipts

While the store cannot be reached, a write SHALL go to an ordered outbox under the
session's data dir, with receipts kept beside it. A second edit to the same key
SHALL take the first entry's `write_id` as its base. A replayed entry whose
`write_id` is already applied SHALL be dropped. Before a live handoff commits, the
server SHALL flush and fsync the outbox and stop writing.

#### Scenario: an acknowledgement is lost

- **WHEN** a write lands but the server restarts before the acknowledgement
- **THEN** the replay finds the `write_id` in `last_writes`, drops the entry, and
  reports success

#### Scenario: the outbox survives a handoff

- **WHEN** two entries are queued and the server hands off to a new binary
- **THEN** the new server replays both, in order, from the same data dir

### Requirement: Server-owned conflicts

The server SHALL list unresolved outbox conflicts through `task.conflicts`, and
resolve them through `task.resolve_conflict {key, write_id, choice, doc?,
expected_rev}`, with `choice` one of `mine`, `theirs` or `merged`. A resolution
SHALL be a compare-and-set. Choosing `theirs` SHALL cancel every queued entry
that depends on the dropped one, and return each one as a conflict.

#### Scenario: two clients resolve one conflict

- **WHEN** two clients resolve the same conflict against the same revision
- **THEN** one succeeds, the other gets `stale_revision`, and both receive
  `task.changed`

### Requirement: Derived status

A reader SHALL show a task's status in this order: a stored `done` or `dropped`;
`running`, when a run from an allowed starter is `running`, or is `starting` for
less than 5 minutes by the bus entry's timestamp; `waiting`, while `waits_on`
names an open or answered asks row; then the stored status. A run that is
`unknown`, or stuck in `starting`, SHALL show as "run unknown". The server SHALL
re-send `task.changed` when a run crosses the 5 minutes.

#### Scenario: a uzi run while the owner Mac sleeps

- **WHEN** worklink writes `run.worklink.rn-1` as `running` for a task whose owner
  Mac is asleep
- **THEN** the other Mac shows the task as `running`, though its stored status is
  `open`

#### Scenario: a forged starter

- **WHEN** a run record for an m4m task appears under `run.ac-mbm5.`
- **THEN** no reader shows the task as `running` because of it

### Requirement: Moving a task between hosts

`task.move` SHALL be driven by a `move.<new host>.<move_id>` record. It SHALL be
refused while a run of the task is starting or running. The old host SHALL mark the
original `moving`. The new host SHALL create `task.<new host>.<sha256(move_id)>`
with `moved_from`. The old host SHALL then mark the original `dropped`, with reason
`moved` and `moved_to`. Each step SHALL be idempotent by `move_id`.

#### Scenario: a retried move

- **WHEN** the new host retries its create after a crash
- **THEN** `kv.create` finds the destination already holding its `move_id`, and
  no second task appears

### Requirement: Recovering a stuck move

The old host SHALL finish a `moving` task whose destination exists, by marking it
`dropped` with `moved_to`. It SHALL cancel a move whose destination is still
missing after a timeout, returning the task to its previous status by
compare-and-set.

#### Scenario: the new host never answers

- **WHEN** a task has been `moving` past the timeout with no destination
- **THEN** the old host restores its previous status, and the task is writable
  again

### Requirement: Run start order

A run SHALL start in this order. First, `kv.create` reserves
`slot.<owner_host>.<task id>`. Second, `run.<starter>.<id>` is written as
`starting`. Third, the engine starts with the run id as its lookup key. The owning
server SHALL derive `task.runs` and the move to `running` from the run records. It
SHALL re-scan every `run.*` record of its tasks on start and after a handoff.

#### Scenario: two starters race

- **WHEN** worklink and the owner both try to start a run of one task
- **THEN** only the one whose `kv.create` of the slot succeeds starts

### Requirement: Pane run id

The server SHALL take the reserved key `HERDR_RUN_ID` from the `env` of
`pane.split` and `tab.create`. It SHALL keep the key in the shell's environment,
store it in `PaneSnapshot.run_id`, and return it as an optional field of
`pane.get` and `pane.list`. The request shapes SHALL NOT change.

#### Scenario: recovery finds the pane

- **WHEN** the server crashes after creating a Claude Code pane but before
  recording the correlation
- **THEN** after restart, `pane.list` shows the pane with its `run_id`, and the
  run is bound to it

### Requirement: Advertised task methods

The server SHALL add the methods `task.add`, `task.list`, `task.show`,
`task.update`, `task.done`, `task.drop`, `task.reopen`, `task.bind`, `task.move`,
`task.conflicts` and `task.resolve_conflict`, and the event `task.changed`. It
SHALL advertise them to the client shell with frozen shape digests. It SHALL
publish the optional resource fact `pane_tasks` (open, pending and conflict
counts, and a revision). The `todo.*` methods SHALL keep their meaning until
they retire.

#### Scenario: an older server

- **WHEN** the CLI sends `task.add` to a server that does not advertise it
- **THEN** the CLI falls back to `todo.add`, and the client disables only the
  task actions
