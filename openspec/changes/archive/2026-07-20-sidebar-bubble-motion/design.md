# Design: Bubble Motion for Priority-Sorted Sidebar Lists

## Context

Both sidebar lists share one server-side comparator (`workspace_attention_priority`, `src/ui/sidebar.rs:236`: blocked=4, done=3, working=2, idle-seen=1, unknown=0; tie-break `last_agent_state_change_seq`). Sort order is recomputed from scratch on every render frame; nothing persists an order. The reshuffle trigger is the `seen` flip on switch (`switch_tab` `src/workspace.rs:444`, `mark_active_tab_seen` `src/app/actions.rs:1248`, `apply_pane_state_change` `src/app/actions.rs:2919`), dropping the viewed entry from tier 3 to tier 1 instantly.

Hit-testing is split: workspace rows resolve clicks against cached rects (`workspace_at_row` → `view.workspace_card_areas`), while agent-panel rows re-run the sort at click time (`agent_detail_target_at` → `agent_panel_entries`). Any fix must therefore stabilize the ordering function itself, not merely defer a repaint.

Decided in Gitea issue https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/19.

## Goals / Non-Goals

**Goals:**

- No instant reshuffles in priority-sorted lists: a row whose tier changes holds its position for a settle delay, then bubbles one row per step to its slot — same treatment for upward and downward moves.
- Follow-up clicks land on the row the user saw: one coherent display order shared by rendering, jump numbers, and hit-testing between animation ticks.
- One reusable primitive with one config surface; both sidebar lists (and future lists) inherit the same feel.
- State icons/colors always live; only position is animated.

**Non-Goals:**

- Easing curves (designed-for, not built: the step scheduler is one internal timing function that a curve can replace later).
- Animating manual-sort lists (they only reorder on explicit user action), tab strip, or pickers — future adopters.
- Cross-frame pixel animation (movement is whole-row, one position per step).
- Any protocol/API/persistence surface — this is TUI presentation state.

## Decisions

### 1. A persisted display order keyed by stable identity, mutated only in ticks

`ListMotion<K: Eq + Hash + Clone>` (new `src/ui/list_motion.rs`) holds `display: Vec<K>` plus per-key motion bookkeeping. Consumers call `tick(now, target: &[K]) -> &[K]` from the scheduled-task path; getters used by render/hit-testing return the last ticked order without mutating. Tick semantics:

1. Keys absent from `target` are removed; keys new to `target` are inserted at their target index (no animation).
2. A key whose display index diverges from its target index starts its settle clock on first divergence; re-converging (state changed back) clears it.
3. Once `now >= divergence + settle`, the key steps one position toward its target index per step interval, until aligned.
4. `next_due(now)` returns the earliest settle expiry or step time for deadline aggregation.

Keys: workspace list uses the sort *unit* key (worktree group key or workspace id — groups already sort as one unit and must move as one); agents panel uses pane/terminal id.

Alternative considered: freezing a snapshot of the whole order for N seconds after a seen-flip (narrow fix). Rejected: doesn't generalize to upward moves, produces a teleport at expiry, and adds one-off state instead of a reusable component.

### 2. Consumers keep computing the live sort; motion reorders the output

`workspace_list_entries_inner` and `agent_panel_entries_with_runtimes` compute the target order exactly as today, then map it through their `ListMotion` instance (only when that list is in priority sort and `ui.sort_motion = "bubble"`). Rendering, `visible_workspace_order` (jump numbers), and both hit-testing paths already consume these entry lists, so they inherit the display order with no further changes. The agent panel's click-time re-sort becomes safe because the motion getter is pure between ticks.

### 3. Scheduling via the existing deadline aggregator

Two `ListMotion` instances live on `App`. Their `next_due` joins the `Option<Instant>` array in `next_loop_deadline_with_resize_poll` (`src/app/runtime.rs:591-609`); `handle_scheduled_tasks` (App-level, reached by both the TUI loop and `handle_scheduled_tasks_headless`) calls tick and requests a render when the order changed. Same pattern as toast/metadata expiry — no new loop machinery.

### 4. Config once, applied everywhere

`[ui]`: `sort_motion = "bubble" | "instant"` (default `bubble`), `sort_motion_settle_ms` (default 2000), `sort_motion_step_ms` (default 150). Wired through the six standard points (config model + default, re-export, AppState, startup + live-reload in `src/app/mod.rs`, `src/main.rs` template, docs). `instant` restores today's behavior exactly. Defaulting to `bubble` is a deliberate behavior change for priority-sorted lists: the instant reshuffle is the bug being fixed.

### 5. Step timing behind one function

The "when is this key's next step due" computation is a single internal function of (steps taken, steps remaining, step_ms). It returns a constant interval now; a bubble-in-nature easing (slow start → accelerate → decelerate into the slot) replaces it later without touching consumers or config semantics (easing may add its own option then).

## Risks / Trade-offs

- [Stale-feeling top slot] A blocked agent's row takes settle + travel time to reach the top → its icon/color flip is instant, and the pending-attention toast/notification already fires immediately; accepted in the direction decision.
- [Click during a step] A click can land in the same frame a step applies → steps move one row per interval, so the worst case equals one row of drift, strictly better than today's whole-list teleport; ticks are the only mutation point, so no mid-frame inconsistency.
- [Two lists animating out of phase] Spaces and agents instances tick independently → acceptable; they are visually separate lists, and both use the same config cadence.
- [Scroll-follow interaction] The sidebar's follow-the-focused-entry scrolling reacts to reorders → it reads the same entry lists, so it follows the display order (the slow bubble), which is the calmer behavior wanted.

## Migration Plan

Pure additive TUI behavior with a config escape hatch (`instant`). No persistence, protocol, or integration-version impact. Rollback: revert the commits or set `ui.sort_motion = "instant"`.

## Open Questions

- Exact placement of the tick call so both loops hit it once per due deadline (App-level `handle_scheduled_tasks` is the candidate; verify the headless path reaches it before render).
- Whether the workspace unit key for ungrouped workspaces should be the workspace id or its stable index — resolve when reading `workspace_list_entries_inner`'s Unit structure.
