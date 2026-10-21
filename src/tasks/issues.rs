//! worklink's mirror of forge issues (bucket `ISSUES`, contracts § 5.3), read
//! by the server and never written.
//!
//! Keys: `issue.<forge>.<forge_id>.<project_id>.<number>` holds one issue;
//! `sync.<forge>.<forge_id>.<project_id>` holds how the last sync of that
//! project went. Each row keeps the KV revision the watch saw for its key
//! (`mirror_rev`), which a confirmation later records as what ac was shown.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::clean::{clean, clean_opt};
use super::ids::Rev;
use super::store::WatchEvent;

const REJECTED_KEPT: usize = 100;
const FIELD_MAX_CHARS: usize = 2048;
const TITLE_MAX_CHARS: usize = 2000;
const LIST_MAX: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IssueRefRecord {
    pub forge: String,
    pub forge_id: String,
    pub host: String,
    pub project_id: u64,
    pub project_path: String,
    pub number: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum IssueState {
    Open,
    Closed,
    #[serde(other)]
    Unknown,
}

/// One issue as worklink mirrors it: no body, no comments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IssueRecord {
    /// The record's version; a newer one than this reader knows is skipped.
    #[serde(default)]
    pub v: Option<u32>,
    #[serde(rename = "ref")]
    pub issue_ref: IssueRefRecord,
    pub title: String,
    pub state: IssueState,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub assignees: Vec<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub web_url: Option<String>,
    /// The forge's own revision marker (its type is the forge's business).
    #[serde(default)]
    pub source_rev: Option<serde_json::Value>,
    #[serde(default)]
    pub last_sync: Option<String>,
    /// Present (true) when `title` was cut; absent otherwise.
    #[serde(default)]
    pub truncated: bool,
}

/// The newest record version this reader understands.
const SUPPORTED_VERSION: u32 = 1;
/// A forge's own revision marker is kept only when it is this small.
const SOURCE_REV_MAX_BYTES: usize = 256;

impl IssueRecord {
    /// Clean every string and cut what is too long (a long title is cut and
    /// flagged, not refused: the row is still the forge's issue).
    fn sanitize(&mut self) {
        let (title, cut) = clean(&self.title, TITLE_MAX_CHARS);
        self.title = title;
        self.truncated |= cut;
        let r = &mut self.issue_ref;
        for field in [
            &mut r.forge,
            &mut r.forge_id,
            &mut r.host,
            &mut r.project_path,
        ] {
            *field = clean(field, FIELD_MAX_CHARS).0;
        }
        for list in [&mut self.labels, &mut self.assignees] {
            list.truncate(LIST_MAX);
            for item in list.iter_mut() {
                *item = clean(item, FIELD_MAX_CHARS).0;
            }
        }
        for field in [
            &mut self.author,
            &mut self.created_at,
            &mut self.updated_at,
            &mut self.web_url,
            &mut self.last_sync,
        ] {
            clean_opt(field, FIELD_MAX_CHARS);
        }
        match self.source_rev.take() {
            Some(serde_json::Value::String(text)) => {
                self.source_rev = Some(serde_json::Value::String(clean(&text, 256).0));
            }
            Some(value) if value.to_string().len() <= SOURCE_REV_MAX_BYTES => {
                self.source_rev = Some(value);
            }
            _ => {}
        }
    }
}

/// How the last sync of one project went (`sync.<forge>.<forge_id>.<project_id>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SyncRecord {
    #[serde(default)]
    pub project: Option<SyncProject>,
    /// `ok` or `error`.
    pub status: String,
    #[serde(default)]
    pub error_class: Option<String>,
    #[serde(default)]
    pub last_attempt: Option<String>,
    #[serde(default)]
    pub last_ok: Option<String>,
    #[serde(default)]
    pub last_full: Option<String>,
    #[serde(default)]
    pub issues: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SyncProject {
    #[serde(rename = "ref")]
    pub project_ref: ProjectRefRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProjectRefRecord {
    pub forge: String,
    pub forge_id: String,
    #[serde(default)]
    pub host: Option<String>,
    pub project_id: u64,
    #[serde(default)]
    pub project_path: Option<String>,
}

impl SyncRecord {
    fn sanitize(&mut self) {
        self.status = clean(&self.status, 64).0;
        for field in [
            &mut self.error_class,
            &mut self.last_attempt,
            &mut self.last_ok,
            &mut self.last_full,
        ] {
            clean_opt(field, FIELD_MAX_CHARS);
        }
        if let Some(project) = self.project.as_mut() {
            let p = &mut project.project_ref;
            p.forge = clean(&p.forge, FIELD_MAX_CHARS).0;
            p.forge_id = clean(&p.forge_id, FIELD_MAX_CHARS).0;
            clean_opt(&mut p.host, FIELD_MAX_CHARS);
            clean_opt(&mut p.project_path, FIELD_MAX_CHARS);
        }
    }
}

/// `(forge, forge_id, project_id, number)`: an issue's place, which also
/// orders the cache.
pub(crate) type IssueKey = (String, String, u64, u64);

/// `(forge, forge_id, project_id)`.
pub(crate) type ProjectKey = (String, String, u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IssueRow {
    /// The bucket key, `issue.<forge>.<forge_id>.<project_id>.<number>`.
    pub name: String,
    /// The KV revision the watch saw for this key.
    pub mirror_rev: Rev,
    pub record: IssueRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SyncRow {
    pub rev: Rev,
    pub record: SyncRecord,
}

/// What a project's issues add up to, kept as rows come and go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectCount {
    pub project_path: String,
    pub open: usize,
    pub total: usize,
}

/// `issue.<forge>.<forge_id>.<project_id>.<number>`: the four parts.
fn parse_issue_key(key: &str) -> Option<IssueKey> {
    let mut parts = key.strip_prefix("issue.")?.split('.');
    let forge = parts.next()?;
    let forge_id = parts.next()?;
    let project_id = parts.next()?.parse().ok()?;
    let number = parts.next()?.parse().ok()?;
    parts
        .next()
        .is_none()
        .then(|| (forge.to_owned(), forge_id.to_owned(), project_id, number))
}

/// `sync.<forge>.<forge_id>.<project_id>`.
fn parse_sync_key(key: &str) -> Option<ProjectKey> {
    let mut parts = key.strip_prefix("sync.")?.split('.');
    let forge = parts.next()?;
    let forge_id = parts.next()?;
    let project_id = parts.next()?.parse().ok()?;
    parts
        .next()
        .is_none()
        .then(|| (forge.to_owned(), forge_id.to_owned(), project_id))
}

/// One watch event, read and cleaned off the app's thread. Applying it is a
/// map insert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum IssueChange {
    Issue {
        name: String,
        rev: Rev,
        record: Box<IssueRecord>,
    },
    IssueGone {
        name: String,
    },
    Sync {
        name: String,
        rev: Rev,
        record: Box<SyncRecord>,
    },
    SyncGone {
        name: String,
    },
    /// A document that cannot be used. The older row for its key goes too:
    /// showing it would be showing what the forge no longer says.
    Rejected {
        name: String,
        why: String,
    },
}

/// Read one event. `None` for a key that is neither an issue nor a sync row.
pub(crate) fn parse_event(event: &WatchEvent) -> Option<IssueChange> {
    let name = &event.key.0;
    let rejected = |why: String| IssueChange::Rejected {
        name: name.clone(),
        why,
    };
    if let Some((forge, forge_id, project_id, number)) = parse_issue_key(name) {
        let Some(bytes) = &event.value else {
            return Some(IssueChange::IssueGone { name: name.clone() });
        };
        let mut record = match serde_json::from_slice::<IssueRecord>(bytes) {
            Ok(record) => record,
            Err(err) => return Some(rejected(format!("unreadable: {err}"))),
        };
        if record.v.is_some_and(|v| v > SUPPORTED_VERSION) {
            return Some(rejected(format!(
                "record version {} is newer than this reader",
                record.v.unwrap_or_default()
            )));
        }
        let r = &record.issue_ref;
        if r.forge != forge
            || r.forge_id != forge_id
            || r.project_id != project_id
            || r.number != number
        {
            return Some(rejected("the document names another issue".into()));
        }
        record.sanitize();
        return Some(IssueChange::Issue {
            name: name.clone(),
            rev: event.rev,
            record: Box::new(record),
        });
    }
    if let Some((forge, forge_id, project_id)) = parse_sync_key(name) {
        let Some(bytes) = &event.value else {
            return Some(IssueChange::SyncGone { name: name.clone() });
        };
        let mut record = match serde_json::from_slice::<SyncRecord>(bytes) {
            Ok(record) => record,
            Err(err) => return Some(rejected(format!("unreadable: {err}"))),
        };
        if let Some(project) = &record.project {
            let p = &project.project_ref;
            if p.forge != forge || p.forge_id != forge_id || p.project_id != project_id {
                return Some(rejected("the document names another project".into()));
            }
        }
        record.sanitize();
        return Some(IssueChange::Sync {
            name: name.clone(),
            rev: event.rev,
            record: Box::new(record),
        });
    }
    None
}

#[derive(Debug, Default)]
pub(crate) struct IssueCache {
    issues: BTreeMap<IssueKey, IssueRow>,
    syncs: BTreeMap<String, SyncRow>,
    projects: BTreeMap<ProjectKey, ProjectCount>,
    rejected: Vec<(String, String)>,
}

impl IssueCache {
    pub(crate) fn len(&self) -> usize {
        self.issues.len()
    }

    pub(crate) fn rejected(&self) -> &[(String, String)] {
        &self.rejected
    }

    /// The issues in key order (forge, forge id, project, number).
    pub(crate) fn issues(&self) -> impl Iterator<Item = &IssueRow> {
        self.issues.values()
    }

    pub(crate) fn syncs(&self) -> impl Iterator<Item = (&String, &SyncRow)> {
        self.syncs.iter()
    }

    /// How many issues each project has, open and in all.
    pub(crate) fn projects(&self) -> impl Iterator<Item = (&ProjectKey, &ProjectCount)> {
        self.projects.iter()
    }

    pub(crate) fn get(&self, name: &str) -> Option<&IssueRow> {
        self.issues.get(&parse_issue_key(name)?)
    }

    /// Forget everything: the bucket is read again from its start.
    pub(crate) fn clear(&mut self) -> bool {
        let any = !self.issues.is_empty() || !self.syncs.is_empty();
        self.issues.clear();
        self.syncs.clear();
        self.projects.clear();
        any
    }

    /// Read and apply one event (what the store thread and the app do in two
    /// steps, together; for tests).
    #[cfg(test)]
    pub(crate) fn apply_event(&mut self, event: &WatchEvent) -> bool {
        parse_event(event).is_some_and(|change| self.apply(change))
    }

    fn count(&mut self, row: &IssueRow, add: bool) {
        let r = &row.record.issue_ref;
        let key = (r.forge.clone(), r.forge_id.clone(), r.project_id);
        if add {
            let entry = self.projects.entry(key).or_insert_with(|| ProjectCount {
                project_path: r.project_path.clone(),
                open: 0,
                total: 0,
            });
            entry.project_path.clone_from(&r.project_path);
            entry.total += 1;
            entry.open += usize::from(row.record.state == IssueState::Open);
        } else if let Some(entry) = self.projects.get_mut(&key) {
            entry.total = entry.total.saturating_sub(1);
            entry.open = entry
                .open
                .saturating_sub(usize::from(row.record.state == IssueState::Open));
            if entry.total == 0 {
                self.projects.remove(&key);
            }
        }
    }

    fn remove_issue(&mut self, name: &str) -> bool {
        let Some(key) = parse_issue_key(name) else {
            return false;
        };
        let Some(old) = self.issues.remove(&key) else {
            return false;
        };
        self.count(&old, false);
        true
    }

    /// Apply one change. Returns whether the cache changed.
    pub(crate) fn apply(&mut self, change: IssueChange) -> bool {
        match change {
            IssueChange::Issue { name, rev, record } => {
                let Some(key) = parse_issue_key(&name) else {
                    return false;
                };
                if self
                    .issues
                    .get(&key)
                    .is_some_and(|row| row.mirror_rev >= rev)
                {
                    return false;
                }
                self.remove_issue(&name);
                let row = IssueRow {
                    name,
                    mirror_rev: rev,
                    record: *record,
                };
                self.count(&row, true);
                self.issues.insert(key, row);
                true
            }
            IssueChange::IssueGone { name } => self.remove_issue(&name),
            IssueChange::Sync { name, rev, record } => {
                if self.syncs.get(&name).is_some_and(|row| row.rev >= rev) {
                    return false;
                }
                self.syncs.insert(
                    name,
                    SyncRow {
                        rev,
                        record: *record,
                    },
                );
                true
            }
            IssueChange::SyncGone { name } => self.syncs.remove(&name).is_some(),
            IssueChange::Rejected { name, why } => {
                tracing::warn!(key = %name, %why, "an issue mirror row was refused");
                self.rejected.push((name.clone(), why));
                if self.rejected.len() > REJECTED_KEPT {
                    let excess = self.rejected.len() - REJECTED_KEPT;
                    self.rejected.drain(..excess);
                }
                self.remove_issue(&name) | self.syncs.remove(&name).is_some()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::store::StoreKey;

    fn issue_event(key: &str, rev: u64, doc: serde_json::Value) -> WatchEvent {
        WatchEvent {
            key: StoreKey(key.into()),
            rev: Rev(rev),
            value: Some(serde_json::to_vec(&doc).unwrap()),
            at: None,
        }
    }

    pub(crate) fn issue_doc(number: u64, title: &str) -> serde_json::Value {
        serde_json::json!({
            "v": 1,
            "ref": {"forge": "gitea", "forge_id": "gitea-lab", "host": "gitea.example",
                    "project_id": 84, "project_path": "ac/worklink", "number": number},
            "title": title, "state": "open", "labels": ["bug"], "assignees": [],
            "author": "ac", "created_at": "2026-10-21T10:00:00Z",
            "updated_at": "2026-10-21T11:00:00Z", "web_url": "https://gitea.example/ac/worklink/issues/1",
            "source_rev": "abc", "last_sync": "2026-10-21T12:00:00Z"
        })
    }

    #[test]
    fn issues_and_syncs_are_cached_by_key_with_the_revision_the_watch_saw() {
        let mut cache = IssueCache::default();
        assert!(cache.apply_event(&issue_event(
            "issue.gitea.gitea-lab.84.7",
            11,
            issue_doc(7, "first")
        )));
        assert!(cache.apply_event(&issue_event(
            "sync.gitea.gitea-lab.84",
            4,
            serde_json::json!({
                "project": {"ref": {"forge": "gitea", "forge_id": "gitea-lab", "project_id": 84,
                                    "project_path": "ac/worklink"}},
                "status": "ok", "last_ok": "2026-10-21T12:00:00Z", "issues": 14
            })
        )));
        let row = cache.get("issue.gitea.gitea-lab.84.7").unwrap();
        assert_eq!(row.mirror_rev, Rev(11));
        assert_eq!(row.record.title, "first");
        assert_eq!(cache.syncs().count(), 1);

        // A re-sync that changes no field still moves the revision.
        assert!(cache.apply_event(&issue_event(
            "issue.gitea.gitea-lab.84.7",
            12,
            issue_doc(7, "first")
        )));
        assert_eq!(
            cache.get("issue.gitea.gitea-lab.84.7").unwrap().mirror_rev,
            Rev(12)
        );
        // An older or equal revision changes nothing.
        assert!(!cache.apply_event(&issue_event(
            "issue.gitea.gitea-lab.84.7",
            12,
            issue_doc(7, "other")
        )));
    }

    #[test]
    fn a_document_that_names_another_issue_or_breaks_a_limit_is_refused() {
        let mut cache = IssueCache::default();
        // Under the key of #7, a document for #8.
        assert!(!cache.apply_event(&issue_event(
            "issue.gitea.gitea-lab.84.7",
            1,
            issue_doc(8, "wrong")
        )));
        // A title that is too long is cut and flagged, not refused.
        assert!(cache.apply_event(&issue_event(
            "issue.gitea.gitea-lab.84.9",
            1,
            issue_doc(9, &"x".repeat(TITLE_MAX_CHARS + 1))
        )));
        let row = cache.get("issue.gitea.gitea-lab.84.9").unwrap();
        assert_eq!(row.record.title.chars().count(), TITLE_MAX_CHARS);
        assert!(row.record.truncated);
        assert!(!cache.apply_event(&WatchEvent {
            key: StoreKey("issue.gitea.gitea-lab.84.10".into()),
            rev: Rev(1),
            value: Some(b"{nope".to_vec()),
            at: None
        }));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.rejected().len(), 2);
        // Not an issues key at all: not this cache's business.
        assert!(!cache.apply_event(&issue_event("mr.gitea.x.1.2", 1, issue_doc(1, "x"))));
    }

    #[test]
    fn a_removed_issue_leaves_the_cache_and_a_newer_state_is_unknown_not_dropped() {
        let mut cache = IssueCache::default();
        let mut doc = issue_doc(7, "state from the future");
        doc["state"] = serde_json::json!("merged");
        assert!(cache.apply_event(&issue_event("issue.gitea.gitea-lab.84.7", 1, doc)));
        assert_eq!(
            cache
                .get("issue.gitea.gitea-lab.84.7")
                .unwrap()
                .record
                .state,
            IssueState::Unknown
        );
        assert!(cache.apply_event(&WatchEvent {
            key: StoreKey("issue.gitea.gitea-lab.84.7".into()),
            rev: Rev(2),
            value: None,
            at: None
        }));
        assert_eq!(cache.len(), 0);
    }
}
