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

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use super::cache::TaskCache;
use super::identity::TaskIdentity;
use super::ids::{Rev, TaskKey, WriteId};
use super::model::{Task, TaskError};
use super::store::{StoreError, StoreResult, TaskStore, WatchEvent, WatchSink};

/// Writes that may wait for the store at once. More are refused, so a store
/// that stopped answering cannot make the server queue without end.
const MAX_PENDING_WRITES: usize = 256;
/// Outcomes kept for the API to take; the oldest go first.
const MAX_OUTCOMES: usize = 1024;

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

/// A write for the store thread.
enum WriteCommand {
    Create {
        key: TaskKey,
        doc: Box<Task>,
        write: WriteId,
    },
    Update {
        key: TaskKey,
        doc: Box<Task>,
        expected: Rev,
        write: WriteId,
    },
}

/// Everything the store thread's one channel carries.
enum Message {
    Write(WriteCommand),
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
    /// How a write ended.
    Outcome {
        write: WriteId,
        outcome: WriteOutcome,
    },
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
}

impl SubmitError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Disabled => "tasks_disabled",
            Self::Stopped => "task_store_stopped",
            Self::Refused(err) => err.code(),
            Self::Busy => "task_store_busy",
        }
    }
}

pub(crate) struct TaskService {
    identity: Option<TaskIdentity>,
    cache: TaskCache,
    commands: Option<Sender<Message>>,
    /// Raised by the thread when it ends, after it let go of the store.
    finished: Option<Receiver<()>>,
    /// Writes sent and not yet answered, by write id.
    pending: HashMap<WriteId, TaskKey>,
    /// Outcomes not yet taken, in arrival order.
    outcomes: Vec<(WriteId, WriteOutcome)>,
    /// Why the store is unusable, when it is.
    failure: Option<String>,
    /// The store's first scan has arrived, so the cache is complete.
    loaded: bool,
}

impl TaskService {
    /// A service with no store: reads are empty and writes are refused.
    pub(crate) fn disabled() -> Self {
        Self {
            identity: None,
            cache: TaskCache::new(),
            commands: None,
            finished: None,
            pending: HashMap::new(),
            outcomes: Vec::new(),
            failure: None,
            loaded: false,
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

    /// Build a store with `open` on a new thread and run it there. `open` is
    /// tried again after a backoff while it answers `Unavailable`; any other
    /// error ends the thread. `emit` delivers each event to the app and
    /// returns false once the app is gone, which ends the thread.
    pub(crate) fn start(
        open: impl FnMut() -> StoreResult<Box<dyn TaskStore>> + Send + 'static,
        identity: TaskIdentity,
        emit: impl Fn(StoreEvent) -> bool + Send + 'static,
    ) -> std::io::Result<Self> {
        Self::start_with(open, identity, Backoff::default(), emit)
    }

    pub(crate) fn start_with(
        open: impl FnMut() -> StoreResult<Box<dyn TaskStore>> + Send + 'static,
        identity: TaskIdentity,
        backoff: Backoff,
        emit: impl Fn(StoreEvent) -> bool + Send + 'static,
    ) -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::channel();
        let (done_sender, done_receiver) = mpsc::channel();
        let watch_sender = sender.clone();
        crate::thread_spawn::spawn_named("herdr-task-store", move || {
            let result = catch_unwind(AssertUnwindSafe(|| {
                run_store(open, receiver, watch_sender, backoff, &emit)
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
        service.commands = Some(sender);
        service.finished = Some(done_receiver);
        Ok(service)
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

    /// Queue a create. Answers at once with the write's id; the outcome
    /// arrives later as a [`StoreEvent::Outcome`].
    pub(crate) fn submit_create(&mut self, task: &Task) -> Result<WriteId, SubmitError> {
        self.check_may_write(task)?;
        let (doc, write) = self.prepare(task)?;
        let key = doc.key();
        self.send(
            WriteCommand::Create {
                key: key.clone(),
                doc: Box::new(doc),
                write: write.clone(),
            },
            write,
            key,
        )
    }

    /// Queue an update of a task last seen at `expected`.
    pub(crate) fn submit_update(
        &mut self,
        task: &Task,
        expected: Rev,
    ) -> Result<WriteId, SubmitError> {
        self.check_may_write(task)?;
        if expected == Rev(0) {
            return Err(SubmitError::Refused(TaskError::BadRevision));
        }
        let (doc, write) = self.prepare(task)?;
        let key = doc.key();
        self.send(
            WriteCommand::Update {
                key: key.clone(),
                doc: Box::new(doc),
                expected,
                write: write.clone(),
            },
            write,
            key,
        )
    }

    /// Validate `task`, give it a fresh write id and record the id in its copy.
    fn prepare(&self, task: &Task) -> Result<(Task, WriteId), SubmitError> {
        task.validate().map_err(SubmitError::Refused)?;
        self.cache
            .check_pane_capacity(task)
            .map_err(SubmitError::Refused)?;
        let write = WriteId::generate();
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
        if self.pending.len() >= MAX_PENDING_WRITES {
            return Err(SubmitError::Busy);
        }
        match &self.identity {
            Some(identity) => identity
                .check_write(&task.owner_host)
                .map_err(SubmitError::Refused),
            None => Ok(()),
        }
    }

    fn send(
        &mut self,
        command: WriteCommand,
        write: WriteId,
        key: TaskKey,
    ) -> Result<WriteId, SubmitError> {
        let commands = self.commands.as_ref().ok_or(SubmitError::Disabled)?;
        if commands.send(Message::Write(command)).is_err() {
            self.mark_stopped("the task store thread is gone".to_owned());
            return Err(SubmitError::Stopped);
        }
        self.pending.insert(write.clone(), key);
        Ok(write)
    }

    fn push_outcome(&mut self, write: WriteId, outcome: WriteOutcome) {
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
        let pending: Vec<WriteId> = self.pending.drain().map(|(write, _)| write).collect();
        for write in pending {
            self.push_outcome(write, WriteOutcome::Unknown(reason.clone()));
        }
    }

    /// Take what the store thread reported. Returns whether anything a reader
    /// can see changed.
    pub(crate) fn apply(&mut self, event: StoreEvent) -> bool {
        match event {
            StoreEvent::Loaded(events) => {
                self.loaded = true;
                self.failure = None;
                for event in &events {
                    self.cache.apply(event);
                }
                true
            }
            StoreEvent::Changed(event) => self.cache.apply(&event),
            StoreEvent::Outcome { write, outcome } => {
                let known = self.pending.remove(&write).is_some();
                self.push_outcome(write, outcome);
                known
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

    /// The outcomes that arrived since the last call, oldest first.
    pub(crate) fn take_outcomes(&mut self) -> Vec<(WriteId, WriteOutcome)> {
        std::mem::take(&mut self.outcomes)
    }

    /// Stop taking writes, let the thread finish what is queued, and wait for
    /// it to let go of the store. Returns whether it did within `timeout`.
    /// For a handoff: the replacement may open the store only after this.
    pub(crate) fn quiesce(&mut self, timeout: Duration) -> bool {
        if let Some(commands) = self.commands.take() {
            let _ = commands.send(Message::Shutdown);
        }
        match self.finished.take() {
            Some(finished) => finished.recv_timeout(timeout).is_ok(),
            None => true,
        }
    }
}

impl Drop for TaskService {
    fn drop(&mut self) {
        // Do not wait for the thread: a slow store must not hold up a quit.
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
    emit: &impl Fn(StoreEvent) -> bool,
) -> Option<String> {
    // Reach the store, trying again while it is only unreachable.
    let mut wait = backoff.initial;
    let mut store = loop {
        match open() {
            Ok(store) => break store,
            Err(StoreError::Unavailable(why)) => {
                if !emit(StoreEvent::Degraded(format!(
                    "the task store is not reachable: {why}"
                ))) {
                    return None;
                }
                if pause(&messages, wait, emit) {
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
        match store.scan("") {
            Ok(found) => {
                let events = found
                    .into_iter()
                    .map(|(key, rev, value)| WatchEvent {
                        key,
                        rev,
                        value: Some(value),
                    })
                    .collect();
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
                if pause(&messages, wait, emit) {
                    return None;
                }
                wait = backoff.next(wait);
            }
        }
    }

    while let Ok(message) = messages.recv() {
        match message {
            Message::Shutdown => return None,
            Message::Watch(event) => {
                if !emit(StoreEvent::Changed(event)) {
                    return None;
                }
            }
            Message::Write(command) => {
                let (write, outcome) = run_write(store.as_mut(), command);
                if !emit(StoreEvent::Outcome { write, outcome }) {
                    return None;
                }
            }
        }
    }
    // Every sender is gone: the service was dropped.
    None
}

/// Wait `delay` for a stop. Writes that arrive meanwhile fail at once, since
/// there is nothing to write to. Returns true when the thread should end.
fn pause(
    messages: &Receiver<Message>,
    delay: Duration,
    emit: &impl Fn(StoreEvent) -> bool,
) -> bool {
    let deadline = std::time::Instant::now() + delay;
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match messages.recv_timeout(left) {
            Ok(Message::Shutdown) | Err(RecvTimeoutError::Disconnected) => return true,
            Err(RecvTimeoutError::Timeout) => return false,
            Ok(Message::Watch(_)) => {}
            Ok(Message::Write(command)) => {
                let outcome = WriteOutcome::Failed("the task store is not connected".to_owned());
                let write = match command {
                    WriteCommand::Create { write, .. } | WriteCommand::Update { write, .. } => {
                        write
                    }
                };
                if !emit(StoreEvent::Outcome { write, outcome }) {
                    return true;
                }
            }
        }
    }
}

fn run_write(store: &mut dyn TaskStore, command: WriteCommand) -> (WriteId, WriteOutcome) {
    let (write, result) = match command {
        WriteCommand::Create { key, doc, write } => {
            let result = store.create(&key, &doc, write.clone());
            (write, result)
        }
        WriteCommand::Update {
            key,
            doc,
            expected,
            write,
        } => {
            let result = store.update(&key, &doc, expected, write.clone());
            (write, result)
        }
    };
    let outcome = match result {
        Ok(rev) => WriteOutcome::Applied { rev },
        Err(StoreError::Conflict { current }) => WriteOutcome::Conflict { current },
        Err(StoreError::AlreadyExists { current }) => WriteOutcome::Exists { current },
        Err(StoreError::Unavailable(why) | StoreError::Io(why)) => WriteOutcome::Failed(why),
        Err(StoreError::Indeterminate(why)) => WriteOutcome::Unknown(why),
    };
    (write, outcome)
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
    use crate::tasks::store::StoreKey;

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
        indeterminate: bool,
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
                indeterminate: false,
                scan_failures: Arc::default(),
                docs: BTreeMap::new(),
                sinks: Vec::new(),
            }
        }

        fn put(&mut self, key: &TaskKey, doc: &Task, rev: Rev) {
            let bytes = serde_json::to_vec(doc).unwrap();
            self.docs.insert(key.to_string(), (rev, bytes.clone()));
            self.sinks.retain(|sink| {
                sink(WatchEvent {
                    key: StoreKey::from(key),
                    rev,
                    value: Some(bytes.clone()),
                })
            });
        }

        fn before_write(&self) -> StoreResult<()> {
            self.writes.fetch_add(1, Ordering::SeqCst);
            assert!(!self.panic_on_write, "the store broke");
            std::thread::sleep(self.delay);
            if self.indeterminate {
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
        fn create(&mut self, key: &TaskKey, doc: &Task, _write: WriteId) -> StoreResult<Rev> {
            self.before_write()?;
            if let Some(current) = self.docs.get(&key.to_string()) {
                return Err(StoreError::AlreadyExists {
                    current: current.clone(),
                });
            }
            self.put(key, doc, Rev(1));
            Ok(Rev(1))
        }

        fn update(
            &mut self,
            key: &TaskKey,
            doc: &Task,
            expected: Rev,
            _write: WriteId,
        ) -> StoreResult<Rev> {
            self.before_write()?;
            match self.docs.get(&key.to_string()) {
                Some((rev, _)) if *rev == expected => {
                    let next = Rev(rev.0 + 1);
                    self.put(key, doc, next);
                    Ok(next)
                }
                current => Err(StoreError::Conflict {
                    current: current.cloned(),
                }),
            }
        }

        fn get(&mut self, key: &StoreKey) -> StoreResult<Option<(Rev, Vec<u8>)>> {
            Ok(self.docs.get(&key.0).cloned())
        }

        fn scan(&mut self, prefix: &str) -> StoreResult<Vec<(StoreKey, Rev, Vec<u8>)>> {
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
                .map(|(key, (rev, bytes))| (StoreKey(key.clone()), *rev, bytes.clone()))
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
        let (events, received) = mpsc::channel();
        let events = Mutex::new(events);
        let service = TaskService::start_with(open, identity, FAST, move |event| {
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
    fn a_publish_that_may_have_landed_is_unknown_not_failed() {
        let mut store = FakeStore::new(Duration::ZERO);
        store.indeterminate = true;
        let (mut service, received) = loaded(store);
        let write = service.submit_create(&task("tk-1")).unwrap();
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.pending_count() == 0
        });
        assert_eq!(
            service.take_outcomes(),
            vec![(write, WriteOutcome::Unknown("the publish timed out".into()))]
        );
    }

    #[test]
    fn a_store_that_stops_answering_cannot_make_the_queue_grow_without_end() {
        let (mut service, _received) = loaded(FakeStore::new(Duration::from_secs(60)));
        for n in 0..MAX_PENDING_WRITES {
            service
                .submit_create(&task(&format!("tk-{n}")))
                .unwrap_or_else(|err| panic!("write {n}: {err:?}"));
        }
        let err = service.submit_create(&task("tk-over")).unwrap_err();
        assert_eq!(err, SubmitError::Busy);
        assert_eq!(err.code(), "task_store_busy");
        assert_eq!(service.pending_count(), MAX_PENDING_WRITES);
    }

    #[test]
    fn a_task_this_build_cannot_rewrite_is_refused() {
        let (mut service, _received) = loaded(FakeStore::new(Duration::ZERO));
        let mut newer = task("tk-1");
        newer.schema = 2;
        let err = service.submit_update(&newer, Rev(1)).unwrap_err();
        assert_eq!(err.code(), "task_schema_newer");
        let mut odd = task("tk-2");
        odd.status = crate::tasks::TaskStatus::Unknown;
        assert_eq!(
            service.submit_update(&odd, Rev(1)).unwrap_err().code(),
            "task_schema_newer"
        );
        assert_eq!(service.pending_count(), 0);
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
    fn an_unreachable_store_is_retried_and_writes_meanwhile_fail_fast() {
        let tries = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&tries);
        let mut store = Some(FakeStore::new(Duration::ZERO));
        let (mut service, received) = start_with_opener(
            move || {
                if counted.fetch_add(1, Ordering::SeqCst) < 3 {
                    return Err(StoreError::Unavailable("the network is not up".into()));
                }
                Ok(Box::new(store.take().unwrap()) as Box<dyn TaskStore>)
            },
            test_identity("m4m"),
        );

        // While it cannot connect the service says why, and refuses to pretend.
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.failure().is_some()
        });
        assert!(service.failure().unwrap().contains("not reachable"));
        let write = service.submit_create(&task("tk-1")).unwrap();
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.pending_count() == 0
        });
        let outcomes = service.take_outcomes();
        assert_eq!(outcomes[0].0, write);
        assert!(
            matches!(&outcomes[0].1, WriteOutcome::Failed(why) if why.contains("not connected")),
            "{outcomes:?}"
        );

        // Once the store answers, it loads and the failure clears.
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.is_loaded()
        });
        assert_eq!(service.failure(), None);
        assert!(tries.load(Ordering::SeqCst) >= 4);
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
