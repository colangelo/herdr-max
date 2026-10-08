# Tasks: sidebar-motion-easing

> Backfilled after implementation (`7390be45`); all items were completed before
> these artifacts were written.

## 1. Curve

- [x] 1.1 `ListMotionEasing` enum + `easing` on `ListMotionTiming`
- [x] 1.2 Progress-aware `next_step_delay`: sine arc over `taken/(taken+remaining)`, scaled between `BUBBLE_FASTEST` and `BUBBLE_SLOWEST`, clamped against f32 rounding
- [x] 1.3 Per-burst `steps_taken` (reset on convergence and `reset()`) and `remaining_steps` (max outstanding displacement)

## 2. Config

- [x] 2.1 `SortMotionEasingConfig` + `ui.sort_motion_easing` with parse test; re-export
- [x] 2.2 `AppState` timing wiring: startup + live reload
- [x] 2.3 `src/main.rs` template + config-reference JSON + changelog

## 3. Validation

- [x] 3.1 Tests: linear constant cadence, bubble slow-edges/quick-middle shape, bounds across burst progress, long-burst hesitation
- [x] 3.2 `just check`
- [x] 3.3 Shipped in beta `0.7.4-ac-beta.20260720112300` and enabled live for judging
