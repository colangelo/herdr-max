//! The backend contract. One trait, two backends: `file` (default) and `kv`
//! (NATS JetStream KV, behind the `tasks-kv` feature).
//!
//! Every call blocks, so only the store task calls a backend; the API handler
//! never does (it answers from the cache).

use super::ids::{Rev, TaskKey, WriteId};
use super::model::Task;

/// Any key in the bucket: `task.<host>.<id>`, `run.<starter>.<id>`,
/// `slot.<host>.<task id>` or `move.<host>.<move id>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct StoreKey(pub String);

impl From<&TaskKey> for StoreKey {
    fn from(key: &TaskKey) -> Self {
        Self(key.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StoreError {
    /// The revision moved: what the key holds now.
    Conflict { current: Option<(Rev, Vec<u8>)> },
    /// A create found the key held, by someone else's write.
    AlreadyExists { current: (Rev, Vec<u8>) },
    /// The backend cannot be reached now; the write belongs in the outbox.
    Unavailable(String),
    /// A local read or write failed, or the request was refused as invalid.
    Io(String),
    /// The write was sent but its fate is unknown (a timed-out publish may
    /// have been stored). Neither applied nor failed: replaying the write id
    /// finds out.
    Indeterminate(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict { .. } => f.write_str("the revision moved"),
            Self::AlreadyExists { .. } => f.write_str("the key already exists"),
            Self::Unavailable(why) | Self::Io(why) | Self::Indeterminate(why) => f.write_str(why),
        }
    }
}

pub(crate) type StoreResult<T> = Result<T, StoreError>;

/// A change seen by a watch, or an entry read by a scan. `value` is `None`
/// when the key was removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WatchEvent {
    pub key: StoreKey,
    pub rev: Rev,
    pub value: Option<Vec<u8>>,
    /// When the store took this revision, by the store's own clock, not the
    /// writer's. `None` when the backend cannot say.
    pub at: Option<std::time::SystemTime>,
}

/// Where a backend delivers watch events. Returns false once nobody listens,
/// which tells the backend to drop the watch.
pub(crate) type WatchSink = Box<dyn Fn(WatchEvent) -> bool + Send + 'static>;

/// Whether the document in `current` already holds `write` in its
/// `last_writes`: an earlier try of the same write landed and only its
/// acknowledgement was lost.
pub(crate) fn landed_before(current: &[u8], write: &WriteId) -> bool {
    serde_json::from_slice::<Task>(current).is_ok_and(|task| task.last_writes.contains(write))
}

pub(crate) trait TaskStore: Send + 'static {
    /// Create `key`. If it already holds a document whose `last_writes`
    /// contains `write`, an earlier try of this same write landed and its
    /// acknowledgement was lost: answer success with that revision. Any other
    /// existing document is `AlreadyExists`.
    fn create(&mut self, key: &TaskKey, doc: &Task, write: WriteId) -> StoreResult<Rev>;

    /// Replace `key` only if it is at revision `expected`, else `Conflict`
    /// with what is there now. As for `create`, a document already holding
    /// `write` in `last_writes` means the write landed before: answer success.
    fn update(
        &mut self,
        key: &TaskKey,
        doc: &Task,
        expected: Rev,
        write: WriteId,
    ) -> StoreResult<Rev>;

    fn get(&mut self, key: &StoreKey) -> StoreResult<Option<(Rev, Vec<u8>)>>;

    /// Every key that starts with `prefix`, as the events a watch would have
    /// delivered for them (so with a value), in key order.
    fn scan(&mut self, prefix: &str) -> StoreResult<Vec<WatchEvent>>;

    /// Report every change under `prefix` from now on to `sink`.
    fn watch(&mut self, prefix: &str, sink: WatchSink) -> StoreResult<()>;
}
