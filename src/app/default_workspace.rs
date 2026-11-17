//! The workspace the server opens for a client that has none. One whose
//! program exits at once would be replaced as fast as it dies: on 2026-11-17
//! a pane program that could not start made 409 workspaces in a minute (fork
//! issue 288). After a few such exits in a row, openings wait, longer each
//! time, and the first wait raises one notice.

use std::time::{Duration, Instant};

/// A default workspace gone this soon after it opened counts as dying at once.
const QUICK_EXIT: Duration = Duration::from_secs(1);
/// Quick exits in a row that are replaced at once.
const FREE_REOPENS: u32 = 3;
const FIRST_DELAY: Duration = Duration::from_secs(1);
const MAX_DELAY: Duration = Duration::from_secs(30);

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Reopen {
    Now,
    Wait,
    /// The first wait of a run of quick exits: raise the notice, then wait.
    StartWaiting {
        quick_exits: u32,
    },
}

/// When to open the next default workspace. Pure: the clock is passed in.
#[derive(Debug, Default)]
pub(crate) struct DefaultWorkspaceReopen {
    opened_at: Option<Instant>,
    quick_exits: u32,
    retry_at: Option<Instant>,
}

impl DefaultWorkspaceReopen {
    /// The client has no workspace now.
    pub(crate) fn poll(&mut self, now: Instant) -> Reopen {
        if let Some(opened_at) = self.opened_at.take() {
            if now.saturating_duration_since(opened_at) < QUICK_EXIT {
                self.quick_exits += 1;
            } else {
                self.quick_exits = 0;
            }
            if self.quick_exits >= FREE_REOPENS {
                let doublings = (self.quick_exits - FREE_REOPENS).min(16);
                let delay = FIRST_DELAY.saturating_mul(1 << doublings).min(MAX_DELAY);
                self.retry_at = Some(now + delay);
                if self.quick_exits == FREE_REOPENS {
                    return Reopen::StartWaiting {
                        quick_exits: self.quick_exits,
                    };
                }
            }
        }
        match self.retry_at {
            Some(at) if now < at => Reopen::Wait,
            _ => {
                self.retry_at = None;
                Reopen::Now
            }
        }
    }

    pub(crate) fn opened(&mut self, now: Instant) {
        self.opened_at = Some(now);
    }

    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.retry_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opens a default workspace that dies `after` later, until the gate
    /// waits; returns how many opened and the clock.
    fn open_until_wait(
        reopen: &mut DefaultWorkspaceReopen,
        mut now: Instant,
        after: Duration,
    ) -> (u32, Instant, Reopen) {
        let mut opened = 0;
        loop {
            match reopen.poll(now) {
                Reopen::Now => {
                    reopen.opened(now);
                    opened += 1;
                    now += after;
                }
                waiting => return (opened, now, waiting),
            }
            assert!(opened < 100, "never waits");
        }
    }

    #[test]
    fn a_default_workspace_that_dies_at_once_is_reopened_a_few_times_then_waits() {
        let mut reopen = DefaultWorkspaceReopen::default();
        let start = Instant::now();

        let (opened, now, waiting) =
            open_until_wait(&mut reopen, start, Duration::from_millis(130));

        assert_eq!(opened, FREE_REOPENS);
        assert_eq!(waiting, Reopen::StartWaiting { quick_exits: 3 });
        assert_eq!(reopen.deadline(), Some(now + FIRST_DELAY));
        assert_eq!(reopen.poll(now + FIRST_DELAY / 2), Reopen::Wait);
        assert_eq!(reopen.poll(now + FIRST_DELAY), Reopen::Now);
    }

    #[test]
    fn the_waits_grow_to_a_cap_with_one_notice() {
        let mut reopen = DefaultWorkspaceReopen::default();
        let mut now = Instant::now();
        let (_, at, _) = open_until_wait(&mut reopen, now, Duration::from_millis(130));
        now = at;
        let mut delays = Vec::new();
        for _ in 0..8 {
            let due = reopen.deadline().expect("waiting");
            delays.push(due - now);
            now = due;
            assert_eq!(reopen.poll(now), Reopen::Now);
            reopen.opened(now);
            now += Duration::from_millis(130);
            assert_eq!(reopen.poll(now), Reopen::Wait, "no second notice");
        }

        let secs: Vec<u64> = delays.iter().map(Duration::as_secs).collect();
        assert_eq!(secs, [1, 2, 4, 8, 16, 30, 30, 30]);
    }

    #[test]
    fn a_default_workspace_that_lives_resets_the_count() {
        let mut reopen = DefaultWorkspaceReopen::default();
        let _ = open_until_wait(&mut reopen, Instant::now(), Duration::from_millis(130));
        let now = reopen.deadline().expect("waiting");
        assert_eq!(reopen.poll(now), Reopen::Now);
        reopen.opened(now);

        let later = now + Duration::from_secs(5);
        assert_eq!(reopen.poll(later), Reopen::Now, "a workspace that lived");
        let (opened, _, waiting) = open_until_wait(&mut reopen, later, Duration::from_millis(130));
        assert_eq!(opened, FREE_REOPENS, "a fresh run of free reopens");
        assert_eq!(waiting, Reopen::StartWaiting { quick_exits: 3 });
    }
}
