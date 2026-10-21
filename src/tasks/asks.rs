//! ac's asks (bucket `ASKS`, contracts § 4), read by the server and never
//! written from here. Keys are `<gestore>.<id>` (worklink's rows
//! `<gestore>.wl.<id>`); a row is a decision, a rule or one of ac's own task
//! rows (`kind: "task"`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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
    fn check_sizes(&self) -> Result<(), &'static str> {
        let long = |text: &Option<String>| {
            text.as_deref()
                .is_some_and(|text| text.chars().count() > FIELD_MAX_CHARS)
        };
        if self.status.chars().count() > 64
            || long(&self.kind)
            || long(&self.title)
            || long(&self.how)
            || long(&self.where_)
            || long(&self.horizon)
            || long(&self.filed_by)
            || long(&self.answer)
            || long(&self.superseded_by)
        {
            return Err("a field is too long");
        }
        Ok(())
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

    /// Apply one change from a scan or a watch. Returns whether the cache
    /// changed.
    pub(crate) fn apply(&mut self, event: &WatchEvent) -> bool {
        let name = &event.key.0;
        let Some(bytes) = &event.value else {
            return self.rows.remove(name).is_some();
        };
        // A key is `<gestore>.<id>`; anything without a dot is not a row.
        if !name.contains('.') {
            return false;
        }
        let record = match serde_json::from_slice::<AskRecord>(bytes) {
            Ok(record) => record,
            Err(err) => return self.reject(name, format!("unreadable: {err}")),
        };
        if let Err(why) = record.check_sizes() {
            return self.reject(name, why.into());
        }
        if self.rows.get(name).is_some_and(|row| row.rev >= event.rev) {
            return false;
        }
        self.rows.insert(
            name.clone(),
            AskEntry {
                rev: event.rev,
                record,
            },
        );
        true
    }

    fn reject(&mut self, name: &str, why: String) -> bool {
        self.rejected.push((name.to_owned(), why));
        if self.rejected.len() > REJECTED_KEPT {
            let excess = self.rejected.len() - REJECTED_KEPT;
            self.rejected.drain(..excess);
        }
        false
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
        assert!(cache.apply(&event(
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
        cache.apply(&event("g.a", 1, row("open", None)));
        cache.apply(&event("g.b", 1, row("answered", None)));
        cache.apply(&event("g.c", 1, row("closed", None)));
        cache.apply(&event("g.d", 1, row("dropped", None)));
        cache.apply(&event("g.e", 1, row("paused", None)));
        // The desk re-filed g.old as g.a.
        cache.apply(&event("g.old", 1, row("dropped", Some("g.a"))));
        // A loop of replacements ends rather than runs for ever.
        cache.apply(&event("g.x", 1, row("open", Some("g.y"))));
        cache.apply(&event("g.y", 1, row("open", Some("g.x"))));

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
        assert!(!cache.apply(&WatchEvent {
            key: StoreKey("g.bad".into()),
            rev: Rev(1),
            value: Some(b"[]".to_vec()),
            at: None
        }));
        assert!(!cache.apply(&event(
            "g.long",
            1,
            serde_json::json!({"status": "open", "title": "x".repeat(FIELD_MAX_CHARS + 1)})
        )));
        assert_eq!(cache.rejected().len(), 2);
        cache.apply(&event("g.ok", 1, serde_json::json!({"status": "open"})));
        assert_eq!(cache.len(), 1);
        assert!(cache.apply(&WatchEvent {
            key: StoreKey("g.ok".into()),
            rev: Rev(2),
            value: None,
            at: None
        }));
        assert_eq!(cache.len(), 0);
    }
}
