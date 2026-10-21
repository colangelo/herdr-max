## Context

The contracts are `~/_sync/dev/planner/docs/ideas/2026-10-21-worklink-contracts.md`
v0.7 (planner 34272f8). Section numbers below are theirs. The anchors are herdr
master at 2e82664c.

What herdr has today:

- Todos sit on `TerminalState`, capped at 50 per pane and 500 characters each
  (`src/terminal/todo.rs:14-16`, `:48-56`). A todo's `link` points at *another*
  pane (`todo.rs:39-46`). They are saved in `PaneSnapshot.todos`
  (`src/persist/snapshot.rs:134-142`).
- Raw pane ids are new on every restore (`src/persist/restore.rs:45-49`).
  Public ids change when a pane moves (`snapshot.rs:148-153`). Todo ids and
  creation times survive a restore (`restore.rs:117-134`).
- API requests run synchronously on the app loop
  (`src/app/api.rs:1215`, `handle_api_request(&mut self) -> String`).
- `todo.*` are advertised to the client shell with frozen shape digests
  (`src/server/client_commands.rs:364-391`). The digest covers only the request
  schema (`client_commands.rs:192-215`).
- The sidebar reads the optional resource fact `pane_todos`
  (`src/protocol/resource_facts.rs:18`, `:48-58`).
- Each named session has its own data dir (`src/session.rs:159-170`). A Mac can
  run several servers at once.
- `pane.split` and `tab.create` take `env` (`src/api/schema/panes.rs:37`,
  `src/api/schema/tabs.rs:38`, applied at `src/app/api/panes.rs:51`), but it is
  not kept anywhere.
- The server leaves the GUI session's bootstrap namespace on purpose
  (`src/platform/macos/bootstrap.rs:109-129`).
- A live handoff carries the `SessionSnapshot` (`src/server/handoff.rs:53-60`).

## Goals / Non-Goals

**Goals:** tasks that both Macs and worklink share. A server that never waits on
the bus. One writer per task. Writes that are safe across a crash, a handoff and
a sleeping Mac. A todo migration with no loss. A panel that follows the client
shell idiom. A default build that upstream can take.

**Non-goals:**

- Forge adapters, cards, the intent ledger and the audit log. Those belong to
  worklink.
- The asks page and `asks.py`. Those belong to CONTEXT.
- Buckets, NATS users and ACLs. Those belong to infra.
- Signed gate approvals (§ 10 point 8 (b)). Their own change comes later, and
  it must land before uzi is enabled on any real repository.
- Batch confirmations, which § 3.0 rules out for Phase 3.
- Windows and Linux signing. Those clients show "confirmations off".

## Decisions

### D1. A `TaskStore` trait with two backends

```rust
pub trait TaskStore: Send + 'static {
    fn create(&mut self, key: &TaskKey, doc: &Task, write: WriteId) -> StoreResult<Rev>;
    fn update(&mut self, key: &TaskKey, doc: &Task, expected: Rev, write: WriteId) -> StoreResult<Rev>;
    fn get(&mut self, key: &StoreKey) -> StoreResult<Option<(Rev, Vec<u8>)>>;
    fn scan(&mut self, prefix: &str) -> StoreResult<Vec<(StoreKey, Rev, Vec<u8>)>>;
    fn watch(&mut self, prefix: &str) -> StoreResult<WatchStream>;
}
```

- **`file` is the default.** It writes one JSON document per key under the
  session's data dir. Each key has its own revision counter. Writes go to a
  temp file, then a rename. Only the store task writes it, and each session
  has its own dir, so no two processes share it.
- **`kv` sits behind the cargo feature `tasks-kv`, off by default.** It uses
  the bucket `TASKS`. It is the only code that links a NATS client crate. That
  is the dependency's reason. Upstream, Windows and Linux builds carry no NATS.
- The backend is set in `[tasks] backend`. Changing it needs a server restart.
  A config reload does not swap stores.

### D2. Cache, watch and store task: the server never waits on the bus

- The server keeps `TaskCache` in server state: plain data, testable with no
  bus. A watch on `task.*`, `run.*`, `slot.*` and `move.*` feeds it.
- A store task on the existing tokio runtime (`Cargo.toml:48`) does every
  backend call. It sends its results back as internal events, which the app
  drains before the next API request (`src/app/api.rs:1220`).
- An API write answers at once, from the cache, with `pending` and its
  `write_id`. The outcome arrives later as `task.changed {write_id, outcome}`.
  The CLI waits for it up to a timeout, then prints "pending sync".
- Render, detection and frame fanout never touch the store. `pane_tasks`
  counts are recomputed when the cache changes, not on each frame. That keeps
  the multiplicative paths of `AGENTS.md` flat.

### D3. Ownership: `owner_host`, the bus session, the typed refusal

- The key is `task.<host>.<id>`. `owner_host` is a stable host id written once
  to `[tasks] host_id` (for example `m4m`, `ac-mbm5`), never the live hostname.
- Only the server whose session is named in `[tasks] bus_session` runs `kv` and
  reads the host's NATS credential. Other sessions refuse task writes with
  `task_not_bus_session {bus_session}`, or run their own `file` backend.
- A write to a task with another `owner_host` is refused with
  `task_owned_elsewhere {owner_host}`. It is never put in the outbox.
- A client attached to both servers sends each write to the connection whose
  host matches `owner_host`. With no such connection, the action shows as
  unavailable, with the reason.
- Bindings: `PaneBinding {host, session, pane_key, label}`. `pane` must be on the
  owner's host and bus session. `related_pane` may name another host, and then
  shows only its label.

### D4. `pane_key` and `run_id` in `PaneSnapshot` (ship before the freeze)

- `pane_key`: a ULID made when a terminal is created. It is kept on
  `TerminalState`, saved in `PaneSnapshot`, restored, and carried by moves and
  handoff. It is exposed as an optional field of `PaneInfo`
  (`src/api/schema/panes.rs:601`).
- `run_id`: the server takes the reserved key `HERDR_RUN_ID` out of the `env`
  that `pane.split` and `tab.create` already accept. It stores it on the
  terminal and in `PaneSnapshot.run_id`, and returns it in `pane.get` and
  `pane.list`. The request shapes do not change, so the frozen digests hold.
  The key is still passed to the shell's environment.
- Both fields are `#[serde(default, skip_serializing_if = ...)]`. Files saved by
  older versions load as before.

### D5. Writes, outbox, receipts (§ 2.4)

- Every write has a `write_id` (ULID). The record keeps the last 8 in
  `last_writes`, and never drops one that a pending entry depends on.
- A user edit carries `expected_rev`. A lost race returns `Conflict` with both
  versions. Only three operations rebase on their own, each re-checking its
  precondition: appending a run id, a status move whose precondition still
  holds, and refreshing `session_ref`.
- The outbox lives under `<session data dir>/tasks/outbox/`, one file per entry:
  `{write_id, key, op, base: Rev | WriteId}`, kept in order per key. Receipts
  (`write_id` → revision) sit beside it while any entry depends on them.
- On replay, an entry whose `write_id` is already in `last_writes` is dropped.
- Before a live handoff commits, the old server flushes and fsyncs the outbox,
  then stops writing. The new server reads the outbox from the same dir.

### D6. Conflicts belong to the server

- `task.conflicts` lists them. `task.resolve_conflict {key, write_id, choice:
  mine | theirs | merged, doc?, expected_rev}` resolves one by compare-and-set.
  A second resolver who loses the race gets `stale_revision` and a refresh.
- Choosing `theirs` cancels every queued entry that names the dropped one as its
  base. Each comes back to its caller as a conflict.
- Every client learns the result from `task.changed`.

### D7. Status precedence (§ 2.3)

The stored statuses are `Open | Running | Done | Dropped`. A reader shows them in
this order:

1. a stored `Done` or `Dropped`;
2. `Running`, for a run from an allowed starter (the owner host or worklink)
   that is `running`, or `starting` for less than 5 minutes, judged by the bus
   entry's timestamp;
3. `Waiting`, while `waits_on` names an asks row that is open or answered;
4. the stored status.

A run that is `unknown`, or stuck in `starting`, shows as "run unknown". A timer
re-sends `task.changed` when a run crosses the 5 minutes. On start, and after a
handoff, the owner scans every `run.*` record for its tasks and reconciles them
through the store task.

### D8. `task.move` through move request records (§ 2.2)

The new host writes `move.<new host>.<move_id>`. Every bus session watches
`move.*`.

- The old host refuses the move while a run is starting or running.
- Otherwise the old host marks the original `moving {move_id, to_host}`.
- The new host then creates `task.<new host>.<sha256(move_id)>` with
  `moved_from`, by `kv.create`.
- Last, the old host marks the original `Dropped`, with reason `moved` and
  `moved_to`.

Each step is idempotent by `move_id`. The old host recovers a stuck move: it
finishes the move if the destination exists, and after a timeout it cancels the
move if the destination does not.

### D9. Runs (§ 6)

- Step 0 reserves `slot.<owner_host>.<task id>` by `kv.create`.
- Step 1 writes `run.<starter>.<id>` in state `starting`, with the reserved id.
- For Claude Code, the pane is created with `HERDR_RUN_ID` in its env. Then
  `agent.start {name, kind, pane_id, args, timeout_ms}`
  (`src/api/schema/agents.rs:173-181`) runs with the routed model and effort as
  `args`.
- Status map (`src/api/schema/common.rs:279-285`): `working` and `idle` mean
  running. `blocked` raises a question. `done` ends the run. `unknown` marks the
  run unknown.
- The owning server derives `task.runs` and the move to Running from the run
  records. Recovery finds a pane by its `run_id`.

### D10. The endpoint contract

- `task.*` methods are appended to `Method`, and `task.changed` to `EventKind`.
  The methods are advertised to the client shell, with digests frozen like the
  todo ones.
- `todo.*` keep their exact meaning on the old store until they retire.
- `pane_tasks` is a new optional resource fact (open, pending, conflict counts,
  revision).
- During the wave, `pane_todos` keeps coming from the old store, so old clients'
  counts match their `todo.list`.
- New `PaneInfo` and `TaskInfo` fields are optional. New enums carry an
  `Unknown` fallback.

### D11. The client shell: work panel and `TaskConflict`

- Two new `ClientShellOverlay` variants: `WorkPanel` and `TaskConflict`
  (`src/client/shell/state.rs:336-356`, `:702-720`). Each one's render goes in
  `overlays.rs`, its keys in `overlay_input.rs`, and its click targets in
  `OverlayRender`. Each gets a rendered-layout test under
  `src/client/shell/tests/`.
- A new `open_work_panel` keybinding action gets its entry in
  `src/input/keybind_help.rs`.
- Data reaches the panel only through server methods: `task.list`, `issue.list`
  for the `ISSUES` mirror with `last_sync`, and `ask.list` for ac's own rows.
  The client never holds NATS credentials. Under `--remote` it can run on the
  other Mac.
- `TaskConflict` shows only the fields that differ, as three columns: base,
  mine and theirs, with who and when. Keys: `m`, `t`, `e`, `Esc`.

### D12. The confirm path (Phase 3): the server never signs

- The client builds the intent's fields and hands them to the signing helper on
  its own Mac. The helper is an app bundle running as a launchd agent in the
  GUI login session. It has hardened runtime and holds the key in the
  data-protection keychain, under `.userPresence`.
- The helper builds the canonical envelope itself, shows it, asks for Touch ID
  or the password, and signs.
- The client passes `{envelope, signature}` to `intent.submit`. The server
  checks the signature against the registered public key of `confirmed.host`,
  then sends a request on `worklink.intent.<host>` from its store task. It
  reports the states of § 3.2 through `intent.changed`.
- The helper's client side lives in `src/platform/macos.rs`. Other platforms
  return "confirmations off".

### D13. Todo migration (§ 2.6)

- **Barrier first.** `todo.add|update|remove|clear` answer `todo_migrating`:
  "todos are migrating; use herdr-beta task add". Old clients show any unknown
  code as a "todo save failed" toast with the server's message
  (`src/client/shell/actions.rs:661-688`).
- **Then capture, then migrate.** The server migrates from a frozen snapshot.
  Ids are deterministic (`tk-m-<sha256 prefix>`), creates use `kv.create`, and
  each todo gets a `migrated_to` stamp. The mapping is in the `pane-todos` delta.
  The owner comes from the pane's `agent_name` (`src/terminal/state.rs:206`).
- **Check.** A per-Mac report compares every todo with its task by content.
- **Retirement.** The store stays read-only for a week, then retires. The
  server stops advertising `todo.*` but keeps reading `PaneSnapshot.todos`.

## Risks / Trade-offs

- **No rollback across the `pane_key` release.** An older binary drops unknown
  `PaneSnapshot` fields when it saves. → Ship it a beta before the freeze, and
  install mbm5 first, per the beta rule.
- **The store task falls behind.** → Writes stay `pending`, and the UI shows
  "pending sync". Nothing blocks the app loop.
- **A sleeping owner Mac.** Its tasks are read-only from the other Mac. This is
  accepted in § 2.1. ac's own items are asks rows and stay writable.
- **Version skew under `--remote`.** A missing `task.*` method disables only that
  action, and the CLI falls back to `todo.*`.
- **The helper app's provisioning.** It needs a profile for
  `keychain-access-groups`. If none can be had, Phase 3 stays off and the panel
  stays read-only.
