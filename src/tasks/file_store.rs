//! The `file` backend: one JSON file per key under a directory the caller
//! owns, each with its own revision counter.
//!
//! Only the store task writes it, and each named session has its own data dir,
//! so no two processes share one. A write goes to a temp file that is synced
//! and then renamed over the key's file, so a crash leaves the old document or
//! the new one, never half of one.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};

use serde::{Deserialize, Serialize};

use super::ids::{Rev, TaskKey, WriteId};
use super::model::Task;
use super::store::{StoreError, StoreKey, StoreResult, TaskStore, WatchEvent, WatchStream};

const EXTENSION: &str = "json";

/// What a key's file holds: the revision and the document.
#[derive(Serialize, Deserialize)]
struct Stored {
    rev: u64,
    value: serde_json::Value,
}

pub(crate) struct FileStore {
    dir: PathBuf,
    watchers: Vec<(String, Sender<WatchEvent>)>,
}

impl FileStore {
    /// Open the store in `dir`, creating it if needed.
    pub(crate) fn open(dir: PathBuf) -> StoreResult<Self> {
        fs::create_dir_all(&dir).map_err(io_error)?;
        Ok(Self {
            dir,
            watchers: Vec::new(),
        })
    }

    /// The file of a key, or `None` for a key that is not a safe file name.
    fn path_for(&self, key: &str) -> Option<PathBuf> {
        let safe = !key.is_empty()
            && !key.starts_with('.')
            && key.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'=')
            });
        safe.then(|| self.dir.join(format!("{key}.{EXTENSION}")))
    }

    fn read(&self, key: &str) -> StoreResult<Option<(Rev, Vec<u8>)>> {
        let Some(path) = self.path_for(key) else {
            return Ok(None);
        };
        read_file(&path)
    }

    fn write(&mut self, key: &str, rev: Rev, value: &Task) -> StoreResult<()> {
        let path = self
            .path_for(key)
            .ok_or_else(|| StoreError::Io(format!("not a usable key: {key}")))?;
        let value = serde_json::to_value(value).map_err(|err| StoreError::Io(err.to_string()))?;
        let bytes = serde_json::to_vec(&Stored { rev: rev.0, value })
            .map_err(|err| StoreError::Io(err.to_string()))?;
        let temp = path.with_extension(format!("{EXTENSION}.tmp"));
        let mut file = fs::File::create(&temp).map_err(io_error)?;
        file.write_all(&bytes).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        drop(file);
        fs::rename(&temp, &path).map_err(io_error)?;
        self.notify(key, rev);
        Ok(())
    }

    /// Tell every watcher whose prefix matches. A watcher that hung up is
    /// dropped.
    fn notify(&mut self, key: &str, rev: Rev) {
        let Ok(Some((_, value))) = self.read(key) else {
            return;
        };
        self.watchers.retain(|(prefix, sender)| {
            if !key.starts_with(prefix.as_str()) {
                return true;
            }
            sender
                .send(WatchEvent {
                    key: StoreKey(key.to_owned()),
                    rev,
                    value: Some(value.clone()),
                })
                .is_ok()
        });
    }

    fn landed_before(current: &[u8], write: &WriteId) -> bool {
        serde_json::from_slice::<Task>(current).is_ok_and(|task| task.last_writes.contains(write))
    }
}

fn io_error(err: std::io::Error) -> StoreError {
    StoreError::Io(err.to_string())
}

fn read_file(path: &Path) -> StoreResult<Option<(Rev, Vec<u8>)>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(io_error(err)),
    };
    let stored: Stored = serde_json::from_slice(&bytes)
        .map_err(|err| StoreError::Io(format!("{}: {err}", path.display())))?;
    let value = serde_json::to_vec(&stored.value).map_err(|err| StoreError::Io(err.to_string()))?;
    Ok(Some((Rev(stored.rev), value)))
}

impl TaskStore for FileStore {
    fn create(&mut self, key: &TaskKey, doc: &Task, write: WriteId) -> StoreResult<Rev> {
        let name = key.to_string();
        if let Some((rev, current)) = self.read(&name)? {
            return if Self::landed_before(&current, &write) {
                Ok(rev)
            } else {
                Err(StoreError::AlreadyExists {
                    current: (rev, current),
                })
            };
        }
        let rev = Rev(1);
        self.write(&name, rev, doc)?;
        Ok(rev)
    }

    fn update(
        &mut self,
        key: &TaskKey,
        doc: &Task,
        expected: Rev,
        write: WriteId,
    ) -> StoreResult<Rev> {
        let name = key.to_string();
        let Some((rev, current)) = self.read(&name)? else {
            return Err(StoreError::Conflict { current: None });
        };
        if rev != expected {
            return if Self::landed_before(&current, &write) {
                Ok(rev)
            } else {
                Err(StoreError::Conflict {
                    current: Some((rev, current)),
                })
            };
        }
        let next = Rev(rev.0 + 1);
        self.write(&name, next, doc)?;
        Ok(next)
    }

    fn get(&mut self, key: &StoreKey) -> StoreResult<Option<(Rev, Vec<u8>)>> {
        self.read(&key.0)
    }

    fn scan(&mut self, prefix: &str) -> StoreResult<Vec<(StoreKey, Rev, Vec<u8>)>> {
        let mut found = Vec::new();
        for entry in fs::read_dir(&self.dir).map_err(io_error)? {
            let path = entry.map_err(io_error)?.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some(EXTENSION) {
                continue;
            }
            let Some(key) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if !key.starts_with(prefix) {
                continue;
            }
            match read_file(&path) {
                Ok(Some((rev, value))) => found.push((StoreKey(key.to_owned()), rev, value)),
                Ok(None) => {}
                // One damaged file must not hide the rest.
                Err(err) => tracing::warn!(key, error = ?err, "skipping an unreadable task file"),
            }
        }
        found.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(found)
    }

    fn watch(&mut self, prefix: &str) -> StoreResult<WatchStream> {
        let (sender, receiver) = mpsc::channel();
        self.watchers.push((prefix.to_owned(), sender));
        Ok(receiver)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::ids::{HostId, TaskId};
    use crate::tasks::model::{Origin, Timestamp, Via};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "herdr-task-store-{}-{}",
                std::process::id(),
                crate::ulid::new_ulid()
            ));
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
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

    fn with_write(mut task: Task, write: &WriteId) -> Task {
        task.record_write(write.clone(), |_| false);
        task
    }

    #[test]
    fn file_store_compare_and_set_conflicts() {
        let scratch = Scratch::new();
        let mut store = FileStore::open(scratch.0.clone()).unwrap();
        let one = task("tk-1");
        let key = one.key();

        // Create starts the key at revision 1; a second create is refused.
        let w1 = WriteId::generate();
        let first = with_write(one.clone(), &w1);
        assert_eq!(store.create(&key, &first, w1.clone()), Ok(Rev(1)));
        let other = WriteId::generate();
        match store.create(&key, &with_write(one.clone(), &other), other) {
            Err(StoreError::AlreadyExists {
                current: (Rev(1), _),
            }) => {}
            result => panic!("expected AlreadyExists, got {result:?}"),
        }

        // An update at the revision it saw moves it on by one.
        let w2 = WriteId::generate();
        let mut second = with_write(first.clone(), &w2);
        second.title = "edited".into();
        assert_eq!(store.update(&key, &second, Rev(1), w2.clone()), Ok(Rev(2)));

        // A stale update is a conflict that carries what is stored now.
        let w3 = WriteId::generate();
        let mut stale = with_write(first.clone(), &w3);
        stale.title = "mine".into();
        match store.update(&key, &stale, Rev(1), w3) {
            Err(StoreError::Conflict {
                current: Some((Rev(2), bytes)),
            }) => {
                let theirs: Task = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(theirs.title, "edited");
            }
            result => panic!("expected Conflict, got {result:?}"),
        }
        let stored = store.get(&StoreKey::from(&key)).unwrap().unwrap();
        assert_eq!(stored.0, Rev(2), "a refused write changes nothing");

        // An update of a key that does not exist is a conflict with no current.
        let missing = task("tk-missing");
        let w4 = WriteId::generate();
        assert_eq!(
            store.update(&missing.key(), &missing, Rev(1), w4),
            Err(StoreError::Conflict { current: None })
        );
    }

    #[test]
    fn a_replayed_write_that_already_landed_is_answered_as_success() {
        let scratch = Scratch::new();
        let mut store = FileStore::open(scratch.0.clone()).unwrap();
        let w1 = WriteId::generate();
        let created = with_write(task("tk-1"), &w1);
        let key = created.key();
        assert_eq!(store.create(&key, &created, w1.clone()), Ok(Rev(1)));
        // The acknowledgement was lost; the same create comes again.
        assert_eq!(store.create(&key, &created, w1), Ok(Rev(1)));

        let w2 = WriteId::generate();
        let edited = with_write(created, &w2);
        assert_eq!(store.update(&key, &edited, Rev(1), w2.clone()), Ok(Rev(2)));
        // And the same update again, still holding the old expected revision.
        assert_eq!(store.update(&key, &edited, Rev(1), w2), Ok(Rev(2)));
        assert_eq!(store.get(&StoreKey::from(&key)).unwrap().unwrap().0, Rev(2));
    }

    #[test]
    fn documents_and_revisions_survive_reopening_and_scan_by_prefix() {
        let scratch = Scratch::new();
        let (a, b) = (task("tk-a"), task("tk-b"));
        {
            let mut store = FileStore::open(scratch.0.clone()).unwrap();
            for task in [&a, &b] {
                let write = WriteId::generate();
                store
                    .create(&task.key(), &with_write(task.clone(), &write), write)
                    .unwrap();
            }
        }
        let mut store = FileStore::open(scratch.0.clone()).unwrap();
        let all = store.scan("task.").unwrap();
        let keys: Vec<&str> = all.iter().map(|(key, _, _)| key.0.as_str()).collect();
        assert_eq!(keys, ["task.m4m.tk-a", "task.m4m.tk-b"]);
        assert!(all.iter().all(|(_, rev, _)| *rev == Rev(1)));
        assert!(store.scan("run.").unwrap().is_empty());
        assert_eq!(store.get(&StoreKey("nope".into())).unwrap(), None);
        assert_eq!(store.get(&StoreKey("../escape".into())).unwrap(), None);
        // No temp file is left behind by a write.
        let leftovers = fs::read_dir(&scratch.0)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "tmp"))
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn a_watcher_sees_writes_under_its_prefix_only() {
        let scratch = Scratch::new();
        let mut store = FileStore::open(scratch.0.clone()).unwrap();
        let tasks = store.watch("task.").unwrap();
        let runs = store.watch("run.").unwrap();
        let one = task("tk-1");
        let write = WriteId::generate();
        store
            .create(&one.key(), &with_write(one.clone(), &write), write)
            .unwrap();

        let event = tasks.try_recv().expect("the task watcher is told");
        assert_eq!(event.key.0, "task.m4m.tk-1");
        assert_eq!(event.rev, Rev(1));
        assert!(event.value.is_some());
        assert!(runs.try_recv().is_err(), "the run watcher hears nothing");
    }

    #[test]
    fn a_damaged_file_does_not_hide_the_others() {
        let scratch = Scratch::new();
        let mut store = FileStore::open(scratch.0.clone()).unwrap();
        let one = task("tk-1");
        let write = WriteId::generate();
        store
            .create(&one.key(), &with_write(one.clone(), &write), write)
            .unwrap();
        fs::write(scratch.0.join("task.m4m.tk-broken.json"), b"{nope").unwrap();
        let found = store.scan("task.").unwrap();
        assert_eq!(found.len(), 1);
        assert!(store.get(&StoreKey("task.m4m.tk-broken".into())).is_err());
    }
}
