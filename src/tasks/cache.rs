//! The server's view of the store: plain data, fed by scans and watches.

use std::collections::BTreeMap;

use super::ids::{Rev, TaskKey};
use super::model::{Task, TaskError, TaskStatus, OPEN_TASKS_PER_PANE};
use super::store::{StoreKey, WatchEvent};

/// How many refused documents the cache remembers; the oldest go first.
const REJECTED_KEPT: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CachedTask {
    pub rev: Rev,
    pub task: Task,
}

#[derive(Debug, Default)]
pub(crate) struct TaskCache {
    tasks: BTreeMap<TaskKey, CachedTask>,
    /// Bumped on every change, so a reader can tell its copy is stale.
    revision: u64,
    /// Documents that were refused, with why. Kept for `herdr config check`
    /// style reports; a forged or damaged document never reaches a reader.
    rejected: Vec<(StoreKey, String)>,
}

impl TaskCache {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn rejected(&self) -> &[(StoreKey, String)] {
        &self.rejected
    }

    pub(crate) fn get(&self, key: &TaskKey) -> Option<&CachedTask> {
        self.tasks.get(key)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&TaskKey, &CachedTask)> {
        self.tasks.iter()
    }

    pub(crate) fn len(&self) -> usize {
        self.tasks.len()
    }

    /// Apply one change from a watch or a scan. Returns whether the cache
    /// changed. A key that is not a task key is not this cache's business. A
    /// document that does not match its key, or fails a limit, is refused. A
    /// revision older than the cached one is ignored.
    pub(crate) fn apply(&mut self, event: &WatchEvent) -> bool {
        let Ok(key) = event.key.0.parse::<TaskKey>() else {
            return false;
        };
        let Some(bytes) = &event.value else {
            let removed = self.tasks.remove(&key).is_some();
            self.revision += u64::from(removed);
            return removed;
        };
        let task = match serde_json::from_slice::<Task>(bytes) {
            Ok(task) => task,
            Err(err) => return self.reject(&event.key, format!("unreadable: {err}")),
        };
        if task.key() != key {
            return self.reject(&event.key, "the document names another key".into());
        }
        if let Err(err) = task.validate() {
            return self.reject(&event.key, format!("breaks a limit: {}", err.code()));
        }
        if self
            .tasks
            .get(&key)
            .is_some_and(|cached| cached.rev >= event.rev)
        {
            return false;
        }
        self.tasks.insert(
            key,
            CachedTask {
                rev: event.rev,
                task,
            },
        );
        self.revision += 1;
        true
    }

    fn reject(&mut self, key: &StoreKey, why: String) -> bool {
        self.rejected.push((key.clone(), why));
        if self.rejected.len() > REJECTED_KEPT {
            let excess = self.rejected.len() - REJECTED_KEPT;
            self.rejected.drain(..excess);
        }
        false
    }

    /// Tasks bound to the pane `(host, pane_key)` that are not closed.
    pub(crate) fn open_count_for_pane(&self, host: &str, pane_key: &str) -> usize {
        self.tasks
            .values()
            .filter(|cached| {
                cached.task.owner_host.as_str() == host
                    && !cached.task.status.is_closed()
                    && cached.task.pane.as_ref().is_some_and(|pane| {
                        pane.host == host && pane.pane_key.as_deref() == Some(pane_key)
                    })
            })
            .count()
    }

    /// Refuse a task whose pane already holds [`OPEN_TASKS_PER_PANE`] other
    /// open tasks. Closed tasks and tasks with no pane are never refused.
    pub(crate) fn check_pane_capacity(&self, task: &Task) -> Result<(), TaskError> {
        let Some(pane) = task.pane.as_ref() else {
            return Ok(());
        };
        let Some(pane_key) = pane.pane_key.as_deref() else {
            return Ok(());
        };
        if task.status == TaskStatus::Done || task.status == TaskStatus::Dropped {
            return Ok(());
        }
        let others = self
            .tasks
            .iter()
            .filter(|(key, cached)| {
                **key != task.key()
                    && cached.task.owner_host == task.owner_host
                    && !cached.task.status.is_closed()
                    && cached
                        .task
                        .pane
                        .as_ref()
                        .is_some_and(|other| other.pane_key.as_deref() == Some(pane_key))
            })
            .count();
        if others >= OPEN_TASKS_PER_PANE {
            return Err(TaskError::PaneFull);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::ids::{HostId, TaskId, WriteId};
    use crate::tasks::model::{Origin, PaneBinding, Timestamp, Via};

    fn task(host: &str, id: &str) -> Task {
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

    fn event(task: &Task, rev: u64) -> WatchEvent {
        WatchEvent {
            key: StoreKey::from(&task.key()),
            rev: Rev(rev),
            value: Some(serde_json::to_vec(task).unwrap()),
            at: None,
        }
    }

    fn bound(mut task: Task, pane_key: &str) -> Task {
        task.pane = Some(PaneBinding {
            host: task.owner_host.to_string(),
            session: "main".into(),
            pane_key: Some(pane_key.into()),
            label: "pane".into(),
        });
        task
    }

    #[test]
    fn the_cache_follows_a_watch_and_ignores_old_revisions() {
        let mut cache = TaskCache::new();
        let mut one = task("m4m", "tk-1");
        assert!(cache.apply(&event(&one, 3)));
        assert_eq!(cache.revision(), 1);
        one.title = "newer".into();
        assert!(cache.apply(&event(&one, 4)));
        one.title = "older".into();
        assert!(!cache.apply(&event(&one, 2)), "an old revision is ignored");
        assert!(
            !cache.apply(&event(&one, 4)),
            "the same revision is ignored"
        );
        assert_eq!(cache.get(&one.key()).unwrap().task.title, "newer");
        assert_eq!(cache.revision(), 2);

        let gone = WatchEvent {
            key: StoreKey::from(&one.key()),
            rev: Rev(5),
            value: None,
            at: None,
        };
        assert!(cache.apply(&gone));
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn the_cache_refuses_forged_and_broken_documents() {
        let mut cache = TaskCache::new();
        let one = task("m4m", "tk-1");

        // A document filed under another host's key.
        let mut forged = event(&one, 1);
        forged.key = StoreKey("task.ac-mbm5.tk-1".into());
        assert!(!cache.apply(&forged));

        // Garbage, and a record that breaks a limit.
        let garbage = WatchEvent {
            key: StoreKey::from(&one.key()),
            rev: Rev(1),
            value: Some(b"{not json".to_vec()),
            at: None,
        };
        assert!(!cache.apply(&garbage));
        let mut long = one.clone();
        long.title = "x".repeat(201);
        assert!(!cache.apply(&event(&long, 1)));

        // A run record is not a task.
        let run = WatchEvent {
            key: StoreKey("run.worklink.rn-1".into()),
            rev: Rev(1),
            value: Some(b"{}".to_vec()),
            at: None,
        };
        assert!(!cache.apply(&run));

        assert_eq!(cache.len(), 0);
        assert_eq!(cache.rejected().len(), 3);
    }

    #[test]
    fn a_document_that_is_too_big_for_a_writer_is_refused() {
        let mut cache = TaskCache::new();
        let mut big = task("m4m", "tk-1");
        big.runs = (0..21).map(|n| format!("rn-{n}")).collect();
        assert!(!cache.apply(&event(&big, 1)));
        let mut big = task("m4m", "tk-2");
        big.last_writes = (0..65)
            .map(|n| WriteId::from_string(format!("w{n}")))
            .collect();
        assert!(!cache.apply(&event(&big, 1)));
        let mut big = task("m4m", "tk-3");
        big.review = Some("x".repeat(201));
        assert!(!cache.apply(&event(&big, 1)));
        let mut big = task("m4m", "tk-4");
        big.pane = Some(PaneBinding {
            host: "m4m".into(),
            session: "main".into(),
            pane_key: None,
            label: "l".repeat(201),
        });
        assert!(!cache.apply(&event(&big, 1)));
        assert_eq!(cache.len(), 0);
        assert!(cache
            .rejected()
            .iter()
            .all(|(_, why)| why.contains("task_record_too_large")));
    }

    #[test]
    fn a_newer_writers_document_is_shown_but_not_rewritable() {
        let mut cache = TaskCache::new();
        let one = task("m4m", "tk-1");
        let mut json = serde_json::to_value(&one).unwrap();
        json["schema"] = 2.into();
        json["status"] = "paused".into();
        json["priority"] = "urgent".into();
        json["origin"]["via"] = "voice".into();
        json["owner"] = serde_json::json!({ "kind": "robot", "name": "x" });
        let newer = WatchEvent {
            key: StoreKey::from(&one.key()),
            rev: Rev(3),
            value: Some(serde_json::to_vec(&json).unwrap()),
            at: None,
        };
        assert!(cache.apply(&newer), "{:?}", cache.rejected());
        let shown = &cache.get(&one.key()).unwrap().task;
        assert_eq!(shown.status, TaskStatus::Unknown);
        assert_eq!(shown.priority, crate::tasks::Priority::Unknown);
        assert_eq!(shown.owner, crate::tasks::Owner::Unknown);
        assert_eq!(shown.title, "a task", "what it can read is kept");
        assert!(!shown.can_be_rewritten());
        assert!(one.can_be_rewritten());
    }

    #[test]
    fn the_list_of_refused_documents_does_not_grow_without_end() {
        let mut cache = TaskCache::new();
        for n in 0..(REJECTED_KEPT + 50) {
            let garbage = WatchEvent {
                key: StoreKey(format!("task.m4m.tk-{n}")),
                rev: Rev(1),
                value: Some(b"{".to_vec()),
                at: None,
            };
            assert!(!cache.apply(&garbage));
        }
        assert_eq!(cache.rejected().len(), REJECTED_KEPT);
        assert_eq!(
            cache.rejected().last().unwrap().0 .0,
            format!("task.m4m.tk-{}", REJECTED_KEPT + 49)
        );
    }

    #[test]
    fn at_most_50_open_tasks_sit_on_one_pane() {
        let mut cache = TaskCache::new();
        for n in 0..OPEN_TASKS_PER_PANE {
            let task = bound(task("m4m", &format!("tk-{n}")), "pane-a");
            assert!(cache.apply(&event(&task, 1)));
        }
        assert_eq!(cache.open_count_for_pane("m4m", "pane-a"), 50);

        let extra = bound(task("m4m", "tk-extra"), "pane-a");
        assert_eq!(cache.check_pane_capacity(&extra), Err(TaskError::PaneFull));
        // Another pane, and a task with no pane, are not limited.
        assert_eq!(
            cache.check_pane_capacity(&bound(task("m4m", "tk-b"), "pane-b")),
            Ok(())
        );
        assert_eq!(cache.check_pane_capacity(&task("m4m", "tk-c")), Ok(()));
        // An existing task being edited does not count against itself.
        let existing = bound(task("m4m", "tk-0"), "pane-a");
        assert_eq!(cache.check_pane_capacity(&existing), Ok(()));
        // A closed task frees a place.
        let mut done = bound(task("m4m", "tk-1"), "pane-a");
        done.status = TaskStatus::Done;
        assert!(cache.apply(&event(&done, 2)));
        assert_eq!(cache.open_count_for_pane("m4m", "pane-a"), 49);
        assert_eq!(cache.check_pane_capacity(&extra), Ok(()));
    }
}
