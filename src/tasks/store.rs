//! The backend contract. One trait, two backends: `file` (default) and `kv`
//! (NATS JetStream KV, behind the `tasks-kv` feature).
//!
//! Every call blocks, so only the store task calls a backend; the API handler
//! never does (it answers from the cache).

use std::sync::mpsc::Receiver;

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
    /// A local read or write failed.
    Io(String),
}

pub(crate) type StoreResult<T> = Result<T, StoreError>;

/// A change seen by a watch. `value` is `None` when the key was removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WatchEvent {
    pub key: StoreKey,
    pub rev: Rev,
    pub value: Option<Vec<u8>>,
}

pub(crate) type WatchStream = Receiver<WatchEvent>;

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

    /// Every key that starts with `prefix`, with its revision and document.
    fn scan(&mut self, prefix: &str) -> StoreResult<Vec<(StoreKey, Rev, Vec<u8>)>>;

    /// Changes under `prefix` from now on.
    fn watch(&mut self, prefix: &str) -> StoreResult<WatchStream>;
}
