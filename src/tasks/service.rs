//! The store task and the server-side handle to it.
//!
//! A backend blocks, so one dedicated thread owns it and does every call. It
//! reports back as [`StoreEvent`]s, which the app drains from its event
//! channel before it handles the next API request. The API side only reads
//! the cache and queues commands: it never waits on the store.

use std::collections::HashMap;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

use super::cache::TaskCache;
use super::ids::{Rev, TaskKey, WriteId};
use super::model::{Task, TaskError};
use super::store::{StoreError, TaskStore, WatchEvent};

/// How often the store thread looks at its watch while no command arrives.
const WATCH_POLL: Duration = Duration::from_millis(25);

/// What the app asks the store thread to do.
enum StoreCommand {
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
    Shutdown,
}

/// What the store thread reports.
#[derive(Debug)]
pub(crate) enum StoreEvent {
    /// Everything the store held at start.
    Loaded(Vec<WatchEvent>),
    /// A change seen by the watch.
    Changed(WatchEvent),
    /// How a write ended.
    Outcome {
        write: WriteId,
        outcome: WriteOutcome,
    },
    /// The store failed in a way no write explains (for example, at start).
    Failed(String),
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
    /// The store could not be reached or failed; the write is not applied.
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SubmitError {
    /// No store runs: tasks are off in this server.
    Disabled,
    /// The store thread is gone.
    Stopped,
    Refused(TaskError),
}

impl SubmitError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Disabled => "tasks_disabled",
            Self::Stopped => "task_store_stopped",
            Self::Refused(err) => err.code(),
        }
    }
}

pub(crate) struct TaskService {
    cache: TaskCache,
    commands: Option<Sender<StoreCommand>>,
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
            cache: TaskCache::new(),
            commands: None,
            pending: HashMap::new(),
            outcomes: Vec::new(),
            failure: None,
            loaded: false,
        }
    }

    /// Run `store` on its own thread. `emit` delivers each event to the app
    /// and returns false once the app is gone, which ends the thread.
    pub(crate) fn start(
        store: Box<dyn TaskStore>,
        emit: impl Fn(StoreEvent) -> bool + Send + 'static,
    ) -> std::io::Result<Self> {
        let (commands, receiver) = mpsc::channel();
        crate::thread_spawn::spawn_named("herdr-task-store", move || {
            run_store(store, receiver, emit);
        })?;
        let mut service = Self::disabled();
        service.commands = Some(commands);
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
        task.validate().map_err(SubmitError::Refused)?;
        self.cache
            .check_pane_capacity(task)
            .map_err(SubmitError::Refused)?;
        let write = WriteId::generate();
        let mut doc = task.clone();
        doc.record_write(write.clone(), |id| self.pending.contains_key(id));
        let key = doc.key();
        self.send(
            StoreCommand::Create {
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
        task.validate().map_err(SubmitError::Refused)?;
        self.cache
            .check_pane_capacity(task)
            .map_err(SubmitError::Refused)?;
        let write = WriteId::generate();
        let mut doc = task.clone();
        doc.record_write(write.clone(), |id| self.pending.contains_key(id));
        let key = doc.key();
        self.send(
            StoreCommand::Update {
                key: key.clone(),
                doc: Box::new(doc),
                expected,
                write: write.clone(),
            },
            write,
            key,
        )
    }

    fn send(
        &mut self,
        command: StoreCommand,
        write: WriteId,
        key: TaskKey,
    ) -> Result<WriteId, SubmitError> {
        let commands = self.commands.as_ref().ok_or(SubmitError::Disabled)?;
        if commands.send(command).is_err() {
            self.commands = None;
            return Err(SubmitError::Stopped);
        }
        self.pending.insert(write.clone(), key);
        Ok(write)
    }

    /// Take what the store thread reported. Returns whether the cache or the
    /// pending set changed.
    pub(crate) fn apply(&mut self, event: StoreEvent) -> bool {
        match event {
            StoreEvent::Loaded(events) => {
                self.loaded = true;
                let mut changed = true;
                for event in &events {
                    changed |= self.cache.apply(event);
                }
                changed
            }
            StoreEvent::Changed(event) => self.cache.apply(&event),
            StoreEvent::Outcome { write, outcome } => {
                let known = self.pending.remove(&write).is_some();
                self.outcomes.push((write, outcome));
                known
            }
            StoreEvent::Failed(message) => {
                self.failure = Some(message);
                true
            }
        }
    }

    /// The outcomes that arrived since the last call, oldest first.
    pub(crate) fn take_outcomes(&mut self) -> Vec<(WriteId, WriteOutcome)> {
        std::mem::take(&mut self.outcomes)
    }
}

impl Drop for TaskService {
    fn drop(&mut self) {
        // Do not wait for the thread: a slow store must not hold up a quit.
        if let Some(commands) = &self.commands {
            let _ = commands.send(StoreCommand::Shutdown);
        }
    }
}

fn run_store(
    mut store: Box<dyn TaskStore>,
    commands: mpsc::Receiver<StoreCommand>,
    emit: impl Fn(StoreEvent) -> bool,
) {
    let watch = match store.watch("") {
        Ok(watch) => watch,
        Err(err) => {
            emit(StoreEvent::Failed(format!("watch failed: {err:?}")));
            return;
        }
    };
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
                return;
            }
        }
        Err(err) => {
            if !emit(StoreEvent::Failed(format!("scan failed: {err:?}"))) {
                return;
            }
        }
    }
    loop {
        while let Ok(event) = watch.try_recv() {
            if !emit(StoreEvent::Changed(event)) {
                return;
            }
        }
        match commands.recv_timeout(WATCH_POLL) {
            Ok(StoreCommand::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
            Ok(command) => {
                let (write, outcome) = run_command(store.as_mut(), command);
                if !emit(StoreEvent::Outcome { write, outcome }) {
                    return;
                }
            }
        }
    }
}

fn run_command(store: &mut dyn TaskStore, command: StoreCommand) -> (WriteId, WriteOutcome) {
    let (write, result) = match command {
        StoreCommand::Create { key, doc, write } => {
            let result = store.create(&key, &doc, write.clone());
            (write, result)
        }
        StoreCommand::Update {
            key,
            doc,
            expected,
            write,
        } => {
            let result = store.update(&key, &doc, expected, write.clone());
            (write, result)
        }
        StoreCommand::Shutdown => unreachable!("shutdown is handled by the loop"),
    };
    let outcome = match result {
        Ok(rev) => WriteOutcome::Applied { rev },
        Err(StoreError::Conflict { current }) => WriteOutcome::Conflict { current },
        Err(StoreError::AlreadyExists { current }) => WriteOutcome::Exists { current },
        Err(StoreError::Unavailable(why) | StoreError::Io(why)) => WriteOutcome::Failed(why),
    };
    (write, outcome)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::mpsc::Receiver;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use super::*;
    use crate::tasks::ids::{HostId, TaskId};
    use crate::tasks::model::{Origin, Timestamp, Via};
    use crate::tasks::store::{StoreKey, StoreResult, WatchStream};

    /// An in-memory store whose writes take `delay`.
    struct SlowStore {
        delay: Duration,
        docs: BTreeMap<String, (Rev, Vec<u8>)>,
        watchers: Vec<Sender<WatchEvent>>,
    }

    impl SlowStore {
        fn new(delay: Duration) -> Self {
            Self {
                delay,
                docs: BTreeMap::new(),
                watchers: Vec::new(),
            }
        }

        fn put(&mut self, key: &TaskKey, doc: &Task, rev: Rev) {
            let bytes = serde_json::to_vec(doc).unwrap();
            self.docs.insert(key.to_string(), (rev, bytes.clone()));
            for watcher in &self.watchers {
                let _ = watcher.send(WatchEvent {
                    key: StoreKey::from(key),
                    rev,
                    value: Some(bytes.clone()),
                });
            }
        }
    }

    impl TaskStore for SlowStore {
        fn create(&mut self, key: &TaskKey, doc: &Task, _write: WriteId) -> StoreResult<Rev> {
            std::thread::sleep(self.delay);
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
            std::thread::sleep(self.delay);
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
            Ok(self
                .docs
                .iter()
                .filter(|(key, _)| key.starts_with(prefix))
                .map(|(key, (rev, bytes))| (StoreKey(key.clone()), *rev, bytes.clone()))
                .collect())
        }

        fn watch(&mut self, _prefix: &str) -> StoreResult<WatchStream> {
            let (sender, receiver) = mpsc::channel();
            self.watchers.push(sender);
            Ok(receiver)
        }
    }

    fn task(id: &str) -> Task {
        Task::new(
            HostId::new("m4m").unwrap(),
            TaskId::new(id).unwrap(),
            "a task",
            Origin {
                host: "m4m".into(),
                by: "x".into(),
                via: Via::Cli,
            },
            Timestamp("2026-10-21T20:00:00Z".into()),
        )
        .unwrap()
    }

    fn start(delay: Duration) -> (TaskService, Receiver<StoreEvent>) {
        let (events, received) = mpsc::channel();
        let events = Mutex::new(events);
        let service = TaskService::start(Box::new(SlowStore::new(delay)), move |event| {
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

    #[test]
    fn slow_store_never_blocks_api() {
        let (mut service, received) = start(Duration::from_millis(1500));
        // Wait for the initial load, so the cache is ready.
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.is_loaded()
        });

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
        let outcomes = service.take_outcomes();
        assert_eq!(
            outcomes,
            vec![(write, WriteOutcome::Applied { rev: Rev(1) })]
        );
        let cached = service.cache().get(&one.key()).unwrap();
        assert_eq!(cached.rev, Rev(1));
        assert_eq!(cached.task.title, "a task");
    }

    #[test]
    fn a_stale_update_comes_back_as_a_conflict_not_an_error() {
        let (mut service, received) = start(Duration::ZERO);
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

        let (mut service, _received) = start(Duration::ZERO);
        let mut long = task("tk-1");
        long.title = "x".repeat(201);
        let err = service.submit_create(&long).unwrap_err();
        assert_eq!(err.code(), "task_title_too_long");
        assert_eq!(service.pending_count(), 0);
    }

    #[test]
    fn the_initial_load_fills_the_cache() {
        let mut store = SlowStore::new(Duration::ZERO);
        let one = task("tk-1");
        store.put(&one.key(), &one, Rev(4));
        let (events, received) = mpsc::channel();
        let events = Mutex::new(events);
        let mut service = TaskService::start(Box::new(store), move |event| {
            events.lock().unwrap().send(event).is_ok()
        })
        .unwrap();
        pump(&mut service, &received, Duration::from_secs(5), |service| {
            service.cache().len() == 1
        });
        assert_eq!(service.cache().get(&one.key()).unwrap().rev, Rev(4));
    }

    #[test]
    fn dropping_the_service_stops_the_thread_without_waiting_for_it() {
        let (events, received) = mpsc::channel();
        let events = Mutex::new(events);
        let service = TaskService::start(
            Box::new(SlowStore::new(Duration::from_secs(30))),
            move |event| events.lock().unwrap().send(event).is_ok(),
        )
        .unwrap();
        let started = Instant::now();
        drop(service);
        assert!(started.elapsed() < Duration::from_millis(250));
        drop(received);
    }
}
