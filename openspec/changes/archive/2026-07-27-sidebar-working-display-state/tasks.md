# Tasks: sidebar-working-display-state

Tracked in https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/39

## 1. Characterization tests (before behavior changes)

- [x] 1.1 Pin today's behavior: `Workspace::aggregate_state` with `{done, working}` returns done — an intentionally-failing-later test documenting the bug
- [x] 1.2 Pin `{blocked, working}` → blocked and `{done, idle-seen}` → done, which must survive the change
- [x] 1.3 Pin the current attention sort order of `apply_agent_view` for a mixed-state fixture, so the sorting behavior is provably unchanged by this work
- [x] 1.4 Assert the three attention tables are currently identical, so collapsing them is provably behavior-preserving at the attention-ranking level. Record that the navigator's `state_priority` (`src/app/actions.rs`) is a *fourth* table which already implements the display order — it is not identical, and collapsing it is a separate equivalence to show

## 2. Shared ranking definitions

- [x] 2.1 Add one shared module defining `display_priority` and `attention_priority` with neutral, non-UI names (per the runtime/client boundary guardrail)
- [x] 2.2 `attention_priority` reproduces the existing table exactly (Blocked 4, Idle+unseen 3, Working 2, Idle+seen 1, Unknown 0)
- [x] 2.3 `display_priority` orders Blocked 4, Working 3, Idle+unseen 2, Idle+seen 1, Unknown 0
- [x] 2.4 Unit-test both rankings as total orders over all five (state, seen) combinations
- [x] 2.5 Delete the four duplicate tables and route every caller to the shared definitions: the three attention copies (`src/workspace/aggregate.rs`, `src/ui/sidebar.rs`, `src/app/api_helpers.rs`) plus the navigator's already-display-ordered `state_priority` (`src/app/actions.rs`). Routing `state_priority` to `display_priority` is behavior-preserving: the tables differ only by a constant offset, and the accumulator seed (`Unknown`) is the minimum in both

## 3. Display aggregate

- [x] 3.1 Switch `Workspace::aggregate_state` to `display_priority`, keeping its `(AgentState, bool)` signature so `state_dot` and `agent_panel_status_key` are untouched. Rename it `display_state` so the ranking it follows is stated at every call site — the unnamed ranking is what caused this bug
- [x] 3.2 Switch the second aggregation site, `space_aggregate_state` (`src/ui/sidebar.rs`), to the display ranking as well — it has its own independent `max_by_key` and feeds the collapsed worktree-space row's `state_dot`. Fixing only `Workspace::aggregate_state` would leave collapsed space rows still masking working agents
- [x] 3.3 Give `Tab` a display aggregate and route both tab-level aggregation sites to it: the navigator tab row (`src/app/actions.rs::tab_aggregate_state`) and the API's `tab_info` (`src/app/creation.rs`)
- [x] 3.4 Audit every aggregate call site; any caller using one for ordering or notification moves to an explicit attention-ranked helper instead of inheriting display order. Add `Workspace::attention_state` and a space-level attention aggregate for those callers
- [x] 3.5 Flip test 1.1 to assert the corrected behavior (`{done, working}` → working)
- [x] 3.6 Add the sibling-focus invariant test: rendered state identical before and after focusing a done sibling

## 4. Sort / motion consistency

- [x] 4.1 Confirm `apply_agent_view` and the workspace priority sort use `attention_priority`; test 1.3 must still pass unchanged
- [x] 4.2 Point `agent_panel_target_keys` and `workspace_unit_target_keys` at the same ranking as the sort they animate
- [x] 4.3 Test that motion target order equals sorted order for a mixed-state fixture (the never-settles guard from design Risk 1)

## 5. API and boundary check

- [x] 5.1 Move the workspace- and tab-level API aggregates (`WorkspaceInfo.agent_status`, `TabInfo.agent_status`) to the display ranking, and document on both fields which ranking they use. Per-pane `agent_status` is untouched. This is a deliberate, stated change of an undocumented aggregate — a container holding a working agent reports `working` — not a silent side effect
- [x] 5.2 Confirm `src/protocol/wire.rs::PROTOCOL_VERSION` needs no bump and no integration-asset version markers are touched

## 6. Verification

- [~] 6.1 `just check`: fmt clean, maintenance script tests clean, 3248/3249 tests pass. The one failure — `live_handoff::live_handoff_keeps_unmanaged_agent_name_bound_to_saved_session` — is unrelated and predates this change (verified by stashing to the base commit); it is a macOS-only test-fixture problem, filed as https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/40. Every test this change touches passes
- [x] 6.2 Live check against a build: verified against an isolated dev server. A workspace holding one `done` pane and one `working` pane reported `working` at both the workspace and tab level, and stayed `working` across focusing the finished sibling — the issue-39 capture, inverted
- [x] 6.3 No regression in priority sorting or bubble motion. Covered by tests rather than an interactive TUI session, because the invariant that matters is exactly what the tests assert: motion target order equals sorted order over a fixture spanning all five states, for both the workspace list and the agent panel

## 7. Docs

- [x] 7.1 Stage the aggregate-state rule under `docs/next/` — the sidebar row rule and the API aggregate's ranking. Not the stable website docs
