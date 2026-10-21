//! Run records (`run.<starter>.<id>`, contracts § 6) and the status a reader
//! derives from them.
//!
//! A run record is written by whoever starts the run; the owner of the task
//! only reads it. Which records count for a task is decided by the key, which
//! the bus lets only its own host write (or worklink): a record under any other
//! starter is ignored, whatever its content says.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use super::ids::{Rev, TaskKey};
use super::model::{Task, TaskStatus, Timestamp};
use super::store::WatchEvent;

/// A run left in `starting` this long is stuck, judged by the bus entry's time.
pub(crate) const STARTING_GRACE: Duration = Duration::from_secs(5 * 60);
/// The starter of runs that uzi executes, besides the owner host itself.
const WORKLINK: &str = "worklink";
const RUN_PREFIX: &str = "run.";
/// How many refused records the cache remembers.
const REJECTED_KEPT: usize = 100;

const FIELD_MAX_CHARS: usize = 200;
const DECISIONS_LIMIT: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RunState {
    Starting,
    Running,
    Done,
    Failed,
    Stopped,
    /// `unknown`, or a state a newer writer made up: never shown as running.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Engine {
    Uzi,
    ClaudeCode,
    Pi,
    Codex,
    #[default]
    #[serde(other)]
    Other,
}

/// One run record, as the starter wrote it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RunRecord {
    /// `rn-<ULID>`.
    pub id: String,
    /// The key of the task this run is for.
    pub task: TaskKey,
    #[serde(default)]
    pub engine: Engine,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    pub state: RunState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decisions: Vec<String>,
}

impl RunRecord {
    /// Bounds for a document read from the bus: a writer never makes one this
    /// big.
    fn check_sizes(&self) -> Result<(), &'static str> {
        let long = |text: &str| text.chars().count() > FIELD_MAX_CHARS;
        let optional_long = |text: &Option<String>| text.as_deref().is_some_and(long);
        if long(&self.id)
            || optional_long(&self.route)
            || optional_long(&self.host)
            || optional_long(&self.session)
            || optional_long(&self.correlation)
            || optional_long(&self.intent)
        {
            return Err("a field is too long");
        }
        if self.decisions.len() > DECISIONS_LIMIT || self.decisions.iter().any(|d| long(d)) {
            return Err("too many or too long decisions");
        }
        Ok(())
    }
}

/// `run.<starter>.<id>`: the starter is a host id or `worklink`, the id one
/// token of letters, digits and `-`.
fn parse_run_key(key: &str) -> Option<(&str, &str)> {
    let (starter, id) = key.strip_prefix(RUN_PREFIX)?.split_once('.')?;
    let token = |text: &str| {
        !text.is_empty()
            && text.len() <= 64
            && text
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    };
    (token(starter) && token(id)).then_some((starter, id))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CachedRun {
    pub rev: Rev,
    pub record: RunRecord,
    /// Who wrote the key: the part of the key, not of the document.
    pub starter: String,
    /// When the bus took this revision (the time of its entry); when the
    /// backend cannot say, when this server first saw it.
    pub at: SystemTime,
}

impl CachedRun {
    /// Whether the owner of a task may believe this record: the owner's own
    /// host and worklink may start runs of its tasks.
    fn starter_allowed_for(&self, task: &Task) -> bool {
        self.starter == task.owner_host.as_str() || self.starter == WORKLINK
    }

    fn starting_deadline(&self) -> SystemTime {
        self.at + STARTING_GRACE
    }

    /// Running, or starting for less than five minutes by the bus entry's time.
    pub(crate) fn is_live(&self, now: SystemTime) -> bool {
        match self.record.state {
            RunState::Running => true,
            RunState::Starting => now < self.starting_deadline(),
            _ => false,
        }
    }
}

/// The server's view of the run records.
#[derive(Debug, Default)]
pub(crate) struct RunCache {
    runs: BTreeMap<String, CachedRun>,
    /// The keys of the runs of each task, whoever started them.
    by_task: HashMap<TaskKey, BTreeSet<String>>,
    rejected: Vec<(String, String)>,
}

impl RunCache {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn len(&self) -> usize {
        self.runs.len()
    }

    pub(crate) fn rejected(&self) -> &[(String, String)] {
        &self.rejected
    }

    /// Apply one change from a scan or a watch. Returns the tasks whose runs
    /// changed. Keys that are not run keys are not this cache's business; a
    /// document that does not match its key or breaks a limit is refused.
    pub(crate) fn apply(&mut self, event: &WatchEvent, now: SystemTime) -> Vec<TaskKey> {
        let name = &event.key.0;
        let Some((starter, id)) = parse_run_key(name) else {
            return Vec::new();
        };
        let Some(bytes) = &event.value else {
            return self.remove(name);
        };
        let record = match serde_json::from_slice::<RunRecord>(bytes) {
            Ok(record) => record,
            Err(err) => return self.reject(name, format!("unreadable: {err}")),
        };
        if record.id != id {
            return self.reject(name, "the document names another run".into());
        }
        if let Err(why) = record.check_sizes() {
            return self.reject(name, why.into());
        }
        if self
            .runs
            .get(name)
            .is_some_and(|cached| cached.rev >= event.rev)
        {
            return Vec::new();
        }
        let mut changed = vec![record.task.clone()];
        if let Some(old) = self.runs.get(name) {
            if old.record.task != record.task {
                changed.push(old.record.task.clone());
                if let Some(keys) = self.by_task.get_mut(&old.record.task) {
                    keys.remove(name);
                }
            }
        }
        self.by_task
            .entry(record.task.clone())
            .or_default()
            .insert(name.clone());
        self.runs.insert(
            name.clone(),
            CachedRun {
                rev: event.rev,
                starter: starter.to_owned(),
                at: event.at.unwrap_or(now),
                record,
            },
        );
        changed
    }

    fn remove(&mut self, name: &str) -> Vec<TaskKey> {
        let Some(old) = self.runs.remove(name) else {
            return Vec::new();
        };
        if let Some(keys) = self.by_task.get_mut(&old.record.task) {
            keys.remove(name);
        }
        vec![old.record.task]
    }

    fn reject(&mut self, name: &str, why: String) -> Vec<TaskKey> {
        self.rejected.push((name.to_owned(), why));
        if self.rejected.len() > REJECTED_KEPT {
            let excess = self.rejected.len() - REJECTED_KEPT;
            self.rejected.drain(..excess);
        }
        Vec::new()
    }

    /// The runs of `task` that its owner believes, oldest first (run ids are
    /// ULIDs, so their order is the order they were made in).
    pub(crate) fn runs_of<'a>(&'a self, task: &'a Task) -> impl Iterator<Item = &'a CachedRun> {
        let mut found: Vec<&CachedRun> = self
            .by_task
            .get(&task.key())
            .into_iter()
            .flatten()
            .filter_map(|name| self.runs.get(name))
            .filter(|run| run.starter_allowed_for(task))
            .collect();
        found.sort_by(|left, right| left.record.id.cmp(&right.record.id));
        found.into_iter()
    }

    /// The next moment a run of a task in `tasks` crosses the five minutes,
    /// after `now`.
    pub(crate) fn next_expiry<'a>(
        &self,
        tasks: impl Iterator<Item = &'a Task>,
        now: SystemTime,
    ) -> Option<SystemTime> {
        tasks
            .flat_map(|task| self.runs_of(task))
            .filter(|run| run.record.state == RunState::Starting)
            .map(CachedRun::starting_deadline)
            .filter(|deadline| *deadline > now)
            .min()
    }

    /// The tasks with a run that crossed the five minutes after `since`, up to
    /// and including `now`: their shown status changed with the clock alone.
    pub(crate) fn expired_between<'a>(
        &self,
        tasks: impl Iterator<Item = &'a Task>,
        since: SystemTime,
        now: SystemTime,
    ) -> Vec<TaskKey> {
        tasks
            .filter(|task| {
                self.runs_of(task).any(|run| {
                    run.record.state == RunState::Starting
                        && run.starting_deadline() > since
                        && run.starting_deadline() <= now
                })
            })
            .map(Task::key)
            .collect()
    }
}

/// What the asks row a task waits on looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AskRow {
    Open,
    Answered,
    Closed,
    /// The row does not exist (any more).
    Missing,
}

/// What a reader shows. Only `Done`, `Dropped`, `Open`, `Running` and `Unknown`
/// are ever stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shown {
    Open,
    Running,
    Waiting,
    /// A run is unknown or stuck in `starting`: shown as "run unknown", never
    /// as running.
    RunUnknown,
    Done,
    Dropped,
    /// A status a newer build wrote.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Derived {
    pub status: Shown,
    /// `waits_on` names a row that no longer exists: shown as such, with no
    /// change of state.
    pub waits_on_missing: bool,
}

/// The status a reader shows for `task`, in this order: (1) a stored Done or
/// Dropped; (2) Running, for a run from an allowed starter that is `running`,
/// or `starting` for less than five minutes by the bus entry's time; (3)
/// Waiting, while `waits_on` names a row that is open or answered; (4) the
/// stored status, or "run unknown" when a run is unknown or stuck.
pub(crate) fn derive(
    task: &Task,
    runs: &RunCache,
    ask: impl Fn(&str) -> AskRow,
    now: SystemTime,
) -> Derived {
    let shown = |status| Derived {
        status,
        waits_on_missing: false,
    };
    match task.status {
        TaskStatus::Done => return shown(Shown::Done),
        TaskStatus::Dropped => return shown(Shown::Dropped),
        _ => {}
    }
    let mut stuck = false;
    for run in runs.runs_of(task) {
        match run.record.state {
            RunState::Running => return shown(Shown::Running),
            RunState::Starting if now < run.starting_deadline() => return shown(Shown::Running),
            RunState::Starting | RunState::Unknown => stuck = true,
            RunState::Done | RunState::Failed | RunState::Stopped => {}
        }
    }
    let mut waits_on_missing = false;
    if let Some(row) = task.waits_on.as_deref() {
        match ask(row) {
            AskRow::Open | AskRow::Answered => {
                return Derived {
                    status: Shown::Waiting,
                    waits_on_missing: false,
                }
            }
            AskRow::Closed => {}
            AskRow::Missing => waits_on_missing = true,
        }
    }
    let status = if stuck {
        Shown::RunUnknown
    } else {
        match task.status {
            TaskStatus::Running => Shown::Running,
            TaskStatus::Open => Shown::Open,
            _ => Shown::Unknown,
        }
    };
    Derived {
        status,
        waits_on_missing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::ids::{HostId, TaskId};
    use crate::tasks::model::{Origin, Via};
    use crate::tasks::store::StoreKey;

    fn task(status: TaskStatus, waits_on: Option<&str>) -> Task {
        let mut task = Task::new(
            HostId::new("m4m").unwrap(),
            TaskId::new("tk-1").unwrap(),
            "a task",
            Origin {
                host: "m4m".into(),
                by: "x".into(),
                via: Via::Cli,
            },
            Timestamp("2026-10-21T20:00:00Z".into()),
        )
        .unwrap();
        task.status = status;
        task.waits_on = waits_on.map(str::to_owned);
        task
    }

    fn run_event(starter: &str, id: &str, state: RunState, at: SystemTime) -> WatchEvent {
        let record = RunRecord {
            id: id.to_owned(),
            task: "task.m4m.tk-1".parse().unwrap(),
            engine: Engine::ClaudeCode,
            route: None,
            host: None,
            session: None,
            correlation: None,
            intent: None,
            state,
            started_at: None,
            ended_at: None,
            decisions: Vec::new(),
        };
        WatchEvent {
            key: StoreKey(format!("run.{starter}.{id}")),
            rev: Rev(1),
            value: Some(serde_json::to_vec(&record).unwrap()),
            at: Some(at),
        }
    }

    fn base() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000)
    }

    fn cache_with(runs: &[(&str, &str, RunState, Duration)]) -> RunCache {
        let mut cache = RunCache::new();
        for (starter, id, state, age) in runs {
            let at = base() - *age;
            cache.apply(&run_event(starter, id, *state, at), base());
        }
        cache
    }

    /// What the task stores, its `waits_on` row, its runs (starter, id, state,
    /// age), what a reader shows, and whether the row is missing.
    type Row = (
        TaskStatus,
        Option<AskRow>,
        Vec<(&'static str, &'static str, RunState, Duration)>,
        Shown,
        bool,
    );

    #[test]
    fn status_precedence_table() {
        use AskRow::{Answered, Closed, Missing, Open as AskOpen};
        use RunState::{Done as RunDone, Running as RunRunning, Starting, Unknown as RunUnknown};
        use TaskStatus::{Dropped, Open, Running};

        let fresh = Duration::from_secs(60);
        let stale = Duration::from_secs(6 * 60);
        let none = Duration::ZERO;
        // (what the task stores, its waits_on row, the runs, what a reader shows)
        let rows: Vec<Row> = vec![
            // 1: a stored Done or Dropped always wins
            (
                TaskStatus::Done,
                Some(AskOpen),
                vec![("m4m", "rn-1", RunRunning, none)],
                Shown::Done,
                false,
            ),
            (
                Dropped,
                None,
                vec![("m4m", "rn-1", RunRunning, none)],
                Shown::Dropped,
                false,
            ),
            // 2: a running run, or a young starting one, from an allowed starter
            (
                Open,
                None,
                vec![("m4m", "rn-1", RunRunning, none)],
                Shown::Running,
                false,
            ),
            (
                Open,
                None,
                vec![("worklink", "rn-1", RunRunning, none)],
                Shown::Running,
                false,
            ),
            (
                Open,
                None,
                vec![("m4m", "rn-1", Starting, fresh)],
                Shown::Running,
                false,
            ),
            // ...which beats Waiting
            (
                Open,
                Some(AskOpen),
                vec![("m4m", "rn-1", RunRunning, none)],
                Shown::Running,
                false,
            ),
            // a run that is unknown, or stuck in starting, is never Running
            (
                Open,
                None,
                vec![("m4m", "rn-1", Starting, stale)],
                Shown::RunUnknown,
                false,
            ),
            (
                Running,
                None,
                vec![("m4m", "rn-1", RunUnknown, none)],
                Shown::RunUnknown,
                false,
            ),
            // a live run next to a stuck one is Running
            (
                Open,
                None,
                vec![
                    ("m4m", "rn-1", Starting, stale),
                    ("m4m", "rn-2", RunRunning, none),
                ],
                Shown::Running,
                false,
            ),
            // 3: Waiting, while the row is open or answered
            (Open, Some(AskOpen), vec![], Shown::Waiting, false),
            (Running, Some(Answered), vec![], Shown::Waiting, false),
            // ...and not once it is closed
            (Open, Some(Closed), vec![], Shown::Open, false),
            // a missing row changes nothing but is said
            (Running, Some(Missing), vec![], Shown::Running, true),
            // Waiting beats "run unknown"
            (
                Open,
                Some(AskOpen),
                vec![("m4m", "rn-1", Starting, stale)],
                Shown::Waiting,
                false,
            ),
            // 4: the stored status; a finished run changes nothing
            (Open, None, vec![], Shown::Open, false),
            (Running, None, vec![], Shown::Running, false),
            (
                Open,
                None,
                vec![("m4m", "rn-1", RunDone, none)],
                Shown::Open,
                false,
            ),
        ];
        for (index, (stored, ask, runs, expected, missing)) in rows.into_iter().enumerate() {
            let task = task(stored, ask.map(|_| "asks.row-1"));
            let cache = cache_with(&runs);
            let derived = derive(&task, &cache, |_| ask.unwrap_or(Closed), base());
            assert_eq!(
                (derived.status, derived.waits_on_missing),
                (expected, missing),
                "row {index}: stored {stored:?}, ask {ask:?}, runs {runs:?}"
            );
        }
    }

    #[test]
    fn forged_starter_ignored() {
        // Another Mac's credential writes a run record that names this Mac's
        // task. It is in the cache, but the task's owner does not believe it.
        let cache = cache_with(&[("ac-mbm5", "rn-1", RunState::Running, Duration::ZERO)]);
        let task = task(TaskStatus::Open, None);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.runs_of(&task).count(), 0);
        let derived = derive(&task, &cache, |_| AskRow::Closed, base());
        assert_eq!(derived.status, Shown::Open);

        // The document cannot vouch for itself: it says it is the owner's own.
        let mut event = run_event("ac-mbm5", "rn-2", RunState::Running, base());
        let mut record: RunRecord = serde_json::from_slice(event.value.as_ref().unwrap()).unwrap();
        record.host = Some("m4m".into());
        record.session = Some("m4m".into());
        event.value = Some(serde_json::to_vec(&record).unwrap());
        let mut cache = RunCache::new();
        cache.apply(&event, base());
        assert_eq!(cache.runs_of(&task).count(), 0);
    }

    #[test]
    fn a_starting_run_is_judged_by_the_bus_time_not_the_starters() {
        let mut cache = RunCache::new();
        let mut event = run_event("m4m", "rn-1", RunState::Starting, base());
        // The document claims it started a minute ago; the bus took it six
        // minutes ago.
        let mut record: RunRecord = serde_json::from_slice(event.value.as_ref().unwrap()).unwrap();
        record.started_at = Some(Timestamp("2026-10-21T20:00:00Z".into()));
        event.value = Some(serde_json::to_vec(&record).unwrap());
        event.at = Some(base() - Duration::from_secs(6 * 60));
        cache.apply(&event, base());
        let task = task(TaskStatus::Open, None);
        assert_eq!(
            derive(&task, &cache, |_| AskRow::Closed, base()).status,
            Shown::RunUnknown
        );

        // A backend that cannot say when: first seen counts.
        let mut cache = RunCache::new();
        let mut event = run_event("m4m", "rn-1", RunState::Starting, base());
        event.at = None;
        cache.apply(&event, base());
        assert_eq!(
            derive(&task, &cache, |_| AskRow::Closed, base()).status,
            Shown::Running
        );
    }

    #[test]
    fn the_five_minutes_are_announced_once_by_the_clock() {
        let cache = cache_with(&[("m4m", "rn-1", RunState::Starting, Duration::from_secs(60))]);
        let task = task(TaskStatus::Open, None);
        let tasks = [task];
        let due = base() - Duration::from_secs(60) + STARTING_GRACE;
        assert_eq!(cache.next_expiry(tasks.iter(), base()), Some(due));
        // Not yet crossed, then crossed, then already announced.
        assert!(cache
            .expired_between(tasks.iter(), base(), due - Duration::from_secs(1))
            .is_empty());
        assert_eq!(
            cache.expired_between(tasks.iter(), base(), due).len(),
            1,
            "the moment it crosses"
        );
        assert!(cache
            .expired_between(tasks.iter(), due, due + Duration::from_secs(60))
            .is_empty());
        assert_eq!(cache.next_expiry(tasks.iter(), due), None);
    }

    #[test]
    fn a_damaged_or_oversized_record_is_refused_and_a_newer_state_is_unknown() {
        let mut cache = RunCache::new();
        let key = |name: &str| StoreKey(format!("run.m4m.{name}"));
        let at = Some(base());
        for (name, value) in [
            ("rn-1", b"{nope".to_vec()),
            (
                "rn-2",
                serde_json::to_vec(&serde_json::json!({
                    "id": "rn-other", "task": "task.m4m.tk-1", "state": "running"
                }))
                .unwrap(),
            ),
            (
                "rn-3",
                serde_json::to_vec(&serde_json::json!({
                    "id": "rn-3", "task": "task.m4m.tk-1", "state": "running",
                    "route": "x".repeat(FIELD_MAX_CHARS + 1)
                }))
                .unwrap(),
            ),
        ] {
            let event = WatchEvent {
                key: key(name),
                rev: Rev(1),
                value: Some(value),
                at,
            };
            assert!(cache.apply(&event, base()).is_empty());
        }
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.rejected().len(), 3);

        let newer = WatchEvent {
            key: key("rn-4"),
            rev: Rev(1),
            value: Some(
                serde_json::to_vec(&serde_json::json!({
                    "id": "rn-4", "task": "task.m4m.tk-1", "state": "paused", "engine": "quantum"
                }))
                .unwrap(),
            ),
            at,
        };
        assert_eq!(cache.apply(&newer, base()).len(), 1);
        let task = task(TaskStatus::Open, None);
        let run = cache.runs_of(&task).next().unwrap();
        assert_eq!(run.record.state, RunState::Unknown);
        assert_eq!(run.record.engine, Engine::Other);
        // Not a run key at all: not this cache's business.
        let other = WatchEvent {
            key: StoreKey("task.m4m.tk-1".into()),
            rev: Rev(1),
            value: None,
            at,
        };
        assert!(cache.apply(&other, base()).is_empty());
    }

    #[test]
    fn a_removed_record_and_an_older_revision_are_handled() {
        let mut cache = RunCache::new();
        let first = run_event("m4m", "rn-1", RunState::Running, base());
        assert_eq!(cache.apply(&first, base()).len(), 1);
        // The same revision again is no news.
        assert!(cache.apply(&first, base()).is_empty());
        let gone = WatchEvent {
            value: None,
            rev: Rev(2),
            ..first
        };
        assert_eq!(cache.apply(&gone, base()).len(), 1);
        assert_eq!(cache.len(), 0);
    }
}
