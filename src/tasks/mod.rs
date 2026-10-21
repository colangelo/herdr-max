//! The task store (fork issue 241): the record, its limits and status machine,
//! the `TaskStore` trait the backends implement, and the server's cache.
//!
//! Plain data, testable without a bus, a disk or a runtime. The store task
//! that drives a backend and the API that reads the cache come later; until
//! they exist nothing outside the tests calls this module.
// The store task and the API (tasks 2.3 and 2.10) are the first callers of these
// types and their re-exports; drop the allow when they land.
#![allow(dead_code, unused_imports)]

mod bus_env;
mod cache;
#[cfg(test)]
mod conformance;
mod file_store;
mod identity;
mod ids;
#[cfg(feature = "tasks-kv")]
mod kv_store;
mod model;
mod service;
mod store;

pub(crate) use bus_env::{load as load_bus_env, BusEnv};
pub(crate) use cache::{CachedTask, TaskCache};
pub(crate) use file_store::FileStore;
pub(crate) use identity::{BackendKind, TaskIdentity};
pub(crate) use ids::{is_valid_host_id, HostId, Rev, TaskId, TaskKey, WriteId};
#[cfg(feature = "tasks-kv")]
pub(crate) use kv_store::{KvSettings, KvStore};
pub(crate) use model::{
    Actor, Client, Closed, Horizon, IssueRef, LegacyTodo, MoveContext, Moving, Origin, Owner,
    PaneBinding, Priority, SessionRef, Task, TaskError, TaskStatus, Timestamp, Via,
};
pub(crate) use service::{
    Backoff, EventContext, StoreEvent, StoreOpener, SubmitError, TaskService, WriteOutcome,
};
pub(crate) use store::{StoreError, StoreKey, StoreResult, TaskStore, WatchEvent, WatchSink};
