//! `task.*` wire types: the server-owned work store (fork issue 241).
//!
//! Every optional field has a serde default, and every enum an `Unknown`
//! fallback, so an older client reads a newer server's answer and a newer
//! client's request still parses on an older one.

use serde::{Deserialize, Serialize};

/// The status a task stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatusInfo {
    Open,
    Running,
    Done,
    Dropped,
    #[serde(other)]
    Unknown,
}

/// The status a reader shows: the stored one, as run records and the asks row
/// a task waits on change it. `run_unknown` is a run that is unknown or stuck
/// in `starting`, never shown as running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskShownInfo {
    Open,
    Running,
    Waiting,
    RunUnknown,
    Done,
    Dropped,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskPriorityInfo {
    Low,
    Normal,
    High,
    #[serde(other)]
    Unknown,
}

/// A forge issue a task refers to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TaskIssueRefInfo {
    pub forge: String,
    pub forge_id: String,
    pub host: String,
    pub project_id: u64,
    pub project_path: String,
    pub number: u64,
}

/// A pane a task sits on. `pane_id` is the public id while the pane still
/// resolves on this server, and absent once it is gone or on another host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TaskPaneInfo {
    pub host: String,
    pub session: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_key: Option<String>,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TaskClosedInfo {
    pub at: String,
    pub by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// One task as the API shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TaskInfo {
    /// `task.<owner_host>.<id>`.
    pub key: String,
    pub id: String,
    pub owner_host: String,
    /// The store's revision of this version; the `expected_rev` of an edit.
    pub rev: u64,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    pub status: TaskStatusInfo,
    pub shown: TaskShownInfo,
    pub priority: TaskPriorityInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizon: Option<String>,
    /// The agent session that owns the task; absent when nobody does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waits_on: Option<String>,
    /// `waits_on` names an asks row that no longer exists.
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub waits_on_missing: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<TaskIssueRefInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<TaskPaneInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related_pane: Option<TaskPaneInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    /// Who made it, and through what (`cli`, `tui`, `api`, ...).
    pub created_by: String,
    pub created_via: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<TaskClosedInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moved_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moved_from: Option<String>,
    /// A write for this task is waiting for the store.
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub pending: bool,
    /// A write for this task is held for a decision.
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub conflict: bool,
}

/// How the store is doing, so a client can say "pending sync" or "not synced".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TaskStoreInfo {
    /// `disabled`, `loading`, `ready` or `unavailable`.
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Writes waiting for the store.
    pub pending: u32,
    /// Writes held for a decision.
    pub conflicts: u32,
}

/// Params for `task.list`. Closed tasks are left out unless asked for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TaskListParams {
    /// Only the tasks bound to this pane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    /// Only the tasks of this agent session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub include_closed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TaskTarget {
    /// `task.<owner_host>.<id>`.
    pub key: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TaskAddParams {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<TaskPriorityInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizon: Option<String>,
    /// The agent session that owns it; none means nobody.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// A pane on this server to bind the task to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waits_on: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<TaskIssueRefInfo>,
    /// Who is asking (`human:ac`, an agent session name); recorded as the
    /// task's origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// Through what (`cli`, `tui`, `api`); `api` when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
}

/// The editable fields of a task. A field left out keeps its value; an empty
/// string clears `body`, `horizon`, `owner`, `waits_on` and `review`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TaskFields {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<TaskPriorityInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waits_on: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<String>,
}

/// Params for `task.update`. `expected_rev` is the revision the caller read: a
/// stale edit comes back as a conflict, never applied over a newer version.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TaskUpdateParams {
    pub key: String,
    pub expected_rev: u64,
    #[serde(flatten)]
    pub fields: TaskFields,
}

/// Params for `task.done`, `task.drop` and `task.reopen`. Without
/// `expected_rev` the move is checked against the version the server holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TaskMoveStatusParams {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_rev: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Params for `task.bind`: put a task on a pane of this server, or take it off.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TaskBindParams {
    pub key: String,
    pub expected_rev: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    /// Remove the binding; wins over `pane_id`.
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub clear: bool,
    /// Bind the related pane (shown by label only) instead of the task's own.
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub related: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskConflictChoice {
    Mine,
    Theirs,
    Merged,
    #[serde(other)]
    Unknown,
}

/// Params for `task.resolve_conflict`. `expected_rev` is the store's revision
/// the caller saw in `task.conflicts` (`theirs_rev`): a resolver who looked at
/// an older conflict gets `stale_revision`. For `merged`, `doc` is applied on
/// top of the held version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TaskResolveConflictParams {
    pub key: String,
    pub write_id: String,
    pub choice: TaskConflictChoice,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<TaskFields>,
    pub expected_rev: u64,
}

/// A write refused as a conflict, waiting for a decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TaskConflictInfo {
    pub write_id: String,
    pub key: String,
    /// What the held write would store.
    pub mine: TaskInfo,
    /// The store's version, when it can be read as a task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theirs: Option<TaskInfo>,
    /// The store's revision when it refused: the `expected_rev` to resolve with
    /// (`0` when the key was absent).
    pub theirs_rev: u64,
}

/// Params for `task.write_status`: how a write fared, for a caller (the CLI)
/// that waits for its outcome by asking again.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TaskWriteStatusParams {
    pub write_id: String,
}

/// How a write ended, in `task.changed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TaskOutcomeInfo {
    Applied {
        rev: u64,
    },
    /// The revision moved: what the store holds now.
    Conflict {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        current: Option<Box<TaskInfo>>,
    },
    /// A create found the key held.
    Exists {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        current: Option<Box<TaskInfo>>,
    },
    Failed {
        message: String,
    },
    /// The write may or may not have been applied.
    Unknown {
        message: String,
    },
}

/// How a `task.resolve_conflict` ended, in `task.changed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TaskResolutionInfo {
    /// The held write was queued again; its own outcome follows.
    Requeued,
    /// The store's version stands; these queued writes came back as conflicts.
    Dropped {
        cancelled: Vec<String>,
    },
    /// Someone settled it first, or the store moved again.
    Stale {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        current: Option<Box<TaskInfo>>,
    },
    Failed {
        message: String,
    },
}

/// Params for `issue.list`: the forge issues worklink mirrors.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct IssueListParams {
    /// Only this project: its numeric id, or its path (`ac/worklink`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// `open` (the default), `closed` or `all`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

/// Params for `ask.list`: ac's rows in the asks bucket.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AskListParams {
    /// Only rows of this kind. `task` when absent; `any` for every kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Rows that are closed or dropped too.
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub include_closed: bool,
}

/// An issue's place on its forge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct IssueRefInfo {
    pub forge: String,
    pub forge_id: String,
    pub host: String,
    pub project_id: u64,
    pub project_path: String,
    pub number: u64,
}

/// One mirrored issue, as worklink wrote it. No body, no comments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct IssueInfo {
    /// The row's key: `issue.<forge>.<forge_id>.<project_id>.<number>`.
    pub key: String,
    #[serde(rename = "ref")]
    pub issue_ref: IssueRefInfo,
    /// The revision of the row that the server's watch saw.
    pub mirror_rev: u64,
    pub title: String,
    /// `open`, `closed` or `unknown`.
    pub state: String,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_url: Option<String>,
    /// The forge's own revision marker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_rev: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sync: Option<String>,
    /// Present (true) when the title was cut.
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub truncated: bool,
}

/// How the last sync of one project went. A project with no sync row is
/// `synced: false`: the panel says "not synced".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct IssueProjectInfo {
    pub forge: String,
    pub forge_id: String,
    pub project_id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_path: Option<String>,
    pub synced: bool,
    /// `ok` or `error`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_attempt: Option<String>,
    /// When the sync last succeeded: the panel's last-sync line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_ok: Option<String>,
}

/// Whether the server has read a bucket, and why not when it has not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MirrorInfo {
    pub loaded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// One row of the asks bucket (the fields this server shows).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AskInfo {
    /// The row's key: `<gestore>.<id>`.
    pub key: String,
    pub rev: u64,
    /// `open`, `answered`, `closed` or `dropped`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub how: Option<String>,
    #[serde(default, rename = "where", skip_serializing_if = "Option::is_none")]
    pub where_: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filed_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}
