//! When the server starts its primary core again (openspec/changes/archive/2026-11-22-core-phase3,
//! D3, D3a, D3b). Pure: the clock is passed in.

use std::time::{Duration, Instant};

/// How long the socket must stay missing or refusing before a start. A
/// planned `hc server restart` leaves it missing about 0.4 s with 50
/// terminals and never refusing (design.md § Context), so restarts and
/// updates never get a start added.
pub(crate) const DOWN_BEFORE_START: Duration = Duration::from_secs(3);
/// Loss events this close together are one burst.
const BURST_WINDOW: Duration = Duration::from_secs(2);
const GIVE_UP_BURSTS: usize = 3;
const GIVE_UP_WINDOW: Duration = Duration::from_secs(10 * 60);

/// How a failed connect found the primary's socket. Anything else (a
/// timeout, a refused hello, an old core) is a core that runs, which herdr
/// never restarts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SocketDown {
    Missing,
    /// The file is there and refuses: a SIGKILLed core leaves it so.
    Refused,
}

/// Whether the primary core may be started now (D3, D3b).
#[derive(Debug, Default)]
pub(crate) struct StartRule {
    ever_connected: bool,
    down_since: Option<Instant>,
    last_start: Option<Instant>,
    /// Terminals finished with `server_stop`: a person stopped the core,
    /// so herdr does not start it while this stands (D3b).
    stopped_by_person: bool,
}

impl StartRule {
    pub(crate) const fn new() -> Self {
        Self {
            ever_connected: false,
            down_since: None,
            last_start: None,
            stopped_by_person: false,
        }
    }

    pub(crate) fn connected(&mut self) {
        self.ever_connected = true;
        self.down_since = None;
        self.stopped_by_person = false;
    }

    pub(crate) fn stopped_by_person(&mut self) {
        self.stopped_by_person = true;
    }

    /// Whether this start, if made, follows a loss: the core ran for this
    /// server before.
    pub(crate) fn after_loss(&self) -> bool {
        self.ever_connected
    }

    /// A connect failed with `down` (`None`: the core runs but did not
    /// answer). `first_start` lets the server's own connector start a core
    /// before its first connection, as phase 2 did.
    pub(crate) fn should_start(
        &mut self,
        now: Instant,
        down: Option<SocketDown>,
        first_start: bool,
        gave_up: bool,
    ) -> bool {
        if down.is_none() {
            self.down_since = None;
            return false;
        }
        let since = *self.down_since.get_or_insert(now);
        if self.stopped_by_person || gave_up {
            return false;
        }
        if self
            .last_start
            .is_some_and(|last| now.saturating_duration_since(last) < DOWN_BEFORE_START)
        {
            return false;
        }
        let start = (first_start && !self.ever_connected)
            || now.saturating_duration_since(since) >= DOWN_BEFORE_START;
        if start {
            self.last_start = Some(now);
        }
        start
    }
}

/// Counts loss bursts and gives up on core after the third within ten
/// minutes (D3a). A suspension of core panes and a start of the primary
/// after it went down are loss events. Counting starts alone would never
/// trip on a holder that keeps dying: the core server lives on.
#[derive(Debug, Default)]
pub(crate) struct CoreLossGuard {
    last_event: Option<Instant>,
    bursts: Vec<Instant>,
    gave_up: bool,
}

impl CoreLossGuard {
    pub(crate) const fn new() -> Self {
        Self {
            last_event: None,
            bursts: Vec::new(),
            gave_up: false,
        }
    }

    /// Records a loss event. True when this one made the guard give up.
    pub(crate) fn record(&mut self, now: Instant) -> bool {
        let same_burst = self
            .last_event
            .is_some_and(|last| now.saturating_duration_since(last) <= BURST_WINDOW);
        self.last_event = Some(now);
        if same_burst || self.gave_up {
            return false;
        }
        self.bursts
            .retain(|start| now.saturating_duration_since(*start) < GIVE_UP_WINDOW);
        self.bursts.push(now);
        self.gave_up = self.bursts.len() >= GIVE_UP_BURSTS;
        self.gave_up
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(start: Instant, millis: u64) -> Instant {
        start + Duration::from_millis(millis)
    }

    // V4, V38 (D3): no start before 3 s of a missing or refused socket; a
    // core that runs but does not answer is never started.
    #[test]
    fn a_core_is_started_only_after_three_seconds_down() {
        let start = Instant::now();
        let mut rule = StartRule::default();
        rule.connected();
        for down in [SocketDown::Missing, SocketDown::Refused] {
            let mut rule = StartRule::default();
            rule.connected();
            assert!(!rule.should_start(start, Some(down), true, false));
            assert!(!rule.should_start(at(start, 2_999), Some(down), true, false));
            assert!(rule.should_start(at(start, 3_000), Some(down), true, false));
            assert!(rule.after_loss());
        }
        assert!(!rule.should_start(start, None, true, false));
        assert!(!rule.should_start(at(start, 10_000), None, true, false));

        // A planned restart: missing 0.4 s, then back.
        assert!(!rule.should_start(start, Some(SocketDown::Missing), true, false));
        assert!(!rule.should_start(at(start, 400), Some(SocketDown::Missing), true, false));
        rule.connected();
        assert!(!rule.should_start(at(start, 3_500), Some(SocketDown::Missing), true, false));
    }

    #[test]
    fn the_first_connection_may_start_a_core_at_once_and_starts_are_spaced() {
        let start = Instant::now();
        let mut rule = StartRule::default();
        assert!(!rule.should_start(start, Some(SocketDown::Missing), false, false));
        let mut first = StartRule::default();
        assert!(first.should_start(start, Some(SocketDown::Missing), true, false));
        assert!(!first.after_loss());
        assert!(!first.should_start(at(start, 1_000), Some(SocketDown::Missing), true, false));
        assert!(first.should_start(at(start, 3_000), Some(SocketDown::Missing), true, false));
    }

    // V37 (D3b): a core a person stopped is not started until it answers.
    #[test]
    fn a_core_stopped_by_a_person_is_never_started_until_it_answers() {
        let start = Instant::now();
        let mut rule = StartRule::default();
        rule.connected();
        rule.stopped_by_person();
        assert!(!rule.should_start(start, Some(SocketDown::Missing), true, false));
        assert!(!rule.should_start(at(start, 60_000), Some(SocketDown::Missing), true, false));
        rule.connected();
        assert!(!rule.should_start(at(start, 61_000), Some(SocketDown::Missing), true, false));
        assert!(rule.should_start(at(start, 64_000), Some(SocketDown::Missing), true, false));
    }

    #[test]
    fn a_guard_that_gave_up_stops_every_start() {
        let start = Instant::now();
        let mut rule = StartRule::default();
        rule.connected();
        assert!(!rule.should_start(start, Some(SocketDown::Refused), true, true));
        assert!(!rule.should_start(at(start, 9_000), Some(SocketDown::Refused), true, true));
    }

    // V4 (D3a): three bursts in ten minutes give up; events within 2 s are
    // one burst; bursts older than ten minutes do not count.
    #[test]
    fn the_guard_gives_up_on_the_third_burst_in_ten_minutes() {
        let start = Instant::now();
        let mut guard = CoreLossGuard::default();
        assert!(!guard.record(start));
        assert!(!guard.record(at(start, 1_000)), "same burst");
        assert!(!guard.record(at(start, 2_900)), "still the same burst");
        assert!(!guard.record(at(start, 60_000)));
        assert!(guard.record(at(start, 120_000)), "third burst");
        assert!(
            !guard.record(at(start, 180_000)),
            "already given up, said once"
        );

        let mut spread = CoreLossGuard::default();
        for minute in [0, 6, 12, 18] {
            assert!(
                !spread.record(at(start, minute * 60_000)),
                "minute {minute}"
            );
        }
    }
}
