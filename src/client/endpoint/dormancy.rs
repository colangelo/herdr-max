//! Remote dormancy: after a while with no input on this client, it stops sending
//! anything of its own to remote machines (health pings, reconnect attempts), so
//! a sleeping remote Mac is not woken by network packets. The first input wakes
//! the remotes at once: live links are probed and waiting reconnects run now.
//! Nothing on a server is touched; a dropped link reconnects on that input.
//! (https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/260)

use std::time::{Duration, Instant};

/// How long the client waits without user input before its remotes go dormant.
pub(crate) const REMOTE_DORMANT_AFTER: Duration = Duration::from_secs(10 * 60);

/// What a call to [`InputClock`] changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dormancy {
    Unchanged,
    /// No input for [`REMOTE_DORMANT_AFTER`]: stop pinging and reconnecting remotes.
    Slept,
    /// The first input after dormancy: probe and reconnect remotes now.
    Woke,
}

/// The client's last user input (keys, text, paste, mouse) and whether its
/// remotes are dormant because of it.
#[derive(Debug)]
pub(crate) struct InputClock {
    last_input: Instant,
    dormant: bool,
    after: Duration,
}

impl InputClock {
    pub(crate) fn new(now: Instant) -> Self {
        Self::with_threshold(now, REMOTE_DORMANT_AFTER)
    }

    fn with_threshold(now: Instant, after: Duration) -> Self {
        Self {
            last_input: now,
            dormant: false,
            after,
        }
    }

    pub(crate) fn input(&mut self, now: Instant) -> Dormancy {
        self.last_input = now;
        if std::mem::replace(&mut self.dormant, false) {
            Dormancy::Woke
        } else {
            Dormancy::Unchanged
        }
    }

    pub(crate) fn tick(&mut self, now: Instant) -> Dormancy {
        if !self.dormant && now.saturating_duration_since(self.last_input) >= self.after {
            self.dormant = true;
            Dormancy::Slept
        } else {
            Dormancy::Unchanged
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remotes_sleep_once_after_the_threshold_and_wake_on_the_next_input() {
        let start = Instant::now();
        let mut clock = InputClock::with_threshold(start, Duration::from_secs(600));
        assert_eq!(
            clock.tick(start + Duration::from_secs(599)),
            Dormancy::Unchanged
        );
        assert_eq!(
            clock.tick(start + Duration::from_secs(600)),
            Dormancy::Slept
        );
        assert_eq!(
            clock.tick(start + Duration::from_secs(900)),
            Dormancy::Unchanged
        );
        assert_eq!(
            clock.input(start + Duration::from_secs(901)),
            Dormancy::Woke
        );
        assert_eq!(
            clock.input(start + Duration::from_secs(902)),
            Dormancy::Unchanged
        );
    }

    #[test]
    fn input_restarts_the_wait() {
        let start = Instant::now();
        let mut clock = InputClock::with_threshold(start, Duration::from_secs(600));
        assert_eq!(
            clock.input(start + Duration::from_secs(500)),
            Dormancy::Unchanged
        );
        assert_eq!(
            clock.tick(start + Duration::from_secs(1000)),
            Dormancy::Unchanged
        );
        assert_eq!(
            clock.tick(start + Duration::from_secs(1100)),
            Dormancy::Slept
        );
    }
}
