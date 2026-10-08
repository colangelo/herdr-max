//! Reusable settle-and-bubble motion for reordering lists.
//!
//! A `ListMotion` owns the *display* order of a list whose *target* order is
//! recomputed elsewhere (e.g. priority sort). Instead of teleporting to the
//! target on the next frame, a diverged entry holds its position for a settle
//! delay, then bubbles one position per step interval until it reaches its
//! slot — up or down alike. New keys appear at their target position
//! immediately and removed keys disappear immediately; only reorders of
//! existing keys are animated.
//!
//! The display order mutates only in [`ListMotion::tick`]. Read paths use
//! [`ListMotion::project`], which is pure, so rendering, hit-testing, and any
//! other consumer observe one coherent order between ticks.

use std::collections::HashMap;
use std::hash::Hash;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ListMotionEasing {
    /// Every step is `step` apart.
    #[default]
    Linear,
    /// Ease in and out across a motion burst: slow to break away, quickest
    /// mid-flight, slowing again as the list settles — like a bubble rising.
    Bubble,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ListMotionTiming {
    /// How long a diverged entry holds its position before it starts moving.
    pub settle: Duration,
    /// Interval between one-position steps once an entry is moving. With
    /// `Bubble` easing this is the mid-burst reference, not a fixed spacing.
    pub step: Duration,
    pub easing: ListMotionEasing,
}

/// Slowest and fastest multiples of `step` the bubble curve produces.
const BUBBLE_SLOWEST: f32 = 2.0;
const BUBBLE_FASTEST: f32 = 0.5;

/// Delay before the next step, given how far the current burst has come.
///
/// The primitive keeps one global cadence (one adjacent swap per interval), so
/// easing is applied across the burst rather than per row. Progress is
/// `taken / (taken + remaining)`, and speed follows a sine arc: slow at both
/// ends, fastest in the middle.
fn next_step_delay(timing: ListMotionTiming, steps_taken: u32, steps_remaining: u32) -> Duration {
    match timing.easing {
        ListMotionEasing::Linear => timing.step,
        ListMotionEasing::Bubble => {
            let total = steps_taken.saturating_add(steps_remaining);
            if total == 0 {
                return timing.step;
            }
            let progress = f32::from(u16::try_from(steps_taken).unwrap_or(u16::MAX))
                / f32::from(u16::try_from(total).unwrap_or(u16::MAX));
            let speed = (std::f32::consts::PI * progress).sin();
            let factor = BUBBLE_FASTEST + (BUBBLE_SLOWEST - BUBBLE_FASTEST) * (1.0 - speed);
            // sin(π) lands a hair below zero in f32, so clamp rather than let
            // rounding push a step past the configured bounds.
            timing
                .step
                .mul_f32(factor.clamp(BUBBLE_FASTEST, BUBBLE_SLOWEST))
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct ListMotion<K> {
    display: Vec<K>,
    /// When each currently-diverged key first left its target position; a key
    /// may move only after holding for the settle delay. Cleared on alignment.
    diverged_since: HashMap<K, Instant>,
    /// Step cadence: one adjacent swap per interval while motion is running.
    next_step_at: Option<Instant>,
    /// Steps performed in the current motion burst; drives the easing curve
    /// and resets whenever the list reaches its target order.
    steps_taken: u32,
}

impl<K: Eq + Hash + Clone> ListMotion<K> {
    pub(crate) fn new() -> Self {
        Self {
            display: Vec::new(),
            diverged_since: HashMap::new(),
            next_step_at: None,
            steps_taken: 0,
        }
    }

    /// Pure projection of `target` through the current display order:
    /// keys already displayed keep their displayed relative order, keys new to
    /// `target` are inserted at their target position, and keys absent from
    /// `target` are dropped. Does not mutate; safe for render and hit-testing.
    pub(crate) fn project(&self, target: &[K]) -> Vec<K> {
        let mut result: Vec<K> = self
            .display
            .iter()
            .filter(|key| target.contains(key))
            .cloned()
            .collect();
        for (target_idx, key) in target.iter().enumerate() {
            if !result.contains(key) {
                let at = target_idx.min(result.len());
                result.insert(at, key.clone());
            }
        }
        result
    }

    /// Advances motion at `now` toward `target` and returns the new display
    /// order. The only mutation point.
    ///
    /// At most one adjacent swap happens per step interval, chosen as the
    /// first out-of-order neighbor pair (by target position) whose keys have
    /// both held their diverged position for the settle delay — i.e. one
    /// bubble-sort step per tick, so a travelling row visibly moves one slot
    /// at a time and the list never teleports.
    pub(crate) fn tick(&mut self, now: Instant, target: &[K], timing: ListMotionTiming) -> &[K] {
        self.display = self.project(target);
        self.refresh_divergence(now, target);
        if self.diverged_since.is_empty() {
            self.next_step_at = None;
            self.steps_taken = 0;
            return &self.display;
        }
        if self.next_step_at.is_some_and(|at| now < at) {
            return &self.display;
        }

        let target_pos =
            |key: &K| -> Option<usize> { target.iter().position(|other| other == key) };
        // A key absent from `diverged_since` is sitting at its target index and
        // owes no settle delay, so it is free to be swapped aside by a
        // travelling neighbour. Requiring it to be *present* and released
        // instead deadlocks any inversion that straddles an aligned key: no
        // swap is ever legal, `diverged_since` never empties, and `next_due`
        // returns a past instant forever, spinning the event loop.
        let released = |diverged_since: &HashMap<K, Instant>, key: &K| {
            diverged_since
                .get(key)
                .is_none_or(|since| now >= *since + timing.settle)
        };
        for idx in 0..self.display.len().saturating_sub(1) {
            let inverted = match (
                target_pos(&self.display[idx]),
                target_pos(&self.display[idx + 1]),
            ) {
                (Some(upper), Some(lower)) => upper > lower,
                _ => false,
            };
            if !inverted
                || !released(&self.diverged_since, &self.display[idx])
                || !released(&self.diverged_since, &self.display[idx + 1])
            {
                continue;
            }
            self.display.swap(idx, idx + 1);
            self.steps_taken = self.steps_taken.saturating_add(1);
            self.next_step_at =
                Some(now + next_step_delay(timing, self.steps_taken, self.remaining_steps(target)));
            self.refresh_divergence(now, target);
            break;
        }

        &self.display
    }

    /// Steps still owed to the furthest-travelling row — how many more swaps
    /// the burst needs before the list is in target order.
    fn remaining_steps(&self, target: &[K]) -> u32 {
        self.display
            .iter()
            .enumerate()
            .filter_map(|(display_idx, key)| {
                let target_idx = target.iter().position(|other| other == key)?;
                Some(display_idx.abs_diff(target_idx))
            })
            .max()
            .map(|steps| u32::try_from(steps).unwrap_or(u32::MAX))
            .unwrap_or(0)
    }

    /// Rebuilds the diverged-key set against `target`: aligned keys forget
    /// their divergence, newly diverged keys start their settle clock at `now`.
    fn refresh_divergence(&mut self, now: Instant, target: &[K]) {
        for (display_idx, key) in self.display.iter().enumerate() {
            if target.iter().position(|other| other == key) == Some(display_idx) {
                self.diverged_since.remove(key);
            } else {
                self.diverged_since.entry(key.clone()).or_insert(now);
            }
        }
        let display = &self.display;
        self.diverged_since.retain(|key, _| display.contains(key));
    }

    /// Earliest instant at which [`tick`](Self::tick) has work to do, for the
    /// loop-deadline aggregator. `None` when the display order is settled.
    pub(crate) fn next_due(&self, timing: ListMotionTiming) -> Option<Instant> {
        let earliest_release = self
            .diverged_since
            .values()
            .map(|since| *since + timing.settle)
            .min()?;
        Some(match self.next_step_at {
            Some(step_at) => step_at.max(earliest_release),
            None => earliest_release,
        })
    }

    /// Forgets all motion state; the next tick snaps to the target order.
    /// Used when motion is disabled or the list leaves priority sort.
    pub(crate) fn reset(&mut self) {
        self.display.clear();
        self.diverged_since.clear();
        self.next_step_at = None;
        self.steps_taken = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIMING: ListMotionTiming = ListMotionTiming {
        settle: Duration::from_millis(2000),
        step: Duration::from_millis(150),
        easing: ListMotionEasing::Linear,
    };

    const BUBBLE_TIMING: ListMotionTiming = ListMotionTiming {
        settle: Duration::from_millis(2000),
        step: Duration::from_millis(150),
        easing: ListMotionEasing::Bubble,
    };

    fn keys(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn tick(motion: &mut ListMotion<String>, now: Instant, target: &[&str]) -> Vec<String> {
        motion.tick(now, &keys(target), TIMING).to_vec()
    }

    #[test]
    fn stable_target_stays_put() {
        let mut motion = ListMotion::new();
        let t0 = Instant::now();
        assert_eq!(
            tick(&mut motion, t0, &["a", "b", "c"]),
            keys(&["a", "b", "c"])
        );
        assert_eq!(motion.next_due(TIMING), None);
        assert_eq!(
            tick(&mut motion, t0 + TIMING.step, &["a", "b", "c"]),
            keys(&["a", "b", "c"])
        );
    }

    #[test]
    fn divergence_holds_through_settle_then_steps() {
        let mut motion = ListMotion::new();
        let t0 = Instant::now();
        tick(&mut motion, t0, &["a", "b", "c"]);

        // "a" drops to the bottom (e.g. done -> seen).
        let target = ["b", "c", "a"];
        assert_eq!(tick(&mut motion, t0, &target), keys(&["a", "b", "c"]));
        assert_eq!(motion.next_due(TIMING), Some(t0 + TIMING.settle));

        // Still held just before settle expiry.
        let almost = t0 + TIMING.settle - Duration::from_millis(1);
        assert_eq!(tick(&mut motion, almost, &target), keys(&["a", "b", "c"]));

        // First step at settle expiry: one position only.
        let settled = t0 + TIMING.settle;
        assert_eq!(tick(&mut motion, settled, &target), keys(&["b", "a", "c"]));

        // Second step one interval later reaches the slot.
        let step2 = settled + TIMING.step;
        assert_eq!(tick(&mut motion, step2, &target), keys(&["b", "c", "a"]));
        assert_eq!(motion.next_due(TIMING), None);
    }

    #[test]
    fn no_step_between_intervals() {
        let mut motion = ListMotion::new();
        let t0 = Instant::now();
        tick(&mut motion, t0, &["a", "b", "c"]);
        let target = ["b", "c", "a"];
        let settled = t0 + TIMING.settle;
        tick(&mut motion, t0, &target);
        tick(&mut motion, settled, &target);
        // Half a step later nothing moves yet.
        let mid = settled + TIMING.step / 2;
        assert_eq!(tick(&mut motion, mid, &target), keys(&["b", "a", "c"]));
        assert_eq!(motion.next_due(TIMING), Some(settled + TIMING.step));
    }

    #[test]
    fn reconvergence_cancels_pending_motion() {
        let mut motion = ListMotion::new();
        let t0 = Instant::now();
        tick(&mut motion, t0, &["a", "b", "c"]);
        tick(&mut motion, t0, &["b", "a", "c"]);
        assert!(motion.next_due(TIMING).is_some());

        // Target returns to the display order before settle expires.
        assert_eq!(
            tick(
                &mut motion,
                t0 + Duration::from_millis(500),
                &["a", "b", "c"]
            ),
            keys(&["a", "b", "c"])
        );
        assert_eq!(motion.next_due(TIMING), None);

        // A later divergence starts a fresh settle clock.
        let t1 = t0 + Duration::from_secs(10);
        tick(&mut motion, t1, &["b", "a", "c"]);
        assert_eq!(motion.next_due(TIMING), Some(t1 + TIMING.settle));
    }

    #[test]
    fn upward_move_uses_same_rules() {
        let mut motion = ListMotion::new();
        let t0 = Instant::now();
        tick(&mut motion, t0, &["a", "b", "c"]);

        // "c" starts working and rises to the top.
        let target = ["c", "a", "b"];
        assert_eq!(tick(&mut motion, t0, &target), keys(&["a", "b", "c"]));
        let settled = t0 + TIMING.settle;
        assert_eq!(tick(&mut motion, settled, &target), keys(&["a", "c", "b"]));
        assert_eq!(
            tick(&mut motion, settled + TIMING.step, &target),
            keys(&["c", "a", "b"])
        );
    }

    #[test]
    fn insertions_and_removals_are_instant() {
        let mut motion = ListMotion::new();
        let t0 = Instant::now();
        tick(&mut motion, t0, &["a", "b", "c"]);

        // New key at the top appears immediately; no settle, no steps.
        assert_eq!(
            tick(&mut motion, t0, &["n", "a", "b", "c"]),
            keys(&["n", "a", "b", "c"])
        );
        assert_eq!(motion.next_due(TIMING), None);

        // Removal is immediate too.
        assert_eq!(
            tick(&mut motion, t0, &["n", "a", "c"]),
            keys(&["n", "a", "c"])
        );
        assert_eq!(motion.next_due(TIMING), None);
    }

    #[test]
    fn project_is_pure_and_matches_display_between_ticks() {
        let mut motion = ListMotion::new();
        let t0 = Instant::now();
        tick(&mut motion, t0, &["a", "b", "c"]);
        let target = keys(&["b", "c", "a"]);
        tick(&mut motion, t0, &["b", "c", "a"]);

        // Repeated projections between ticks are identical (hit-testing and
        // render agree) and never mutate.
        let p1 = motion.project(&target);
        let p2 = motion.project(&target);
        assert_eq!(p1, p2);
        assert_eq!(p1, keys(&["a", "b", "c"]));

        // Projection of a target with a brand-new key places it at its slot.
        let with_new = keys(&["n", "b", "c", "a"]);
        assert_eq!(motion.project(&with_new), keys(&["n", "a", "b", "c"]));
    }

    #[test]
    fn mid_flight_retarget_steps_toward_new_target() {
        let mut motion = ListMotion::new();
        let t0 = Instant::now();
        tick(&mut motion, t0, &["a", "b", "c", "d"]);

        // "a" heads for the bottom...
        let down = ["b", "c", "d", "a"];
        tick(&mut motion, t0, &down);
        let settled = t0 + TIMING.settle;
        assert_eq!(
            tick(&mut motion, settled, &down),
            keys(&["b", "a", "c", "d"])
        );

        // ...but then its agent starts working again: new target puts it first.
        // "b" had reached its slot, so it re-arms its own settle hold before
        // yielding — no row moves without having held for the settle delay.
        let up = ["a", "b", "c", "d"];
        let next = settled + TIMING.step;
        assert_eq!(tick(&mut motion, next, &up), keys(&["b", "a", "c", "d"]));
        let released = next + TIMING.settle;
        assert_eq!(
            tick(&mut motion, released, &up),
            keys(&["a", "b", "c", "d"])
        );
        assert_eq!(motion.next_due(TIMING), None);
    }

    #[test]
    fn linear_easing_keeps_a_constant_cadence() {
        // Every gap is exactly `step`, whatever the burst length.
        let mut motion = ListMotion::new();
        let t0 = Instant::now();
        let start = keys(&["a", "b", "c", "d", "e"]);
        let target = keys(&["e", "a", "b", "c", "d"]);
        motion.tick(t0, &start, TIMING);
        motion.tick(t0, &target, TIMING);

        let mut at = t0 + TIMING.settle;
        for _ in 0..4 {
            motion.tick(at, &target, TIMING);
            let Some(next) = motion.next_due(TIMING) else {
                break;
            };
            assert_eq!(next.duration_since(at), TIMING.step);
            at = next;
        }
    }

    #[test]
    fn bubble_easing_is_slow_at_the_edges_and_quick_mid_burst() {
        // "e" travels the full list, so the burst is long enough for the curve
        // to show: the first and last gaps are slower than `step`, and the
        // mid-burst gap is quicker.
        let mut motion = ListMotion::new();
        let t0 = Instant::now();
        let start = keys(&["a", "b", "c", "d", "e"]);
        let target = keys(&["e", "a", "b", "c", "d"]);
        motion.tick(t0, &start, BUBBLE_TIMING);
        motion.tick(t0, &target, BUBBLE_TIMING);

        let mut gaps = Vec::new();
        let mut at = t0 + BUBBLE_TIMING.settle;
        for _ in 0..4 {
            motion.tick(at, &target, BUBBLE_TIMING);
            let Some(next) = motion.next_due(BUBBLE_TIMING) else {
                break;
            };
            gaps.push(next.duration_since(at));
            at = next;
        }

        assert!(gaps.len() >= 3, "expected a multi-step burst: {gaps:?}");
        let first = gaps[0];
        let quickest = *gaps.iter().min().expect("gaps");
        let last = *gaps.last().expect("gaps");
        assert!(
            first > quickest,
            "burst should accelerate away from its first step: {gaps:?}"
        );
        assert!(
            last > quickest,
            "burst should decelerate into the slot: {gaps:?}"
        );
        assert!(
            quickest < BUBBLE_TIMING.step,
            "mid-burst should be quicker than the reference step: {gaps:?}"
        );
        // The curve stretches over the burst, so the edges only grow slower
        // than the reference step once the travel is long enough — which is
        // why short reshuffles read as "snappier" rather than "eased".
        assert!(
            first < BUBBLE_TIMING.step,
            "a 4-step burst stays under the reference step at its edges: {gaps:?}"
        );
        let long_edge = next_step_delay(BUBBLE_TIMING, 1, 15);
        assert!(
            long_edge > BUBBLE_TIMING.step,
            "a long burst should visibly hesitate at its edges: {long_edge:?}"
        );
    }

    #[test]
    fn bubble_easing_stays_within_its_configured_bounds() {
        let slowest = TIMING.step.mul_f32(BUBBLE_SLOWEST);
        let fastest = TIMING.step.mul_f32(BUBBLE_FASTEST);
        for (taken, remaining) in [(0, 0), (1, 0), (0, 9), (1, 8), (5, 5), (9, 1), (99, 1)] {
            let delay = next_step_delay(BUBBLE_TIMING, taken, remaining);
            assert!(
                delay >= fastest && delay <= slowest,
                "delay {delay:?} out of bounds for taken={taken} remaining={remaining}"
            );
        }
        // Linear ignores progress entirely.
        assert_eq!(next_step_delay(TIMING, 3, 7), TIMING.step);
    }

    #[test]
    fn swap_partner_alignment_clears_bookkeeping() {
        let mut motion = ListMotion::new();
        let t0 = Instant::now();
        tick(&mut motion, t0, &["a", "b"]);

        // Pure swap: one step resolves both keys.
        let target = ["b", "a"];
        tick(&mut motion, t0, &target);
        let settled = t0 + TIMING.settle;
        assert_eq!(tick(&mut motion, settled, &target), keys(&["b", "a"]));
        assert_eq!(motion.next_due(TIMING), None);
    }

    #[test]
    fn reset_snaps_to_target() {
        let mut motion = ListMotion::new();
        let t0 = Instant::now();
        tick(&mut motion, t0, &["a", "b", "c"]);
        tick(&mut motion, t0, &["c", "a", "b"]);
        motion.reset();
        assert_eq!(
            tick(&mut motion, t0, &["c", "a", "b"]),
            keys(&["c", "a", "b"])
        );
        assert_eq!(motion.next_due(TIMING), None);
    }

    /// Regression: an inversion whose members include a key already sitting at
    /// its target index must still resolve. `released` answers false for an
    /// aligned key because `refresh_divergence` drops it from
    /// `diverged_since`, so gating a swap on *both* neighbours being released
    /// wedges the list: no swap is ever legal, `diverged_since` never empties,
    /// and `next_due` keeps returning a past instant, which spins the event
    /// loop at 100% CPU forever.
    #[test]
    fn inversion_around_an_aligned_key_still_resolves() {
        let mut motion = ListMotion::new();
        let t0 = Instant::now();
        tick(&mut motion, t0, &["b", "a", "c"]);
        // "a" is already at index 1 in this target; "b" and "c" must cross it.
        let target = ["c", "a", "b"];
        tick(&mut motion, t0, &target);
        let mut now = t0 + TIMING.settle;
        for _ in 0..64 {
            tick(&mut motion, now, &target);
            now += TIMING.step;
        }
        assert_eq!(tick(&mut motion, now, &target), keys(&target));
        assert_eq!(
            motion.next_due(TIMING),
            None,
            "a settled list must not keep asking the event loop to wake"
        );
    }
}
