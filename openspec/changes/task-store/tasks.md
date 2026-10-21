## 1. Prerequisites (ship before the freeze)

- [ ] 1.1 `pane_key` on `TerminalState`, saved and restored in `PaneSnapshot`, kept by
  moves, optional in `PaneInfo`. Test: `pane_key_survives_restore_move_and_handoff`
- [ ] 1.2 `HERDR_RUN_ID` taken from the `pane.split` and `tab.create` env into
  `PaneSnapshot.run_id`, returned by `pane.get` and `pane.list`, still passed to the
  shell. Tests: `run_id_from_env_is_kept_across_restore`, plus the existing digest test
  unchanged
- [ ] 1.3 `[tasks] host_id`, `bus_session`, `backend` in config, with validation.
  Test: `tasks_config_rejects_missing_host_id_for_kv`
- [ ] 1.4 A `tasks-kv` cargo feature skeleton, off by default, and a `just` check
  that the default build links no NATS crate. Test: `default_build_has_no_nats`
  (cargo tree)
- [ ] 1.5 Old session files load, and new optional fields are skipped when empty.
  Test: `session_file_from_2e82664c_loads_unchanged`
- [ ] 1.6 Ship as a beta, install on mbm5 first, and confirm every pane has a saved
  `pane_key` on both Macs. Proof: `herdr-beta pane list --json` shows them

## 2. Phase 2: the store and the read-only panel

- [ ] 2.1 `src/tasks/`: `Task`, `TaskKey`, `TaskStore` trait, `TaskCache`. Tests:
  `task_record_limits`, `status_machine_refuses_bad_moves`
- [ ] 2.2 The `file` backend under the session data dir, with per-key revisions.
  Test: `file_store_compare_and_set_conflicts`
- [ ] 2.3 The store task and internal events; API answers from the cache.
  Test: `slow_store_never_blocks_api` (a fake store that sleeps)
- [ ] 2.4 The `kv` backend behind `tasks-kv`: create, update, watch, scan.
  Test: `kv_backend_conformance` (against a local nats-server, feature-gated)
- [ ] 2.5 `owner_host`, `task_owned_elsewhere`, `task_not_bus_session`.
  Tests: `foreign_task_write_refused_not_queued`, `non_bus_session_refuses`
- [ ] 2.6 `write_id`, `last_writes`, the outbox and receipts, chained bases, replay.
  Tests: `lost_ack_replay_is_dropped`, `chained_entries_replay_in_order`
- [ ] 2.7 Outbox flush before a handoff commits.
  Test: `outbox_survives_live_handoff` (throwaway session)
- [ ] 2.8 Conflicts: `task.conflicts`, `task.resolve_conflict`, cancelled descendants.
  Tests: `theirs_cancels_descendants`, `second_resolver_gets_stale_revision`
- [ ] 2.9 Derived status, the 5-minute timer, the run rescan on start and handoff.
  Tests: `status_precedence_table`, `forged_starter_ignored`, `rescan_rebuilds_runs`
- [ ] 2.10 `task.*` methods, `task.changed`, client-shell advertisement with frozen
  digests. Test: the digest list in `client_commands.rs` gains the `task.*` entries
- [ ] 2.11 `pane_tasks` resource fact. Test: `older_client_ignores_pane_tasks`
- [ ] 2.12 CLI `herdr-beta task add|list|show|update|done|drop|reopen|bind`, with
  `--json`, the outcome wait, and the `todo.*` fallback.
  Test: `cli_falls_back_when_task_methods_missing`
- [ ] 2.13 `issue.list` and `ask.list`, read by the server from `ISSUES` and `ASKS`.
  Test: `issue_rows_equal_mirror_records`
- [ ] 2.14 `WorkPanel` overlay: four views, `last_sync`, the `open_work_panel` action
  with its help entry. Tests: rendered layouts under `src/client/shell/tests/`,
  `keybind_help_lists_open_work_panel`
- [ ] 2.15 Tab-bar work count. Test: `tab_bar_work_count_from_pane_tasks`
- [ ] 2.16 Oracle: the panel rows equal the forge API's for `uzi-sandbox` (diff test,
  owned with worklink)
- [ ] 2.17 Render scaling: `just bench-render-scale` with 1 and 15 panes, delta reported

## 3. Phase 3: actions one by one

- [ ] 3.1 The signing helper app (macOS, hardened runtime, data-protection keychain,
  `.userPresence`). Test: an R3 conformance run in which a pane-started server, a
  per-user-context server and an ssh-started server all fail to sign without the dialog
- [ ] 3.2 The helper builds and shows the canonical envelope itself, and refuses
  control characters. Test: `helper_refuses_display_mismatch`
- [ ] 3.3 The client side in `src/platform/macos.rs`; other platforms report
  "confirmations off". Test: `non_macos_confirmations_off`
- [ ] 3.4 `intent.submit` and `intent.changed`: verify the signature against
  `confirmed.host`'s key, then send the request from the store task. The server never
  signs. Test: `server_rejects_bad_signature`
- [ ] 3.5 Previews and keys `c`, `s`, `a`, `x`, `u`, each behind its own switch.
  Tests: rendered layouts per preview, `unknown_state_never_retried`
- [ ] 3.6 `TaskConflict` overlay with `m`, `t`, `e` and `Esc`.
  Tests: rendered layout, `conflict_overlay_resolves_through_server`
- [ ] 3.7 Per action, the crash and replay tests of plan § 6, with worklink
- [ ] 3.9 `signing-keys.toml` loading (`[tasks] signing_keys`) and P-256 verification
  per design D12, against `tests/fixtures/signing.json` copied from ac/worklink#14.
  Tests: `signing_vectors_verify`, `signing_vectors_negative_cases_refused`,
  `fixture_matches_worklink_copy` (sha256)
- [ ] 3.8 Done and not-doing keys on ac's own rows: the server runs
  `asks.py task-answer <key> done|not-doing [--take-back] --fingerprint <fp shown> --via herdr
  --client <id>` on its own Mac, with a FIXED environment (PATH, HOME and only what asks.py
  needs), never the server's own: no `NATS_*`, so asks.py uses the relay file and never
  herdr's read-only `herdr-<host>` credential, and no `GESTORE_ASKS_*` (asks.py:115-125) that a
  pane or test could have leaked in; herdr never holds the relay credential (#241 comment
  24885, contracts v0.8 § 2.5). Test: `ask_answer_child_env_is_fixed`

## 4. Phase 5: todos become tasks

- [ ] 4.1 The CLI alias: `todo add|done|undone|edit|rm|clear` call `task.*`, with u64
  ids resolved through `legacy`. Test: `todo_alias_maps_rm_to_dropped`
- [ ] 4.2 The barrier: `todo_migrating` on every todo write.
  Test: `old_client_toast_shows_migrating_message`
- [ ] 4.3 Capture, deterministic ids, `migrated_to`, the mapping.
  Tests: `migration_is_idempotent`, `migration_keeps_link_as_related_pane`
- [ ] 4.4 The report, checked by content. Test: `report_flags_changed_text`
- [ ] 4.5 Live: run on mbm5, then on m4m, and attach both reports to issue 241
- [ ] 4.6 After one read-only week: stop advertising `todo.*` and `pane_todos`, but keep
  reading `PaneSnapshot.todos`. Test: `retired_server_loads_old_todos_file`
- [ ] 4.7 `h` hand to pane and `t` new task go live in the panel

## 5. Phase 6: the Claude Code engine

- [ ] 5.1 Start slot, run record, then the pane with `HERDR_RUN_ID`, then
  `agent.start`. Test: `slot_blocks_second_starter`
- [ ] 5.2 Status map and run end. Test: `idle_does_not_end_a_run`
- [ ] 5.3 Recovery by `run_id` after a crash. Test: `recovery_binds_pane_by_run_id`
- [ ] 5.4 `task.move` via `move.<host>.<move_id>`, refused during a run.
  Tests: `move_retry_makes_one_destination`, `stuck_move_is_cancelled`
- [ ] 5.5 The engine conformance cases of the contracts (§ 6.3)
