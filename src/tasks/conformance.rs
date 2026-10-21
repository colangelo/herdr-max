//! The behaviour every backend must have, run against the file backend always
//! and against the kv backend when it is built (feature `tasks-kv`).
//!
//! Revisions are only compared with each other, never with fixed numbers: the
//! file backend counts per key, the bus counts per bucket.

use std::time::Duration;

use super::ids::{HostId, Rev, TaskId, WriteId};
use super::model::{Origin, Task, Timestamp, Via};
use super::store::{StoreError, StoreKey, TaskStore};

fn task(id: &str, title: &str) -> Task {
    let mut task = Task::new(
        HostId::new("m4m").unwrap(),
        TaskId::new(id).unwrap(),
        title,
        Origin {
            host: "m4m".into(),
            by: "conformance".into(),
            via: Via::Cli,
        },
        Timestamp("2026-10-21T20:00:00Z".into()),
    )
    .unwrap();
    task.last_writes.clear();
    task
}

fn with_write(mut task: Task, write: &WriteId) -> Task {
    task.record_write(write.clone(), |_| false);
    task
}

fn title_of(bytes: &[u8]) -> String {
    serde_json::from_slice::<Task>(bytes).unwrap().title
}

/// Run every case on `store`, whose keys under `task.` start out empty.
pub(super) fn run(store: &mut dyn TaskStore) {
    let (sender, watch) = std::sync::mpsc::channel();
    store
        .watch("task.", Box::new(move |event| sender.send(event).is_ok()))
        .expect("a watch starts");

    // Create, read back, and refuse a second create by another write.
    let one = task("tk-conf-1", "first");
    let key = one.key();
    let w1 = WriteId::generate();
    let created = with_write(one.clone(), &w1);
    let rev1 = store.create(&key, &created, w1.clone()).expect("create");
    let (read_rev, bytes) = store
        .get(&StoreKey::from(&key))
        .unwrap()
        .expect("the key is there");
    assert_eq!(read_rev, rev1);
    assert_eq!(title_of(&bytes), "first");
    let other = WriteId::generate();
    match store.create(&key, &with_write(one.clone(), &other), other) {
        Err(StoreError::AlreadyExists { current: (rev, _) }) => assert_eq!(rev, rev1),
        result => panic!("a second create must be refused: {result:?}"),
    }

    // A replayed create whose acknowledgement was lost has succeeded already.
    assert_eq!(store.create(&key, &created, w1), Ok(rev1));

    // An update at the revision it saw moves the key on.
    let w2 = WriteId::generate();
    let mut edited = with_write(created.clone(), &w2);
    edited.title = "edited".into();
    let rev2 = store
        .update(&key, &edited, rev1, w2.clone())
        .expect("update");
    assert!(rev2 > rev1);

    // The same update again, after it landed, is answered as success.
    assert_eq!(store.update(&key, &edited, rev1, w2), Ok(rev2));

    // A stale edit by another write is a conflict that carries the stored one.
    let w3 = WriteId::generate();
    let mut stale = with_write(created, &w3);
    stale.title = "mine".into();
    match store.update(&key, &stale, rev1, w3) {
        Err(StoreError::Conflict {
            current: Some((rev, bytes)),
        }) => {
            assert_eq!(rev, rev2);
            assert_eq!(title_of(&bytes), "edited");
        }
        result => panic!("a stale update must conflict: {result:?}"),
    }
    assert_eq!(store.get(&StoreKey::from(&key)).unwrap().unwrap().0, rev2);

    // An update of a key that is not there is a conflict with nothing current.
    let ghost = task("tk-conf-ghost", "ghost");
    let w4 = WriteId::generate();
    assert_eq!(
        store.update(&ghost.key(), &with_write(ghost.clone(), &w4), Rev(1), w4),
        Err(StoreError::Conflict { current: None })
    );

    // Revision 0 never matches: it must not create a key that is not there.
    let w6 = WriteId::generate();
    assert_eq!(
        store.update(&ghost.key(), &with_write(ghost.clone(), &w6), Rev(0), w6),
        Err(StoreError::Conflict { current: None })
    );
    assert_eq!(store.get(&StoreKey::from(&ghost.key())).unwrap(), None);

    // Scan lists by prefix.
    let two = task("tk-conf-2", "second");
    let w5 = WriteId::generate();
    store
        .create(&two.key(), &with_write(two.clone(), &w5), w5)
        .expect("second task");
    let keys: Vec<String> = store
        .scan("task.")
        .unwrap()
        .into_iter()
        .map(|event| event.key.0)
        .collect();
    assert_eq!(keys, ["task.m4m.tk-conf-1", "task.m4m.tk-conf-2"]);
    assert!(store.scan("run.").unwrap().is_empty());

    // A missing key reads as absent.
    assert_eq!(
        store
            .get(&StoreKey("task.m4m.tk-conf-nope".into()))
            .unwrap(),
        None
    );

    // The watch saw the three writes that landed, in order, under its prefix.
    let mut seen = Vec::new();
    while seen.len() < 3 {
        let event = watch
            .recv_timeout(Duration::from_secs(5))
            .expect("the watch reports each write");
        if event.value.is_some() {
            seen.push((event.key.0.clone(), event.rev));
        }
    }
    let rev_of_first_two = [rev1, rev2];
    assert_eq!(seen[0].0, "task.m4m.tk-conf-1");
    assert_eq!(seen[0].1, rev_of_first_two[0]);
    assert_eq!(seen[1].1, rev_of_first_two[1]);
    assert_eq!(seen[2].0, "task.m4m.tk-conf-2");
}
