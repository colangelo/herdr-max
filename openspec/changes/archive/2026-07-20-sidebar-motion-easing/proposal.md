# Sidebar Motion Easing

> Backfilled artifact: implemented in `7390be45` before these artifacts were
> written. Recorded so the capability lands in `openspec/specs/`.

## Why

Bubble motion shipped with a deliberately flat step cadence — `next_step_delay` returned a constant, leaving easing as a documented seam. A travelling row therefore crawls at constant speed rather than behaving like a bubble (break away slowly, accelerate, settle gently).

Decided in Gitea issue https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/23.

## What Changes

- New `ui.sort_motion_easing = "linear" | "bubble"`, default `linear` (the shipped behavior, unchanged on upgrade).
- `bubble` eases the cadence across a reshuffle: progress is `steps_taken / (steps_taken + steps_remaining)` over the burst, speed follows a sine arc, and the delay is the reference step scaled between 0.5× mid-flight and 2.0× at the edges (clamped).
- Easing spans the burst rather than individual rows, matching the primitive's single global cadence of one adjacent swap per interval.

## Capabilities

### Modified Capabilities

- `sidebar-sort-motion`: the step cadence gains an optional easing curve and its configuration.

## Impact

- `src/ui/list_motion.rs`: `ListMotionEasing`, `easing` on `ListMotionTiming`, progress-aware `next_step_delay`, per-burst `steps_taken`, and `remaining_steps`.
- `src/config/model.rs`, `src/config.rs`, `src/app/state.rs`, `src/app/mod.rs` (startup + live reload), `src/main.rs`, config-reference data.
- TUI presentation only.
