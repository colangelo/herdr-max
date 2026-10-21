//! ac's asks (bucket `ASKS`, contracts § 4), read by the server and never
//! written from here. Keys are `<gestore>.<id>` (worklink's rows
//! `<gestore>.wl.<id>`); a row is a decision, a rule or one of ac's own task
//! rows (`kind: "task"`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::clean::{clean, clean_opt};
use super::ids::Rev;
use super::runs::AskRow;
use super::store::WatchEvent;

const REJECTED_KEPT: usize = 100;
const FIELD_MAX_CHARS: usize = 4000;
/// How many `superseded_by` hops a reader follows.
const SUPERSEDED_HOPS: usize = 8;

/// The fields of an asks row that this server shows. The row has more (the
/// options, the gate, the delivery); they are not read here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AskRecord {
    /// `open`, `answered`, `closed` or `dropped`.
    pub status: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub how: Option<String>,
    #[serde(default, rename = "where")]
    pub where_: Option<String>,
    #[serde(default)]
    pub horizon: Option<String>,
    #[serde(default)]
    pub filed_by: Option<String>,
    #[serde(default)]
    pub answer: Option<String>,
    #[serde(default)]
    pub answered_at: Option<String>,
    #[serde(default)]
    pub superseded_by: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

impl AskRecord {
    /// Clean every string and cut what is too long: a long answer is still
    /// the row ac wrote.
    fn sanitize(&mut self) {
        self.status = clean(&self.status, 64).0;
        for field in [
            &mut self.kind,
            &mut self.title,
            &mut self.how,
            &mut self.where_,
            &mut self.horizon,
            &mut self.filed_by,
            &mut self.answer,
            &mut self.answered_at,
            &mut self.superseded_by,
            &mut self.source,
            &mut self.created_at,
            &mut self.updated_at,
        ] {
            clean_opt(field, FIELD_MAX_CHARS);
        }
    }

    pub(crate) fn is_task(&self) -> bool {
        self.kind.as_deref() == Some("task")
    }

    /// What a task waiting on this row sees.
    fn state(&self) -> AskRow {
        match self.status.as_str() {
            "open" => AskRow::Open,
            "answered" => AskRow::Answered,
            "closed" | "dropped" => AskRow::Closed,
            _ => AskRow::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AskEntry {
    pub rev: Rev,
    pub record: AskRecord,
}

/// One watch event, read and cleaned off the app's thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AskChange {
    Row {
        name: String,
        rev: Rev,
        record: Box<AskRecord>,
    },
    Gone {
        name: String,
    },
    /// A document that cannot be used; the older row for its key goes too.
    Rejected {
        name: String,
        why: String,
    },
}

/// Read one event. `None` for a key that is not a row (no gestore prefix).
pub(crate) fn parse_event(event: &WatchEvent) -> Option<AskChange> {
    let name = &event.key.0;
    // A key is `<gestore>.<id>`; anything without a dot is not a row.
    if !name.contains('.') {
        return None;
    }
    let Some(bytes) = &event.value else {
        return Some(AskChange::Gone { name: name.clone() });
    };
    match serde_json::from_slice::<AskRecord>(bytes) {
        Ok(mut record) => {
            record.sanitize();
            Some(AskChange::Row {
                name: name.clone(),
                rev: event.rev,
                record: Box::new(record),
            })
        }
        Err(err) => Some(AskChange::Rejected {
            name: name.clone(),
            why: format!("unreadable: {err}"),
        }),
    }
}

#[derive(Debug, Default)]
pub(crate) struct AskCache {
    rows: BTreeMap<String, AskEntry>,
    rejected: Vec<(String, String)>,
}

impl AskCache {
    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    pub(crate) fn rejected(&self) -> &[(String, String)] {
        &self.rejected
    }

    pub(crate) fn rows(&self) -> impl Iterator<Item = (&String, &AskEntry)> {
        self.rows.iter()
    }

    /// Forget everything: the bucket is read again from its start.
    pub(crate) fn clear(&mut self) -> bool {
        let any = !self.rows.is_empty();
        self.rows.clear();
        any
    }

    /// Read and apply one event, together (for tests).
    #[cfg(test)]
    pub(crate) fn apply_event(&mut self, event: &WatchEvent) -> bool {
        parse_event(event).is_some_and(|change| self.apply(change))
    }

    /// Apply one change. Returns whether the cache changed.
    pub(crate) fn apply(&mut self, change: AskChange) -> bool {
        match change {
            AskChange::Row { name, rev, record } => {
                if self.rows.get(&name).is_some_and(|row| row.rev >= rev) {
                    return false;
                }
                self.rows.insert(
                    name,
                    AskEntry {
                        rev,
                        record: *record,
                    },
                );
                true
            }
            AskChange::Gone { name } => self.rows.remove(&name).is_some(),
            AskChange::Rejected { name, why } => {
                tracing::warn!(key = %name, %why, "an ask row was refused");
                self.rejected.push((name.clone(), why));
                if self.rejected.len() > REJECTED_KEPT {
                    let excess = self.rejected.len() - REJECTED_KEPT;
                    self.rejected.drain(..excess);
                }
                self.rows.remove(&name).is_some()
            }
        }
    }

    /// How the row `key` looks to a task that waits on it: a row the desk
    /// re-filed is followed through `superseded_by`; a row that does not exist
    /// (any more) is `Missing`.
    pub(crate) fn state_of(&self, key: &str) -> AskRow {
        let mut key = key;
        for _ in 0..SUPERSEDED_HOPS {
            let Some(entry) = self.rows.get(key) else {
                return AskRow::Missing;
            };
            match entry.record.superseded_by.as_deref() {
                Some(next) if next != key => key = next,
                _ => return entry.record.state(),
            }
        }
        AskRow::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::store::StoreKey;

    fn event(key: &str, rev: u64, doc: serde_json::Value) -> WatchEvent {
        WatchEvent {
            key: StoreKey(key.into()),
            rev: Rev(rev),
            value: Some(serde_json::to_vec(&doc).unwrap()),
            at: None,
        }
    }

    #[test]
    fn a_row_is_read_for_the_fields_this_server_shows() {
        let mut cache = AskCache::default();
        assert!(cache.apply_event(&event(
            "gestore-lab.t1021-01",
            3,
            serde_json::json!({
                "id": "t1021-01", "gestore": "gestore-lab", "status": "open", "kind": "task",
                "title": "review the port", "how": "read the diff", "where": "herdr-max #241",
                "horizon": "now", "filed_by": "herdr-wrk",
                "options": [{"label": "Done"}], "free_text": false
            })
        )));
        let (key, row) = cache.rows().next().unwrap();
        assert_eq!(key, "gestore-lab.t1021-01");
        assert!(row.record.is_task());
        assert_eq!(row.record.where_.as_deref(), Some("herdr-max #241"));
        assert_eq!(row.rev, Rev(3));
    }

    #[test]
    fn what_a_waiting_task_sees_follows_the_row_and_its_replacement() {
        let mut cache = AskCache::default();
        let row = |status: &str, superseded: Option<&str>| {
            let mut doc = serde_json::json!({"status": status});
            if let Some(next) = superseded {
                doc["superseded_by"] = serde_json::json!(next);
            }
            doc
        };
        cache.apply_event(&event("g.a", 1, row("open", None)));
        cache.apply_event(&event("g.b", 1, row("answered", None)));
        cache.apply_event(&event("g.c", 1, row("closed", None)));
        cache.apply_event(&event("g.d", 1, row("dropped", None)));
        cache.apply_event(&event("g.e", 1, row("paused", None)));
        // The desk re-filed g.old as g.a.
        cache.apply_event(&event("g.old", 1, row("dropped", Some("g.a"))));
        // A loop of replacements ends rather than runs for ever.
        cache.apply_event(&event("g.x", 1, row("open", Some("g.y"))));
        cache.apply_event(&event("g.y", 1, row("open", Some("g.x"))));

        assert_eq!(cache.state_of("g.a"), AskRow::Open);
        assert_eq!(cache.state_of("g.b"), AskRow::Answered);
        assert_eq!(cache.state_of("g.c"), AskRow::Closed);
        assert_eq!(cache.state_of("g.d"), AskRow::Closed);
        assert_eq!(cache.state_of("g.e"), AskRow::Unknown);
        assert_eq!(cache.state_of("g.old"), AskRow::Open, "followed");
        assert_eq!(cache.state_of("g.nope"), AskRow::Missing);
        assert_eq!(cache.state_of("g.x"), AskRow::Unknown);
    }

    #[test]
    fn damaged_or_oversized_rows_are_refused_and_removals_apply() {
        let mut cache = AskCache::default();
        assert!(!cache.apply_event(&WatchEvent {
            key: StoreKey("g.bad".into()),
            rev: Rev(1),
            value: Some(b"[]".to_vec()),
            at: None
        }));
        // Too long is cut, not refused.
        assert!(cache.apply_event(&event(
            "g.long",
            1,
            serde_json::json!({"status": "open", "title": "x\u{1b}".repeat(FIELD_MAX_CHARS + 1)})
        )));
        let title = cache.rows().next().unwrap().1.record.title.clone().unwrap();
        assert_eq!(title.chars().count(), FIELD_MAX_CHARS);
        assert!(!title.chars().any(char::is_control));
        assert_eq!(cache.rejected().len(), 1);
        cache.apply_event(&event("g.ok", 1, serde_json::json!({"status": "open"})));
        assert_eq!(cache.len(), 2);
        // A damaged document takes the older row of its key with it.
        assert!(cache.apply_event(&WatchEvent {
            key: StoreKey("g.long".into()),
            rev: Rev(2),
            value: Some(b"[]".to_vec()),
            at: None
        }));
        assert_eq!(cache.len(), 1);
        assert!(cache.apply_event(&WatchEvent {
            key: StoreKey("g.ok".into()),
            rev: Rev(2),
            value: None,
            at: None
        }));
        assert_eq!(cache.len(), 0);
    }
}
