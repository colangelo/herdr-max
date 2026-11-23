//! What the view server knows of its clients (core-phase5 D6): how many are
//! attached and when one last typed, read by the watcher thread to decide
//! whether to read the machine's state and whether to wake it, and by the
//! server loop to exit 10 min after its last client left. The view keeps no
//! state, so a herdr update needs no handoff: a client that reconnects with
//! a new version starts a new view.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// How long a view with no client stays up (D6 Lifetime).
pub(crate) const IDLE_EXIT_AFTER: Duration = Duration::from_secs(10 * 60);

/// Shared between the server loop (writer) and the watcher (reader).
#[derive(Default)]
pub(crate) struct ViewPresence {
    clients: AtomicUsize,
    /// Milliseconds since `epoch` of the last client input; 0 for none.
    last_input_ms: AtomicU64,
    epoch: once_epoch::Epoch,
}

mod once_epoch {
    use std::sync::OnceLock;
    use std::time::Instant;

    #[derive(Default)]
    pub(crate) struct Epoch(OnceLock<Instant>);

    impl Epoch {
        pub(crate) fn get(&self) -> Instant {
            *self.0.get_or_init(Instant::now)
        }
    }
}

impl ViewPresence {
    pub(crate) fn set_clients(&self, clients: usize) {
        self.clients.store(clients, Ordering::Release);
    }

    pub(crate) fn clients(&self) -> usize {
        self.clients.load(Ordering::Acquire)
    }

    /// A client sent input (a key, text, paste or mouse report) now.
    pub(crate) fn input(&self, now: Instant) {
        let since = now.saturating_duration_since(self.epoch.get()).as_millis() as u64;
        self.last_input_ms.store(since.max(1), Ordering::Release);
    }

    /// Whether a client typed within `window` before `now`.
    pub(crate) fn typed_within(&self, now: Instant, window: Duration) -> bool {
        match self.last_input_ms.load(Ordering::Acquire) {
            0 => false,
            ms => {
                let at = self.epoch.get() + Duration::from_millis(ms);
                now.saturating_duration_since(at) <= window
            }
        }
    }
}

/// The server loop's clock for the idle exit: when the last client left.
#[derive(Debug, Default)]
pub(crate) struct IdleExit {
    empty_since: Option<Instant>,
}

impl IdleExit {
    /// Records the client count at `now`; true once the view has had no
    /// client for `IDLE_EXIT_AFTER`. A view that never had a client waits
    /// from its start.
    pub(crate) fn due(&mut self, clients: usize, now: Instant) -> bool {
        if clients > 0 {
            self.empty_since = None;
            return false;
        }
        let since = *self.empty_since.get_or_insert(now);
        now.saturating_duration_since(since) >= IDLE_EXIT_AFTER
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_view_exits_ten_minutes_after_its_last_client() {
        let start = Instant::now();
        let mut idle = IdleExit::default();
        assert!(!idle.due(0, start), "no client yet: the wait starts");
        assert!(
            !idle.due(1, start + Duration::from_secs(5)),
            "a client resets it"
        );
        let left = start + Duration::from_secs(60);
        assert!(!idle.due(0, left));
        assert!(!idle.due(0, left + IDLE_EXIT_AFTER - Duration::from_secs(1)));
        assert!(idle.due(0, left + IDLE_EXIT_AFTER));
    }

    #[test]
    fn input_is_remembered_for_a_window() {
        let presence = ViewPresence::default();
        let now = Instant::now();
        assert!(!presence.typed_within(now, Duration::from_secs(30)));
        presence.input(now);
        assert!(presence.typed_within(now + Duration::from_secs(29), Duration::from_secs(30)));
        assert!(!presence.typed_within(now + Duration::from_secs(31), Duration::from_secs(30)));
    }
}
