# Tasks: sidebar-bubble-motion

## 1. Motion primitive

- [x] 1.1 Create `src/ui/list_motion.rs`: `ListMotion<K>` with persisted display order, per-key settle clocks, stepped movement, insert/remove handling, `tick(now, target)` and `next_due(now)`; all time injected
- [x] 1.2 Unit tests with a fake clock: settle timing, cancel-on-reconverge, up/down stepping, mid-flight retarget, insert/remove, no mutation outside tick, `next_due` correctness

## 2. Config surface

- [x] 2.1 `src/config/model.rs`: `SortMotionConfig` enum (`bubble`/`instant`) + `sort_motion`, `sort_motion_settle_ms`, `sort_motion_step_ms` fields with defaults; extend the `[ui]` parse test; re-export from `src/config.rs`
- [x] 2.2 `src/app/state.rs`: matching AppState fields + `test_new()` defaults; `src/app/mod.rs`: startup construction AND live-reload block
- [x] 2.3 `src/main.rs`: commented entries in the generated config template
- [x] 2.4 Docs: `docs/next/website/src/content/docs/configuration.mdx` + `docs/next/CHANGELOG.md`

## 3. Sidebar integration

- [x] 3.1 Add two `ListMotion` instances to `App` (spaces units, agent panel); route `workspace_list_entries_inner` and `agent_panel_entries_with_runtimes` priority output through them when `sort_motion = bubble`; resolve the workspace unit key question (group key vs workspace id)
- [x] 3.2 Verify jump numbers (`visible_workspace_order`), workspace rect cache, and both hit-testing paths consume the display order with no further changes; add a characterization test if any path bypasses it
- [x] 3.3 Keep icons/colors/status live during motion (should already hold — verify by test or inspection)

## 4. Scheduling

- [x] 4.1 Add motion deadlines to `next_loop_deadline_with_resize_poll` (`src/app/runtime.rs`); tick from App-level scheduled tasks so both the TUI and headless loops advance motion and request a render on change

## 5. Validation

- [x] 5.1 `just check` passes
- [ ] 5.2 Live dogfood: with several agents running, click a done agent's notification — row holds ≥ settle delay (second click lands on the same row), then bubbles down step by step; an agent starting/finishing elsewhere bubbles up the same way; `ui.sort_motion = "instant"` + reload restores today's behavior
- [ ] 5.3 Update Gitea issue https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/19 with the resolution and close it after verification
