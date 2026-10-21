//! The store task and the server-side handle to it.
//!
//! A backend blocks, so one dedicated thread builds it, owns it and does every
//! call. It reports back as [`StoreEvent`]s, which the app drains from its
//! event channel before it handles the next API request. The API side only
//! reads the cache and queues commands: it never waits on the store.
//!
//! Commands and watch changes share one channel, so the thread sleeps until
//! there is something to do. If the thread ends, for any reason, the service
//! learns it from a `Stopped` event and fails every write still pending.

use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use super::cache::TaskCache;
use super::identity::TaskIdentity;
use super::ids::{Rev, TaskKey, WriteId};
use super::model::{MoveContext, Task, TaskError, TaskStatus, Timestamp, RUNS_KEPT};
use super::outbox::{Base, Entry, Op, Outbox};
use super::runs::{derive, AskRow, Derived, RunCache};
use super::store::{landed_last, StoreError, StoreResult, TaskStore, WatchEvent, WatchSink};

/// Writes that may wait for the store at once. More are refused, so a store
/// that stopped answering cannot make the server queue without end.
const MAX_PENDING_WRITES: usize = 256;
/// Outcomes kept for the API to take; the oldest go first.
const MAX_OUTCOMES: usize = 1024;

/// Which service an event came from, and whether that service is being
/// stopped. A sender that has to wait (a full channel) drops the event instead
/// once `stopping` is set, so stopping the service can never deadlock on it.
#[derive(Debug, Clone)]
pub(crate) struct EventContext {
    pub generation: u64,
    stop: Arc<AtomicBool>,
}

impl EventContext {
    /// Whether the service has been asked to stop. Read it again on every turn
    /// of a wait: it can change while the sender is blocked.
    pub(crate) fn stopping(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }
}

/// Builds the backend. Runs on the store thread, and is tried again after a
/// backoff while it answers `Unavailable`.
pub(crate) type StoreOpener = Box<dyn FnMut() -> StoreResult<Box<dyn TaskStore>> + Send>;

/// How long the thread waits before trying again to reach a store that is down.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Backoff {
    pub initial: Duration,
    pub max: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            initial: Duration::from_secs(1),
            max: Duration::from_secs(30),
        }
    }
}

impl Backoff {
    fn next(self, current: Duration) -> Duration {
        (current * 2).min(self.max)
    }
}

/// A write for the store thread: it goes into the outbox first.
struct WriteCommand {
    write: WriteId,
    op: Op,
    doc: Box<Task>,
    /// What an update is written against: the revision the caller saw, or the
    /// earlier write it was built on.
    base: Option<Base>,
    /// Made by the server itself; see [`Entry::auto`].
    auto: bool,
}

/// How a held write is to be settled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Choice {
    /// Keep the held write: it is written again on top of the store's version.
    Mine,
    /// Take the store's version: the held write is dropped, and so is every
    /// write queued on top of it.
    Theirs,
    /// Write this document instead.
    Merged(Box<Task>),
}

/// A write the store refused as a conflict, waiting for a decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HeldConflict {
    pub write: WriteId,
    pub key: TaskKey,
    /// What the held write would store.
    pub mine: Task,
    /// The store's revision when it refused (`None`: the key was absent).
    pub theirs_rev: Option<Rev>,
    pub theirs: Option<serde_json::Value>,
}

impl HeldConflict {
    fn from_entry(entry: &Entry) -> Option<Self> {
        let conflict = entry.conflict.as_ref()?;
        Some(Self {
            write: entry.write.clone(),
            key: entry.key.clone(),
            mine: entry.doc.clone(),
            theirs_rev: conflict.theirs_rev,
            theirs: conflict.theirs.clone(),
        })
    }

    /// The store's side as the bytes and revision a conflict outcome carries.
    fn current(&self) -> Option<(Rev, Vec<u8>)> {
        let rev = self.theirs_rev?;
        let bytes = serde_json::to_vec(self.theirs.as_ref()?).ok()?;
        Some((rev, bytes))
    }
}

/// How a request to settle a held write ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResolveOutcome {
    /// The held write is queued again; its own outcome follows.
    Requeued,
    /// The store's version stands. These queued writes, which were built on
    /// the dropped one, came back to their callers as conflicts.
    Dropped {
        cancelled: Vec<WriteId>,
    },
    /// The conflict is not the one the caller saw (it was settled, or the
    /// store moved again): what the store holds now, to look at again.
    Stale {
        current: Option<(Rev, Vec<u8>)>,
    },
    Failed(String),
}

/// How a request to settle a held write ended.
#[derive(Debug)]
pub(crate) struct Resolution {
    ticket: u64,
    write: WriteId,
    key: TaskKey,
    outcome: ResolveOutcome,
    /// A write that goes on carries its document, so it counts as pending
    /// again.
    requeued: Option<Task>,
}

struct ResolveCommand {
    ticket: u64,
    write: WriteId,
    key: TaskKey,
    choice: Choice,
    /// The store's revision the caller decided against.
    expected: Rev,
}

/// Everything the store thread's one channel carries.
enum Message {
    Write(WriteCommand),
    Resolve(ResolveCommand),
    Watch(WatchEvent),
    Shutdown,
}

/// What the store thread reports.
#[derive(Debug)]
pub(crate) enum StoreEvent {
    /// Everything the store held, read after the store was reached.
    Loaded(Vec<WatchEvent>),
    /// A change seen by the watch.
    Changed(WatchEvent),
    /// Writes found waiting in the outbox when the thread started, from an
    /// earlier run of the server: they count as pending until they settle.
    Queued(Vec<(WriteId, TaskKey, Box<Task>)>),
    /// A write is on disk in the outbox: from now on a crash does not lose it.
    Accepted { write: WriteId },
    /// How a write ended.
    Outcome {
        write: WriteId,
        outcome: WriteOutcome,
    },
    /// A write is held for a decision (also sent for each one found in the
    /// outbox at start).
    Held(Box<HeldConflict>),
    /// How a request to settle a held write ended. A write that goes on
    /// carries its document, so it counts as pending again.
    Resolved(Box<Resolution>),
    /// The store cannot be reached or read right now; the thread keeps trying.
    Degraded(String),
    /// The thread has ended; nothing more will be applied or reported.
    Stopped(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WriteOutcome {
    Applied {
        rev: Rev,
    },
    /// The revision moved (or the key is gone): what is stored now.
    Conflict {
        current: Option<(Rev, Vec<u8>)>,
    },
    /// A create found the key held by another write.
    Exists {
        current: (Rev, Vec<u8>),
    },
    /// The write was not applied.
    Failed(String),
    /// The write may or may not have been applied; a replay of its id decides.
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SubmitError {
    /// No store runs: tasks are off in this server.
    Disabled,
    /// The store thread is gone.
    Stopped,
    Refused(TaskError),
    /// Too many writes are waiting for the store.
    Busy,
    /// The conflict is not the one the caller saw; what the store holds now.
    Stale {
        current: Option<(Rev, Vec<u8>)>,
    },
}

impl SubmitError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Disabled => "tasks_disabled",
            Self::Stopped => "task_store_stopped",
            Self::Refused(err) => err.code(),
            Self::Busy => "task_store_busy",
            Self::Stale { .. } => "stale_revision",
        }
    }
}

/// How to start the service.
pub(crate) struct StartOptions {
    pub identity: TaskIdentity,
    pub backoff: Backoff,
    /// Where queued writes are kept, durably, until the store has them.
    pub outbox_dir: PathBuf,
}

/// What a pane shows of its tasks, kept up to date as the cache changes so a
/// frame or a snapshot never scans the tasks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PaneTaskCounts {
    /// Open tasks bound to the pane.
    pub open: u32,
    /// Writes for those tasks waiting for the store.
    pub pending: u32,
    /// Writes for those tasks held for a decision.
    pub conflicts: u32,
    /// Changes whenever any of the three does, or a task of the pane changes,
    /// so a client holding the tasks knows to read them again.
    pub revision: u64,
}

pub(crate) struct TaskService {
    /// Tells events of this service from those of one it replaced.
    generation: u64,
    /// Set when the service is asked to stop; the thread looks at it between
    /// steps.
    stop: Arc<AtomicBool>,
    identity: Option<TaskIdentity>,
    cache: TaskCache,
    commands: Option<Sender<Message>>,
    /// Raised by the thread when it ends, after it let go of the store.
    finished: Option<Receiver<()>>,
    /// Writes sent and not yet answered, by write id.
    pending: HashMap<WriteId, TaskKey>,
    /// The newest version of each task that has a write pending: what the next
    /// edit is built on, so two quick edits do not undo each other.
    latest: HashMap<TaskKey, (WriteId, Task)>,
    /// Outcomes not yet taken, in arrival order.
    outcomes: Vec<(WriteId, WriteOutcome)>,
    /// The last few outcomes by write id, kept after they were taken so a
    /// caller that asks later still learns how its write ended.
    recent: HashMap<WriteId, WriteOutcome>,
    recent_order: std::collections::VecDeque<WriteId>,
    /// Why the store is unusable, when it is.
    failure: Option<String>,
    /// The store's first scan has arrived, so the cache is complete.
    loaded: bool,
    /// The counts each pane shows, by the pane's stable `pane_key`.
    pane_counts: HashMap<String, PaneTaskCounts>,
    /// The cache revision each pane's counts were last taken at.
    pane_cache_rev: HashMap<String, u64>,
    /// The run records, beside the tasks.
    runs: RunCache,
    /// Writes this server made on its own (rebuilding `runs`, the move to
    /// Running), with the revision each was built on. Their outcomes are not
    /// reported to anyone.
    auto_writes: HashMap<WriteId, (TaskKey, Rev)>,
    /// The time up to which the five-minute rule has been looked at.
    status_mark: SystemTime,
    clock: fn() -> SystemTime,
    /// Writes known to be on disk in the outbox (a subset of `pending`).
    saved: HashSet<WriteId>,
    /// Writes held for a decision, in the order they were refused.
    conflicts: Vec<HeldConflict>,
    /// Requests to settle a held write that have no answer yet.
    open_tickets: HashSet<u64>,
    /// How requests to settle a held write ended, not yet taken.
    resolutions: Vec<(u64, ResolveOutcome)>,
    next_ticket: u64,
}

impl TaskService {
    /// A service with no store: reads are empty and writes are refused.
    pub(crate) fn disabled() -> Self {
        Self {
            generation: 0,
            stop: Arc::default(),
            identity: None,
            cache: TaskCache::new(),
            commands: None,
            finished: None,
            pending: HashMap::new(),
            latest: HashMap::new(),
            outcomes: Vec::new(),
            recent: HashMap::new(),
            recent_order: std::collections::VecDeque::new(),
            failure: None,
            loaded: false,
            pane_counts: HashMap::new(),
            pane_cache_rev: HashMap::new(),
            runs: RunCache::new(),
            auto_writes: HashMap::new(),
            status_mark: SystemTime::now(),
            clock: SystemTime::now,
            saved: HashSet::new(),
            conflicts: Vec::new(),
            open_tickets: HashSet::new(),
            resolutions: Vec::new(),
            next_ticket: 1,
        }
    }

    /// No store runs, but writes are still judged by `identity`, so a server
    /// that must not use the bus says why instead of "tasks are off".
    pub(crate) fn without_store(identity: TaskIdentity) -> Self {
        let mut service = Self::disabled();
        service.identity = Some(identity);
        service
    }

    /// As [`Self::without_store`], with the reason the store is not usable.
    pub(crate) fn unavailable(identity: TaskIdentity, reason: String) -> Self {
        let mut service = Self::without_store(identity);
        service.failure = Some(reason);
        service
    }

    /// Build a store with `open` on a new thread and run it there, with its
    /// outbox in `options.outbox_dir`. `open` is tried again after a backoff
    /// while it answers `Unavailable`; any other error ends the thread. `emit`
    /// delivers each event to the app and returns false once the app is gone,
    /// which ends the thread.
    pub(crate) fn start(
        open: impl FnMut() -> StoreResult<Box<dyn TaskStore>> + Send + 'static,
        options: StartOptions,
        emit: impl Fn(EventContext, StoreEvent) -> bool + Send + 'static,
    ) -> std::io::Result<Self> {
        static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
        let generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread_stop_ref = Arc::clone(&stop);
        let (sender, receiver) = mpsc::channel();
        let (done_sender, done_receiver) = mpsc::channel();
        let watch_sender = sender.clone();
        let StartOptions {
            identity,
            backoff,
            outbox_dir,
        } = options;
        crate::thread_spawn::spawn_named("herdr-task-store", move || {
            let context = EventContext {
                generation,
                stop: thread_stop,
            };
            let emit = move |event| emit(context.clone(), event);
            let result = catch_unwind(AssertUnwindSafe(|| {
                run_store(
                    open,
                    receiver,
                    watch_sender,
                    backoff,
                    &thread_stop_ref,
                    outbox_dir,
                    &emit,
                )
            }));
            let reason = match result {
                Ok(reason) => reason,
                Err(_) => Some("the task store thread panicked".to_owned()),
            };
            if let Some(reason) = reason {
                emit(StoreEvent::Stopped(reason));
            }
            let _ = done_sender.send(());
        })?;
        let mut service = Self::without_store(identity);
        service.generation = generation;
        service.stop = stop;
        service.commands = Some(sender);
        service.finished = Some(done_receiver);
        Ok(service)
    }

    /// The version of a task to build an edit on: the newest one that has a
    /// write pending, else the cached one with its revision. A second edit made
    /// while the first is still queued then keeps the first.
    pub(crate) fn current(&self, key: &TaskKey) -> Option<(Option<Rev>, &Task)> {
        match self.latest.get(key) {
            Some((_, task)) => Some((self.cache.get(key).map(|cached| cached.rev), task)),
            None => self
                .cache
                .get(key)
                .map(|cached| (Some(cached.rev), &cached.task)),
        }
    }

    pub(crate) fn cache(&self) -> &TaskCache {
        &self.cache
    }

    pub(crate) fn is_running(&self) -> bool {
        self.commands.is_some()
    }

    pub(crate) fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub(crate) fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    /// Writes sent whose outcome has not arrived ("pending sync").
    pub(crate) fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Whether the write is on disk, so that a crash no longer loses it. A write
    /// answers "pending" at once, before this is true.
    pub(crate) fn is_saved(&self, write: &WriteId) -> bool {
        self.saved.contains(write)
    }

    /// How a write ended, if it did lately.
    pub(crate) fn recent_outcome(&self, write: &WriteId) -> Option<&WriteOutcome> {
        self.recent.get(write)
    }

    /// Whether the write is still waiting for the store.
    pub(crate) fn is_pending_write(&self, write: &WriteId) -> bool {
        self.pending.contains_key(write)
    }

    /// The writes held for a decision, oldest first.
    pub(crate) fn conflicts(&self) -> &[HeldConflict] {
        &self.conflicts
    }

    /// Settle the write held under `write`. `expected` is the store's revision
    /// the caller decided against (`Rev(0)` when the key was absent): a caller
    /// who looked at an older conflict gets `stale_revision` and what the store
    /// holds now. Answers at once with a ticket; how it ended arrives later as
    /// a [`StoreEvent::Resolved`], and a write that goes on reports its own
    /// outcome after that.
    pub(crate) fn resolve_conflict(
        &mut self,
        key: &TaskKey,
        write: &WriteId,
        choice: Choice,
        expected: Rev,
    ) -> Result<u64, SubmitError> {
        let held = self
            .conflicts
            .iter()
            .find(|held| &held.write == write && &held.key == key);
        let Some(held) = held else {
            return Err(SubmitError::Stale {
                current: self.cached_bytes(key),
            });
        };
        if held.theirs_rev.unwrap_or(Rev(0)) != expected {
            return Err(SubmitError::Stale {
                current: held.current().or_else(|| self.cached_bytes(key)),
            });
        }
        let choice = match choice {
            Choice::Merged(doc) => {
                if &doc.key() != key {
                    return Err(SubmitError::Refused(TaskError::KeyMismatch));
                }
                self.check_may_write(&doc)?;
                let (doc, _) = self.prepare_as(&doc, write.clone())?;
                Choice::Merged(Box::new(doc))
            }
            other => {
                self.check_may_write(&held.mine)?;
                other
            }
        };
        let ticket = self.next_ticket;
        self.next_ticket += 1;
        let command = ResolveCommand {
            ticket,
            write: write.clone(),
            key: key.clone(),
            choice,
            expected,
        };
        let commands = self.commands.as_ref().ok_or(SubmitError::Disabled)?;
        if commands.send(Message::Resolve(command)).is_err() {
            self.mark_stopped("the task store thread is gone".to_owned());
            return Err(SubmitError::Stopped);
        }
        self.open_tickets.insert(ticket);
        Ok(ticket)
    }

    /// How requests to settle a held write ended since the last call.
    pub(crate) fn take_resolutions(&mut self) -> Vec<(u64, ResolveOutcome)> {
        std::mem::take(&mut self.resolutions)
    }

    pub(crate) fn identity(&self) -> Option<&TaskIdentity> {
        self.identity.as_ref()
    }

    /// The keys with a write waiting for the store.
    pub(crate) fn pending_keys(&self) -> HashSet<&TaskKey> {
        self.pending.values().collect()
    }

    /// The keys with a write held for a decision.
    pub(crate) fn conflict_keys(&self) -> HashSet<&TaskKey> {
        self.conflicts.iter().map(|held| &held.key).collect()
    }

    /// How the store is doing: `disabled`, `loading`, `ready` or `unavailable`,
    /// with the reason when there is one.
    pub(crate) fn state(&self) -> (&'static str, Option<&str>) {
        match (&self.failure, self.commands.is_some(), self.loaded) {
            (Some(why), _, _) => ("unavailable", Some(why.as_str())),
            (None, false, _) => ("disabled", None),
            (None, true, false) => ("loading", None),
            (None, true, true) => ("ready", None),
        }
    }

    /// Replace the clock the five-minute rule is judged by.
    #[cfg(test)]
    pub(crate) fn set_clock(&mut self, clock: fn() -> SystemTime) {
        self.clock = clock;
        self.status_mark = clock();
    }

    /// The time the five-minute rule is judged against.
    pub(crate) fn now(&self) -> SystemTime {
        (self.clock)()
    }

    /// The run records the server holds.
    pub(crate) fn runs(&self) -> &RunCache {
        &self.runs
    }

    /// What a reader shows for a task: its stored status, as the run records
    /// and the asks row it waits on change it. `ask` says how a row looks.
    pub(crate) fn shown_status(
        &self,
        key: &TaskKey,
        ask: impl Fn(&str) -> AskRow,
    ) -> Option<Derived> {
        let cached = self.cache.get(key)?;
        Some(derive(&cached.task, &self.runs, ask, (self.clock)()))
    }

    /// How long until a run crosses the five minutes and a task's shown status
    /// changes with the clock alone, for the timer that announces it.
    pub(crate) fn next_status_deadline(&self) -> Option<Duration> {
        let now = (self.clock)();
        let tasks = self.cache.iter().map(|(_, cached)| &cached.task);
        let deadline = self.runs.next_expiry(tasks, now)?;
        Some(deadline.duration_since(now).unwrap_or_default())
    }

    /// The tasks whose shown status changed with the clock alone since the last
    /// call: a run crossed the five minutes. Call it when the timer fires.
    pub(crate) fn due_status_changes(&mut self) -> Vec<TaskKey> {
        let now = (self.clock)();
        let tasks = self.cache.iter().map(|(_, cached)| &cached.task);
        let due = self.runs.expired_between(tasks, self.status_mark, now);
        self.status_mark = now;
        due
    }

    /// Bring every task this server owns in line with its run records.
    fn reconcile_all(&mut self, now: SystemTime) {
        let Some(identity) = &self.identity else {
            return;
        };
        let keys: Vec<TaskKey> = self
            .cache
            .iter()
            .filter(|(key, _)| key.host == identity.host)
            .map(|(key, _)| key.clone())
            .collect();
        for key in keys {
            self.reconcile_task(&key, now);
        }
    }

    /// The owner derives `runs` and the move to Running from the run records
    /// (contracts § 2.4). Rebuilds `task.runs` from the records of allowed
    /// starters, and moves an open task to Running while a run is live. Writes
    /// only what differs, so it settles after one pass.
    fn reconcile_task(&mut self, key: &TaskKey, now: SystemTime) -> Option<WriteId> {
        if !self.loaded || self.commands.is_none() || self.latest.contains_key(key) {
            return None;
        }
        let identity = self.identity.as_ref()?;
        if key.host != identity.host {
            return None;
        }
        let by = format!("herdr-{}", identity.host);
        let cached = self.cache.get(key)?;
        let (rev, task) = (cached.rev, &cached.task);
        if task.status.is_closed() || task.moving.is_some() || !task.can_be_rewritten() {
            return None;
        }
        let runs: Vec<_> = self.runs.runs_of(task).collect();
        let mut wanted: Vec<String> = runs.iter().map(|run| run.record.id.clone()).collect();
        wanted.drain(..wanted.len().saturating_sub(RUNS_KEPT));
        let live = runs.iter().filter(|run| run.is_live(now)).count();
        let mut updated = task.clone();
        let mut changed = wanted != task.runs;
        updated.runs = wanted;
        if task.status == TaskStatus::Open && live > 0 {
            let ctx = MoveContext {
                other_run_running: live > 1,
                run_is_current: true,
                by,
                reason: None,
                at: Timestamp::now(),
            };
            changed |= updated.move_status(TaskStatus::Running, &ctx).is_ok();
        }
        if !changed {
            return None;
        }
        let write = self.submit_update_as(&updated, rev, true).ok()?;
        self.auto_writes.insert(write.clone(), (key.clone(), rev));
        Some(write)
    }

    /// The cached version of a task, as the bytes a store would hold.
    fn cached_bytes(&self, key: &TaskKey) -> Option<(Rev, Vec<u8>)> {
        let cached = self.cache.get(key)?;
        Some((cached.rev, serde_json::to_vec(&cached.task).ok()?))
    }

    /// Queue a create. Answers at once with the write's id; the outcome
    /// arrives later as a [`StoreEvent::Outcome`].
    pub(crate) fn submit_create(&mut self, task: &Task) -> Result<WriteId, SubmitError> {
        self.check_may_write(task)?;
        let (doc, write) = self.prepare(task)?;
        self.send(WriteCommand {
            write,
            op: Op::Create,
            doc: Box::new(doc),
            base: None,
            auto: false,
        })
    }

    /// Queue an update of a task last seen at `expected`.
    pub(crate) fn submit_update(
        &mut self,
        task: &Task,
        expected: Rev,
    ) -> Result<WriteId, SubmitError> {
        self.submit_update_as(task, expected, false)
    }

    fn submit_update_as(
        &mut self,
        task: &Task,
        expected: Rev,
        auto: bool,
    ) -> Result<WriteId, SubmitError> {
        self.check_may_write(task)?;
        if expected == Rev(0) {
            return Err(SubmitError::Refused(TaskError::BadRevision));
        }
        let (doc, write) = self.prepare(task)?;
        // An edit made while an earlier one is still queued is built on it,
        // whatever has been sent since.
        let base = match self.latest.get(&doc.key()) {
            Some((ahead, _)) => Base::After(ahead.clone()),
            None => Base::Rev(expected),
        };
        self.send(WriteCommand {
            write,
            op: Op::Update,
            doc: Box::new(doc),
            base: Some(base),
            auto,
        })
    }

    /// Validate `task`, give it a fresh write id and record the id in its copy.
    fn prepare(&self, task: &Task) -> Result<(Task, WriteId), SubmitError> {
        self.prepare_as(task, WriteId::generate())
    }

    /// As [`Self::prepare`], for a write that already has its id.
    fn prepare_as(&self, task: &Task, write: WriteId) -> Result<(Task, WriteId), SubmitError> {
        task.validate().map_err(SubmitError::Refused)?;
        self.cache
            .check_pane_capacity(task)
            .map_err(SubmitError::Refused)?;
        let mut doc = task.clone();
        doc.record_write(write.clone(), |id| self.pending.contains_key(id));
        Ok((doc, write))
    }

    /// A write goes only to the store of the server that owns the task, and
    /// only from the session that may use the bus. Refused before anything is
    /// validated or queued.
    fn check_may_write(&self, task: &Task) -> Result<(), SubmitError> {
        if !task.can_be_rewritten() {
            return Err(SubmitError::Refused(TaskError::NotRewritable));
        }
        // A held write is in the outbox too, though it is no longer pending.
        if self.pending.len() + self.conflicts.len() >= MAX_PENDING_WRITES {
            return Err(SubmitError::Busy);
        }
        match &self.identity {
            Some(identity) => identity
                .check_write(&task.owner_host)
                .map_err(SubmitError::Refused),
            None => Ok(()),
        }
    }

    fn send(&mut self, command: WriteCommand) -> Result<WriteId, SubmitError> {
        let commands = self.commands.as_ref().ok_or(SubmitError::Disabled)?;
        let write = command.write.clone();
        let doc = (*command.doc).clone();
        if commands.send(Message::Write(command)).is_err() {
            self.mark_stopped("the task store thread is gone".to_owned());
            return Err(SubmitError::Stopped);
        }
        self.pending.insert(write.clone(), doc.key());
        self.latest.insert(doc.key(), (write.clone(), doc));
        Ok(write)
    }

    /// Forget the newest-version note of a key once nothing is pending for it.
    fn settle_latest(&mut self, key: &TaskKey) {
        if !self.pending.values().any(|pending| pending == key) {
            self.latest.remove(key);
        }
    }

    fn push_outcome(&mut self, write: WriteId, outcome: WriteOutcome) {
        if self.recent.insert(write.clone(), outcome.clone()).is_none() {
            self.recent_order.push_back(write.clone());
            if self.recent_order.len() > MAX_OUTCOMES {
                if let Some(old) = self.recent_order.pop_front() {
                    self.recent.remove(&old);
                }
            }
        }
        self.outcomes.push((write, outcome));
        if self.outcomes.len() > MAX_OUTCOMES {
            let excess = self.outcomes.len() - MAX_OUTCOMES;
            self.outcomes.drain(..excess);
        }
    }

    /// The thread is gone: nothing pending will ever be answered.
    fn mark_stopped(&mut self, reason: String) {
        self.commands = None;
        self.failure = Some(reason.clone());
        // One of them may have been in flight when the thread ended: its fate
        // is unknown, and a replay of its write id decides.
        self.latest.clear();
        self.saved.clear();
        let pending: Vec<WriteId> = self.pending.drain().map(|(write, _)| write).collect();
        for write in pending {
            if self.auto_writes.remove(&write).is_none() {
                self.push_outcome(write, WriteOutcome::Unknown(reason.clone()));
            }
        }
        // Nobody is left to answer a request to settle a held write.
        let mut tickets: Vec<u64> = self.open_tickets.drain().collect();
        tickets.sort_unstable();
        for ticket in tickets {
            self.resolutions
                .push((ticket, ResolveOutcome::Failed(reason.clone())));
        }
    }

    /// Take what the store thread reported. Returns whether anything a reader
    /// can see changed.
    pub(crate) fn apply(&mut self, event: StoreEvent) -> bool {
        let changed = self.apply_event(event);
        if changed {
            self.refresh_pane_counts();
        }
        changed
    }

    /// The counts a pane shows, if it has any tasks, open or waiting.
    pub(crate) fn pane_counts(&self, pane_key: &str) -> Option<PaneTaskCounts> {
        self.pane_counts.get(pane_key).copied()
    }

    /// Count, per pane, the open tasks bound to it and the writes waiting or
    /// held for them. The version a task will have once its write lands is the
    /// one counted. Panes whose numbers did not change keep their revision.
    fn refresh_pane_counts(&mut self) {
        let host = self.identity.as_ref().map(|identity| &identity.host);
        let mut counts: HashMap<String, PaneTaskCounts> = HashMap::new();
        let bound = |task: &Task| -> Option<String> {
            let pane = task.pane.as_ref()?;
            (host.is_none_or(|host| host.as_str() == pane.host))
                .then(|| pane.pane_key.clone())
                .flatten()
        };
        // A task with a write pending counts as that write will leave it.
        let mut seen: HashSet<&TaskKey> = HashSet::new();
        for (key, (_, task)) in &self.latest {
            seen.insert(key);
            if let Some(pane_key) = bound(task) {
                let entry = counts.entry(pane_key).or_default();
                entry.pending += 1;
                if !task.status.is_closed() {
                    entry.open += 1;
                }
            }
        }
        for (key, cached) in self.cache.iter() {
            if seen.contains(key) {
                continue;
            }
            if let Some(pane_key) = bound(&cached.task) {
                let entry = counts.entry(pane_key).or_default();
                if !cached.task.status.is_closed() {
                    entry.open += 1;
                }
            }
        }
        for held in &self.conflicts {
            if let Some(pane_key) = bound(&held.mine) {
                counts.entry(pane_key).or_default().conflicts += 1;
            }
        }
        // A cache change moves the revision of the panes it touches.
        let global = self.cache.revision();
        for (pane_key, entry) in &mut counts {
            let before = self.pane_counts.get(pane_key);
            entry.revision = match before {
                Some(old)
                    if (old.open, old.pending, old.conflicts)
                        == (entry.open, entry.pending, entry.conflicts)
                        && self.pane_cache_rev.get(pane_key) == Some(&global) =>
                {
                    old.revision
                }
                Some(old) => old.revision + 1,
                None => 1,
            };
        }
        self.pane_cache_rev = counts.keys().map(|key| (key.clone(), global)).collect();
        self.pane_counts = counts;
    }

    fn apply_event(&mut self, event: StoreEvent) -> bool {
        match event {
            StoreEvent::Loaded(events) => {
                self.loaded = true;
                self.failure = None;
                let now = (self.clock)();
                for event in &events {
                    self.cache.apply(event);
                    self.runs.apply(event, now);
                }
                // On start and after a handoff the run records may have moved
                // while nobody was watching: rebuild what they decide.
                self.reconcile_all(now);
                true
            }
            StoreEvent::Changed(event) => {
                let now = (self.clock)();
                let task_changed = self.cache.apply(&event);
                let runs_of = self.runs.apply(&event, now);
                let any_run = !runs_of.is_empty();
                for key in runs_of {
                    self.reconcile_task(&key, now);
                }
                if task_changed {
                    if let Ok(key) = event.key.0.parse::<TaskKey>() {
                        self.reconcile_task(&key, now);
                    }
                }
                task_changed || any_run
            }
            StoreEvent::Queued(writes) => {
                for (write, key, doc) in writes {
                    self.saved.insert(write.clone());
                    self.pending.insert(write.clone(), key.clone());
                    self.latest.insert(key, (write, *doc));
                }
                true
            }
            StoreEvent::Accepted { write } => {
                // Told late, after the outcome, it is no news.
                if self.pending.contains_key(&write) {
                    self.saved.insert(write);
                }
                true
            }
            StoreEvent::Outcome { write, outcome } => {
                self.saved.remove(&write);
                // A write already reported (as unknown, when the thread was
                // stopped or replaced) is not reported a second time.
                let Some(key) = self.pending.remove(&write) else {
                    return false;
                };
                self.settle_latest(&key);
                if let Some((_, built_on)) = self.auto_writes.remove(&write) {
                    // Nobody asked for it, so nobody is told. A refusal because
                    // the task moved on is made again on the new version.
                    if let WriteOutcome::Failed(why) = &outcome {
                        tracing::warn!(%key, %why, "a task update made by the server failed");
                    }
                    let moved_on = self
                        .cache
                        .get(&key)
                        .is_some_and(|cached| cached.rev > built_on);
                    if matches!(outcome, WriteOutcome::Conflict { .. }) && moved_on {
                        self.reconcile_task(&key, (self.clock)());
                    }
                    return true;
                }
                self.push_outcome(write, outcome);
                true
            }
            StoreEvent::Held(held) => {
                match self
                    .conflicts
                    .iter_mut()
                    .find(|old| old.write == held.write)
                {
                    Some(old) => *old = *held,
                    None => self.conflicts.push(*held),
                }
                true
            }
            StoreEvent::Resolved(resolution) => {
                let Resolution {
                    ticket,
                    write,
                    key,
                    outcome,
                    requeued,
                } = *resolution;
                self.open_tickets.remove(&ticket);
                if !matches!(
                    outcome,
                    ResolveOutcome::Stale { .. } | ResolveOutcome::Failed(_)
                ) {
                    self.conflicts.retain(|held| held.write != write);
                }
                // A write that goes on is waiting for the store again.
                if let Some(doc) = requeued {
                    self.saved.insert(write.clone());
                    self.pending.insert(write.clone(), key.clone());
                    self.latest.insert(key, (write, doc));
                }
                self.resolutions.push((ticket, outcome));
                true
            }
            StoreEvent::Degraded(message) => {
                self.failure = Some(message);
                true
            }
            StoreEvent::Stopped(reason) => {
                self.mark_stopped(reason);
                true
            }
        }
    }

    /// The outcomes that arrived since the last call, without taking them.
    #[cfg(test)]
    pub(crate) fn take_outcomes_peek(&self) -> &[(WriteId, WriteOutcome)] {
        &self.outcomes
    }

    /// The outcomes that arrived since the last call, oldest first.
    pub(crate) fn take_outcomes(&mut self) -> Vec<(WriteId, WriteOutcome)> {
        std::mem::take(&mut self.outcomes)
    }

    /// Which service the events of this one are tagged with.
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// Whether a live handoff must wait for this store to let go. Only the
    /// file store needs it (its directory lock); the bus store is protected by
    /// compare-and-set, so the replacement may connect at once.
    pub(crate) fn needs_exclusive_release(&self) -> bool {
        self.identity
            .as_ref()
            .is_some_and(|identity| identity.backend == super::identity::BackendKind::File)
    }

    /// Stop taking writes, let the thread finish what is queued, and wait for
    /// it to let go of the store. Returns whether it did within `timeout`.
    pub(crate) fn quiesce(&mut self, timeout: Duration) -> bool {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(commands) = self.commands.take() {
            let _ = commands.send(Message::Shutdown);
        }
        match self.finished.take() {
            Some(finished) => finished.recv_timeout(timeout).is_ok(),
            None => true,
        }
    }

    /// [`Self::quiesce`] for a live handoff: whether the handoff may go on.
    /// A store that must be released (the file one) and was not stops it; a
    /// bus store that did not stop in time is only logged, since a write it
    /// has in flight settles by compare-and-set and the replacement's rescan.
    pub(crate) fn quiesce_for_handoff(&mut self, timeout: Duration) -> bool {
        let released = self.quiesce(timeout);
        if !released && !self.needs_exclusive_release() {
            tracing::warn!("the task store did not stop in time; the handoff goes on");
            return true;
        }
        released
    }

    /// End this service for good, and hand back what its owner has not yet
    /// been told: outcomes not taken, and every write still pending, now
    /// unknown (its thread may have been cut off mid-write). The next service
    /// takes them over with [`Self::adopt`].
    pub(crate) fn retire(mut self) -> Vec<(WriteId, WriteOutcome)> {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(commands) = self.commands.take() {
            let _ = commands.send(Message::Shutdown);
        }
        self.saved.clear();
        let pending: Vec<WriteId> = self.pending.drain().map(|(write, _)| write).collect();
        let mut carried = std::mem::take(&mut self.outcomes);
        carried.extend(
            pending
                .into_iter()
                .filter(|write| !self.auto_writes.contains_key(write))
                .map(|write| {
                    (
                        write,
                        WriteOutcome::Unknown("the task store was restarted".to_owned()),
                    )
                }),
        );
        carried
    }

    /// Take over what a retired service left unreported.
    pub(crate) fn adopt(&mut self, carried: Vec<(WriteId, WriteOutcome)>) {
        for (write, outcome) in carried {
            self.push_outcome(write, outcome);
        }
    }
}

impl Drop for TaskService {
    fn drop(&mut self) {
        // Do not wait for the thread: a slow store must not hold up a quit.
        self.stop.store(true, Ordering::SeqCst);
        if let Some(commands) = &self.commands {
            let _ = commands.send(Message::Shutdown);
        }
    }
}

/// The thread body. Returns why it ended, or `None` for an orderly stop.
fn run_store(
    mut open: impl FnMut() -> StoreResult<Box<dyn TaskStore>>,
    messages: Receiver<Message>,
    watch_sender: Sender<Message>,
    backoff: Backoff,
    stop: &AtomicBool,
    outbox_dir: PathBuf,
    emit: &impl Fn(StoreEvent) -> bool,
) -> Option<String> {
    // The outbox is on this machine's disk, so it opens first: writes made
    // while the store is out of reach are kept in it.
    let mut wait = backoff.initial;
    let mut outbox = loop {
        match Outbox::open(outbox_dir.clone()) {
            Ok(outbox) => break outbox,
            // Another server still holds it (a handoff in progress): wait.
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                if stop.load(Ordering::SeqCst) || pause(&messages, wait, None, emit) {
                    return None;
                }
                wait = backoff.next(wait);
            }
            Err(err) => return Some(format!("cannot open the task outbox: {err}")),
        }
    };

    // Reach the store, trying again while it is only unreachable.
    let mut wait = backoff.initial;
    let mut store = loop {
        if stop.load(Ordering::SeqCst) {
            finish(&mut outbox, &messages, emit);
            return None;
        }
        match open() {
            // A stop that came while it was connecting: let go at once.
            Ok(_) if stop.load(Ordering::SeqCst) => {
                finish(&mut outbox, &messages, emit);
                return None;
            }
            Ok(store) => break store,
            Err(StoreError::Unavailable(why)) => {
                if !emit(StoreEvent::Degraded(format!(
                    "the task store is not reachable: {why}"
                ))) {
                    return None;
                }
                if pause(&messages, wait, Some(&mut outbox), emit) {
                    finish(&mut outbox, &messages, emit);
                    return None;
                }
                wait = backoff.next(wait);
            }
            Err(err) => return Some(format!("cannot open the task store: {err}")),
        }
    };

    // Watch first, so nothing that lands during the scan is missed.
    let sink: WatchSink = Box::new(move |event| watch_sender.send(Message::Watch(event)).is_ok());
    if let Err(err) = store.watch("", sink) {
        return Some(format!("cannot watch the task store: {err}"));
    }
    let mut wait = backoff.initial;
    loop {
        if stop.load(Ordering::SeqCst) {
            finish(&mut outbox, &messages, emit);
            return None;
        }
        match store.scan("") {
            Ok(events) => {
                if !emit(StoreEvent::Loaded(events)) {
                    return None;
                }
                break;
            }
            Err(err) => {
                if !emit(StoreEvent::Degraded(format!(
                    "cannot read the task store: {err}"
                ))) {
                    return None;
                }
                if pause(&messages, wait, Some(&mut outbox), emit) {
                    finish(&mut outbox, &messages, emit);
                    return None;
                }
                wait = backoff.next(wait);
            }
        }
    }

    // Writes left by an earlier run, or queued while the store was away.
    // (a held one is not waiting for the store but for a decision, below)
    let waiting: Vec<_> = outbox
        .entries()
        .iter()
        .filter(|entry| entry.conflict.is_none())
        .map(|entry| {
            (
                entry.write.clone(),
                entry.key.clone(),
                Box::new(entry.doc.clone()),
            )
        })
        .collect();
    if !waiting.is_empty() && !emit(StoreEvent::Queued(waiting)) {
        return None;
    }
    // Some of them may be waiting for a decision, from before the restart.
    let held: Vec<_> = outbox
        .entries()
        .iter()
        .filter_map(HeldConflict::from_entry)
        .collect();
    for held in held {
        if !emit(StoreEvent::Held(Box::new(held))) {
            return None;
        }
    }

    // Try the outbox now, then whenever something arrives or a retry is due.
    let mut retry_wait = backoff.initial;
    let mut retry_due = flush(&mut outbox, store.as_mut(), emit);
    loop {
        let message = match retry_due {
            Some(true) => match messages.recv_timeout(retry_wait) {
                Ok(message) => Some(message),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            },
            _ => match messages.recv() {
                Ok(message) => Some(message),
                Err(_) => break,
            },
        };
        match message {
            Some(Message::Shutdown) => break,
            Some(Message::Watch(event)) => {
                if !emit(StoreEvent::Changed(event)) {
                    return None;
                }
                continue;
            }
            Some(Message::Write(command)) => {
                let write = command.write.clone();
                match queue(&mut outbox, command) {
                    Ok(()) => {
                        if !emit(StoreEvent::Accepted { write }) {
                            return None;
                        }
                    }
                    Err(outcome) => {
                        if !emit(outcome) {
                            return None;
                        }
                        continue;
                    }
                }
            }
            Some(Message::Resolve(command)) => {
                for event in resolve(&mut outbox, store.as_mut(), command) {
                    if !emit(event) {
                        return None;
                    }
                }
            }
            None => {}
        }
        let before = outbox.len();
        retry_due = flush(&mut outbox, store.as_mut(), emit);
        retry_wait = match retry_due {
            Some(true) if outbox.len() == before => backoff.next(retry_wait),
            _ => backoff.initial,
        };
    }
    finish(&mut outbox, &messages, emit);
    None
}

/// Before the thread ends: the writes still waiting in the channel go into the
/// outbox, so the next owner finds them, and the directory entries are made
/// durable before it reads them.
fn finish(outbox: &mut Outbox, messages: &Receiver<Message>, emit: &impl Fn(StoreEvent) -> bool) {
    let mut told = Vec::new();
    while let Ok(message) = messages.try_recv() {
        match message {
            Message::Write(command) => {
                let write = command.write.clone();
                if queue(outbox, command).is_ok() {
                    told.push(write);
                }
            }
            // Nobody will settle it now; the caller must not wait for ever.
            Message::Resolve(command) => {
                let _ = emit(StoreEvent::Resolved(Box::new(Resolution {
                    ticket: command.ticket,
                    write: command.write,
                    key: command.key,
                    outcome: ResolveOutcome::Failed("the task store is stopping".to_owned()),
                    requeued: None,
                })));
            }
            Message::Watch(_) | Message::Shutdown => {}
        }
    }
    outbox.sync();
    // On disk and synced: now it can be said.
    for write in told {
        let _ = emit(StoreEvent::Accepted { write });
    }
}

/// Put a write into the outbox. A write that cannot even be saved is refused
/// at once, as an outcome to report.
fn queue(outbox: &mut Outbox, command: WriteCommand) -> Result<(), StoreEvent> {
    let WriteCommand {
        write,
        op,
        doc,
        base,
        auto,
    } = command;
    outbox
        .push_as(write.clone(), op, *doc, base, auto)
        .map_err(|err| StoreEvent::Outcome {
            write,
            outcome: WriteOutcome::Failed(format!("cannot queue the write: {err}")),
        })
}

/// Wait `delay` for a stop. Writes that arrive meanwhile go into the outbox
/// (when there is one open yet). Returns true when the thread should end.
fn pause(
    messages: &Receiver<Message>,
    delay: Duration,
    mut outbox: Option<&mut Outbox>,
    emit: &impl Fn(StoreEvent) -> bool,
) -> bool {
    let deadline = std::time::Instant::now() + delay;
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match messages.recv_timeout(left) {
            Ok(Message::Shutdown) | Err(RecvTimeoutError::Disconnected) => return true,
            Err(RecvTimeoutError::Timeout) => return false,
            Ok(Message::Watch(_)) => {}
            Ok(Message::Resolve(command)) => {
                // Settling a conflict needs the store's current version.
                let refused = StoreEvent::Resolved(Box::new(Resolution {
                    ticket: command.ticket,
                    write: command.write,
                    key: command.key,
                    outcome: ResolveOutcome::Failed("the task store is not reachable".to_owned()),
                    requeued: None,
                }));
                if !emit(refused) {
                    return true;
                }
            }
            Ok(Message::Write(command)) => {
                let write = command.write.clone();
                let told = match outbox.as_deref_mut() {
                    Some(outbox) => queue(outbox, command)
                        .map(|()| StoreEvent::Accepted { write })
                        .unwrap_or_else(|refused| refused),
                    None => StoreEvent::Outcome {
                        write: command.write,
                        outcome: WriteOutcome::Failed("the task outbox is not open yet".to_owned()),
                    },
                };
                if !emit(told) {
                    return true;
                }
            }
        }
    }
}

/// Send what the outbox holds to the store, the first entry of each key at a
/// time, until nothing more can go. Returns `Some(true)` when the store could
/// not be reached and a retry is due, `Some(false)` when only held entries
/// (conflicts) remain, and `None` when the outbox is empty.
fn flush(
    outbox: &mut Outbox,
    store: &mut dyn TaskStore,
    emit: &impl Fn(StoreEvent) -> bool,
) -> Option<bool> {
    // Keys the store could not settle this time: left for the next one, so a
    // single stuck document does not hold up the others.
    let mut skipped: HashSet<WriteId> = HashSet::new();
    loop {
        let ready: Vec<Entry> = outbox
            .ready()
            .into_iter()
            .filter(|entry| !skipped.contains(&entry.write))
            .cloned()
            .collect();
        if ready.is_empty() {
            if !skipped.is_empty() {
                return Some(true);
            }
            return (outbox.len() > 0).then_some(false);
        }
        let mut progressed = false;
        for entry in ready {
            match try_entry(outbox, store, &entry) {
                Attempt::Done(events) => {
                    progressed = true;
                    for event in events {
                        if !emit(event) {
                            return None;
                        }
                    }
                }
                Attempt::Skip => {
                    skipped.insert(entry.write.clone());
                }
                // The store is out of reach: nothing else will do better.
                Attempt::Retry => return Some(true),
            }
        }
        if !progressed {
            return Some(!skipped.is_empty());
        }
    }
}

enum Attempt {
    /// The entry is settled (or held); what to report.
    Done(Vec<StoreEvent>),
    /// The store could not say about this entry; try it again later, and go
    /// on with the others.
    Skip,
    /// The store cannot be reached; try again later.
    Retry,
}

fn try_entry(outbox: &mut Outbox, store: &mut dyn TaskStore, entry: &Entry) -> Attempt {
    let result = match entry.op {
        Op::Create => store.create(&entry.key, &entry.doc, entry.write.clone()),
        Op::Update => match expected_rev(outbox, store, entry) {
            Ok(expected) => store.update(&entry.key, &entry.doc, expected, entry.write.clone()),
            Err(err) => Err(err),
        },
    };
    let outcome = |outcome| StoreEvent::Outcome {
        write: entry.write.clone(),
        outcome,
    };
    match result {
        Ok(rev) => match outbox.complete(&entry.write, rev) {
            Ok(()) => Attempt::Done(vec![outcome(WriteOutcome::Applied { rev })]),
            // The store has it but the outbox could not record that: try again,
            // and the write id will be recognised.
            Err(_) => Attempt::Retry,
        },
        // A write the server made on its own is not worth a person's decision:
        // it is dropped and made again from the new version.
        Err(StoreError::Conflict { current }) if entry.auto => {
            let removed = outbox
                .fail(&entry.write)
                .unwrap_or_else(|_| vec![entry.write.clone()]);
            Attempt::Done(
                removed
                    .into_iter()
                    .map(|write| StoreEvent::Outcome {
                        write,
                        outcome: WriteOutcome::Conflict {
                            current: current.clone(),
                        },
                    })
                    .collect(),
            )
        }
        Err(StoreError::Conflict { current }) => {
            let mut events = held_event(outbox, entry, current.clone());
            events.push(outcome(WriteOutcome::Conflict { current }));
            Attempt::Done(events)
        }
        Err(StoreError::AlreadyExists { current }) => {
            let mut events = held_event(outbox, entry, Some(current.clone()));
            events.push(outcome(WriteOutcome::Exists { current }));
            Attempt::Done(events)
        }
        Err(StoreError::Unavailable(_)) => Attempt::Retry,
        // Perhaps for now (a publish that may have landed, a full disk): the
        // write stays where it is and is tried again.
        Err(StoreError::Indeterminate(_) | StoreError::Io(_)) => Attempt::Skip,
        Err(StoreError::Refused(why)) => {
            let removed = outbox
                .fail(&entry.write)
                .unwrap_or_else(|_| vec![entry.write.clone()]);
            Attempt::Done(
                removed
                    .into_iter()
                    .map(|write| StoreEvent::Outcome {
                        write,
                        outcome: WriteOutcome::Failed(why.clone()),
                    })
                    .collect(),
            )
        }
    }
}

/// Keep an entry the store refused, with the store's version beside it, and
/// say that it is held.
fn held_event(
    outbox: &mut Outbox,
    entry: &Entry,
    current: Option<(Rev, Vec<u8>)>,
) -> Vec<StoreEvent> {
    let (rev, theirs) = match current {
        Some((rev, bytes)) => (Some(rev), serde_json::from_slice(&bytes).ok()),
        None => (None, None),
    };
    if let Err(err) = outbox.mark_conflict(&entry.write, rev, theirs) {
        tracing::warn!(error = %err, "could not record a task conflict");
    }
    outbox
        .held(&entry.write)
        .and_then(HeldConflict::from_entry)
        .map(|held| StoreEvent::Held(Box::new(held)))
        .into_iter()
        .collect()
}

/// Settle a held write: compare-and-set against the conflict the caller saw.
fn resolve(
    outbox: &mut Outbox,
    store: &mut dyn TaskStore,
    command: ResolveCommand,
) -> Vec<StoreEvent> {
    let ResolveCommand {
        ticket,
        write,
        key,
        choice,
        expected,
    } = command;
    let ended = |outcome, requeued| {
        StoreEvent::Resolved(Box::new(Resolution {
            ticket,
            write: write.clone(),
            key: key.clone(),
            outcome,
            requeued,
        }))
    };
    let held = outbox.held(&write).and_then(HeldConflict::from_entry);
    let Some(held) = held.filter(|held| held.theirs_rev.unwrap_or(Rev(0)) == expected) else {
        // Settled by someone else, or the store moved on: show what it holds.
        return match store.get(&super::store::StoreKey::from(&key)) {
            Ok(current) => vec![ended(ResolveOutcome::Stale { current }, None)],
            Err(err) => vec![ended(ResolveOutcome::Failed(err.to_string()), None)],
        };
    };
    match choice {
        Choice::Theirs => {
            let removed = match outbox.fail(&write) {
                Ok(removed) => removed,
                Err(err) => {
                    return vec![ended(
                        ResolveOutcome::Failed(format!("cannot drop the held write: {err}")),
                        None,
                    )];
                }
            };
            let cancelled: Vec<WriteId> = removed.into_iter().filter(|id| *id != write).collect();
            let mut events: Vec<StoreEvent> = cancelled
                .iter()
                .map(|id| StoreEvent::Outcome {
                    write: id.clone(),
                    outcome: WriteOutcome::Conflict {
                        current: held.current(),
                    },
                })
                .collect();
            events.push(ended(ResolveOutcome::Dropped { cancelled }, None));
            events
        }
        Choice::Mine | Choice::Merged(_) => {
            // The store's version is the base; if it was absent, write anew.
            let (op, base) = match held.theirs_rev {
                Some(rev) => (Op::Update, Some(Base::Rev(rev))),
                None => (Op::Create, None),
            };
            let doc = match choice {
                Choice::Merged(doc) => Some(*doc),
                _ => None,
            };
            let queued = doc.clone().unwrap_or(held.mine);
            match outbox.release(&write, op, base, doc) {
                Ok(()) => vec![ended(ResolveOutcome::Requeued, Some(queued))],
                Err(err) => vec![ended(
                    ResolveOutcome::Failed(format!("cannot release the held write: {err}")),
                    None,
                )],
            }
        }
    }
}

/// The revision to write an update against. Its own, or the one a write ahead
/// of it in the outbox produced (its receipt). If that receipt is gone, the
/// store itself says whether that write landed.
fn expected_rev(outbox: &Outbox, store: &mut dyn TaskStore, entry: &Entry) -> StoreResult<Rev> {
    if let Some(rev) = outbox.expected_rev(entry) {
        return Ok(rev);
    }
    let Some(Base::After(ahead)) = &entry.base else {
        return Err(StoreError::Conflict { current: None });
    };
    let current = store.get(&super::store::StoreKey::from(&entry.key))?;
    match current {
        // Only if it is the newest write there: a foreign one that landed after
        // it would be overwritten.
        Some((rev, bytes)) if landed_last(&bytes, ahead) => Ok(rev),
        current => Err(StoreError::Conflict { current }),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use super::*;
    use crate::tasks::identity::{test_identity, BackendKind};
    use crate::tasks::ids::{HostId, TaskId};
    use crate::tasks::model::{Origin, Timestamp, Via};
    use crate::tasks::store::{landed_before, StoreKey};
    use std::path::PathBuf;

    /// Outbox directories made by this test, removed when its thread ends.
    struct Cleanup(Vec<PathBuf>);

    impl Drop for Cleanup {
        fn drop(&mut self) {
            for dir in &self.0 {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }

    thread_local! {
        static CLEANUP: std::cell::RefCell<Cleanup> = const { std::cell::RefCell::new(Cleanup(Vec::new())) };
    }

    fn scratch_outbox() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "herdr-service-outbox-{}-{}",
            std::process::id(),
            crate::ulid::new_ulid()
        ));
        CLEANUP.with(|cleanup| cleanup.borrow_mut().0.push(dir.clone()));
        dir
    }

    fn options(identity: TaskIdentity) -> StartOptions {
        StartOptions {
            identity,
            backoff: FAST,
            outbox_dir: scratch_outbox(),
        }
    }

    const FAST: Backoff = Backoff {
        initial: Duration::from_millis(10),
        max: Duration::from_millis(40),
    };

    /// An in-memory store with knobs for the ways a real one misbehaves.
    struct FakeStore {
        /// How many writes reached the store.
        writes: Arc<AtomicUsize>,
        /// Set when the store is dropped, which is when a real one lets go.
        released: Arc<AtomicBool>,
        delay: Duration,
        panic_on_write: bool,
        indeterminate: Arc<AtomicBool>,
        /// Keys whose writes the store keeps answering "publish error" for.
        stuck_keys: Arc<Mutex<Vec<String>>>,
        /// An error every write is answered with while it is set.
        fail_with: Arc<Mutex<Option<StoreError>>>,
        /// Apply the next write but answer that its publish timed out.
        lose_ack_once: Arc<AtomicBool>,
        /// The titles of the documents written, in the order they were applied.
        log: Arc<Mutex<Vec<String>>>,
        scan_failures: Arc<AtomicUsize>,
        docs: BTreeMap<String, (Rev, Vec<u8>)>,
        sinks: Vec<WatchSink>,
    }

    impl FakeStore {
        fn new(delay: Duration) -> Self {
            Self {
                writes: Arc::default(),
                released: Arc::default(),
                delay,
                panic_on_write: false,
                indeterminate: Arc::default(),
                stuck_keys: Arc::default(),
                fail_with: Arc::default(),
                lose_ack_once: Arc::default(),
                log: Arc::default(),
                scan_failures: Arc::default(),
                docs: BTreeMap::new(),
                sinks: Vec::new(),
            }
        }

        fn put(&mut self, key: &TaskKey, doc: &Task, rev: Rev) {
            self.log.lock().unwrap().push(doc.title.clone());
            let bytes = serde_json::to_vec(doc).unwrap();
            self.docs.insert(key.to_string(), (rev, bytes.clone()));
            self.sinks.retain(|sink| {
                sink(WatchEvent {
                    key: StoreKey::from(key),
                    rev,
                    value: Some(bytes.clone()),
                    at: None,
                })
            });
        }

        /// A document under any key, such as a run record.
        fn put_raw(&mut self, key: &str, value: &serde_json::Value, rev: Rev) {
            self.docs
                .insert(key.to_owned(), (rev, serde_json::to_vec(value).unwrap()));
        }

        fn maybe_lose_ack(&self) -> StoreResult<()> {
            if self.lose_ack_once.swap(false, Ordering::SeqCst) {
                return Err(StoreError::Indeterminate(
                    "the acknowledgement was lost".into(),
                ));
            }
            Ok(())
        }

        fn before_write(&self, key: &TaskKey) -> StoreResult<()> {
            self.writes.fetch_add(1, Ordering::SeqCst);
            assert!(!self.panic_on_write, "the store broke");
            std::thread::sleep(self.delay);
            if let Some(error) = self.fail_with.lock().unwrap().clone() {
                return Err(error);
            }
            if self.indeterminate.load(Ordering::SeqCst)
                || self.stuck_keys.lock().unwrap().contains(&key.to_string())
            {
                return Err(StoreError::Indeterminate("the publish timed out".into()));
            }
            Ok(())
        }
    }

    impl Drop for FakeStore {
        fn drop(&mut self) {
            self.released.store(true, Ordering::SeqCst);
        }
    }

    impl TaskStore for FakeStore {
        fn create(&mut self, key: &TaskKey, doc: &Task, write: WriteId) -> StoreResult<Rev> {
            self.before_write(key)?;
            if let Some(current) = self.docs.get(&key.to_string()) {
                if landed_before(&current.1, &write) {
                    return Ok(current.0);
                }
                return Err(StoreError::AlreadyExists {
                    current: current.clone(),
                });
            }
            self.put(key, doc, Rev(1));
            self.maybe_lose_ack()?;
            Ok(Rev(1))
        }

        fn update(
            &mut self,
            key: &TaskKey,
            doc: &Task,
            expected: Rev,
            write: WriteId,
        ) -> StoreResult<Rev> {
            self.before_write(key)?;
            match self.docs.get(&key.to_string()) {
                Some((rev, _)) if *rev == expected => {
                    let next = Rev(rev.0 + 1);
                    self.put(key, doc, next);
                    self.maybe_lose_ack()?;
                    Ok(next)
                }
                Some((rev, bytes)) if landed_before(bytes, &write) => Ok(*rev),
                current => Err(StoreError::Conflict {
                    current: current.cloned(),
                }),
            }
        }

        fn get(&mut self, key: &StoreKey) -> StoreResult<Option<(Rev, Vec<u8>)>> {
            Ok(self.docs.get(&key.0).cloned())
        }

        fn scan(&mut self, prefix: &str) -> StoreResult<Vec<WatchEvent>> {
            if self
                .scan_failures
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                    left.checked_sub(1)
                })
                .is_ok()
            {
                return Err(StoreError::Io("the disk hiccuped".into()));
            }
            Ok(self
                .docs
                .iter()
                .filter(|(key, _)| key.starts_with(prefix))
                .map(|(key, (rev, bytes))| WatchEvent {
                    key: StoreKey(key.clone()),
                    rev: *rev,
                    value: Some(bytes.clone()),
                    at: None,
                })
                .collect())
        }

        fn watch(&mut self, _prefix: &str, sink: WatchSink) -> StoreResult<()> {
            self.sinks.push(sink);
            Ok(())
        }
    }

    fn task_owned_by(host: &str, id: &str) -> Task {
        Task::new(
            HostId::new(host).unwrap(),
            TaskId::new(id).unwrap(),
            "a task",
            Origin {
                host: host.into(),
                by: "x".into(),
                via: Via::Cli,
            },
            Timestamp("2026-10-21T20:00:00Z".into()),
        )
        .unwrap()
    }

    fn task(id: &str) -> Task {
        task_owned_by("m4m", id)
    }

    fn one_shot(store: FakeStore) -> impl FnMut() -> StoreResult<Box<dyn TaskStore>> + Send {
        let mut store = Some(store);
        move || {
            store
                .take()
                .map(|store| Box::new(store) as Box<dyn TaskStore>)
                .ok_or_else(|| StoreError::Io("opened twice".into()))
        }
    }

    fn start_with_store(store: FakeStore) -> (TaskService, Receiver<StoreEvent>) {
        start_with_opener(one_shot(store), test_identity("m4m"))
    }

    fn start_with_opener(
        open: impl FnMut() -> StoreResult<Box<dyn TaskStore>> + Send + 'static,
        identity: TaskIdentity,
    ) -> (TaskService, Receiver<StoreEvent>) {
        start_with_options(open, options(identity))
    }

    fn start_with_options(
        open: impl FnMut() -> StoreResult<Box<dyn TaskStore>> + Send + 'static,
        options: StartOptions,
    ) -> (TaskService, Receiver<StoreEvent>) {
        let (events, received) = mpsc::channel();
        let events = Mutex::new(events);
        let service = TaskService::start(open, options, move |_, event| {
            events.lock().unwrap().send(event).is_ok()
        })
        .unwrap();
        (service, received)
    }

    /// Feed events to the service until `done` says so, or the time is up.
    fn pump(
        service: &mut TaskService,
        received: &Receiver<StoreEvent>,
        limit: Duration,
        done: impl Fn(&TaskService) -> bool,
    ) {
        let deadline = Instant::now() + limit;
        while !done(service) {
            let left = deadline.saturating_duration_since(Instant::now());
            match received.recv_timeout(left) {
                Ok(event) => {
                    service.apply(event);
                }
                Err(_) => panic!("timed out waiting for the store"),
            }
        }
    }

    fn loaded(store: FakeStore) -> (TaskService, Receiver<StoreEvent>) {
        let (mut service, received) = start_with_store(store);
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.is_loaded()
        });
        (service, received)
    }

    #[test]
    fn slow_store_never_blocks_api() {
        let (mut service, received) = loaded(FakeStore::new(Duration::from_millis(1500)));

        // The write answers at once, though the store takes 1.5 s to do it.
        let one = task("tk-1");
        let started = Instant::now();
        let write = service.submit_create(&one).expect("the write is queued");
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "submit waited for the store: {:?}",
            started.elapsed()
        );
        assert_eq!(service.pending_count(), 1);

        // Reads keep answering from the cache while the write is in flight.
        let started = Instant::now();
        assert_eq!(service.cache().len(), 0);
        assert!(service.cache().get(&one.key()).is_none());
        assert!(started.elapsed() < Duration::from_millis(250));
        assert!(service.take_outcomes().is_empty(), "no outcome yet");

        // The outcome and the cache change arrive later.
        pump(
            &mut service,
            &received,
            Duration::from_secs(10),
            |service| service.pending_count() == 0 && service.cache().len() == 1,
        );
        assert_eq!(
            service.take_outcomes(),
            vec![(write, WriteOutcome::Applied { rev: Rev(1) })]
        );
        let cached = service.cache().get(&one.key()).unwrap();
        assert_eq!(cached.rev, Rev(1));
        assert_eq!(cached.task.title, "a task");
    }

    #[test]
    fn an_update_of_revision_zero_is_refused_before_it_is_queued() {
        let store = FakeStore::new(Duration::ZERO);
        let writes = Arc::clone(&store.writes);
        let (mut service, _received) = loaded(store);
        let err = service.submit_update(&task("tk-1"), Rev(0)).unwrap_err();
        assert_eq!(err.code(), "task_bad_revision");
        assert_eq!(service.pending_count(), 0);
        assert_eq!(writes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_stale_update_comes_back_as_a_conflict_not_an_error() {
        let (mut service, received) = loaded(FakeStore::new(Duration::ZERO));
        let one = task("tk-1");
        service.submit_create(&one).unwrap();
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.cache().len() == 1 && service.pending_count() == 0
        });
        service.take_outcomes();

        let mut edited = one.clone();
        edited.title = "mine".into();
        let write = service.submit_update(&edited, Rev(7)).unwrap();
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.pending_count() == 0
        });
        let outcomes = service.take_outcomes();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].0, write);
        assert!(matches!(
            &outcomes[0].1,
            WriteOutcome::Conflict {
                current: Some((Rev(1), _))
            }
        ));
        assert_eq!(
            service.cache().get(&one.key()).unwrap().task.title,
            "a task"
        );
    }

    #[test]
    fn a_disabled_service_refuses_writes_and_a_bad_task_never_reaches_the_store() {
        let mut service = TaskService::disabled();
        assert!(!service.is_running());
        assert_eq!(
            service.submit_create(&task("tk-1")),
            Err(SubmitError::Disabled)
        );

        let store = FakeStore::new(Duration::ZERO);
        let writes = Arc::clone(&store.writes);
        let (mut service, _received) = start_with_store(store);
        let mut long = task("tk-1");
        long.title = "x".repeat(201);
        let err = service.submit_create(&long).unwrap_err();
        assert_eq!(err.code(), "task_title_too_long");
        assert_eq!(service.pending_count(), 0);
        assert_eq!(writes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn the_initial_load_fills_the_cache() {
        let mut store = FakeStore::new(Duration::ZERO);
        let one = task("tk-1");
        store.put(&one.key(), &one, Rev(4));
        let (service, _received) = loaded(store);
        assert_eq!(service.cache().get(&one.key()).unwrap().rev, Rev(4));
    }

    #[test]
    fn dropping_the_service_stops_the_thread_without_waiting_for_it() {
        let (service, received) = start_with_store(FakeStore::new(Duration::from_secs(30)));
        let started = Instant::now();
        drop(service);
        assert!(started.elapsed() < Duration::from_millis(250));
        drop(received);
    }

    #[test]
    fn foreign_task_write_refused_not_queued() {
        let store = FakeStore::new(Duration::ZERO);
        let writes = Arc::clone(&store.writes);
        let (mut service, received) = loaded(store);

        let theirs = task_owned_by("ac-mbm5", "tk-1");
        for result in [
            service.submit_create(&theirs),
            service.submit_update(&theirs, Rev(1)),
        ] {
            let err = result.unwrap_err();
            assert_eq!(err.code(), "task_owned_elsewhere");
            assert_eq!(
                err,
                SubmitError::Refused(TaskError::OwnedElsewhere {
                    owner_host: "ac-mbm5".into()
                })
            );
        }
        assert_eq!(service.pending_count(), 0, "a refused write is not queued");

        // The own host's task goes through, and only it reaches the store.
        service.submit_create(&task("tk-2")).unwrap();
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.pending_count() == 0
        });
        assert_eq!(writes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn non_bus_session_refuses() {
        let throwaway = TaskIdentity {
            session: "throwaway".into(),
            bus_session: "default".into(),
            backend: BackendKind::Kv,
            ..test_identity("m4m")
        };
        // No store is started for it, so no credential is read, and the answer
        // names the bus session rather than saying tasks are off.
        let mut service = TaskService::without_store(throwaway.clone());
        let err = service.submit_create(&task("tk-1")).unwrap_err();
        assert_eq!(err.code(), "task_not_bus_session");
        assert_eq!(
            err,
            SubmitError::Refused(TaskError::NotBusSession {
                bus_session: "default".into()
            })
        );
        assert!(!throwaway.may_use_bus());

        // The bus session itself may use the bus; a throwaway on the file
        // backend runs its own store and needs none.
        let bus = TaskIdentity {
            session: "default".into(),
            ..throwaway.clone()
        };
        assert!(bus.may_use_bus());
        let own_file = TaskIdentity {
            backend: BackendKind::File,
            ..throwaway
        };
        assert!(own_file.may_use_bus());
        assert_eq!(own_file.check_write(&HostId::new("m4m").unwrap()), Ok(()));
    }

    #[tokio::test]
    async fn the_store_is_built_on_its_own_thread_even_when_started_inside_a_runtime() {
        // The app is built inside a runtime. A backend that blocks on a runtime
        // of its own (the kv one does) would panic if it were built here.
        let built_on = Arc::new(Mutex::new(None));
        let record = Arc::clone(&built_on);
        let (mut service, received) = start_with_opener(
            move || {
                *record.lock().unwrap() = std::thread::current().name().map(str::to_owned);
                let inner = tokio::runtime::Builder::new_current_thread()
                    .build()
                    .map_err(|err| StoreError::Io(err.to_string()))?;
                inner.block_on(async {});
                Ok(Box::new(FakeStore::new(Duration::ZERO)) as Box<dyn TaskStore>)
            },
            test_identity("m4m"),
        );
        let deadline = Duration::from_secs(5);
        tokio::task::spawn_blocking(move || {
            pump(&mut service, &received, deadline, |service| {
                service.is_loaded()
            });
            service
        })
        .await
        .unwrap();
        assert_eq!(
            built_on.lock().unwrap().as_deref(),
            Some("herdr-task-store")
        );
    }

    #[test]
    fn an_unreachable_store_is_retried_and_writes_meanwhile_wait_in_the_outbox() {
        let tries = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&tries);
        let store = FakeStore::new(Duration::ZERO);
        let log = Arc::clone(&store.log);
        let mut store = Some(store);
        let (mut service, received) = start_with_opener(
            move || {
                if counted.fetch_add(1, Ordering::SeqCst) < 3 {
                    return Err(StoreError::Unavailable("the network is not up".into()));
                }
                Ok(Box::new(store.take().unwrap()) as Box<dyn TaskStore>)
            },
            test_identity("m4m"),
        );

        // While it cannot connect the service says why, and takes writes.
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.failure().is_some()
        });
        assert!(service.failure().unwrap().contains("not reachable"));
        let write = service.submit_create(&task("tk-1")).unwrap();
        assert_eq!(service.pending_count(), 1);

        // Once the store answers, the queued write goes in, and the failure clears.
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.pending_count() == 0
        });
        assert_eq!(
            service.take_outcomes(),
            vec![(write, WriteOutcome::Applied { rev: Rev(1) })]
        );
        assert_eq!(service.failure(), None);
        assert_eq!(log.lock().unwrap().len(), 1);
        assert!(tries.load(Ordering::SeqCst) >= 4);
    }

    #[test]
    fn lost_ack_replay_is_dropped() {
        // The store takes the write, then its answer is lost. The write is
        // sent again with the same id; the store recognises it and the task is
        // changed once, not twice.
        let store = FakeStore::new(Duration::ZERO);
        let log = Arc::clone(&store.log);
        let lose = Arc::clone(&store.lose_ack_once);
        let (mut service, received) = loaded(store);
        let one = task("tk-1");
        service.submit_create(&one).unwrap();
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.pending_count() == 0
        });
        service.take_outcomes();

        let mut edited = one.clone();
        edited.title = "edited".into();
        lose.store(true, Ordering::SeqCst);
        let write = service.submit_update(&edited, Rev(1)).unwrap();
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.pending_count() == 0
        });
        assert_eq!(
            service.take_outcomes(),
            vec![(write, WriteOutcome::Applied { rev: Rev(2) })],
            "one outcome, at the revision the first try produced"
        );
        assert_eq!(*log.lock().unwrap(), ["a task", "edited"], "applied once");
    }

    #[test]
    fn chained_entries_replay_in_order() {
        // The store is down while three edits of one task are made; each is
        // built on the one before. When it comes back they go in, in order,
        // each against the revision the previous one produced.
        let store = FakeStore::new(Duration::ZERO);
        let log = Arc::clone(&store.log);
        let up = Arc::new(AtomicBool::new(false));
        let gate = Arc::clone(&up);
        let mut store = Some(store);
        let (mut service, received) = start_with_opener(
            move || {
                if !gate.load(Ordering::SeqCst) {
                    return Err(StoreError::Unavailable("down".into()));
                }
                Ok(Box::new(store.take().unwrap()) as Box<dyn TaskStore>)
            },
            test_identity("m4m"),
        );
        let created = task("tk-1");
        let w1 = service.submit_create(&created).unwrap();
        let key = created.key();
        let mut second = service.current(&key).unwrap().1.clone();
        second.title = "second".into();
        let w2 = service.submit_update(&second, Rev(1)).unwrap();
        let mut third = service.current(&key).unwrap().1.clone();
        assert_eq!(
            third.title, "second",
            "the next edit builds on the queued one"
        );
        third.title = "third".into();
        let w3 = service.submit_update(&third, Rev(1)).unwrap();

        up.store(true, Ordering::SeqCst);
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.pending_count() == 0
        });
        assert_eq!(
            service.take_outcomes(),
            vec![
                (w1, WriteOutcome::Applied { rev: Rev(1) }),
                (w2, WriteOutcome::Applied { rev: Rev(2) }),
                (w3, WriteOutcome::Applied { rev: Rev(3) }),
            ]
        );
        assert_eq!(*log.lock().unwrap(), ["a task", "second", "third"]);
    }

    #[test]
    fn a_queued_write_survives_a_handoff_of_the_service() {
        // The old server's store is down, so two writes wait in the outbox. It
        // hands over: the replacement reads the same outbox and sends both, in
        // order, when its own store answers.
        let dir = scratch_outbox();
        let (mut old, _old_received) = start_with_options(
            || Err(StoreError::Unavailable("down".into())),
            StartOptions {
                outbox_dir: dir.clone(),
                ..options(test_identity("m4m"))
            },
        );
        let one = task("tk-1");
        let w1 = old.submit_create(&one).unwrap();
        let mut edited = one.clone();
        edited.title = "edited".into();
        let w2 = old.submit_update(&edited, Rev(1)).unwrap();
        assert!(
            old.quiesce(Duration::from_secs(5)),
            "the old server lets go"
        );
        drop(old);

        let store = FakeStore::new(Duration::ZERO);
        let log = Arc::clone(&store.log);
        let (mut new, received) = start_with_options(
            one_shot(store),
            StartOptions {
                outbox_dir: dir.clone(),
                ..options(test_identity("m4m"))
            },
        );
        pump(&mut new, &received, Duration::from_secs(5), |service| {
            service.is_loaded()
        });
        // What it found waiting counts as pending until it settles.
        pump(&mut new, &received, Duration::from_secs(5), |service| {
            service.pending_count() == 0 && service.cache().len() == 1
        });
        assert_eq!(*log.lock().unwrap(), ["a task", "edited"], "both, in order");
        let _ = (w1, w2);
        assert!(
            std::fs::read_dir(&dir)
                .unwrap()
                .filter_map(Result::ok)
                .all(|entry| entry.path().extension().is_none_or(|ext| ext != "json")),
            "the outbox is empty once the store has them"
        );
    }

    #[test]
    fn a_conflict_holds_the_chain_behind_it_and_the_rest_goes_on() {
        let store = FakeStore::new(Duration::ZERO);
        let log = Arc::clone(&store.log);
        let (mut service, received) = loaded(store);
        let one = task("tk-1");
        service.submit_create(&one).unwrap();
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.pending_count() == 0
        });
        service.take_outcomes();
        let applied_before = log.lock().unwrap().len();

        // Two edits made against a revision that has moved, then an unrelated one.
        let mut stale = one.clone();
        stale.title = "stale".into();
        let first = service.submit_update(&stale, Rev(9)).unwrap();
        let mut behind = stale.clone();
        behind.title = "behind it".into();
        let second = service.submit_update(&behind, Rev(9)).unwrap();
        let other = task("tk-2");
        let third = service.submit_create(&other).unwrap();

        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.take_outcomes_peek().len() >= 2
        });
        let outcomes = service.take_outcomes();
        let reported: Vec<&WriteId> = outcomes.iter().map(|(write, _)| write).collect();
        assert!(reported.contains(&&first) && reported.contains(&&third));
        assert!(
            !reported.contains(&&second),
            "the chain behind a conflict waits"
        );
        assert!(matches!(
            outcomes
                .iter()
                .find(|(write, _)| *write == first)
                .unwrap()
                .1,
            WriteOutcome::Conflict { .. }
        ));
        assert_eq!(
            log.lock().unwrap().len(),
            applied_before + 1,
            "only the unrelated create reached the store"
        );
        assert_eq!(
            service.pending_count(),
            1,
            "the held write is still waiting"
        );
    }

    /// A task created and stored at revision 1, then two edits queued against
    /// a revision that has moved: the first is held as a conflict, the second
    /// waits behind it. Returns the held write and the one behind it.
    fn two_held_edits(
        service: &mut TaskService,
        received: &Receiver<StoreEvent>,
        created: &Task,
    ) -> (WriteId, WriteId) {
        service.submit_create(created).unwrap();
        pump(service, received, Duration::from_secs(5), |service| {
            service.pending_count() == 0
        });
        service.take_outcomes();
        let mut stale = created.clone();
        stale.title = "stale".into();
        let first = service.submit_update(&stale, Rev(9)).unwrap();
        let mut behind = service.current(&created.key()).unwrap().1.clone();
        behind.title = "behind it".into();
        let second = service.submit_update(&behind, Rev(9)).unwrap();
        pump(service, received, Duration::from_secs(5), |service| {
            !service.conflicts().is_empty() && !service.take_outcomes_peek().is_empty()
        });
        service.take_outcomes();
        (first, second)
    }

    #[test]
    fn a_held_write_is_listed_with_the_stores_version() {
        let (mut service, received) = loaded(FakeStore::new(Duration::ZERO));
        let created = task("tk-1");
        let (first, _) = two_held_edits(&mut service, &received, &created);
        let held = service.conflicts();
        assert_eq!(held.len(), 1, "only the head of the chain is held");
        assert_eq!(held[0].write, first);
        assert_eq!(held[0].key, created.key());
        assert_eq!(held[0].mine.title, "stale");
        assert_eq!(held[0].theirs_rev, Some(Rev(1)));
        assert_eq!(held[0].theirs.as_ref().unwrap()["title"], "a task");
    }

    #[test]
    fn theirs_cancels_descendants() {
        let store = FakeStore::new(Duration::ZERO);
        let log = Arc::clone(&store.log);
        let (mut service, received) = loaded(store);
        let created = task("tk-1");
        let (first, second) = two_held_edits(&mut service, &received, &created);
        let key = created.key();
        assert_eq!(service.pending_count(), 1, "the write behind it waits");

        let ticket = service
            .resolve_conflict(&key, &first, Choice::Theirs, Rev(1))
            .unwrap();
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.pending_count() == 0 && service.conflicts().is_empty()
        });
        assert_eq!(
            service.take_resolutions(),
            vec![(
                ticket,
                ResolveOutcome::Dropped {
                    cancelled: vec![second.clone()]
                }
            )]
        );
        let outcomes = service.take_outcomes();
        assert_eq!(outcomes.len(), 1, "the write behind it comes back");
        assert_eq!(outcomes[0].0, second);
        assert!(
            matches!(
                &outcomes[0].1,
                WriteOutcome::Conflict {
                    current: Some((Rev(1), _))
                }
            ),
            "as a conflict against the store's version: {:?}",
            outcomes[0].1
        );
        assert_eq!(
            *log.lock().unwrap(),
            ["a task"],
            "neither edit reached the store"
        );
        assert!(service
            .current(&key)
            .is_some_and(|(_, task)| task.title == "a task"));
    }

    #[test]
    fn second_resolver_gets_stale_revision() {
        let store = FakeStore::new(Duration::ZERO);
        let log = Arc::clone(&store.log);
        let (mut service, received) = loaded(store);
        let created = task("tk-1");
        let (first, second) = two_held_edits(&mut service, &received, &created);
        let key = created.key();

        // Two clients answer the same conflict before either result is back.
        let winner = service
            .resolve_conflict(&key, &first, Choice::Mine, Rev(1))
            .unwrap();
        let loser = service
            .resolve_conflict(&key, &first, Choice::Theirs, Rev(1))
            .unwrap();
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.pending_count() == 0
                && service.resolutions.len() == 2
                && service
                    .cache()
                    .get(&key)
                    .is_some_and(|cached| cached.rev == Rev(3))
        });
        let resolutions = service.take_resolutions();
        assert_eq!(resolutions[0], (winner, ResolveOutcome::Requeued));
        match &resolutions[1] {
            (
                ticket,
                ResolveOutcome::Stale {
                    current: Some((rev, bytes)),
                },
            ) => {
                assert_eq!(*ticket, loser);
                assert_eq!(*rev, Rev(3), "the refresh is what the store holds now");
                let doc: Task = serde_json::from_slice(bytes).unwrap();
                assert_eq!(doc.title, "behind it");
            }
            other => panic!("expected a stale answer, got {other:?}"),
        }
        // The winner's choice stood: both edits went in, in order.
        assert_eq!(
            *log.lock().unwrap(),
            ["a task", "stale", "behind it"],
            "mine was written on top of theirs, and the chain followed"
        );
        let applied = service.take_outcomes();
        assert_eq!(
            applied,
            vec![
                (first.clone(), WriteOutcome::Applied { rev: Rev(2) }),
                (second, WriteOutcome::Applied { rev: Rev(3) }),
            ]
        );

        // A third one, once the result is known, is turned away at once.
        let err = service
            .resolve_conflict(&key, &first, Choice::Mine, Rev(1))
            .unwrap_err();
        assert_eq!(err.code(), "stale_revision");
        assert!(matches!(
            err,
            SubmitError::Stale {
                current: Some((Rev(3), _))
            }
        ));
    }

    #[test]
    fn a_caller_who_saw_an_older_revision_is_told_to_look_again() {
        let (mut service, received) = loaded(FakeStore::new(Duration::ZERO));
        let created = task("tk-1");
        let (first, _) = two_held_edits(&mut service, &received, &created);
        let err = service
            .resolve_conflict(&created.key(), &first, Choice::Mine, Rev(7))
            .unwrap_err();
        assert!(matches!(
            err,
            SubmitError::Stale {
                current: Some((Rev(1), _))
            }
        ));
        assert_eq!(service.conflicts().len(), 1, "the conflict is untouched");
    }

    #[test]
    fn merged_writes_the_given_document_and_a_foreign_one_is_refused() {
        let store = FakeStore::new(Duration::ZERO);
        let log = Arc::clone(&store.log);
        let (mut service, received) = loaded(store);
        let created = task("tk-1");
        let (first, _) = two_held_edits(&mut service, &received, &created);
        let key = created.key();

        let mut elsewhere = created.clone();
        elsewhere.id = TaskId::new("tk-2").unwrap();
        let err = service
            .resolve_conflict(&key, &first, Choice::Merged(Box::new(elsewhere)), Rev(1))
            .unwrap_err();
        assert_eq!(err.code(), "task_key_mismatch");

        let mut merged = created.clone();
        merged.title = "merged by hand".into();
        service
            .resolve_conflict(&key, &first, Choice::Merged(Box::new(merged)), Rev(1))
            .unwrap();
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.pending_count() == 0
                && service
                    .cache()
                    .get(&key)
                    .is_some_and(|cached| cached.rev == Rev(3))
        });
        assert_eq!(
            *log.lock().unwrap(),
            ["a task", "merged by hand", "behind it"]
        );
        let (_, task) = service.current(&key).unwrap();
        assert!(
            task.last_writes.contains(&first),
            "the merged document carries the write id, so a replay is recognised"
        );
    }

    #[test]
    fn a_held_write_is_still_held_after_a_restart() {
        let dir = scratch_outbox();
        let store = FakeStore::new(Duration::ZERO);
        let (mut old, received) = start_with_options(
            one_shot(store),
            StartOptions {
                outbox_dir: dir.clone(),
                ..options(test_identity("m4m"))
            },
        );
        pump(&mut old, &received, Duration::from_secs(5), |service| {
            service.is_loaded()
        });
        let created = task("tk-1");
        let (first, _) = two_held_edits(&mut old, &received, &created);
        assert!(old.quiesce(Duration::from_secs(5)));
        drop(old);

        // The replacement finds the same store contents and the same outbox.
        let mut store = FakeStore::new(Duration::ZERO);
        store.put(&created.key(), &created, Rev(1));
        let (mut new, received) = start_with_options(
            one_shot(store),
            StartOptions {
                outbox_dir: dir,
                ..options(test_identity("m4m"))
            },
        );
        pump(&mut new, &received, Duration::from_secs(5), |service| {
            !service.conflicts().is_empty()
        });
        assert_eq!(new.conflicts()[0].write, first);
        assert_eq!(new.conflicts()[0].theirs_rev, Some(Rev(1)));
        assert!(
            !new.pending.contains_key(&first),
            "a held write waits for a decision, not for the store"
        );
    }

    #[test]
    fn a_write_is_saved_only_once_it_is_on_disk_and_its_outcome_ends_that() {
        let (mut service, received) = loaded(FakeStore::new(Duration::ZERO));
        let write = service.submit_create(&task("tk-1")).unwrap();
        assert!(
            !service.is_saved(&write),
            "answered pending, but nothing says it is on disk yet"
        );
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.is_saved(&write) || service.pending_count() == 0
        });
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.pending_count() == 0
        });
        assert!(!service.is_saved(&write), "settled: nothing left to save");
        assert_eq!(service.take_outcomes().len(), 1);
    }

    #[test]
    fn a_write_waiting_for_a_store_that_is_down_is_saved_before_it_is_sent() {
        let store = FakeStore::new(Duration::ZERO);
        let up = Arc::new(AtomicBool::new(false));
        let gate = Arc::clone(&up);
        let mut store = Some(store);
        let (mut service, received) = start_with_opener(
            move || {
                if !gate.load(Ordering::SeqCst) {
                    return Err(StoreError::Unavailable("down".into()));
                }
                Ok(Box::new(store.take().unwrap()) as Box<dyn TaskStore>)
            },
            test_identity("m4m"),
        );
        let write = service.submit_create(&task("tk-1")).unwrap();
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.is_saved(&write)
        });
        assert_eq!(service.pending_count(), 1, "still waiting for the store");
    }

    #[test]
    fn a_stopped_thread_answers_every_waiting_resolve_request() {
        let (mut service, received) = loaded(FakeStore::new(Duration::ZERO));
        let created = task("tk-1");
        let (first, _) = two_held_edits(&mut service, &received, &created);
        let ticket = service
            .resolve_conflict(&created.key(), &first, Choice::Theirs, Rev(1))
            .unwrap();
        service.apply(StoreEvent::Stopped("the thread is gone".into()));
        assert_eq!(
            service.take_resolutions(),
            vec![(
                ticket,
                ResolveOutcome::Failed("the thread is gone".to_owned())
            )]
        );
    }

    #[test]
    fn a_thread_that_ends_answers_what_is_still_in_its_channel() {
        let dir = scratch_outbox();
        let mut outbox = Outbox::open(dir).unwrap();
        let (sender, receiver) = mpsc::channel();
        let one = task("tk-1");
        let write = WriteId::generate();
        sender
            .send(Message::Write(WriteCommand {
                write: write.clone(),
                op: Op::Create,
                doc: Box::new(one.clone()),
                base: None,
                auto: false,
            }))
            .unwrap();
        sender
            .send(Message::Resolve(ResolveCommand {
                ticket: 7,
                write: WriteId::generate(),
                key: one.key(),
                choice: Choice::Mine,
                expected: Rev(1),
            }))
            .unwrap();
        let told = Mutex::new(Vec::new());
        finish(&mut outbox, &receiver, &|event| {
            told.lock().unwrap().push(event);
            true
        });
        assert_eq!(outbox.len(), 1, "the write was saved");
        let told = told.into_inner().unwrap();
        assert!(told.iter().any(
            |event| matches!(event, StoreEvent::Accepted { write: saved } if *saved == write)
        ));
        assert!(told.iter().any(|event| matches!(
            event,
            StoreEvent::Resolved(resolution)
                if resolution.ticket == 7
                    && matches!(resolution.outcome, ResolveOutcome::Failed(_))
        )));
    }

    #[test]
    fn one_stuck_document_does_not_hold_up_the_others() {
        let store = FakeStore::new(Duration::ZERO);
        let stuck = Arc::clone(&store.stuck_keys);
        let (mut service, received) = loaded(store);
        let (a, b) = (task("tk-a"), task("tk-b"));
        stuck.lock().unwrap().push(a.key().to_string());
        let write_a = service.submit_create(&a).unwrap();
        let write_b = service.submit_create(&b).unwrap();
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.pending_count() == 1
        });
        assert_eq!(
            service.take_outcomes(),
            vec![(write_b, WriteOutcome::Applied { rev: Rev(1) })],
            "the other document went in"
        );
        stuck.lock().unwrap().clear();
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.pending_count() == 0
        });
        assert_eq!(
            service.take_outcomes(),
            vec![(write_a, WriteOutcome::Applied { rev: Rev(1) })],
            "and the stuck one follows once the store answers"
        );
    }

    #[test]
    fn a_local_io_error_is_retried_and_a_definite_refusal_fails_the_write() {
        let store = FakeStore::new(Duration::ZERO);
        let fail_with = Arc::clone(&store.fail_with);
        let (mut service, received) = loaded(store);

        *fail_with.lock().unwrap() = Some(StoreError::Io("the disk is full".into()));
        let write = service.submit_create(&task("tk-1")).unwrap();
        let quiet = Instant::now() + Duration::from_millis(300);
        while Instant::now() < quiet {
            if let Ok(event) = received.recv_timeout(Duration::from_millis(20)) {
                service.apply(event);
            }
        }
        assert_eq!(service.pending_count(), 1, "kept, not thrown away");
        assert!(service.take_outcomes().is_empty());
        *fail_with.lock().unwrap() = None;
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.pending_count() == 0
        });
        assert_eq!(
            service.take_outcomes(),
            vec![(write, WriteOutcome::Applied { rev: Rev(1) })]
        );

        *fail_with.lock().unwrap() = Some(StoreError::Refused("not a usable key".into()));
        let doomed = service.submit_create(&task("tk-2")).unwrap();
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.pending_count() == 0
        });
        assert_eq!(
            service.take_outcomes(),
            vec![(doomed, WriteOutcome::Failed("not a usable key".to_owned()))]
        );
    }

    #[test]
    fn a_replay_without_a_receipt_needs_the_write_ahead_to_be_the_newest_there() {
        let dir = scratch_outbox();
        let mut outbox = Outbox::open(dir).unwrap();
        let ahead = WriteId::generate();
        let foreign = WriteId::generate();
        let one = task("tk-1");
        let write = WriteId::generate();
        outbox
            .push(
                write.clone(),
                Op::Update,
                one.clone(),
                Some(Base::After(ahead.clone())),
            )
            .unwrap();
        let entry = outbox.entries()[0].clone();

        // The write ahead landed, and a foreign one landed after it: writing on
        // top would overwrite that one.
        let mut store = FakeStore::new(Duration::ZERO);
        let mut doc = one.clone();
        doc.last_writes = vec![ahead.clone(), foreign.clone()];
        store.put(&one.key(), &doc, Rev(3));
        assert!(matches!(
            expected_rev(&outbox, &mut store, &entry),
            Err(StoreError::Conflict {
                current: Some((Rev(3), _))
            })
        ));

        // The write ahead is the newest there: its revision is the base.
        let mut store = FakeStore::new(Duration::ZERO);
        doc.last_writes = vec![foreign, ahead];
        store.put(&one.key(), &doc, Rev(3));
        assert_eq!(expected_rev(&outbox, &mut store, &entry), Ok(Rev(3)));
    }

    fn run_record(id: &str, task: &Task, state: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "task": task.key().to_string(),
            "engine": "claude_code",
            "state": state,
        })
    }

    #[test]
    fn rescan_rebuilds_runs() {
        // The server starts (or a handoff finishes) and finds run records that
        // were written while nobody watched: a finished run, a live one, and
        // one forged by another Mac's credential.
        let one = task("tk-1");
        let mut store = FakeStore::new(Duration::ZERO);
        let writes = Arc::clone(&store.writes);
        store.put(&one.key(), &one, Rev(1));
        store.put_raw("run.m4m.rn-01", &run_record("rn-01", &one, "done"), Rev(1));
        store.put_raw(
            "run.worklink.rn-02",
            &run_record("rn-02", &one, "running"),
            Rev(1),
        );
        store.put_raw(
            "run.ac-mbm5.rn-03",
            &run_record("rn-03", &one, "running"),
            Rev(1),
        );
        let (mut service, received) = start_with_store(store);
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service
                .cache()
                .get(&one.key())
                .is_some_and(|cached| cached.rev == Rev(2))
        });
        let cached = &service.cache().get(&one.key()).unwrap().task;
        assert_eq!(
            cached.runs,
            ["rn-01", "rn-02"],
            "the records of the owner and of worklink, not the forged one"
        );
        assert_eq!(cached.status, TaskStatus::Running, "a live run started it");
        assert_eq!(writes.load(Ordering::SeqCst), 1, "one write, once");
        assert!(
            service.take_outcomes().is_empty(),
            "nobody asked for it, nobody is told"
        );
        assert_eq!(service.pending_count(), 0);

        // It settles: nothing more is written, however long it listens.
        let quiet = Instant::now() + Duration::from_millis(300);
        while Instant::now() < quiet {
            if let Ok(event) = received.recv_timeout(Duration::from_millis(20)) {
                service.apply(event);
            }
        }
        assert_eq!(writes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_new_run_record_is_followed_while_the_server_watches() {
        let one = task("tk-1");
        let mut store = FakeStore::new(Duration::ZERO);
        store.put(&one.key(), &one, Rev(1));
        let (mut service, received) = start_with_store(store);
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.is_loaded()
        });
        assert_eq!(service.cache().get(&one.key()).unwrap().task.runs.len(), 0);

        // A run record arrives through the watch, as the starter writes it.
        let record = run_record("rn-09", &one, "starting");
        service.apply(StoreEvent::Changed(WatchEvent {
            key: StoreKey("run.m4m.rn-09".into()),
            rev: Rev(1),
            value: Some(serde_json::to_vec(&record).unwrap()),
            at: Some(SystemTime::now()),
        }));
        assert_eq!(service.pending_count(), 1, "the rebuild is on its way");
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service
                .cache()
                .get(&one.key())
                .is_some_and(|cached| cached.rev == Rev(2))
        });
        let cached = &service.cache().get(&one.key()).unwrap().task;
        assert_eq!(cached.runs, ["rn-09"]);
        assert_eq!(cached.status, TaskStatus::Running);
    }

    #[test]
    fn the_five_minutes_are_announced_by_the_service_clock() {
        thread_local! {
            static NOW: std::cell::Cell<u64> = const { std::cell::Cell::new(2_000_000_000) };
        }
        fn clock() -> SystemTime {
            SystemTime::UNIX_EPOCH + Duration::from_secs(NOW.with(std::cell::Cell::get))
        }
        let one = task("tk-1");
        let (mut service, _received) = loaded(FakeStore::new(Duration::ZERO));
        service.clock = clock;
        service.status_mark = clock();
        service.cache.apply(&WatchEvent {
            key: StoreKey::from(&one.key()),
            rev: Rev(1),
            value: Some(serde_json::to_vec(&one).unwrap()),
            at: None,
        });
        let started = clock();
        service.runs.apply(
            &WatchEvent {
                key: StoreKey("run.m4m.rn-01".into()),
                rev: Rev(1),
                value: Some(serde_json::to_vec(&run_record("rn-01", &one, "starting")).unwrap()),
                at: Some(started),
            },
            started,
        );
        let shown = |service: &TaskService| {
            service
                .shown_status(&one.key(), |_| AskRow::Closed)
                .unwrap()
                .status
        };
        assert_eq!(shown(&service), crate::tasks::Shown::Running);
        assert_eq!(
            service.next_status_deadline(),
            Some(Duration::from_secs(300))
        );
        assert!(service.due_status_changes().is_empty());

        NOW.with(|now| now.set(now.get() + 301));
        assert_eq!(shown(&service), crate::tasks::Shown::RunUnknown);
        assert_eq!(service.due_status_changes(), vec![one.key()]);
        assert!(service.due_status_changes().is_empty(), "announced once");
        assert_eq!(service.next_status_deadline(), None);
    }

    #[test]
    fn a_conflict_on_a_write_the_server_made_is_dropped_not_held() {
        let outbox_dir = scratch_outbox();
        let mut outbox = Outbox::open(outbox_dir).unwrap();
        let one = task("tk-1");
        let mut store = FakeStore::new(Duration::ZERO);
        store.put(&one.key(), &one, Rev(5));
        let write = WriteId::generate();
        outbox
            .push_as(
                write.clone(),
                Op::Update,
                one.clone(),
                Some(Base::Rev(Rev(1))),
                true,
            )
            .unwrap();
        let entry = outbox.entries()[0].clone();
        let Attempt::Done(events) = try_entry(&mut outbox, &mut store, &entry) else {
            panic!("expected the entry to be settled");
        };
        assert_eq!(outbox.len(), 0, "dropped, not kept for a decision");
        assert!(events
            .iter()
            .all(|event| !matches!(event, StoreEvent::Held(_))));
        assert!(matches!(
            events.as_slice(),
            [StoreEvent::Outcome {
                outcome: WriteOutcome::Conflict { .. },
                ..
            }]
        ));
    }

    #[test]
    fn held_writes_count_toward_the_cap_on_waiting_writes() {
        // The store is down, so every write stays pending; some more are held
        // as conflicts. Together they are what the outbox holds.
        let (mut service, _received) = start_with_opener(
            || Err(StoreError::Unavailable("down".into())),
            test_identity("m4m"),
        );
        let held = MAX_PENDING_WRITES - 56;
        for index in 0..held {
            let doc = task(&format!("tk-held-{index}"));
            service.apply(StoreEvent::Held(Box::new(HeldConflict {
                write: WriteId::generate(),
                key: doc.key(),
                mine: doc,
                theirs_rev: Some(Rev(1)),
                theirs: None,
            })));
        }
        assert_eq!(service.conflicts().len(), held);
        for index in 0..56 {
            service
                .submit_create(&task(&format!("tk-wait-{index}")))
                .unwrap();
        }
        let refused = service.submit_create(&task("tk-one-too-many")).unwrap_err();
        assert_eq!(refused, SubmitError::Busy);
        assert_eq!(refused.code(), "task_store_busy");
    }

    /// A store that applies a write and then says its answer was lost.
    struct LoseAck<S> {
        inner: S,
        lose: Arc<AtomicBool>,
    }

    impl<S: TaskStore> TaskStore for LoseAck<S> {
        fn create(&mut self, key: &TaskKey, doc: &Task, write: WriteId) -> StoreResult<Rev> {
            let rev = self.inner.create(key, doc, write)?;
            if self.lose.swap(false, Ordering::SeqCst) {
                return Err(StoreError::Indeterminate("the answer was lost".into()));
            }
            Ok(rev)
        }

        fn update(
            &mut self,
            key: &TaskKey,
            doc: &Task,
            expected: Rev,
            write: WriteId,
        ) -> StoreResult<Rev> {
            let rev = self.inner.update(key, doc, expected, write)?;
            if self.lose.swap(false, Ordering::SeqCst) {
                return Err(StoreError::Indeterminate("the answer was lost".into()));
            }
            Ok(rev)
        }

        fn get(&mut self, key: &StoreKey) -> StoreResult<Option<(Rev, Vec<u8>)>> {
            self.inner.get(key)
        }

        fn scan(&mut self, prefix: &str) -> StoreResult<Vec<WatchEvent>> {
            self.inner.scan(prefix)
        }

        fn watch(&mut self, prefix: &str, sink: WatchSink) -> StoreResult<()> {
            self.inner.watch(prefix, sink)
        }
    }

    #[test]
    fn a_lost_acknowledgement_is_recognised_by_the_file_backend_too() {
        let lose = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&lose);
        let dir = scratch_outbox().with_extension("store");
        let cleanup = dir.clone();
        CLEANUP.with(|c| c.borrow_mut().0.push(cleanup));
        let mut open = Some(crate::tasks::file_store::FileStore::open(dir).unwrap());
        let (mut service, received) = start_with_opener(
            move || {
                open.take()
                    .map(|inner| {
                        Box::new(LoseAck {
                            inner,
                            lose: Arc::clone(&flag),
                        }) as Box<dyn TaskStore>
                    })
                    .ok_or_else(|| StoreError::Io("opened twice".into()))
            },
            test_identity("m4m"),
        );
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.is_loaded()
        });
        let one = task("tk-1");
        service.submit_create(&one).unwrap();
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.pending_count() == 0 && service.cache().get(&one.key()).is_some()
        });
        service.take_outcomes();

        let mut edited = service.current(&one.key()).unwrap().1.clone();
        edited.title = "edited".into();
        lose.store(true, Ordering::SeqCst);
        let write = service.submit_update(&edited, Rev(1)).unwrap();
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.pending_count() == 0
        });
        assert_eq!(
            service.take_outcomes(),
            vec![(write, WriteOutcome::Applied { rev: Rev(2) })],
            "the replay found its own write and did not apply it twice"
        );
    }

    #[test]
    fn a_publish_that_may_have_landed_is_retried_not_reported() {
        let store = FakeStore::new(Duration::ZERO);
        let flag = Arc::clone(&store.indeterminate);
        flag.store(true, Ordering::SeqCst);
        let (mut service, received) = loaded(store);
        let write = service.submit_create(&task("tk-1")).unwrap();
        // Told nothing yet: the write id settles it when the store answers.
        let quiet = Instant::now() + Duration::from_millis(300);
        while Instant::now() < quiet {
            if let Ok(event) = received.recv_timeout(Duration::from_millis(20)) {
                service.apply(event);
            }
        }
        assert_eq!(service.pending_count(), 1);
        assert!(service.take_outcomes().is_empty());
        flag.store(false, Ordering::SeqCst);
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.pending_count() == 0
        });
        assert_eq!(
            service.take_outcomes(),
            vec![(write, WriteOutcome::Applied { rev: Rev(1) })]
        );
    }

    #[test]
    fn a_failed_first_scan_is_retried_not_left_empty() {
        let mut store = FakeStore::new(Duration::ZERO);
        let one = task("tk-1");
        store.put(&one.key(), &one, Rev(2));
        store.scan_failures.store(2, Ordering::SeqCst);
        let (service, _received) = loaded(store);
        assert_eq!(service.cache().get(&one.key()).unwrap().rev, Rev(2));
    }

    #[test]
    fn a_dead_store_thread_is_noticed_and_fails_every_pending_write() {
        let mut store = FakeStore::new(Duration::ZERO);
        store.panic_on_write = true;
        let (mut service, received) = loaded(store);
        let first = service.submit_create(&task("tk-1")).unwrap();
        let second = service.submit_create(&task("tk-2")).unwrap();

        pump(&mut service, &received, Duration::from_secs(5), |service| {
            !service.is_running()
        });
        assert_eq!(service.pending_count(), 0);
        assert!(service.failure().unwrap().contains("panicked"));
        let failed: Vec<WriteId> = service
            .take_outcomes()
            .into_iter()
            .inspect(|(_, outcome)| assert!(matches!(outcome, WriteOutcome::Unknown(_))))
            .map(|(write, _)| write)
            .collect();
        assert!(failed.contains(&first) && failed.contains(&second));
        assert_eq!(
            service.submit_create(&task("tk-3")).unwrap_err().code(),
            "tasks_disabled",
            "a service whose thread died takes no more writes"
        );
    }

    #[test]
    fn a_store_that_cannot_be_opened_stops_the_service_with_the_reason() {
        let (mut service, received) = start_with_opener(
            || Err(StoreError::Io("the directory is locked".into())),
            test_identity("m4m"),
        );
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            !service.is_running()
        });
        assert!(service.failure().unwrap().contains("locked"));
    }

    #[test]
    fn quiesce_finishes_queued_writes_and_returns_only_after_the_store_is_released() {
        let store = FakeStore::new(Duration::from_millis(100));
        let writes = Arc::clone(&store.writes);
        let released = Arc::clone(&store.released);
        let (mut service, _received) = loaded(store);
        service.submit_create(&task("tk-1")).unwrap();
        service.submit_create(&task("tk-2")).unwrap();

        assert!(service.quiesce(Duration::from_secs(5)));
        assert_eq!(writes.load(Ordering::SeqCst), 2, "queued writes finished");
        assert!(released.load(Ordering::SeqCst), "the store was let go");
        assert_eq!(
            service.submit_create(&task("tk-3")).unwrap_err().code(),
            "tasks_disabled"
        );
    }

    #[test]
    fn quiesce_gives_up_on_a_store_that_does_not_answer() {
        let store = FakeStore::new(Duration::from_secs(3));
        let (mut service, _received) = loaded(store);
        service.submit_create(&task("tk-1")).unwrap();
        let started = Instant::now();
        assert!(!service.quiesce(Duration::from_millis(200)));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
