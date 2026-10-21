## ADDED Requirements

### Requirement: Todo writes stop for the migration

When the todo migration starts, the server SHALL refuse `todo.add`,
`todo.update`, `todo.remove` and `todo.clear` with the error code
`todo_migrating` and the message "todos are migrating; use herdr-beta task add".
`todo.list` and `todo.changed` SHALL keep working. The `pane_todos` resource fact
SHALL keep coming from the todo store until it retires.

#### Scenario: an old client saves a todo

- **WHEN** a client from before this change saves a todo during the migration
- **THEN** it shows a "todo save failed" toast with the server's message, and the
  todo store is unchanged

#### Scenario: the CLI alias during the wave

- **WHEN** an agent runs `herdr-beta todo add "x"` on a server that advertises
  `task.add`
- **THEN** the CLI creates a task bound to the current pane, and prints one
  deprecation line

### Requirement: Migrated task ids

The migration SHALL run from a snapshot captured after the barrier. Each todo
SHALL become the task `tk-m-<h>`, where `<h>` is the first 32 hex digits of
SHA-256 over `host_id NUL pane_key NUL todo_id NUL created_at_unix`. The task
SHALL be created only if absent. The todo SHALL be stamped with `migrated_to`.

#### Scenario: a migration is run twice

- **WHEN** the migration runs again after a crash
- **THEN** every task id is the same, no task is created twice, and the report
  checks each task's content again

### Requirement: Todos map to tasks without loss

The migration SHALL map each todo as follows:

- the holding pane → `pane`;
- `link` → `related_pane`;
- the first line, cut at 200 → `title`, with the whole text in `body` when longer;
- `done` → `done`, with `closed.by = process:todo-migration`;
- `priority` and the times → unchanged;
- the pane's agent name → `owner`, else `unassigned`;
- a text starting `your call:` → `review = "your-call"`.

#### Scenario: a long todo

- **WHEN** a 450-character todo with two lines is migrated
- **THEN** the title is its first line cut at 200, and the body holds all 450
  characters

### Requirement: The migration report

The migration SHALL write nothing to `ASKS`. A per-Mac report SHALL list every
todo, the task it became, and compare text, done, priority and both pane
bindings by content against the snapshot.

#### Scenario: a changed text is caught

- **WHEN** a migrated task's title differs from its todo's first line
- **THEN** the report marks that todo as failed, and the todo store stays
  read-only

### Requirement: The todo store retires

The todo store SHALL stay read-only for a week after a clean report. Then the
server SHALL stop advertising `todo.*` and stop publishing `pane_todos`. It
SHALL keep reading `PaneSnapshot.todos`, so older session files still load.

#### Scenario: a client after retirement

- **WHEN** an old client opens its todo panel on a retired server
- **THEN** the panel action is unavailable, and the client stays connected
