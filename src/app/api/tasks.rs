//! `task.*` handlers: the server-owned view of the task store (fork issue 241).
//!
//! Every handler answers from the in-memory cache and never waits on the
//! store. A write answers `pending` with its `write_id` at once; how it ended
//! arrives later as `task.changed`.

use std::collections::{HashMap, HashSet};
use std::time::{Instant, SystemTime};

use super::responses::{encode_error, encode_success};
use crate::api::schema::{
    EventData, EventEnvelope, EventKind, ResponseResult, TaskAddParams, TaskBindParams,
    TaskClosedInfo, TaskConflictChoice, TaskConflictInfo, TaskFields, TaskInfo, TaskIssueRefInfo,
    TaskListParams, TaskMoveStatusParams, TaskOutcomeInfo, TaskPaneInfo, TaskPriorityInfo,
    TaskResolutionInfo, TaskResolveConflictParams, TaskShownInfo, TaskStatusInfo, TaskStoreInfo,
    TaskTarget, TaskUpdateParams,
};
use crate::app::App;
use crate::tasks::{
    AskRow, Choice, IssueRef, MoveContext, Origin, Owner, PaneBinding, Priority, ResolveOutcome,
    Rev, Shown, SubmitError, Task, TaskError, TaskId, TaskKey, TaskStatus, Timestamp, Via, WriteId,
    WriteOutcome,
};

/// Everything a conversion needs that is the same for every task of a call.
struct InfoContext {
    /// Public pane id of each pane, by its stable `pane_key`.
    panes: HashMap<String, String>,
    pending: HashSet<TaskKey>,
    conflicts: HashSet<TaskKey>,
    now: SystemTime,
}

fn error_text(err: &SubmitError) -> String {
    match err {
        SubmitError::Disabled => "tasks are off in this server".to_owned(),
        SubmitError::Stopped => "the task store has stopped".to_owned(),
        SubmitError::Refused(err) => err.message(),
        SubmitError::Busy => "too many writes are waiting for the store".to_owned(),
        SubmitError::Stale { current } => match current {
            Some((rev, _)) => format!(
                "the conflict is not the one you saw; the store is at revision {}, read it again",
                rev.0
            ),
            None => "the conflict is not the one you saw; read it again".to_owned(),
        },
    }
}

fn submit_failed(id: String, err: &SubmitError) -> String {
    encode_error(id, err.code(), error_text(err))
}

fn refused(id: String, err: &TaskError) -> String {
    encode_error(id, err.code(), err.message())
}

fn bad_key(id: String, key: &str) -> String {
    encode_error(id, "task_bad_key", format!("{key} is not a task key"))
}

fn not_found(id: String, key: &TaskKey) -> String {
    encode_error(id, "task_not_found", format!("no task {key}"))
}

fn status_info(status: TaskStatus) -> TaskStatusInfo {
    match status {
        TaskStatus::Open => TaskStatusInfo::Open,
        TaskStatus::Running => TaskStatusInfo::Running,
        TaskStatus::Done => TaskStatusInfo::Done,
        TaskStatus::Dropped => TaskStatusInfo::Dropped,
        TaskStatus::Unknown => TaskStatusInfo::Unknown,
    }
}

fn shown_info(shown: Shown) -> TaskShownInfo {
    match shown {
        Shown::Open => TaskShownInfo::Open,
        Shown::Running => TaskShownInfo::Running,
        Shown::Waiting => TaskShownInfo::Waiting,
        Shown::RunUnknown => TaskShownInfo::RunUnknown,
        Shown::Done => TaskShownInfo::Done,
        Shown::Dropped => TaskShownInfo::Dropped,
        Shown::Unknown => TaskShownInfo::Unknown,
    }
}

fn priority_info(priority: Priority) -> TaskPriorityInfo {
    match priority {
        Priority::Low => TaskPriorityInfo::Low,
        Priority::Normal => TaskPriorityInfo::Normal,
        Priority::High => TaskPriorityInfo::High,
        Priority::Unknown => TaskPriorityInfo::Unknown,
    }
}

fn priority_from(info: TaskPriorityInfo) -> Option<Priority> {
    match info {
        TaskPriorityInfo::Low => Some(Priority::Low),
        TaskPriorityInfo::Normal => Some(Priority::Normal),
        TaskPriorityInfo::High => Some(Priority::High),
        TaskPriorityInfo::Unknown => None,
    }
}

fn horizon_name(horizon: crate::tasks::Horizon) -> Option<&'static str> {
    use crate::tasks::Horizon::{Icebox, Later, Next, Now, Unknown};
    match horizon {
        Now => Some("now"),
        Next => Some("next"),
        Later => Some("later"),
        Icebox => Some("icebox"),
        Unknown => None,
    }
}

fn horizon_from(name: &str) -> Option<crate::tasks::Horizon> {
    use crate::tasks::Horizon::{Icebox, Later, Next, Now};
    match name {
        "now" => Some(Now),
        "next" => Some(Next),
        "later" => Some(Later),
        "icebox" => Some(Icebox),
        _ => None,
    }
}

fn via_from(name: Option<&str>) -> Via {
    match name {
        Some("cli") => Via::Cli,
        Some("tui") => Via::Tui,
        Some("worklink") => Via::Worklink,
        _ => Via::Api,
    }
}

fn via_name(via: Via) -> &'static str {
    match via {
        Via::Cli => "cli",
        Via::Tui => "tui",
        Via::Api => "api",
        Via::Migration => "migration",
        Via::Worklink => "worklink",
        Via::Unknown => "unknown",
    }
}

fn issue_ref_from(info: &TaskIssueRefInfo) -> IssueRef {
    IssueRef {
        forge: info.forge.clone(),
        forge_id: info.forge_id.clone(),
        host: info.host.clone(),
        project_id: info.project_id,
        project_path: info.project_path.clone(),
        number: info.number,
    }
}

/// An empty string clears a text field; anything else sets it.
fn set_text(field: &mut Option<String>, value: &Option<String>) {
    if let Some(value) = value {
        *field = (!value.is_empty()).then(|| value.clone());
    }
}

/// Apply the fields a caller named onto `task`. An answer of `Err` is the
/// error code and message to give.
fn apply_fields(task: &mut Task, fields: &TaskFields) -> Result<(), (&'static str, String)> {
    if let Some(title) = &fields.title {
        task.title = title.clone();
    }
    set_text(&mut task.body, &fields.body);
    set_text(&mut task.waits_on, &fields.waits_on);
    set_text(&mut task.review, &fields.review);
    if let Some(priority) = fields.priority {
        task.priority = priority_from(priority)
            .ok_or(("task_bad_priority", "that priority is not known".to_owned()))?;
    }
    if let Some(horizon) = &fields.horizon {
        task.horizon = if horizon.is_empty() {
            None
        } else {
            Some(horizon_from(horizon).ok_or((
                "task_bad_horizon",
                format!("{horizon} is not a horizon (now, next, later, icebox)"),
            ))?)
        };
    }
    if let Some(owner) = &fields.owner {
        task.owner = if owner.is_empty() {
            Owner::Unassigned
        } else {
            // Keep the session hint when the same session keeps the task.
            match &task.owner {
                Owner::Agent { name, session_ref } if name == owner => Owner::Agent {
                    name: name.clone(),
                    session_ref: session_ref.clone(),
                },
                _ => Owner::Agent {
                    name: owner.clone(),
                    session_ref: None,
                },
            }
        };
    }
    task.updated_at = Timestamp::now();
    Ok(())
}

impl App {
    fn task_info_context(&self) -> InfoContext {
        let mut panes = HashMap::new();
        for (ws_idx, ws) in self.state.workspaces.iter().enumerate() {
            for pane_id in ws.tabs.iter().flat_map(|tab| tab.layout.pane_ids()) {
                let Some(public) = self.public_pane_id(ws_idx, pane_id) else {
                    continue;
                };
                if let Some(terminal) = self.state.pane_terminal(pane_id) {
                    panes.insert(terminal.pane_key.clone(), public);
                }
            }
        }
        InfoContext {
            panes,
            pending: self.tasks.pending_keys().into_iter().cloned().collect(),
            conflicts: self.tasks.conflict_keys().into_iter().cloned().collect(),
            now: self.tasks.now(),
        }
    }

    fn task_pane_info(&self, binding: &PaneBinding, context: &InfoContext) -> TaskPaneInfo {
        let on_this_host = self
            .tasks
            .identity()
            .is_some_and(|identity| identity.host.as_str() == binding.host);
        TaskPaneInfo {
            host: binding.host.clone(),
            session: binding.session.clone(),
            pane_key: binding.pane_key.clone(),
            label: binding.label.clone(),
            pane_id: binding
                .pane_key
                .as_ref()
                .filter(|_| on_this_host)
                .and_then(|key| context.panes.get(key).cloned()),
        }
    }

    /// One task as the API shows it, with what the cache knows beside it.
    fn task_info(&self, rev: Rev, task: &Task, context: &InfoContext) -> TaskInfo {
        let key = task.key();
        let derived = crate::tasks::derive(task, self.tasks.runs(), |_| AskRow::Open, context.now);
        TaskInfo {
            key: key.to_string(),
            id: task.id.to_string(),
            owner_host: task.owner_host.to_string(),
            rev: rev.0,
            title: task.title.clone(),
            body: task.body.clone(),
            status: status_info(task.status),
            shown: shown_info(derived.status),
            priority: priority_info(task.priority),
            horizon: task.horizon.and_then(horizon_name).map(str::to_owned),
            owner: match &task.owner {
                Owner::Agent { name, .. } => Some(name.clone()),
                _ => None,
            },
            waits_on: task.waits_on.clone(),
            waits_on_missing: derived.waits_on_missing,
            refs: task
                .refs
                .iter()
                .map(|issue| TaskIssueRefInfo {
                    forge: issue.forge.clone(),
                    forge_id: issue.forge_id.clone(),
                    host: issue.host.clone(),
                    project_id: issue.project_id,
                    project_path: issue.project_path.clone(),
                    number: issue.number,
                })
                .collect(),
            runs: task.runs.clone(),
            pane: task
                .pane
                .as_ref()
                .map(|binding| self.task_pane_info(binding, context)),
            related_pane: task
                .related_pane
                .as_ref()
                .map(|binding| self.task_pane_info(binding, context)),
            client: match task.client {
                crate::tasks::Client::Aruba => Some("aruba".to_owned()),
                _ => None,
            },
            created_by: task.origin.by.clone(),
            created_via: via_name(task.origin.via).to_owned(),
            review: task.review.clone(),
            created_at: task.created_at.0.clone(),
            updated_at: task.updated_at.0.clone(),
            closed: task.closed.as_ref().map(|closed| TaskClosedInfo {
                at: closed.at.0.clone(),
                by: closed.by.clone(),
                reason: closed.reason.clone(),
            }),
            moved_to: task.moved_to.as_ref().map(ToString::to_string),
            moved_from: task.moved_from.as_ref().map(ToString::to_string),
            pending: context.pending.contains(&key),
            conflict: context.conflicts.contains(&key),
        }
    }

    /// A task document the store holds, as the API shows it, if it reads as one.
    fn task_info_from_bytes(
        &self,
        rev: Rev,
        bytes: &[u8],
        context: &InfoContext,
    ) -> Option<TaskInfo> {
        let task = serde_json::from_slice::<Task>(bytes).ok()?;
        Some(self.task_info(rev, &task, context))
    }

    fn store_info(&self) -> TaskStoreInfo {
        let (state, detail) = self.tasks.state();
        TaskStoreInfo {
            state: state.to_owned(),
            detail: detail.map(str::to_owned),
            pending: self.tasks.pending_count() as u32,
            conflicts: self.tasks.conflicts().len() as u32,
        }
    }

    /// The version of a task an edit is built on, with its stored revision.
    fn task_to_edit(&self, key: &TaskKey) -> Option<(Option<Rev>, Task)> {
        self.tasks
            .current(key)
            .map(|(rev, task)| (rev, task.clone()))
    }

    pub(super) fn handle_task_list(&mut self, id: String, params: TaskListParams) -> String {
        let context = self.task_info_context();
        let wanted_pane_key = match params.pane_id.as_deref() {
            Some(raw) => {
                let Some((_, pane_id)) = self.parse_pane_id(raw) else {
                    return encode_error(id, "pane_not_found", format!("pane {raw} not found"));
                };
                match self.state.pane_terminal(pane_id) {
                    Some(terminal) => Some(terminal.pane_key.clone()),
                    None => {
                        return encode_error(id, "pane_not_found", format!("pane {raw} not found"))
                    }
                }
            }
            None => None,
        };
        let tasks = self
            .tasks
            .cache()
            .iter()
            .filter(|(_, cached)| params.include_closed || !cached.task.status.is_closed())
            .filter(|(_, cached)| {
                wanted_pane_key.as_ref().is_none_or(|pane_key| {
                    cached
                        .task
                        .pane
                        .as_ref()
                        .is_some_and(|pane| pane.pane_key.as_ref() == Some(pane_key))
                })
            })
            .filter(|(_, cached)| {
                params.owner.as_ref().is_none_or(|owner| {
                    matches!(&cached.task.owner, Owner::Agent { name, .. } if name == owner)
                })
            })
            .map(|(_, cached)| self.task_info(cached.rev, &cached.task, &context))
            .collect();
        encode_success(
            id,
            ResponseResult::TaskList {
                tasks,
                store: self.store_info(),
            },
        )
    }

    pub(super) fn handle_task_show(&mut self, id: String, params: TaskTarget) -> String {
        let Ok(key) = params.key.parse::<TaskKey>() else {
            return bad_key(id, &params.key);
        };
        let context = self.task_info_context();
        let Some(cached) = self.tasks.cache().get(&key) else {
            return not_found(id, &key);
        };
        let task = self.task_info(cached.rev, &cached.task, &context);
        encode_success(
            id,
            ResponseResult::Task {
                task: Box::new(task),
                write_id: None,
                pending: false,
            },
        )
    }

    /// The pane binding for a pane of this server, by its public id.
    fn binding_for_pane(&self, raw: &str) -> Option<PaneBinding> {
        let identity = self.tasks.identity()?;
        let (ws_idx, pane_id) = self.parse_pane_id(raw)?;
        let terminal = self.state.pane_terminal(pane_id)?;
        let label = self
            .pane_info(ws_idx, pane_id)
            .and_then(|pane| pane.label.or(pane.agent))
            .or_else(|| crate::app::actions::launch_label(terminal.launch_argv.as_ref()))
            .unwrap_or_else(|| raw.to_owned());
        Some(PaneBinding {
            host: identity.host.to_string(),
            session: identity.session.clone(),
            pane_key: Some(terminal.pane_key.clone()),
            label,
        })
    }

    fn answer_write(
        &self,
        id: String,
        result: Result<WriteId, SubmitError>,
        rev: Rev,
        task: &Task,
    ) -> String {
        match result {
            Ok(write) => {
                let context = self.task_info_context();
                encode_success(
                    id,
                    ResponseResult::Task {
                        task: Box::new(self.task_info(rev, task, &context)),
                        write_id: Some(write.to_string()),
                        pending: true,
                    },
                )
            }
            Err(err) => submit_failed(id, &err),
        }
    }

    pub(super) fn handle_task_add(&mut self, id: String, params: TaskAddParams) -> String {
        let Some(identity) = self.tasks.identity().cloned() else {
            return submit_failed(id, &SubmitError::Disabled);
        };
        let origin = Origin {
            host: identity.host.to_string(),
            by: params.by.clone().unwrap_or_else(|| "unknown".to_owned()),
            via: via_from(params.via.as_deref()),
        };
        let mut task = match Task::new(
            identity.host.clone(),
            TaskId::generate(),
            &params.title,
            origin,
            Timestamp::now(),
        ) {
            Ok(task) => task,
            Err(err) => return refused(id, &err),
        };
        let fields = TaskFields {
            body: params.body.clone(),
            priority: params.priority,
            horizon: params.horizon.clone(),
            owner: params.owner.clone(),
            waits_on: params.waits_on.clone(),
            ..TaskFields::default()
        };
        if let Err((code, message)) = apply_fields(&mut task, &fields) {
            return encode_error(id, code, message);
        }
        task.created_at = task.updated_at.clone();
        task.refs = params.refs.iter().map(issue_ref_from).collect();
        if let Some(raw) = params.pane_id.as_deref() {
            match self.binding_for_pane(raw) {
                Some(binding) => task.pane = Some(binding),
                None => return encode_error(id, "pane_not_found", format!("pane {raw} not found")),
            }
        }
        let result = self.tasks.submit_create(&task);
        self.answer_write(id, result, Rev(0), &task)
    }

    pub(super) fn handle_task_update(&mut self, id: String, params: TaskUpdateParams) -> String {
        let Ok(key) = params.key.parse::<TaskKey>() else {
            return bad_key(id, &params.key);
        };
        let Some((_, mut task)) = self.task_to_edit(&key) else {
            return not_found(id, &key);
        };
        if let Err((code, message)) = apply_fields(&mut task, &params.fields) {
            return encode_error(id, code, message);
        }
        let result = self.tasks.submit_update(&task, Rev(params.expected_rev));
        self.answer_write(id, result, Rev(params.expected_rev), &task)
    }

    fn handle_task_move_status(
        &mut self,
        id: String,
        params: TaskMoveStatusParams,
        to: TaskStatus,
    ) -> String {
        let Ok(key) = params.key.parse::<TaskKey>() else {
            return bad_key(id, &params.key);
        };
        let Some((stored, mut task)) = self.task_to_edit(&key) else {
            return not_found(id, &key);
        };
        let Some(expected) = params.expected_rev.map(Rev).or(stored) else {
            return encode_error(
                id,
                "task_not_stored",
                "the task is not stored yet; try again once its write has landed",
            );
        };
        let context = MoveContext {
            other_run_running: false,
            run_is_current: true,
            by: params.by.unwrap_or_else(|| "unknown".to_owned()),
            reason: params.reason,
            at: Timestamp::now(),
        };
        if let Err(err) = task.move_status(to, &context) {
            return refused(id, &err);
        }
        let result = self.tasks.submit_update(&task, expected);
        self.answer_write(id, result, expected, &task)
    }

    pub(super) fn handle_task_done(&mut self, id: String, params: TaskMoveStatusParams) -> String {
        self.handle_task_move_status(id, params, TaskStatus::Done)
    }

    pub(super) fn handle_task_drop(&mut self, id: String, params: TaskMoveStatusParams) -> String {
        self.handle_task_move_status(id, params, TaskStatus::Dropped)
    }

    pub(super) fn handle_task_reopen(
        &mut self,
        id: String,
        params: TaskMoveStatusParams,
    ) -> String {
        self.handle_task_move_status(id, params, TaskStatus::Open)
    }

    pub(super) fn handle_task_bind(&mut self, id: String, params: TaskBindParams) -> String {
        let Ok(key) = params.key.parse::<TaskKey>() else {
            return bad_key(id, &params.key);
        };
        let Some((_, mut task)) = self.task_to_edit(&key) else {
            return not_found(id, &key);
        };
        let binding = if params.clear {
            None
        } else {
            let Some(raw) = params.pane_id.as_deref() else {
                return encode_error(
                    id,
                    "task_bind_needs_pane",
                    "name a pane_id, or set clear to remove the binding",
                );
            };
            match self.binding_for_pane(raw) {
                Some(binding) => Some(binding),
                None => return encode_error(id, "pane_not_found", format!("pane {raw} not found")),
            }
        };
        if params.related {
            task.related_pane = binding;
        } else {
            task.pane = binding;
        }
        task.updated_at = Timestamp::now();
        let result = self.tasks.submit_update(&task, Rev(params.expected_rev));
        self.answer_write(id, result, Rev(params.expected_rev), &task)
    }

    pub(super) fn handle_task_conflicts(&mut self, id: String) -> String {
        let context = self.task_info_context();
        let conflicts = self
            .tasks
            .conflicts()
            .iter()
            .map(|held| TaskConflictInfo {
                write_id: held.write.to_string(),
                key: held.key.to_string(),
                mine: self.task_info(Rev(0), &held.mine, &context),
                theirs: held.theirs.as_ref().and_then(|value| {
                    let bytes = serde_json::to_vec(value).ok()?;
                    self.task_info_from_bytes(held.theirs_rev?, &bytes, &context)
                }),
                theirs_rev: held.theirs_rev.map_or(0, |rev| rev.0),
            })
            .collect();
        encode_success(id, ResponseResult::TaskConflicts { conflicts })
    }

    pub(super) fn handle_task_resolve_conflict(
        &mut self,
        id: String,
        params: TaskResolveConflictParams,
    ) -> String {
        let Ok(key) = params.key.parse::<TaskKey>() else {
            return bad_key(id, &params.key);
        };
        let write = WriteId::from_string(params.write_id.clone());
        let choice = match params.choice {
            TaskConflictChoice::Mine => Choice::Mine,
            TaskConflictChoice::Theirs => Choice::Theirs,
            TaskConflictChoice::Merged => {
                let held = self
                    .tasks
                    .conflicts()
                    .iter()
                    .find(|held| held.write == write && held.key == key);
                let Some(held) = held else {
                    // Settled by someone else already: the same answer as a
                    // resolver that arrives late.
                    return self.stale_answer(id, &key, &write, params.expected_rev);
                };
                let mut merged = held.mine.clone();
                if let Some(fields) = &params.doc {
                    if let Err((code, message)) = apply_fields(&mut merged, fields) {
                        return encode_error(id, code, message);
                    }
                }
                Choice::Merged(Box::new(merged))
            }
            TaskConflictChoice::Unknown => {
                return encode_error(id, "task_bad_choice", "choose mine, theirs or merged");
            }
        };
        match self
            .tasks
            .resolve_conflict(&key, &write, choice, Rev(params.expected_rev))
        {
            Ok(ticket) => encode_success(id, ResponseResult::TaskResolving { ticket }),
            Err(err) => submit_failed(id, &err),
        }
    }

    fn stale_answer(
        &mut self,
        id: String,
        key: &TaskKey,
        write: &WriteId,
        expected: u64,
    ) -> String {
        match self
            .tasks
            .resolve_conflict(key, write, Choice::Theirs, Rev(expected))
        {
            Err(err) => submit_failed(id, &err),
            // Nothing is held under that id, so this cannot be taken.
            Ok(_) => encode_error(id, "stale_revision", "the conflict is not the one you saw"),
        }
    }

    /// Make sure a timer will fire when the next run crosses its five minutes.
    pub(super) fn arm_task_status_timer(&mut self) {
        let Some(wait) = self.tasks.next_status_deadline() else {
            return;
        };
        let now = Instant::now();
        let due = now + wait;
        // An earlier timer that is still to fire covers this one: it re-arms.
        if self
            .task_status_timer
            .is_some_and(|armed| armed > now && armed <= due)
        {
            return;
        }
        self.task_status_timer = Some(due);
        let tx = self.event_tx.clone();
        let generation = self.tasks.generation();
        let spawned = crate::thread_spawn::spawn_named("herdr-task-timer", move || {
            std::thread::sleep(wait);
            let _ = tx.blocking_send(crate::events::AppEvent::TaskStatusDue { generation });
        });
        if let Err(err) = spawned {
            tracing::warn!(%err, "cannot start the task status timer");
            self.task_status_timer = None;
        }
    }

    /// The timer fired: tell subscribers which tasks changed their shown status
    /// with the clock alone, and arm the next one.
    pub(super) fn task_status_timer_fired(&mut self) {
        self.task_status_timer = None;
        let revision = self.tasks.cache().revision();
        for key in self.tasks.due_status_changes() {
            self.emit_event(EventEnvelope {
                event: EventKind::TaskChanged,
                data: EventData::TaskChanged {
                    key: Some(key.to_string()),
                    revision,
                    write_id: None,
                    outcome: None,
                    ticket: None,
                    resolution: None,
                },
            });
        }
        self.arm_task_status_timer();
    }

    /// What the store thread reported: tell subscribers how writes ended, how
    /// conflicts were settled, and that the cache changed.
    pub(super) fn publish_task_changes(&mut self, changed: bool) {
        let context = self.task_info_context();
        let revision = self.tasks.cache().revision();
        let mut events = Vec::new();
        for (write, outcome) in self.tasks.take_outcomes() {
            let current = |rev: Rev, bytes: &[u8]| self.task_info_from_bytes(rev, bytes, &context);
            let info = match outcome {
                WriteOutcome::Applied { rev } => TaskOutcomeInfo::Applied { rev: rev.0 },
                WriteOutcome::Conflict { current: found } => TaskOutcomeInfo::Conflict {
                    current: found
                        .and_then(|(rev, bytes)| current(rev, &bytes))
                        .map(Box::new),
                },
                WriteOutcome::Exists {
                    current: (rev, bytes),
                } => TaskOutcomeInfo::Exists {
                    current: current(rev, &bytes).map(Box::new),
                },
                WriteOutcome::Failed(message) => TaskOutcomeInfo::Failed { message },
                WriteOutcome::Unknown(message) => TaskOutcomeInfo::Unknown { message },
            };
            events.push(EventData::TaskChanged {
                key: None,
                revision,
                write_id: Some(write.to_string()),
                outcome: Some(info),
                ticket: None,
                resolution: None,
            });
        }
        for (ticket, outcome) in self.tasks.take_resolutions() {
            let info = match outcome {
                ResolveOutcome::Requeued => TaskResolutionInfo::Requeued,
                ResolveOutcome::Dropped { cancelled } => TaskResolutionInfo::Dropped {
                    cancelled: cancelled.iter().map(ToString::to_string).collect(),
                },
                ResolveOutcome::Stale { current } => TaskResolutionInfo::Stale {
                    current: current
                        .and_then(|(rev, bytes)| self.task_info_from_bytes(rev, &bytes, &context))
                        .map(Box::new),
                },
                ResolveOutcome::Failed(message) => TaskResolutionInfo::Failed { message },
            };
            events.push(EventData::TaskChanged {
                key: None,
                revision,
                write_id: None,
                outcome: None,
                ticket: Some(ticket),
                resolution: Some(info),
            });
        }
        if changed {
            events.push(EventData::TaskChanged {
                key: None,
                revision,
                write_id: None,
                outcome: None,
                ticket: None,
                resolution: None,
            });
        }
        for data in events {
            self.emit_event(EventEnvelope {
                event: EventKind::TaskChanged,
                data,
            });
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::time::{Duration, Instant};

    use crate::api::schema::{
        EventData, EventKind, Method, Request, TaskAddParams, TaskBindParams, TaskConflictChoice,
        TaskListParams, TaskMoveStatusParams, TaskOutcomeInfo, TaskResolveConflictParams,
        TaskTarget, TaskUpdateParams,
    };
    use crate::app::App;
    use crate::config::Config;
    use crate::workspace::Workspace;

    struct Harness {
        app: App,
        pane_id: String,
        dir: std::path::PathBuf,
        _runtime: tokio::runtime::Runtime,
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// An app whose task service runs on a real file store in a scratch dir.
    fn harness() -> Harness {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let dir = std::env::temp_dir().join(format!(
            "herdr-task-api-{}-{}",
            std::process::id(),
            crate::ulid::new_ulid()
        ));
        Harness {
            app: app_with_store(&dir),
            pane_id: String::new(),
            dir,
            _runtime: runtime,
        }
        .with_pane()
    }

    fn app_with_store(dir: &std::path::Path) -> App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.workspaces = vec![Workspace::test_new("tasks")];
        app.state.ensure_test_terminals();
        app.policy.persist_session = true;
        app.task_store_dir = dir.join("store");
        app.start_task_store();
        app
    }

    impl Harness {
        fn with_pane(mut self) -> Self {
            let pane_id = self.app.state.workspaces[0].tabs[0].root_pane;
            self.pane_id = self.app.public_pane_id(0, pane_id).unwrap();
            self
        }

        fn request(&mut self, method: Method) -> serde_json::Value {
            let raw = self.app.handle_api_request(Request {
                id: "test".into(),
                method,
            });
            serde_json::from_str(&raw).unwrap()
        }

        fn wait(&mut self, mut done: impl FnMut(&mut App) -> bool) {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                self.app.drain_all_internal_events();
                if done(&mut self.app) {
                    return;
                }
                assert!(Instant::now() < deadline, "timed out");
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        fn loaded(mut self) -> Self {
            self.wait(|app| app.tasks.is_loaded());
            self
        }

        /// Add a task and wait until the store has it. Returns its key.
        fn add(&mut self, title: &str) -> String {
            let response = self.request(Method::TaskAdd(TaskAddParams {
                title: title.into(),
                by: Some("herdr-wrk".into()),
                ..TaskAddParams::default()
            }));
            let key = response["result"]["task"]["key"]
                .as_str()
                .unwrap()
                .to_owned();
            self.wait(|app| app.tasks.pending_count() == 0 && app.tasks.cache().len() >= 1);
            key
        }

        fn task_changed(&self, since: u64) -> Vec<serde_json::Value> {
            self.app
                .event_hub
                .events_after(since)
                .into_iter()
                .filter(|(_, event)| event.event == EventKind::TaskChanged)
                .map(|(_, event)| serde_json::to_value(event.data).unwrap())
                .collect()
        }
    }

    #[test]
    fn task_add_answers_pending_at_once_and_the_outcome_arrives_as_task_changed() {
        let mut h = harness().loaded();
        let before = h.app.event_hub.current_sequence();

        let response = h.request(Method::TaskAdd(TaskAddParams {
            title: "write the store".into(),
            priority: Some(crate::api::schema::TaskPriorityInfo::High),
            owner: Some("herdr-wrk".into()),
            by: Some("herdr-wrk".into()),
            via: Some("cli".into()),
            ..TaskAddParams::default()
        }));
        let result = &response["result"];
        assert_eq!(result["type"], "task");
        assert_eq!(result["pending"], true);
        let write_id = result["write_id"].as_str().expect("a write id").to_owned();
        let task = &result["task"];
        assert_eq!(task["title"], "write the store");
        assert_eq!(task["status"], "open");
        assert_eq!(task["priority"], "high");
        assert_eq!(task["owner"], "herdr-wrk");
        assert_eq!(task["created_by"], "herdr-wrk");
        assert_eq!(task["created_via"], "cli");
        assert_eq!(task["pending"], true, "its write has not landed yet");
        let key = task["key"].as_str().unwrap().to_owned();
        assert!(key.starts_with("task.local.tk-"), "{key}");

        h.wait(|app| app.tasks.pending_count() == 0 && app.tasks.cache().len() == 1);
        let outcomes: Vec<_> = h
            .task_changed(before)
            .into_iter()
            .filter(|data| data["write_id"] == write_id.as_str())
            .collect();
        assert_eq!(outcomes.len(), 1, "one outcome for the write");
        assert_eq!(outcomes[0]["outcome"]["status"], "applied");
        assert_eq!(outcomes[0]["outcome"]["rev"], 1);

        let shown = h.request(Method::TaskShow(TaskTarget { key: key.clone() }));
        assert_eq!(shown["result"]["task"]["rev"], 1);
        assert!(
            shown["result"]["task"]["pending"].is_null(),
            "a false flag is left out of the answer"
        );
        let listed = h.request(Method::TaskList(TaskListParams::default()));
        assert_eq!(listed["result"]["tasks"].as_array().unwrap().len(), 1);
        assert_eq!(listed["result"]["store"]["state"], "ready");
        assert_eq!(listed["result"]["store"]["pending"], 0);
    }

    #[test]
    fn a_title_that_is_too_long_is_refused_and_nothing_is_stored() {
        let mut h = harness().loaded();
        let response = h.request(Method::TaskAdd(TaskAddParams {
            title: "x".repeat(201),
            ..TaskAddParams::default()
        }));
        assert_eq!(response["error"]["code"], "task_title_too_long");
        assert_eq!(h.app.tasks.pending_count(), 0);
    }

    #[test]
    fn done_drop_and_reopen_move_the_status_through_the_store() {
        let mut h = harness().loaded();
        let key = h.add("finish me");
        let done = h.request(Method::TaskDone(TaskMoveStatusParams {
            key: key.clone(),
            by: Some("herdr-wrk".into()),
            reason: Some("shipped".into()),
            ..TaskMoveStatusParams::default()
        }));
        assert_eq!(done["result"]["task"]["status"], "done");
        h.wait(|app| app.tasks.pending_count() == 0);
        h.wait(|app| {
            app.tasks
                .cache()
                .iter()
                .all(|(_, cached)| cached.rev == crate::tasks::Rev(2))
        });
        let shown = h.request(Method::TaskShow(TaskTarget { key: key.clone() }));
        assert_eq!(shown["result"]["task"]["status"], "done");
        assert_eq!(shown["result"]["task"]["closed"]["reason"], "shipped");

        // A done task is not listed unless asked for.
        let listed = h.request(Method::TaskList(TaskListParams::default()));
        assert!(listed["result"]["tasks"].as_array().unwrap().is_empty());
        let all = h.request(Method::TaskList(TaskListParams {
            include_closed: true,
            ..TaskListParams::default()
        }));
        assert_eq!(all["result"]["tasks"].as_array().unwrap().len(), 1);

        // Done cannot be dropped, only reopened.
        let refused = h.request(Method::TaskDrop(TaskMoveStatusParams {
            key: key.clone(),
            ..TaskMoveStatusParams::default()
        }));
        assert_eq!(refused["error"]["code"], "task_bad_status_move");
        let reopened = h.request(Method::TaskReopen(TaskMoveStatusParams {
            key,
            ..TaskMoveStatusParams::default()
        }));
        assert_eq!(reopened["result"]["task"]["status"], "open");
    }

    #[test]
    fn a_stale_edit_is_held_listed_and_settled_once() {
        let mut h = harness().loaded();
        let key = h.add("original");
        let before = h.app.event_hub.current_sequence();

        let stale = h.request(Method::TaskUpdate(TaskUpdateParams {
            key: key.clone(),
            expected_rev: 9,
            fields: crate::api::schema::TaskFields {
                title: Some("mine".into()),
                ..Default::default()
            },
        }));
        let write_id = stale["result"]["write_id"].as_str().unwrap().to_owned();
        h.wait(|app| !app.tasks.conflicts().is_empty());
        let conflicted: Vec<_> = h
            .task_changed(before)
            .into_iter()
            .filter(|data| data["write_id"] == write_id.as_str())
            .collect();
        assert_eq!(conflicted[0]["outcome"]["status"], "conflict");
        assert_eq!(conflicted[0]["outcome"]["current"]["title"], "original");

        let listed = h.request(Method::TaskConflicts(crate::api::schema::EmptyParams {}));
        let conflicts = listed["result"]["conflicts"].as_array().unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0]["write_id"], write_id.as_str());
        assert_eq!(conflicts[0]["mine"]["title"], "mine");
        assert_eq!(conflicts[0]["theirs"]["title"], "original");
        assert_eq!(conflicts[0]["theirs_rev"], 1);
        let shown = h.request(Method::TaskShow(TaskTarget { key: key.clone() }));
        assert_eq!(shown["result"]["task"]["conflict"], true);

        let resolve = |choice, expected_rev| {
            Method::TaskResolveConflict(TaskResolveConflictParams {
                key: key.clone(),
                write_id: write_id.clone(),
                choice,
                doc: None,
                expected_rev,
            })
        };
        let answered = h.request(resolve(TaskConflictChoice::Mine, 1));
        assert_eq!(answered["result"]["type"], "task_resolving");
        h.wait(|app| app.tasks.conflicts().is_empty() && app.tasks.pending_count() == 0);
        h.wait(|app| {
            app.tasks
                .cache()
                .iter()
                .all(|(_, cached)| cached.task.title == "mine")
        });
        let events = h.task_changed(before);
        assert!(
            events
                .iter()
                .any(|data| data["resolution"]["status"] == "requeued"),
            "the resolution is told: {events:?}"
        );

        // A second resolver, who saw the same conflict, is turned away.
        let late = h.request(resolve(TaskConflictChoice::Theirs, 1));
        assert_eq!(late["error"]["code"], "stale_revision");
    }

    #[test]
    fn a_task_of_another_host_is_refused_and_not_queued() {
        let dir = std::env::temp_dir().join(format!(
            "herdr-task-api-elsewhere-{}-{}",
            std::process::id(),
            crate::ulid::new_ulid()
        ));
        let theirs = {
            let mut task = crate::tasks::Task::new(
                crate::tasks::HostId::new("ac-mbm5").unwrap(),
                crate::tasks::TaskId::new("tk-theirs").unwrap(),
                "their task",
                crate::tasks::Origin {
                    host: "ac-mbm5".into(),
                    by: "x".into(),
                    via: crate::tasks::Via::Cli,
                },
                crate::tasks::Timestamp("2026-10-21T20:00:00Z".into()),
            )
            .unwrap();
            task.updated_at = task.created_at.clone();
            task
        };
        {
            // The store the server will open already holds the other Mac's task.
            let mut store = crate::tasks::FileStore::open(dir.join("store")).unwrap();
            let write = crate::tasks::WriteId::generate();
            crate::tasks::TaskStore::create(&mut store, &theirs.key(), &theirs, write).unwrap();
        }
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let mut h = Harness {
            app: app_with_store(&dir),
            pane_id: String::new(),
            dir,
            _runtime: runtime,
        }
        .with_pane()
        .loaded();
        h.wait(|app| app.tasks.cache().len() == 1);

        let response = h.request(Method::TaskDone(TaskMoveStatusParams {
            key: theirs.key().to_string(),
            ..TaskMoveStatusParams::default()
        }));
        assert_eq!(response["error"]["code"], "task_owned_elsewhere");
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("ac-mbm5"));
        assert_eq!(h.app.tasks.pending_count(), 0, "nothing was queued");
    }

    #[test]
    fn a_task_binds_to_a_pane_by_its_stable_key_and_lists_by_pane() {
        let mut h = harness().loaded();
        let pane_id = h.pane_id.clone();
        let response = h.request(Method::TaskAdd(TaskAddParams {
            title: "on this pane".into(),
            pane_id: Some(pane_id.clone()),
            ..TaskAddParams::default()
        }));
        let pane = &response["result"]["task"]["pane"];
        assert_eq!(pane["pane_id"], pane_id.as_str());
        assert!(pane["pane_key"].is_string(), "{pane}");
        assert_eq!(pane["host"], "local");
        h.wait(|app| app.tasks.pending_count() == 0);

        let on_pane = h.request(Method::TaskList(TaskListParams {
            pane_id: Some(pane_id.clone()),
            ..TaskListParams::default()
        }));
        assert_eq!(on_pane["result"]["tasks"].as_array().unwrap().len(), 1);

        let key = response["result"]["task"]["key"]
            .as_str()
            .unwrap()
            .to_owned();
        h.wait(|app| app.tasks.cache().len() == 1);
        let cleared = h.request(Method::TaskBind(TaskBindParams {
            key,
            expected_rev: 1,
            clear: true,
            ..TaskBindParams::default()
        }));
        assert_eq!(cleared["result"]["type"], "task");
        h.wait(|app| app.tasks.pending_count() == 0);
        h.wait(|app| {
            app.tasks
                .cache()
                .iter()
                .all(|(_, cached)| cached.task.pane.is_none())
        });
        let elsewhere = h.request(Method::TaskList(TaskListParams {
            pane_id: Some(pane_id),
            ..TaskListParams::default()
        }));
        assert!(elsewhere["result"]["tasks"].as_array().unwrap().is_empty());

        let unknown = h.request(Method::TaskAdd(TaskAddParams {
            title: "nowhere".into(),
            pane_id: Some("w9:p9".into()),
            ..TaskAddParams::default()
        }));
        assert_eq!(unknown["error"]["code"], "pane_not_found");
    }

    #[test]
    fn a_server_without_a_task_store_says_so() {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        let raw = app.handle_api_request(Request {
            id: "test".into(),
            method: Method::TaskAdd(TaskAddParams {
                title: "x".into(),
                ..TaskAddParams::default()
            }),
        });
        let response: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(response["error"]["code"], "tasks_disabled");
        let listed: serde_json::Value = serde_json::from_str(&app.handle_api_request(Request {
            id: "test".into(),
            method: Method::TaskList(TaskListParams::default()),
        }))
        .unwrap();
        assert_eq!(listed["result"]["store"]["state"], "disabled");
    }

    #[test]
    fn a_run_that_crosses_five_minutes_is_announced_for_its_task() {
        thread_local! {
            static NOW: std::cell::Cell<u64> = const { std::cell::Cell::new(2_000_000_000) };
        }
        fn clock() -> std::time::SystemTime {
            std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(NOW.with(std::cell::Cell::get))
        }
        let mut h = harness().loaded();
        let key = h.add("being started");
        h.app.tasks.set_clock(clock);
        let task_key = key.parse::<crate::tasks::TaskKey>().unwrap();
        let record = serde_json::json!({
            "id": "rn-1", "task": key, "state": "starting", "engine": "claude_code"
        });
        h.app.tasks.apply(crate::tasks::StoreEvent::Changed(
            crate::tasks::WatchEvent {
                key: crate::tasks::StoreKey("run.local.rn-1".into()),
                rev: crate::tasks::Rev(1),
                value: Some(serde_json::to_vec(&record).unwrap()),
                at: Some(clock()),
            },
        ));
        let shown = h.request(Method::TaskShow(TaskTarget { key: key.clone() }));
        assert_eq!(shown["result"]["task"]["shown"], "running");

        let before = h.app.event_hub.current_sequence();
        NOW.with(|now| now.set(now.get() + 301));
        h.app.task_status_timer_fired();
        let shown = h.request(Method::TaskShow(TaskTarget { key: key.clone() }));
        assert_eq!(shown["result"]["task"]["shown"], "run_unknown");
        let announced: Vec<_> = h
            .task_changed(before)
            .into_iter()
            .filter(|data| data["key"] == key.as_str())
            .collect();
        assert_eq!(announced.len(), 1, "told once, for that task");
        let _ = task_key;
    }

    #[test]
    fn unused_event_shapes_stay_in_the_schema() {
        // The outcome and event types are part of the wire contract; keep them
        // reachable from here so a rename fails loudly.
        let data = EventData::TaskChanged {
            key: None,
            revision: 1,
            write_id: Some("w".into()),
            outcome: Some(TaskOutcomeInfo::Applied { rev: 1 }),
            ticket: None,
            resolution: None,
        };
        let json = serde_json::to_value(data).unwrap();
        assert_eq!(json["type"], "task_changed");
        assert_eq!(json["outcome"]["status"], "applied");
    }
}
