//! The task record (contracts § 2.3), its limits and its stored status machine.

use serde::{Deserialize, Serialize};

use super::ids::{HostId, TaskId, TaskKey, WriteId};

pub(crate) const SCHEMA_VERSION: u32 = 1;
pub(crate) const TITLE_MAX_CHARS: usize = 200;
pub(crate) const BODY_MAX_CHARS: usize = 4000;
pub(crate) const RUNS_KEPT: usize = 20;
pub(crate) const LAST_WRITES_KEPT: usize = 8;
pub(crate) const OPEN_TASKS_PER_PANE: usize = 50;
/// A document from the bus may carry more write ids than a writer keeps,
/// because ones a pending entry depends on stay; this is the hard limit.
const LAST_WRITES_LIMIT: usize = 64;
const REFS_LIMIT: usize = 50;
/// Longest free-form field (labels, names, ids) of a document from the bus.
const FIELD_MAX_CHARS: usize = 200;

/// Who did something: `human:ac`, a fleet session name, or `worklink`.
pub(crate) type Actor = String;

/// An RFC 3339 time in UTC.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct Timestamp(pub String);

impl Timestamp {
    pub(crate) fn now() -> Self {
        let now = time::OffsetDateTime::now_utc();
        let text = now
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();
        Self(text)
    }
}

/// The stored statuses. A reader may show others (`Waiting`, "run unknown")
/// that are derived and never written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TaskStatus {
    Open,
    Running,
    Done,
    Dropped,
    /// A status this build does not know, written by a newer one. Shown, never
    /// written back.
    #[serde(other)]
    Unknown,
}

impl TaskStatus {
    pub(crate) fn is_closed(self) -> bool {
        matches!(self, Self::Done | Self::Dropped)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Priority {
    Low,
    #[default]
    Normal,
    High,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Horizon {
    Now,
    Next,
    Later,
    Icebox,
    #[serde(other)]
    Unknown,
}

/// A hint at the agent session behind an owner; the server refreshes it when
/// the pane's agent reports a new id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SessionRef {
    pub source: String,
    pub agent: String,
    pub kind: crate::agent_resume::AgentSessionRefKind,
    pub value: String,
}

/// A task is owned by an agent session or by nobody, never by a person.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Owner {
    Agent {
        /// The fleet session name: the identity across `/clear` and respawn.
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_ref: Option<SessionRef>,
    },
    #[default]
    Unassigned,
    #[serde(other)]
    Unknown,
}

/// A pane a task sits on. Never holds a raw pane id: those change on restore
/// and on a move.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PaneBinding {
    pub host: String,
    pub session: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_key: Option<String>,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IssueRef {
    pub forge: String,
    pub forge_id: String,
    pub host: String,
    pub project_id: u64,
    pub project_path: String,
    pub number: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Client {
    #[default]
    None,
    Aruba,
    #[serde(other)]
    Unknown,
}

impl Client {
    fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Via {
    Cli,
    Tui,
    Api,
    Migration,
    Worklink,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Origin {
    pub host: String,
    pub by: Actor,
    pub via: Via,
}

/// The todo a task was migrated from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LegacyTodo {
    pub host: String,
    pub pane_key: String,
    pub todo_id: u64,
    pub created_at_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Closed {
    pub at: Timestamp,
    pub by: Actor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A move to another host in progress (contracts § 2.2): both copies are
/// read-only until the old one is dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Moving {
    pub move_id: String,
    pub to_host: String,
}

/// One JSON document per task; the store's revision is its version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Task {
    pub id: TaskId,
    pub owner_host: HostId,
    pub schema: u32,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    pub status: TaskStatus,
    #[serde(default)]
    pub priority: Priority,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizon: Option<Horizon>,
    #[serde(default)]
    pub owner: Owner,
    /// `<gestore>.<id>` of the asks row that waits on a person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waits_on: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<IssueRef>,
    /// The last [`RUNS_KEPT`] run ids: a cache; the run records are the truth.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<PaneBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related_pane: Option<PaneBinding>,
    #[serde(default, skip_serializing_if = "Client::is_none")]
    pub client: Client,
    pub origin: Origin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy: Option<LegacyTodo>,
    /// The last [`LAST_WRITES_KEPT`] write ids applied, newest last.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub last_writes: Vec<WriteId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<Closed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moving: Option<Moving>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moved_from: Option<TaskKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moved_to: Option<TaskKey>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TaskError {
    TitleEmpty,
    TitleNotOneLine,
    TitleTooLong,
    BodyTooLong,
    OwnerInvalid,
    BadStatusMove {
        from: TaskStatus,
        to: TaskStatus,
    },
    /// Open to Running needs that no other run of the task is running.
    RunAlreadyActive,
    /// Running to Open needs that the run that ends is still the current one.
    RunNotCurrent,
    PaneFull,
    /// An update names revision 0, which no stored task has.
    BadRevision,
    /// A document offered for a task is for a different task.
    KeyMismatch,
    /// A document from the bus is bigger than a writer would make it.
    TooLarge(&'static str),
    /// The document was written by a newer build, or holds a value this build
    /// does not know; it is shown but not rewritten.
    NotRewritable,
    /// The task belongs to another host, which is the only one that writes it.
    OwnedElsewhere {
        owner_host: String,
    },
    /// This server is not the session that talks to the bus.
    NotBusSession {
        bus_session: String,
    },
}

impl TaskError {
    /// The error code an API answer carries.
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::TitleEmpty => "task_title_empty",
            Self::TitleNotOneLine => "task_title_not_one_line",
            Self::TitleTooLong => "task_title_too_long",
            Self::BodyTooLong => "task_body_too_long",
            Self::OwnerInvalid => "task_owner_invalid",
            Self::BadStatusMove { .. } => "task_bad_status_move",
            Self::RunAlreadyActive => "task_run_active",
            Self::RunNotCurrent => "task_run_not_current",
            Self::PaneFull => "task_pane_full",
            Self::BadRevision => "task_bad_revision",
            Self::KeyMismatch => "task_key_mismatch",
            Self::TooLarge(_) => "task_record_too_large",
            Self::NotRewritable => "task_schema_newer",
            Self::OwnedElsewhere { .. } => "task_owned_elsewhere",
            Self::NotBusSession { .. } => "task_not_bus_session",
        }
    }
}

/// What a status move needs to know, and what it records.
pub(crate) struct MoveContext {
    /// Another run of this task is `running` (blocks Open to Running).
    pub other_run_running: bool,
    /// The run that ends is still the task's current one (needed for Running
    /// to Open).
    pub run_is_current: bool,
    pub by: Actor,
    pub reason: Option<String>,
    pub at: Timestamp,
}

pub(crate) fn check_title(title: &str) -> Result<(), TaskError> {
    if title.trim().is_empty() {
        return Err(TaskError::TitleEmpty);
    }
    if title.contains(['\n', '\r']) {
        return Err(TaskError::TitleNotOneLine);
    }
    if title.chars().count() > TITLE_MAX_CHARS {
        return Err(TaskError::TitleTooLong);
    }
    Ok(())
}

impl Task {
    /// A new open task, checked.
    pub(crate) fn new(
        owner_host: HostId,
        id: TaskId,
        title: &str,
        origin: Origin,
        now: Timestamp,
    ) -> Result<Self, TaskError> {
        check_title(title)?;
        Ok(Self {
            id,
            owner_host,
            schema: SCHEMA_VERSION,
            title: title.to_owned(),
            body: None,
            status: TaskStatus::Open,
            priority: Priority::default(),
            horizon: None,
            owner: Owner::Unassigned,
            waits_on: None,
            refs: Vec::new(),
            runs: Vec::new(),
            pane: None,
            related_pane: None,
            client: Client::None,
            origin,
            legacy: None,
            last_writes: Vec::new(),
            review: None,
            created_at: now.clone(),
            updated_at: now,
            closed: None,
            moving: None,
            moved_from: None,
            moved_to: None,
        })
    }

    pub(crate) fn key(&self) -> TaskKey {
        TaskKey::new(self.owner_host.clone(), self.id.clone())
    }

    /// Every limit of the record, for a document read from the store or built
    /// from a request.
    pub(crate) fn validate(&self) -> Result<(), TaskError> {
        check_title(&self.title)?;
        if self
            .body
            .as_ref()
            .is_some_and(|body| body.chars().count() > BODY_MAX_CHARS)
        {
            return Err(TaskError::BodyTooLong);
        }
        if let Owner::Agent { name, .. } = &self.owner {
            if name.trim().is_empty() || name == "ac" || name.starts_with("human:") {
                return Err(TaskError::OwnerInvalid);
            }
            if name.chars().count() > FIELD_MAX_CHARS {
                return Err(TaskError::TooLarge("owner"));
            }
        }
        self.check_sizes()
    }

    /// The bounds a document read from the bus must keep, so one writer cannot
    /// make a record the cache or the panel cannot hold.
    fn check_sizes(&self) -> Result<(), TaskError> {
        let too_long = |text: &str| text.chars().count() > FIELD_MAX_CHARS;
        if self.runs.len() > RUNS_KEPT || self.runs.iter().any(|run| too_long(run)) {
            return Err(TaskError::TooLarge("runs"));
        }
        if self.last_writes.len() > LAST_WRITES_LIMIT {
            return Err(TaskError::TooLarge("last_writes"));
        }
        if self.refs.len() > REFS_LIMIT
            || self.refs.iter().any(|issue| {
                too_long(&issue.forge)
                    || too_long(&issue.forge_id)
                    || too_long(&issue.host)
                    || too_long(&issue.project_path)
            })
        {
            return Err(TaskError::TooLarge("refs"));
        }
        let long_binding = |binding: &Option<PaneBinding>| {
            binding.as_ref().is_some_and(|pane| {
                too_long(&pane.host)
                    || too_long(&pane.session)
                    || too_long(&pane.label)
                    || pane.pane_key.as_deref().is_some_and(too_long)
            })
        };
        if long_binding(&self.pane) || long_binding(&self.related_pane) {
            return Err(TaskError::TooLarge("pane"));
        }
        if self.review.as_deref().is_some_and(too_long)
            || self.waits_on.as_deref().is_some_and(too_long)
        {
            return Err(TaskError::TooLarge("review"));
        }
        Ok(())
    }

    /// Whether this build may write the document back: it must be of a schema
    /// this build knows and hold no value it cannot name, or rewriting would
    /// lose what a newer build put there.
    pub(crate) fn can_be_rewritten(&self) -> bool {
        self.schema <= SCHEMA_VERSION
            && !matches!(self.status, TaskStatus::Unknown)
            && !matches!(self.priority, Priority::Unknown)
            && !matches!(self.horizon, Some(Horizon::Unknown))
            && !matches!(self.owner, Owner::Unknown)
            && !matches!(self.client, Client::Unknown)
            && !matches!(self.origin.via, Via::Unknown)
    }

    /// Append a run id, keeping the last [`RUNS_KEPT`].
    pub(crate) fn push_run(&mut self, run_id: &str) {
        if self.runs.last().map(String::as_str) == Some(run_id) {
            return;
        }
        self.runs.push(run_id.to_owned());
        if self.runs.len() > RUNS_KEPT {
            let excess = self.runs.len() - RUNS_KEPT;
            self.runs.drain(..excess);
        }
    }

    /// Note an applied write. Keeps the last [`LAST_WRITES_KEPT`], and never
    /// drops an id that `needed` says a pending entry still depends on, nor the
    /// id just pushed.
    pub(crate) fn record_write(&mut self, write: WriteId, needed: impl Fn(&WriteId) -> bool) {
        if self.last_writes.contains(&write) {
            return;
        }
        self.last_writes.push(write);
        let newest = self.last_writes.len() - 1;
        let mut excess = self.last_writes.len().saturating_sub(LAST_WRITES_KEPT);
        let mut index = 0;
        self.last_writes.retain(|kept| {
            let here = index;
            index += 1;
            // The id just pushed is the one a replay must recognise.
            if here == newest || excess == 0 || needed(kept) {
                true
            } else {
                excess -= 1;
                false
            }
        });
    }

    /// Move the stored status along the table of contracts § 2.3, or refuse.
    ///
    /// Open to Running (no other run running); Running to Open (that run is
    /// still current); Running to Done; Open or Running to Done or Dropped;
    /// Done or Dropped to Open (reopen, which clears `closed`).
    pub(crate) fn move_status(
        &mut self,
        to: TaskStatus,
        ctx: &MoveContext,
    ) -> Result<(), TaskError> {
        use TaskStatus::{Done, Dropped, Open, Running};
        let from = self.status;
        match (from, to) {
            (Open, Running) if ctx.other_run_running => return Err(TaskError::RunAlreadyActive),
            (Running, Open) if !ctx.run_is_current => return Err(TaskError::RunNotCurrent),
            (Open, Running)
            | (Running, Open)
            | (Open | Running, Done | Dropped)
            | (Done | Dropped, Open) => {}
            _ => return Err(TaskError::BadStatusMove { from, to }),
        }
        self.status = to;
        self.closed = to.is_closed().then(|| Closed {
            at: ctx.at.clone(),
            by: ctx.by.clone(),
            reason: ctx.reason.clone(),
        });
        self.updated_at = ctx.at.clone();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> HostId {
        HostId::new("m4m").unwrap()
    }

    fn origin() -> Origin {
        Origin {
            host: "m4m".into(),
            by: "herdr-wrk".into(),
            via: Via::Cli,
        }
    }

    fn task() -> Task {
        Task::new(
            host(),
            TaskId::new("tk-1").unwrap(),
            "write the store",
            origin(),
            Timestamp("2026-10-21T20:00:00Z".into()),
        )
        .unwrap()
    }

    fn ctx() -> MoveContext {
        MoveContext {
            other_run_running: false,
            run_is_current: true,
            by: "herdr-wrk".into(),
            reason: Some("because".into()),
            at: Timestamp("2026-10-21T21:00:00Z".into()),
        }
    }

    fn new_with_title(title: &str) -> Result<Task, TaskError> {
        Task::new(
            host(),
            TaskId::new("tk-1").unwrap(),
            title,
            origin(),
            Timestamp::now(),
        )
    }

    #[test]
    fn task_record_limits() {
        // Title: 1 to 200 characters, one line.
        assert!(new_with_title("x").is_ok());
        assert!(
            new_with_title(&"é".repeat(200)).is_ok(),
            "200 characters, not bytes"
        );
        assert_eq!(
            new_with_title(&"x".repeat(201)).unwrap_err().code(),
            "task_title_too_long"
        );
        assert_eq!(new_with_title("").unwrap_err(), TaskError::TitleEmpty);
        assert_eq!(new_with_title("  ").unwrap_err(), TaskError::TitleEmpty);
        assert_eq!(
            new_with_title("two\nlines").unwrap_err(),
            TaskError::TitleNotOneLine
        );

        // Body: at most 4000.
        let mut task = task();
        task.body = Some("b".repeat(4000));
        assert_eq!(task.validate(), Ok(()));
        task.body = Some("b".repeat(4001));
        assert_eq!(task.validate().unwrap_err().code(), "task_body_too_long");
        task.body = None;

        // The owner is an agent or nobody, never a person.
        for name in ["ac", "human:ac", " "] {
            task.owner = Owner::Agent {
                name: name.into(),
                session_ref: None,
            };
            assert_eq!(task.validate(), Err(TaskError::OwnerInvalid), "{name}");
        }
        task.owner = Owner::Agent {
            name: "herdr-wrk".into(),
            session_ref: None,
        };
        assert_eq!(task.validate(), Ok(()));

        // Runs keep the last 20.
        for n in 0..25 {
            task.push_run(&format!("rn-{n}"));
        }
        assert_eq!(task.runs.len(), RUNS_KEPT);
        assert_eq!(task.runs.first().map(String::as_str), Some("rn-5"));
        assert_eq!(task.runs.last().map(String::as_str), Some("rn-24"));
        task.push_run("rn-24");
        assert_eq!(task.runs.len(), RUNS_KEPT, "the same run twice is one");

        // Write ids keep the last 8, except one a pending entry depends on.
        let ids: Vec<WriteId> = (0..12)
            .map(|n| WriteId::from_string(format!("w{n:02}")))
            .collect();
        let pinned = ids[1].clone();
        for id in &ids {
            task.record_write(id.clone(), |kept| *kept == pinned);
        }
        assert_eq!(task.last_writes.len(), LAST_WRITES_KEPT);
        assert!(task.last_writes.contains(&pinned), "a depended-on id stays");
        assert_eq!(task.last_writes.last(), ids.last());
        assert!(!task.last_writes.contains(&ids[0]));
        task.record_write(ids[11].clone(), |_| false);
        assert_eq!(
            task.last_writes.len(),
            LAST_WRITES_KEPT,
            "a replay adds nothing"
        );
    }

    #[test]
    fn record_write_never_trims_the_id_it_just_pushed() {
        let mut task = task();
        let pinned: Vec<WriteId> = (0..LAST_WRITES_KEPT)
            .map(|n| WriteId::from_string(format!("pinned{n}")))
            .collect();
        for id in &pinned {
            task.record_write(id.clone(), |_| true);
        }
        // Eight queued edits depend on the eight kept ids; a ninth write must
        // still be recognised when it is replayed.
        let ninth = WriteId::from_string("ninth".into());
        task.record_write(ninth.clone(), |kept| pinned.contains(kept));
        assert!(task.last_writes.contains(&ninth), "{:?}", task.last_writes);
        assert_eq!(task.last_writes.len(), LAST_WRITES_KEPT + 1);
        // Once nothing depends on the old ones they go, oldest first.
        let tenth = WriteId::from_string("tenth".into());
        task.record_write(tenth.clone(), |_| false);
        assert_eq!(task.last_writes.len(), LAST_WRITES_KEPT);
        assert_eq!(task.last_writes.last(), Some(&tenth));
        assert!(task.last_writes.contains(&ninth));
    }

    #[test]
    fn status_machine_refuses_bad_moves() {
        use TaskStatus::{Done, Dropped, Open, Running};

        // The allowed moves.
        let allowed = [
            (Open, Running),
            (Running, Open),
            (Running, Done),
            (Open, Done),
            (Open, Dropped),
            (Running, Dropped),
            (Done, Open),
            (Dropped, Open),
        ];
        for (from, to) in allowed {
            let mut task = task();
            task.status = from;
            assert_eq!(task.move_status(to, &ctx()), Ok(()), "{from:?} -> {to:?}");
            assert_eq!(task.status, to);
        }

        // Every other pair is refused and leaves the task as it was.
        let all = [Open, Running, Done, Dropped];
        for from in all {
            for to in all {
                if allowed.contains(&(from, to)) {
                    continue;
                }
                let mut task = task();
                task.status = from;
                let before = task.clone();
                assert_eq!(
                    task.move_status(to, &ctx()),
                    Err(TaskError::BadStatusMove { from, to }),
                    "{from:?} -> {to:?}"
                );
                assert_eq!(task, before);
            }
        }

        // Preconditions.
        let mut task = task();
        let busy = MoveContext {
            other_run_running: true,
            ..ctx()
        };
        assert_eq!(
            task.move_status(Running, &busy),
            Err(TaskError::RunAlreadyActive)
        );
        task.status = Running;
        let stale = MoveContext {
            run_is_current: false,
            ..ctx()
        };
        assert_eq!(
            task.move_status(Open, &stale),
            Err(TaskError::RunNotCurrent)
        );
        assert_eq!(task.status, Running);

        // Closing records who and why; reopening clears it.
        task.move_status(Done, &ctx()).unwrap();
        let closed = task.closed.clone().expect("a closed task says how");
        assert_eq!(closed.by, "herdr-wrk");
        assert_eq!(closed.reason.as_deref(), Some("because"));
        task.move_status(Open, &ctx()).unwrap();
        assert_eq!(task.closed, None);
    }

    #[test]
    fn a_task_round_trips_through_json_and_skips_empty_fields() {
        let task = task();
        let json = serde_json::to_value(&task).unwrap();
        for absent in ["body", "runs", "refs", "pane", "closed", "moving", "client"] {
            assert!(json.get(absent).is_none(), "{absent}: {json}");
        }
        assert_eq!(json["status"], "open");
        assert_eq!(json["schema"], 1);
        let back: Task = serde_json::from_value(json).unwrap();
        assert_eq!(back, task);
    }
}
