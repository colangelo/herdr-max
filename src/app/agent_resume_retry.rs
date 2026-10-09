//! A restored agent whose resume command dies at boot (a network that is not
//! up yet, a daemon that is still starting) is left at a bare shell prompt.
//! Retry the typed command a bounded number of times, then say so (fork
//! issue 177).

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn retry_at(t0: Instant) -> ResumeRetry {
        ResumeRetry::new(
            crate::layout::PaneId::from_raw(1),
            "codex resume abc".into(),
            "codex".into(),
            t0,
        )
    }

    #[test]
    fn a_running_agent_ends_the_watch() {
        let t0 = Instant::now();
        let mut retry = retry_at(t0);
        assert_eq!(retry.check_at, t0 + Duration::from_secs(30));
        assert_eq!(retry.advance(t0 + Duration::from_secs(30), true, false), RetryDecision::Done);
    }

    #[test]
    fn a_stand_in_that_fails_once_is_retried_once_and_then_left_alone() {
        let t0 = Instant::now();
        let mut retry = retry_at(t0);
        // 30 s after launch the pane is back at its prompt: type the command again.
        let first = t0 + Duration::from_secs(30);
        assert_eq!(retry.advance(first, false, true), RetryDecision::Retry);
        assert_eq!(retry.retries, 1);
        assert_eq!(retry.check_at, first + Duration::from_secs(120));
        // 2 min later the agent is up.
        assert_eq!(
            retry.advance(first + Duration::from_secs(120), true, false),
            RetryDecision::Done
        );
    }

    #[test]
    fn an_agent_that_never_starts_is_retried_twice_then_reported_and_never_a_third_time() {
        let t0 = Instant::now();
        let mut retry = retry_at(t0);
        let mut now = t0;
        let mut decisions = Vec::new();
        for _ in 0..3 {
            now = retry.check_at;
            decisions.push(retry.advance(now, false, true));
        }
        assert_eq!(
            decisions,
            vec![RetryDecision::Retry, RetryDecision::Retry, RetryDecision::GiveUp]
        );
        assert_eq!(retry.retries, 2);
        // The waits are 30 s, then 120 s, then 30 s for the last look.
        assert_eq!(now, t0 + Duration::from_secs(30 + 120 + 30));
    }

    #[test]
    fn an_agent_still_booting_is_polled_a_few_times_and_never_typed_into() {
        let t0 = Instant::now();
        let mut retry = retry_at(t0);
        let mut now = t0 + Duration::from_secs(30);
        for polls in 1..=RESUME_RETRY_MAX_POLLS {
            assert_eq!(retry.advance(now, false, false), RetryDecision::Wait);
            assert_eq!(retry.polls, polls);
            now = retry.check_at;
        }
        // Still not at a prompt after the polls: something else owns the pane.
        assert_eq!(retry.advance(now, false, false), RetryDecision::Done);
        assert_eq!(retry.retries, 0);
    }

    #[test]
    fn the_earliest_check_is_the_next_wakeup() {
        let t0 = Instant::now();
        let mut retries = std::collections::HashMap::new();
        assert_eq!(next_check(&retries), None);
        retries.insert("a".to_owned(), retry_at(t0));
        retries.insert("b".to_owned(), retry_at(t0 + Duration::from_secs(5)));
        assert_eq!(next_check(&retries), Some(t0 + Duration::from_secs(30)));
    }
}
