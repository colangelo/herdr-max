# Tasks: resurface-working-panes-after-handoff

## 1. Characterization tests (before behavior changes)

- [x] 1.1 Add a handoff manifest serde roundtrip test capturing the current `HandoffRuntimeState` pane schema, then extend it for the new optional agent-state field (absent → Idle)
- [x] 1.2 Add a restore seeding characterization test: imported pane with `initial_restore_agent` is seeded Idle today; cold-restore pane with a resume plan is seeded Idle (`src/persist/restore.rs`)

## 2. Manifest schema and capture

- [x] 2.1 Add optional live agent-state field to the per-pane handoff manifest in `src/handoff_runtime.rs` (serde default = absent), with a serializable representation of Working/Blocked/Idle
- [x] 2.2 Populate the field on the sending side when building the handoff manifest from each pane's current `TerminalState.state`
- [x] 2.3 Verify `src/protocol/wire.rs::PROTOCOL_VERSION` needs no bump (handoff manifest is not the client wire protocol) and that no integration-asset versions are touched

## 3. Restore seeding

- [x] 3.1 In `src/persist/restore.rs`, seed imported (handoff) panes with the manifest agent state instead of hardcoded `AgentState::Idle`; keep Idle for non-imported panes
- [x] 3.2 Confirm sidebar spinner renders from the seeded Working state without any detection event (state-only test via `AppState::test_new` if practical)

## 4. Forced detection rescan hook

- [x] 4.1 Add a `PaneRuntime` method (e.g. `force_detection_rescan()`) that bumps `detection_content_seq` (`mark_detection_content_changed`) and notifies `detect_reset`, defeating `should_skip_idle_screen_scan`
- [x] 4.2 Unit-test that the idle-scan skip is bypassed after the forced rescan (pure logic in `src/pane/agent_detection.rs` where possible)

## 5. Post-commit background sweep

- [x] 5.1 In `src/server/headless.rs`, after handoff commit and `unpause_handoff_readers()`, spawn a background task sweeping imported agent panes in workspace/tab/pane order
- [x] 5.2 Per pane: SIGWINCH repaint nudge (reuse `nudge_child_redraw_after_handoff`), then `force_detection_rescan()`, then a named-constant stagger (~150ms) before the next pane
- [x] 5.3 Skip panes with `AgentDetection::Disabled` and respect hook-authority suppression (the detection loop already suppresses screen detection when `full_lifecycle_authority_active`)
- [x] 5.4 Remove the deferred first-attach nudge (`pending_handoff_repaint_nudge` / `nudge_handoff_panes_on_first_client_attach`), keeping the normal first-attach viewport resize

## 6. Hook-authority verification

- [x] 6.1 Verify `full_lifecycle_authority_active` does not survive handoff (runtime-only), so screen detection is active post-restart until hooks re-assert authority; adjust sweep skip logic if that assumption fails

## 7. Validation

- [x] 7.1 `just check` (fmt + nextest + maintenance tests) passes
- [x] 7.2 Live dogfood: build, trigger a live handoff (dev server restart) with agents working in multiple background workspaces, confirm spinners resurface across all spaces within seconds without touching any pane, and a finished agent settles back to idle
- [x] 7.3 Update Gitea issue https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/17 with the resolution and close it after verification
