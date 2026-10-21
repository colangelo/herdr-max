//! worklink's mirror of forge issues (bucket `ISSUES`, contracts § 5.3), read
//! by the server and never written.
//!
//! Keys: `issue.<forge>.<forge_id>.<project_id>.<number>` holds one issue;
//! `sync.<forge>.<forge_id>.<project_id>` holds how the last sync of that
//! project went. Each row keeps the KV revision the watch saw for its key
//! (`mirror_rev`), which a confirmation later records as what ac was shown.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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

impl IssueRecord {
    fn check_sizes(&self) -> Result<(), &'static str> {
        let long = |text: &str| text.chars().count() > FIELD_MAX_CHARS;
        let optional_long = |text: &Option<String>| text.as_deref().is_some_and(long);
        let r = &self.issue_ref;
        if self.title.chars().count() > TITLE_MAX_CHARS
            || long(&r.forge)
            || long(&r.forge_id)
            || long(&r.host)
            || long(&r.project_path)
            || optional_long(&self.author)
            || optional_long(&self.created_at)
            || optional_long(&self.updated_at)
            || optional_long(&self.web_url)
            || optional_long(&self.last_sync)
        {
            return Err("a field is too long");
        }
        if self.labels.len() > LIST_MAX
            || self.assignees.len() > LIST_MAX
            || self.labels.iter().any(|label| long(label))
            || self.assignees.iter().any(|name| long(name))
        {
            return Err("too many or too long labels or assignees");
        }
        Ok(())
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IssueRow {
    /// The KV revision the watch saw for this key.
    pub mirror_rev: Rev,
    pub record: IssueRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SyncRow {
    pub rev: Rev,
    pub record: SyncRecord,
}

/// `issue.<forge>.<forge_id>.<project_id>.<number>`: the four parts.
fn parse_issue_key(key: &str) -> Option<(&str, &str, u64, u64)> {
    let mut parts = key.strip_prefix("issue.")?.split('.');
    let forge = parts.next()?;
    let forge_id = parts.next()?;
    let project_id = parts.next()?.parse().ok()?;
    let number = parts.next()?.parse().ok()?;
    parts
        .next()
        .is_none()
        .then_some((forge, forge_id, project_id, number))
}

/// `sync.<forge>.<forge_id>.<project_id>`.
fn parse_sync_key(key: &str) -> Option<(&str, &str, u64)> {
    let mut parts = key.strip_prefix("sync.")?.split('.');
    let forge = parts.next()?;
    let forge_id = parts.next()?;
    let project_id = parts.next()?.parse().ok()?;
    parts
        .next()
        .is_none()
        .then_some((forge, forge_id, project_id))
}

#[derive(Debug, Default)]
pub(crate) struct IssueCache {
    issues: BTreeMap<String, IssueRow>,
    syncs: BTreeMap<String, SyncRow>,
    rejected: Vec<(String, String)>,
}

impl IssueCache {
    pub(crate) fn len(&self) -> usize {
        self.issues.len()
    }

    pub(crate) fn rejected(&self) -> &[(String, String)] {
        &self.rejected
    }

    pub(crate) fn issues(&self) -> impl Iterator<Item = (&String, &IssueRow)> {
        self.issues.iter()
    }

    pub(crate) fn syncs(&self) -> impl Iterator<Item = (&String, &SyncRow)> {
        self.syncs.iter()
    }

    pub(crate) fn get(&self, key: &str) -> Option<&IssueRow> {
        self.issues.get(key)
    }

    /// Apply one change from a scan or a watch. Returns whether the cache
    /// changed. A document that does not match its key or breaks a limit is
    /// refused; keys that are neither issues nor syncs are not this cache's.
    pub(crate) fn apply(&mut self, event: &WatchEvent) -> bool {
        let name = &event.key.0;
        if let Some((forge, forge_id, project_id, number)) = parse_issue_key(name) {
            let Some(bytes) = &event.value else {
                return self.issues.remove(name).is_some();
            };
            let record = match serde_json::from_slice::<IssueRecord>(bytes) {
                Ok(record) => record,
                Err(err) => return self.reject(name, format!("unreadable: {err}")),
            };
            let r = &record.issue_ref;
            if r.forge != forge
                || r.forge_id != forge_id
                || r.project_id != project_id
                || r.number != number
            {
                return self.reject(name, "the document names another issue".into());
            }
            if let Err(why) = record.check_sizes() {
                return self.reject(name, why.into());
            }
            if self
                .issues
                .get(name)
                .is_some_and(|row| row.mirror_rev >= event.rev)
            {
                return false;
            }
            self.issues.insert(
                name.clone(),
                IssueRow {
                    mirror_rev: event.rev,
                    record,
                },
            );
            return true;
        }
        if let Some((forge, forge_id, project_id)) = parse_sync_key(name) {
            let Some(bytes) = &event.value else {
                return self.syncs.remove(name).is_some();
            };
            let record = match serde_json::from_slice::<SyncRecord>(bytes) {
                Ok(record) => record,
                Err(err) => return self.reject(name, format!("unreadable: {err}")),
            };
            if let Some(project) = &record.project {
                let p = &project.project_ref;
                if p.forge != forge || p.forge_id != forge_id || p.project_id != project_id {
                    return self.reject(name, "the document names another project".into());
                }
            }
            if self.syncs.get(name).is_some_and(|row| row.rev >= event.rev) {
                return false;
            }
            self.syncs.insert(
                name.clone(),
                SyncRow {
                    rev: event.rev,
                    record,
                },
            );
            return true;
        }
        false
    }

    fn reject(&mut self, name: &str, why: String) -> bool {
        self.rejected.push((name.to_owned(), why));
        if self.rejected.len() > REJECTED_KEPT {
            let excess = self.rejected.len() - REJECTED_KEPT;
            self.rejected.drain(..excess);
        }
        false
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
        assert!(cache.apply(&issue_event(
            "issue.gitea.gitea-lab.84.7",
            11,
            issue_doc(7, "first")
        )));
        assert!(cache.apply(&issue_event(
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
        assert!(cache.apply(&issue_event(
            "issue.gitea.gitea-lab.84.7",
            12,
            issue_doc(7, "first")
        )));
        assert_eq!(
            cache.get("issue.gitea.gitea-lab.84.7").unwrap().mirror_rev,
            Rev(12)
        );
        // An older or equal revision changes nothing.
        assert!(!cache.apply(&issue_event(
            "issue.gitea.gitea-lab.84.7",
            12,
            issue_doc(7, "other")
        )));
    }

    #[test]
    fn a_document_that_names_another_issue_or_breaks_a_limit_is_refused() {
        let mut cache = IssueCache::default();
        // Under the key of #7, a document for #8.
        assert!(!cache.apply(&issue_event(
            "issue.gitea.gitea-lab.84.7",
            1,
            issue_doc(8, "wrong")
        )));
        assert!(!cache.apply(&issue_event(
            "issue.gitea.gitea-lab.84.9",
            1,
            issue_doc(9, &"x".repeat(TITLE_MAX_CHARS + 1))
        )));
        assert!(!cache.apply(&WatchEvent {
            key: StoreKey("issue.gitea.gitea-lab.84.10".into()),
            rev: Rev(1),
            value: Some(b"{nope".to_vec()),
            at: None
        }));
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.rejected().len(), 3);
        // Not an issues key at all: not this cache's business.
        assert!(!cache.apply(&issue_event("mr.gitea.x.1.2", 1, issue_doc(1, "x"))));
    }

    #[test]
    fn a_removed_issue_leaves_the_cache_and_a_newer_state_is_unknown_not_dropped() {
        let mut cache = IssueCache::default();
        let mut doc = issue_doc(7, "state from the future");
        doc["state"] = serde_json::json!("merged");
        assert!(cache.apply(&issue_event("issue.gitea.gitea-lab.84.7", 1, doc)));
        assert_eq!(
            cache
                .get("issue.gitea.gitea-lab.84.7")
                .unwrap()
                .record
                .state,
            IssueState::Unknown
        );
        assert!(cache.apply(&WatchEvent {
            key: StoreKey("issue.gitea.gitea-lab.84.7".into()),
            rev: Rev(2),
            value: None,
            at: None
        }));
        assert_eq!(cache.len(), 0);
    }
}
