//! The task store (fork issue 241): the record, its limits and status machine,
//! the `TaskStore` trait the backends implement, and the server's cache.
//!
//! Plain data, testable without a bus, a disk or a runtime. The store task
//! that drives a backend and the API that reads the cache come later; until
//! they exist nothing outside the tests calls this module.
// The store task and the API (tasks 2.3 and 2.10) are the first callers of these
// types and their re-exports; drop the allow when they land.
#![allow(dead_code, unused_imports)]

mod asks;
mod bus_env;
mod cache;
mod clean;
#[cfg(test)]
mod conformance;
mod file_store;
mod identity;
mod ids;
mod issues;
#[cfg(feature = "tasks-kv")]
mod kv_store;
mod model;
mod outbox;
mod repo;
mod runs;
mod service;
mod store;

pub(crate) use asks::parse_event as parse_ask_event;
pub(crate) use asks::{AskCache, AskChange, AskEntry, AskRecord};
pub(crate) use bus_env::{load as load_bus_env, BusEnv};
pub(crate) use cache::{CachedTask, TaskCache};
pub(crate) use file_store::FileStore;
pub(crate) use identity::{BackendKind, TaskIdentity};
pub(crate) use ids::{is_valid_host_id, HostId, Rev, TaskId, TaskKey, WriteId};
pub(crate) use issues::parse_event as parse_issue_event;
pub(crate) use issues::{
    IssueCache, IssueChange, IssueRecord, IssueRow, IssueState, ProjectCount, SyncRecord, SyncRow,
};
#[cfg(feature = "tasks-kv")]
pub(crate) use kv_store::{KvSettings, KvStore};
pub(crate) use model::{
    Actor, Client, Closed, Horizon, IssueRef, LegacyTodo, MoveContext, Moving, Origin, Owner,
    PaneBinding, Priority, SessionRef, Task, TaskError, TaskStatus, Timestamp, Via,
};
pub(crate) use repo::{parse_remote_url, pick_remote, RepoRemote, RepoResolver, RepoState};
pub(crate) use runs::{derive, AskRow, Derived, RunCache, RunRecord, RunState, Shown};
pub(crate) use service::{
    Backoff, Choice, EventContext, HeldConflict, MirrorChanges, MirrorStatus, ResolveOutcome,
    StartOptions, StoreEvent, StoreOpener, SubmitError, TaskService, WriteOutcome,
};
pub(crate) use store::{
    Mirror, MirrorEvent, MirrorSink, StoreError, StoreKey, StoreResult, TaskStore, WatchEvent,
    WatchSink,
};
