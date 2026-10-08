# Bubble Motion for Priority-Sorted Sidebar Lists

## Why

With priority sort active, switching to a just-finished agent pane instantly reshuffles the spaces list and the agents panel: viewing the pane flips its `seen` flag, its tier drops from done to idle-seen, and the next frame re-sorts. The clicked row bubbles down and working rows jump up so fast that a follow-up click lands on a different row; keyboard switches feel jarring too. Sort order is recomputed from scratch every frame with no persisted order, so this needs a real motion component, not a timing tweak.

Design decided in Gitea issue https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/19.

## What Changes

- New reusable list-motion primitive ("bubble"): a persisted display order of stable keys that follows the live sort's target order with a settle delay (~2s) and stepped one-row-at-a-time movement (~150ms/step), for upward and downward moves alike. New rows insert instantly, removed rows vanish instantly, state icons/colors stay live — only position travels late.
- Display order mutates only in an explicit animation tick, never lazily, so rendering, workspace jump numbers, and mouse hit-testing consume one coherent order between ticks — eliminating the misclick.
- The spaces list and the agents panel adopt the primitive when in priority sort; manual/spaces sort is unaffected.
- Config, set once and applied to every consumer: `ui.sort_motion = "bubble" | "instant"` (default `bubble`), `ui.sort_motion_settle_ms` (default 2000), `ui.sort_motion_step_ms` (default 150). Live-reloadable.
- Step timing goes through one internal timing function so an easing curve (slow start, accelerate, decelerate into the slot — like a bubble in nature) can drop in later without touching consumers.

## Capabilities

### New Capabilities

- `sidebar-sort-motion`: Reusable settle-and-bubble motion for priority-sorted sidebar lists (spaces and agents), including its config surface and the coherent-order guarantee for hit-testing and jump numbers.

### Modified Capabilities

<!-- none — no existing specs cover sidebar sorting -->

## Impact

- New module `src/ui/list_motion.rs` (pure, clock-injected core).
- `src/ui/sidebar.rs`: `workspace_list_entries_inner` and `agent_panel_entries_with_runtimes` route their priority-sorted output through the motion state.
- `src/app/mod.rs` / `src/app/state.rs` / `src/app/runtime.rs`: motion state on `App`, new deadline in the loop-deadline aggregator, tick in scheduled tasks (TUI and headless paths).
- `src/config/model.rs`, `src/config.rs`, `src/main.rs` template, live-reload block in `src/app/mod.rs`: the three `ui.sort_motion*` options.
- Docs: `docs/next` configuration reference + changelog.
- TUI-presentation only: no protocol, API, or persistence changes.
