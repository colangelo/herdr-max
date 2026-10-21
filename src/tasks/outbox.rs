//! The outbox: every write is a durable entry on disk before it is answered
//! "pending", and stays there until the store has taken it.
//!
//! One file per entry under `<dir>/`, named `<seq>-<write id>.json`, so the
//! order of the names is the order of the writes. Entries for one key form a
//! chain: a second edit queued behind the first takes the first's write id as
//! its base, and is replayed against the revision the first one produced. That
//! revision is kept as a receipt (`<dir>/receipts/<write id>.json`) for as long
//! as an entry depends on it.
//!
//! A write is never lost to a crash, and never applied twice: replay hands the
//! backend the write id, and a backend answers success for one it already holds.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::ids::{Rev, TaskKey, WriteId};
use super::model::Task;

const LOCK_FILE: &str = ".lock";
const RECEIPTS_DIR: &str = "receipts";
/// How many damaged entries are kept aside.
const CORRUPT_KEPT: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Op {
    Create,
    Update,
}

/// What an entry is written against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Base {
    /// The revision the caller saw.
    Rev(Rev),
    /// The revision that an earlier queued write of the same key produced.
    After(WriteId),
}

/// The store's side of a conflict, kept with the entry that lost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Conflict {
    pub theirs_rev: Option<Rev>,
    pub theirs: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Entry {
    pub write: WriteId,
    pub key: TaskKey,
    pub op: Op,
    pub doc: Task,
    /// `None` for a create.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<Base>,
    /// Set once the store refused it as a conflict; the entry then waits for a
    /// decision, and so does everything queued behind it for the same key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict: Option<Conflict>,
    /// Made by the server itself (rebuilding `runs`, the move to Running), not
    /// by a person: if the store refuses it as a conflict it is dropped, not
    /// held for a decision, since the next run record makes it again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto: bool,
    #[serde(skip)]
    seq: u64,
}

#[derive(Serialize, Deserialize)]
struct Receipt {
    write: WriteId,
    key: TaskKey,
    rev: Rev,
}

pub(crate) struct Outbox {
    dir: PathBuf,
    entries: Vec<Entry>,
    next_seq: u64,
    _lock: fs::File,
}

impl Outbox {
    /// Open the outbox in `dir`, creating it. One outbox owns a directory: a
    /// second one is refused until the first is dropped.
    pub(crate) fn open(dir: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(dir.join(RECEIPTS_DIR))?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(LOCK_FILE))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    format!("another herdr server holds the outbox in {}", dir.display()),
                ));
            }
            Err(fs::TryLockError::Error(err)) => return Err(err),
        }
        let mut outbox = Self {
            dir,
            entries: Vec::new(),
            next_seq: 1,
            _lock: lock,
        };
        outbox.load()?;
        Ok(outbox)
    }

    fn load(&mut self) -> io::Result<()> {
        let mut found = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let path = entry?.path();
            match path.extension().and_then(|ext| ext.to_str()) {
                Some("tmp") => {
                    let _ = fs::remove_file(&path);
                }
                Some("json") => match read_entry(&path)? {
                    Some(entry) => found.push(entry),
                    None => self.set_aside(&path),
                },
                _ => {}
            }
        }
        found.sort_by_key(|entry| entry.seq);
        self.next_seq = found.last().map_or(1, |entry| entry.seq + 1);
        self.entries = found;
        self.drop_orphan_receipts();
        Ok(())
    }

    /// Keep a damaged entry for a person to look at, out of the way. Only the
    /// newest [`CORRUPT_KEPT`] are kept.
    fn set_aside(&self, path: &Path) {
        let aside = path.with_extension("json.corrupt");
        tracing::warn!(file = %path.display(), "setting aside a corrupt outbox entry");
        let _ = fs::rename(path, aside);
        let Ok(found) = fs::read_dir(&self.dir) else {
            return;
        };
        let mut asides: Vec<PathBuf> = found
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "corrupt"))
            .collect();
        // Names start with the sequence number, so the oldest sort first.
        asides.sort();
        let excess = asides.len().saturating_sub(CORRUPT_KEPT);
        for old in asides.into_iter().take(excess) {
            let _ = fs::remove_file(old);
        }
    }

    /// Receipts nothing depends on are leftovers of a crash between steps.
    fn drop_orphan_receipts(&self) {
        let needed: HashSet<&WriteId> = self
            .entries
            .iter()
            .filter_map(|entry| match &entry.base {
                Some(Base::After(write)) => Some(write),
                _ => None,
            })
            .collect();
        let Ok(receipts) = fs::read_dir(self.dir.join(RECEIPTS_DIR)) else {
            return;
        };
        for receipt in receipts.flatten() {
            let path = receipt.path();
            let write = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(|stem| WriteId::from_string(stem.to_owned()));
            if write.as_ref().is_none_or(|write| !needed.contains(write)) {
                let _ = fs::remove_file(path);
            }
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Every entry in write order.
    pub(crate) fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Queue a write, durably, before anything else happens to it. `base` is
    /// what it is written against: the revision the caller saw, or the earlier
    /// write it was built on (which may have been sent since).
    pub(crate) fn push(
        &mut self,
        write: WriteId,
        op: Op,
        doc: Task,
        base: Option<Base>,
    ) -> io::Result<()> {
        self.push_as(write, op, doc, base, false)
    }

    /// As [`Self::push`]; `auto` marks a write the server made on its own.
    pub(crate) fn push_as(
        &mut self,
        write: WriteId,
        op: Op,
        doc: Task,
        base: Option<Base>,
        auto: bool,
    ) -> io::Result<()> {
        let key = doc.key();
        let entry = Entry {
            write,
            key,
            op,
            doc,
            base,
            conflict: None,
            auto,
            seq: self.next_seq,
        };
        self.persist(&entry)?;
        self.next_seq += 1;
        self.entries.push(entry);
        Ok(())
    }

    /// The first entry of each key that may be tried now: not behind a conflict.
    pub(crate) fn ready(&self) -> Vec<&Entry> {
        let mut seen = HashSet::new();
        self.entries
            .iter()
            .filter(|entry| seen.insert(&entry.key))
            .filter(|entry| entry.conflict.is_none())
            .collect()
    }

    /// The revision an entry is to be written against, if it is known: the
    /// caller's own, or the receipt of the write it follows.
    pub(crate) fn expected_rev(&self, entry: &Entry) -> Option<Rev> {
        match entry.base.as_ref()? {
            Base::Rev(rev) => Some(*rev),
            Base::After(write) => self.receipt(write),
        }
    }

    pub(crate) fn receipt(&self, write: &WriteId) -> Option<Rev> {
        let bytes = fs::read(self.receipt_path(write)).ok()?;
        serde_json::from_slice::<Receipt>(&bytes)
            .ok()
            .map(|receipt| receipt.rev)
    }

    /// The store took the write at `rev`. A receipt is kept while a later entry
    /// is based on it; the receipt of this entry's own base is dropped, since
    /// this was its only dependent. The entry goes last, so a crash in between
    /// leaves something to replay rather than something lost.
    pub(crate) fn complete(&mut self, write: &WriteId, rev: Rev) -> io::Result<()> {
        let Some(index) = self.entries.iter().position(|entry| &entry.write == write) else {
            return Ok(());
        };
        let entry = self.entries.remove(index);
        let has_dependent = self
            .entries
            .iter()
            .any(|other| other.base.as_ref() == Some(&Base::After(write.clone())));
        if has_dependent {
            let receipt = Receipt {
                write: write.clone(),
                key: entry.key.clone(),
                rev,
            };
            if let Err(err) = self.write_file(&self.receipt_path(write), &receipt) {
                self.entries.insert(index, entry);
                return Err(err);
            }
            // The receipt must be on disk before the entry that produced it goes.
            sync_dir(&self.dir.join(RECEIPTS_DIR));
        }
        remove_durably(&self.entry_path(&entry), &self.dir)?;
        if let Some(Base::After(base)) = &entry.base {
            let _ = fs::remove_file(self.receipt_path(base));
        }
        Ok(())
    }

    /// The store refused the write for good. It goes, and so does everything
    /// queued behind it for the key, which was written on top of it. Returns the
    /// write ids removed, the first one's own among them.
    pub(crate) fn fail(&mut self, write: &WriteId) -> io::Result<Vec<WriteId>> {
        let mut removed = Vec::new();
        let mut doomed = vec![write.clone()];
        while let Some(next) = doomed.pop() {
            let Some(index) = self.entries.iter().position(|entry| entry.write == next) else {
                continue;
            };
            let entry = self.entries.remove(index);
            remove_durably(&self.entry_path(&entry), &self.dir)?;
            let _ = fs::remove_file(self.receipt_path(&entry.write));
            // The write ahead of it kept a receipt for this entry alone.
            if let Some(Base::After(ahead)) = &entry.base {
                let _ = fs::remove_file(self.receipt_path(ahead));
            }
            doomed.extend(
                self.entries
                    .iter()
                    .filter(|other| other.base.as_ref() == Some(&Base::After(entry.write.clone())))
                    .map(|other| other.write.clone()),
            );
            removed.push(entry.write);
        }
        Ok(removed)
    }

    /// Drop one entry and leave the ones queued behind it where they are.
    /// Returns what was behind it (the entries that named it as their base),
    /// so the caller can decide about them.
    pub(crate) fn remove_head(&mut self, write: &WriteId) -> io::Result<Vec<Entry>> {
        let Some(index) = self.entries.iter().position(|entry| &entry.write == write) else {
            return Ok(Vec::new());
        };
        let entry = self.entries.remove(index);
        remove_durably(&self.entry_path(&entry), &self.dir)?;
        let _ = fs::remove_file(self.receipt_path(&entry.write));
        if let Some(Base::After(ahead)) = &entry.base {
            let _ = fs::remove_file(self.receipt_path(ahead));
        }
        Ok(self
            .entries
            .iter()
            .filter(|other| other.base.as_ref() == Some(&Base::After(write.clone())))
            .cloned()
            .collect())
    }

    /// Remember that the store holds a newer version, and hold the entry.
    pub(crate) fn mark_conflict(
        &mut self,
        write: &WriteId,
        theirs_rev: Option<Rev>,
        theirs: Option<serde_json::Value>,
    ) -> io::Result<()> {
        let Some(index) = self.entries.iter().position(|entry| &entry.write == write) else {
            return Ok(());
        };
        self.entries[index].conflict = Some(Conflict { theirs_rev, theirs });
        let entry = self.entries[index].clone();
        self.persist(&entry)
    }

    /// The entry waiting for a decision under `write`, if there is one.
    pub(crate) fn held(&self, write: &WriteId) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|entry| &entry.write == write && entry.conflict.is_some())
    }

    /// Let a held entry go again: written as `op` against `base`, with `doc`
    /// in place of its own when the decision is a merge. The chain queued
    /// behind it follows once it is taken.
    pub(crate) fn release(
        &mut self,
        write: &WriteId,
        op: Op,
        base: Option<Base>,
        doc: Option<Task>,
    ) -> io::Result<()> {
        let Some(index) = self.entries.iter().position(|entry| &entry.write == write) else {
            return Ok(());
        };
        let mut entry = self.entries[index].clone();
        let old_base = entry.base.take();
        entry.conflict = None;
        entry.op = op;
        entry.base = base;
        if let Some(doc) = doc {
            entry.doc = doc;
        }
        self.persist(&entry)?;
        self.entries[index] = entry;
        // It no longer follows the write ahead of it, which kept a receipt for
        // it alone.
        if let Some(Base::After(ahead)) = old_base {
            let _ = fs::remove_file(self.receipt_path(&ahead));
        }
        Ok(())
    }

    /// Make every directory change so far durable.
    pub(crate) fn sync(&self) {
        sync_dir(&self.dir);
    }

    /// Who is waiting: every queued write and its key.
    pub(crate) fn pending(&self) -> Vec<(WriteId, TaskKey)> {
        self.entries
            .iter()
            .map(|entry| (entry.write.clone(), entry.key.clone()))
            .collect()
    }

    /// The last write queued for each key, to build the next edit on.
    pub(crate) fn latest_docs(&self) -> HashMap<TaskKey, &Task> {
        let mut latest = HashMap::new();
        for entry in &self.entries {
            latest.insert(entry.key.clone(), &entry.doc);
        }
        latest
    }

    fn entry_path(&self, entry: &Entry) -> PathBuf {
        self.dir
            .join(format!("{:020}-{}.json", entry.seq, entry.write))
    }

    fn receipt_path(&self, write: &WriteId) -> PathBuf {
        self.dir
            .join(RECEIPTS_DIR)
            .join(format!("{}.json", write.as_str().to_ascii_lowercase()))
    }

    fn persist(&self, entry: &Entry) -> io::Result<()> {
        self.write_file(&self.entry_path(entry), entry)?;
        sync_dir(&self.dir);
        Ok(())
    }

    /// Write `value` to `path` through a temp file, so the path holds the old
    /// content or the new one, never half.
    fn write_file(&self, path: &Path, value: &impl Serialize) -> io::Result<()> {
        let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
        let temp = path.with_extension(format!(
            "json.{}.tmp",
            crate::ulid::new_ulid().to_ascii_lowercase()
        ));
        let mut file = fs::File::create(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)
    }
}

/// Read one entry file. `Ok(None)` means the file is not a readable entry (a
/// bad name or bad content) and belongs aside; a file that cannot be read at
/// all is an error, and nothing is set aside because of it.
fn read_entry(path: &Path) -> io::Result<Option<Entry>> {
    let seq = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.split_once('-'))
        .and_then(|(seq, _)| seq.parse::<u64>().ok());
    let bytes = fs::read(path)?;
    let (Some(seq), Ok(mut entry)) = (seq, serde_json::from_slice::<Entry>(&bytes)) else {
        return Ok(None);
    };
    entry.seq = seq;
    Ok(Some(entry))
}

fn remove_durably(path: &Path, dir: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    sync_dir(dir);
    Ok(())
}

#[cfg(unix)]
fn sync_dir(dir: &Path) {
    if let Err(err) = fs::File::open(dir).and_then(|dir| dir.sync_all()) {
        tracing::warn!(error = %err, "could not sync the outbox directory");
    }
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::ids::{HostId, TaskId};
    use crate::tasks::model::{Origin, Timestamp, Via};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!(
                "herdr-outbox-{}-{}",
                std::process::id(),
                crate::ulid::new_ulid()
            )))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn task(id: &str, title: &str) -> Task {
        Task::new(
            HostId::new("m4m").unwrap(),
            TaskId::new(id).unwrap(),
            title,
            Origin {
                host: "m4m".into(),
                by: "x".into(),
                via: Via::Cli,
            },
            Timestamp("2026-10-21T20:00:00Z".into()),
        )
        .unwrap()
    }

    fn w(name: &str) -> WriteId {
        WriteId::from_string(name.to_owned())
    }

    #[test]
    fn a_pushed_write_is_on_disk_in_order_and_survives_reopening() {
        let scratch = Scratch::new();
        {
            let mut outbox = Outbox::open(scratch.0.clone()).unwrap();
            outbox
                .push(w("w1"), Op::Create, task("tk-a", "a"), None)
                .unwrap();
            outbox
                .push(w("w2"), Op::Create, task("tk-b", "b"), None)
                .unwrap();
            outbox
                .push(
                    w("w3"),
                    Op::Update,
                    task("tk-a", "a2"),
                    Some(Base::After(w("w1"))),
                )
                .unwrap();
            assert_eq!(outbox.len(), 3);
        }
        let outbox = Outbox::open(scratch.0.clone()).unwrap();
        let writes: Vec<&str> = outbox.entries().iter().map(|e| e.write.as_str()).collect();
        assert_eq!(writes, ["w1", "w2", "w3"]);
        // A new push after reopening goes after the old ones.
        let mut outbox = outbox;
        outbox
            .push(w("w4"), Op::Create, task("tk-c", "c"), None)
            .unwrap();
        assert_eq!(outbox.entries().last().unwrap().write.as_str(), "w4");
    }

    #[test]
    fn only_the_head_of_each_chain_is_ready() {
        let scratch = Scratch::new();
        let mut outbox = Outbox::open(scratch.0.clone()).unwrap();
        outbox
            .push(
                w("w1"),
                Op::Update,
                task("tk-a", "one"),
                Some(Base::Rev(Rev(7))),
            )
            .unwrap();
        outbox
            .push(
                w("w2"),
                Op::Update,
                task("tk-a", "two"),
                Some(Base::After(w("w1"))),
            )
            .unwrap();
        outbox
            .push(
                w("w3"),
                Op::Update,
                task("tk-b", "other"),
                Some(Base::Rev(Rev(2))),
            )
            .unwrap();
        let bases: Vec<_> = outbox.entries().iter().map(|e| e.base.clone()).collect();
        assert_eq!(
            bases,
            [
                Some(Base::Rev(Rev(7))),
                Some(Base::After(w("w1"))),
                Some(Base::Rev(Rev(2)))
            ]
        );
        // Only the head of each key is ready, and the chain waits behind it.
        let ready: Vec<&str> = outbox.ready().iter().map(|e| e.write.as_str()).collect();
        assert_eq!(ready, ["w1", "w3"]);
        // A create has no base, and an edit queued behind a create follows it.
        outbox
            .push(w("w4"), Op::Create, task("tk-c", "new"), None)
            .unwrap();
        outbox
            .push(
                w("w5"),
                Op::Update,
                task("tk-c", "new2"),
                Some(Base::After(w("w4"))),
            )
            .unwrap();
        assert_eq!(outbox.entries()[3].base, None);
        assert_eq!(outbox.entries()[4].base, Some(Base::After(w("w4"))));
    }

    #[test]
    fn a_receipt_lives_exactly_as_long_as_a_later_entry_depends_on_it() {
        let scratch = Scratch::new();
        let mut outbox = Outbox::open(scratch.0.clone()).unwrap();
        outbox
            .push(
                w("w1"),
                Op::Update,
                task("tk-a", "one"),
                Some(Base::Rev(Rev(7))),
            )
            .unwrap();
        outbox
            .push(
                w("w2"),
                Op::Update,
                task("tk-a", "two"),
                Some(Base::After(w("w1"))),
            )
            .unwrap();
        outbox
            .push(
                w("w3"),
                Op::Update,
                task("tk-a", "three"),
                Some(Base::After(w("w2"))),
            )
            .unwrap();
        assert_eq!(outbox.expected_rev(&outbox.entries()[0]), Some(Rev(7)));
        assert_eq!(
            outbox.expected_rev(&outbox.entries()[1]),
            None,
            "w1 not done"
        );

        outbox.complete(&w("w1"), Rev(8)).unwrap();
        assert_eq!(outbox.receipt(&w("w1")), Some(Rev(8)));
        assert_eq!(outbox.expected_rev(&outbox.entries()[0]), Some(Rev(8)));

        outbox.complete(&w("w2"), Rev(9)).unwrap();
        assert_eq!(outbox.receipt(&w("w1")), None, "its only dependent is done");
        assert_eq!(outbox.receipt(&w("w2")), Some(Rev(9)));

        outbox.complete(&w("w3"), Rev(10)).unwrap();
        assert_eq!(outbox.receipt(&w("w2")), None);
        assert_eq!(outbox.receipt(&w("w3")), None, "no dependent, no receipt");
        assert_eq!(outbox.len(), 0);
    }

    #[test]
    fn a_receipt_survives_a_restart_and_a_stray_one_is_dropped() {
        let scratch = Scratch::new();
        {
            let mut outbox = Outbox::open(scratch.0.clone()).unwrap();
            outbox
                .push(
                    w("w1"),
                    Op::Update,
                    task("tk-a", "one"),
                    Some(Base::Rev(Rev(7))),
                )
                .unwrap();
            outbox
                .push(
                    w("w2"),
                    Op::Update,
                    task("tk-a", "two"),
                    Some(Base::After(w("w1"))),
                )
                .unwrap();
            outbox.complete(&w("w1"), Rev(8)).unwrap();
        }
        // A leftover from a crash between steps.
        fs::write(scratch.0.join("receipts/stray.json"), b"{}").unwrap();
        let outbox = Outbox::open(scratch.0.clone()).unwrap();
        assert_eq!(outbox.receipt(&w("w1")), Some(Rev(8)));
        assert_eq!(outbox.expected_rev(&outbox.entries()[0]), Some(Rev(8)));
        assert!(!scratch.0.join("receipts/stray.json").exists());
    }

    #[test]
    fn a_conflict_holds_its_entry_and_the_chain_behind_it_across_a_restart() {
        let scratch = Scratch::new();
        {
            let mut outbox = Outbox::open(scratch.0.clone()).unwrap();
            outbox
                .push(
                    w("w1"),
                    Op::Update,
                    task("tk-a", "one"),
                    Some(Base::Rev(Rev(7))),
                )
                .unwrap();
            outbox
                .push(
                    w("w2"),
                    Op::Update,
                    task("tk-a", "two"),
                    Some(Base::After(w("w1"))),
                )
                .unwrap();
            outbox
                .push(
                    w("w3"),
                    Op::Update,
                    task("tk-b", "other"),
                    Some(Base::Rev(Rev(1))),
                )
                .unwrap();
            outbox
                .mark_conflict(
                    &w("w1"),
                    Some(Rev(9)),
                    Some(serde_json::json!({"title": "theirs"})),
                )
                .unwrap();
            let ready: Vec<&str> = outbox.ready().iter().map(|e| e.write.as_str()).collect();
            assert_eq!(ready, ["w3"], "tk-a waits for a decision, tk-b goes on");
        }
        let outbox = Outbox::open(scratch.0.clone()).unwrap();
        let held = &outbox.entries()[0];
        assert_eq!(held.conflict.as_ref().unwrap().theirs_rev, Some(Rev(9)));
        assert_eq!(outbox.ready().len(), 1);
    }

    #[test]
    fn a_write_the_store_refuses_for_good_takes_the_chain_behind_it() {
        let scratch = Scratch::new();
        let mut outbox = Outbox::open(scratch.0.clone()).unwrap();
        for (id, title, base) in [
            ("w1", "one", Base::Rev(Rev(7))),
            ("w2", "two", Base::After(w("w1"))),
            ("w3", "three", Base::After(w("w2"))),
        ] {
            outbox
                .push(w(id), Op::Update, task("tk-a", title), Some(base))
                .unwrap();
        }
        outbox
            .push(
                w("w4"),
                Op::Update,
                task("tk-b", "other"),
                Some(Base::Rev(Rev(1))),
            )
            .unwrap();
        let mut removed: Vec<String> = outbox
            .fail(&w("w1"))
            .unwrap()
            .iter()
            .map(|id| id.as_str().to_owned())
            .collect();
        removed.sort();
        assert_eq!(removed, ["w1", "w2", "w3"]);
        assert_eq!(outbox.len(), 1);
        assert_eq!(outbox.entries()[0].write.as_str(), "w4");
    }

    #[test]
    fn a_released_entry_goes_again_on_the_new_base_and_keeps_its_chain() {
        let scratch = Scratch::new();
        let mut outbox = Outbox::open(scratch.0.clone()).unwrap();
        outbox
            .push(
                w("w1"),
                Op::Update,
                task("tk-a", "one"),
                Some(Base::Rev(Rev(7))),
            )
            .unwrap();
        outbox
            .push(
                w("w2"),
                Op::Update,
                task("tk-a", "two"),
                Some(Base::After(w("w1"))),
            )
            .unwrap();
        outbox
            .mark_conflict(&w("w1"), Some(Rev(9)), Some(serde_json::json!({})))
            .unwrap();
        assert!(outbox.held(&w("w1")).is_some());
        assert!(outbox.held(&w("w2")).is_none());
        assert!(outbox.ready().is_empty());

        outbox
            .release(
                &w("w1"),
                Op::Update,
                Some(Base::Rev(Rev(9))),
                Some(task("tk-a", "merged")),
            )
            .unwrap();
        assert!(outbox.held(&w("w1")).is_none());
        let ready = outbox.ready();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].write.as_str(), "w1");
        assert_eq!(ready[0].doc.title, "merged");
        drop(outbox);

        let outbox = Outbox::open(scratch.0.clone()).unwrap();
        assert_eq!(outbox.entries()[0].base, Some(Base::Rev(Rev(9))));
        assert_eq!(outbox.entries()[0].doc.title, "merged");
        assert!(outbox.entries()[0].conflict.is_none());
        assert_eq!(outbox.entries()[1].base, Some(Base::After(w("w1"))));
    }

    #[test]
    fn dropping_an_entry_takes_the_receipt_its_base_kept() {
        let scratch = Scratch::new();
        let mut outbox = Outbox::open(scratch.0.clone()).unwrap();
        outbox
            .push(w("w1"), Op::Create, task("tk-a", "one"), None)
            .unwrap();
        outbox
            .push(
                w("w2"),
                Op::Update,
                task("tk-a", "two"),
                Some(Base::After(w("w1"))),
            )
            .unwrap();
        outbox.complete(&w("w1"), Rev(1)).unwrap();
        assert_eq!(outbox.receipt(&w("w1")), Some(Rev(1)));
        outbox.fail(&w("w2")).unwrap();
        assert_eq!(outbox.receipt(&w("w1")), None, "nothing depends on it now");
    }

    #[test]
    fn receipts_of_generated_ids_survive_a_reopen() {
        // Real write ids, not hand-written lower-case ones: a receipt is found
        // again by the id it was written under.
        let scratch = Scratch::new();
        let (first, second) = (WriteId::generate(), WriteId::generate());
        {
            let mut outbox = Outbox::open(scratch.0.clone()).unwrap();
            outbox
                .push(first.clone(), Op::Create, task("tk-a", "one"), None)
                .unwrap();
            outbox
                .push(
                    second.clone(),
                    Op::Update,
                    task("tk-a", "two"),
                    Some(Base::After(first.clone())),
                )
                .unwrap();
            outbox.complete(&first, Rev(4)).unwrap();
        }
        let outbox = Outbox::open(scratch.0.clone()).unwrap();
        assert_eq!(outbox.receipt(&first), Some(Rev(4)));
        assert_eq!(outbox.expected_rev(&outbox.entries()[0]), Some(Rev(4)));
    }

    #[test]
    fn releasing_an_entry_onto_a_revision_drops_the_receipt_it_followed() {
        let scratch = Scratch::new();
        let mut outbox = Outbox::open(scratch.0.clone()).unwrap();
        outbox
            .push(w("w1"), Op::Create, task("tk-a", "one"), None)
            .unwrap();
        outbox
            .push(
                w("w2"),
                Op::Update,
                task("tk-a", "two"),
                Some(Base::After(w("w1"))),
            )
            .unwrap();
        outbox.complete(&w("w1"), Rev(1)).unwrap();
        outbox.mark_conflict(&w("w2"), Some(Rev(5)), None).unwrap();
        assert_eq!(outbox.receipt(&w("w1")), Some(Rev(1)));
        outbox
            .release(&w("w2"), Op::Update, Some(Base::Rev(Rev(5))), None)
            .unwrap();
        assert_eq!(outbox.receipt(&w("w1")), None);
    }

    #[test]
    fn only_the_newest_corrupt_entries_are_kept_aside() {
        let scratch = Scratch::new();
        fs::create_dir_all(&scratch.0).unwrap();
        for seq in 1..=(CORRUPT_KEPT + 5) {
            fs::write(scratch.0.join(format!("{seq:020}-bad.json")), b"{nope").unwrap();
        }
        let outbox = Outbox::open(scratch.0.clone()).unwrap();
        assert_eq!(outbox.len(), 0);
        let mut asides: Vec<String> = fs::read_dir(&scratch.0)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".corrupt"))
            .collect();
        asides.sort();
        assert_eq!(asides.len(), CORRUPT_KEPT);
        assert!(
            asides[0].starts_with(&format!("{:020}", 6)),
            "the oldest went: {asides:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_entry_that_cannot_be_read_is_not_taken_for_a_corrupt_one() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = Scratch::new();
        {
            let mut outbox = Outbox::open(scratch.0.clone()).unwrap();
            outbox
                .push(w("w1"), Op::Create, task("tk-a", "one"), None)
                .unwrap();
        }
        let path = fs::read_dir(&scratch.0)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .find(|path| path.extension().is_some_and(|ext| ext == "json"))
            .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&path).is_ok() {
            // Running as a user who can read anything: the case cannot be made.
            return;
        }
        assert!(Outbox::open(scratch.0.clone()).is_err());
        assert!(path.exists(), "left where it is, not set aside");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(Outbox::open(scratch.0.clone()).unwrap().len(), 1);
    }

    #[test]
    fn a_second_outbox_on_one_directory_is_refused_while_the_first_lives() {
        let scratch = Scratch::new();
        let first = Outbox::open(scratch.0.clone()).unwrap();
        let err = Outbox::open(scratch.0.clone()).err().expect("refused");
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
        drop(first);
        assert!(Outbox::open(scratch.0.clone()).is_ok());
    }

    #[test]
    fn leftover_temp_files_go_and_a_corrupt_entry_is_set_aside() {
        let scratch = Scratch::new();
        drop(Outbox::open(scratch.0.clone()).unwrap());
        fs::write(
            scratch.0.join("00000000000000000001-w9.json.01hz.tmp"),
            b"half",
        )
        .unwrap();
        fs::write(scratch.0.join("00000000000000000002-w8.json"), b"{nope").unwrap();
        let outbox = Outbox::open(scratch.0.clone()).unwrap();
        assert_eq!(outbox.len(), 0);
        assert!(!scratch
            .0
            .join("00000000000000000001-w9.json.01hz.tmp")
            .exists());
        assert!(scratch
            .0
            .join("00000000000000000002-w8.json.corrupt")
            .exists());
    }

    #[test]
    fn the_latest_queued_version_of_each_key_is_the_one_to_build_the_next_edit_on() {
        let scratch = Scratch::new();
        let mut outbox = Outbox::open(scratch.0.clone()).unwrap();
        outbox
            .push(
                w("w1"),
                Op::Update,
                task("tk-a", "one"),
                Some(Base::Rev(Rev(7))),
            )
            .unwrap();
        outbox
            .push(
                w("w2"),
                Op::Update,
                task("tk-a", "two"),
                Some(Base::After(w("w1"))),
            )
            .unwrap();
        let latest = outbox.latest_docs();
        assert_eq!(latest[&task("tk-a", "x").key()].title, "two");
    }
}
