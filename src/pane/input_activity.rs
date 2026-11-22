//! When a pane last had input, on every input path (core-phase3 D9): bytes a
//! client or caller wrote, a queued Submit from the time it is queued until
//! it is delivered, and a note sent to the pane's agent. The idle move reads
//! it; resume and restore launches never write it.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

#[derive(Debug, Default)]
pub(crate) struct InputActivity {
    /// Unix ms of the newest input; 0 when there was none.
    last_ms: AtomicU64,
    /// Queued Submits the writer has not delivered yet.
    pending_submits: AtomicU32,
}

impl InputActivity {
    pub(crate) fn stamp(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| {
                u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
            });
        self.last_ms.fetch_max(now, Ordering::Relaxed);
    }

    pub(crate) fn last_at_ms(&self) -> Option<i64> {
        let stamp = self.last_ms.load(Ordering::Relaxed);
        (stamp > 0).then(|| i64::try_from(stamp).unwrap_or(i64::MAX))
    }

    /// A queued Submit is input now until it is delivered.
    pub(crate) fn submit_pending(&self) -> bool {
        self.pending_submits.load(Ordering::Acquire) > 0
    }

    pub(crate) fn submit_queued(&self) {
        self.stamp();
        self.pending_submits.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn submit_settled(&self) {
        self.stamp();
        let _ = self
            .pending_submits
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                count.checked_sub(1)
            });
    }
}
