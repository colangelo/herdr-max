## Why

Herdr's todos are pane notes, at most 50 per pane, saved in one Mac's
`session.json`. They cannot be shared by two Macs, read by a service, bound to a
forge issue, or tied to a run of an agent. The work-tracking plan replaces them
with tasks that both Macs and the `worklink` service share. It adds an Issues
panel inside herdr that shows forge issues, and lets ac act on them with
confirmed, signed writes.

Phase 1 of that plan wrote the contracts, with no code. They are
`~/_sync/dev/planner/docs/ideas/2026-10-21-worklink-contracts.md` v0.7
(planner commit 34272f8, every owner signed off on 2026-10-21). This change is
herdr-max's part of them: contract 1 (§ 2, the task model and store), herdr's
side of contract 2 (§ 3, the confirm path) and contract 5 (§ 6, runs). Tracked
as https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/241.

**Hard precondition** (ac, 2026-10-21, *"Mistake gate for now"*; contracts
§ 10 point 8): **uzi stays on `uzi-sandbox`, opening merge requests only, until
gate approvals are signed.** Enabling uzi on any real repository first needs
the signed gate approvals of contracts § 10 point 8 (b). Nothing in this change
relaxes that.

## What Changes

- A task store in the server behind a `TaskStore` trait. The `file` backend is
  the default. A `kv` backend (NATS JetStream KV bucket `TASKS`) sits behind an
  off-by-default cargo feature `tasks-kv`.
- A server-owned task cache, a background store task, compare-and-set writes
  with write ids, an ordered outbox with receipts, and server-owned conflicts.
- One writer per task: `owner_host`, the typed refusal `task_owned_elsewhere`,
  `[tasks] bus_session`, and `task.move` driven by move request records.
- New advertised methods `task.*` and the event `task.changed`, plus the optional
  resource fact `pane_tasks`. `todo.*` keep their meaning until they retire.
- Before the freeze: a stable `pane_key` and a `run_id` (set from the reserved env
  key `HERDR_RUN_ID`) in `PaneSnapshot`.
- The client shell gets a work panel popup (read-only first, then the actions),
  and the `TaskConflict` overlay.
- The run record contract of § 6, and the Claude Code engine on `agent.start`.
- The todo migration, with a write barrier, a check of every migrated task's
  content, and the todo store's retirement.

## Impact

- Server: a new `src/tasks/` module, `src/app/api/tasks.rs`,
  `src/api/schema/tasks.rs`, `src/persist/snapshot.rs` and `restore.rs`
  (`pane_key`, `run_id`, `migrated_to`), `src/server/client_commands.rs`
  (advertised methods), `src/protocol/resource_facts.rs` (`pane_tasks`).
- Client: new overlays in `src/client/shell/`, `src/input/keybind_help.rs`.
- Platform: the signing helper's client side in `src/platform/macos.rs`. Other
  platforms report "confirmations off".
- Upstream: the default build has no NATS code. The `kv` feature, the panel's
  actions and the helper are fork-only.
- No rollback across the release that adds `pane_key`. An older binary drops
  unknown `PaneSnapshot` fields when it next saves.
