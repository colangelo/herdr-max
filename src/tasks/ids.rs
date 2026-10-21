//! Identifiers: hosts, tasks, store keys, write ids and revisions.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

const HOST_ID_MAX_BYTES: usize = 63;
const TASK_ID_MAX_BYTES: usize = 64;

/// A stable machine name (`m4m`, `ac-mbm5`): lowercase letters, digits and `-`.
/// No dot, since the key `task.<host>.<id>` is split on dots.
pub(crate) fn is_valid_host_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= HOST_ID_MAX_BYTES
        && !value.starts_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct HostId(String);

impl HostId {
    pub(crate) fn new(value: &str) -> Option<Self> {
        is_valid_host_id(value).then(|| Self(value.to_owned()))
    }

    /// The owner name of a server with no `[tasks] host_id`.
    pub(crate) fn local() -> Self {
        Self("local".to_owned())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for HostId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// `tk-<ULID>`, or `tk-m-<hash>` for a migrated todo. One key token: letters,
/// digits and `-` only.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct TaskId(String);

impl TaskId {
    pub(crate) fn new(value: &str) -> Option<Self> {
        let rest = value.strip_prefix("tk-")?;
        // Lowercase only: on a case-insensitive disk `tk-ABC` and `tk-abc`
        // would be one file.
        let valid = !rest.is_empty()
            && value.len() <= TASK_ID_MAX_BYTES
            && value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        valid.then(|| Self(value.to_owned()))
    }

    pub(crate) fn generate() -> Self {
        Self(format!(
            "tk-{}",
            crate::ulid::new_ulid().to_ascii_lowercase()
        ))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The key of a task in the store: `task.<owner host>.<id>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct TaskKey {
    pub host: HostId,
    pub id: TaskId,
}

impl TaskKey {
    pub(crate) const PREFIX: &'static str = "task.";

    pub(crate) fn new(host: HostId, id: TaskId) -> Self {
        Self { host, id }
    }
}

impl fmt::Display for TaskKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "task.{}.{}", self.host, self.id)
    }
}

impl FromStr for TaskKey {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, ()> {
        let rest = value.strip_prefix(Self::PREFIX).ok_or(())?;
        let (host, id) = rest.split_once('.').ok_or(())?;
        Ok(Self {
            host: HostId::new(host).ok_or(())?,
            id: TaskId::new(id).ok_or(())?,
        })
    }
}

impl Serialize for TaskKey {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for TaskKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse()
            .map_err(|()| serde::de::Error::custom(format!("not a task key: {text}")))
    }
}

/// The id of one write (a ULID). The record keeps the last few applied. Always
/// kept in lower case, whoever wrote it: ids name files on a case-insensitive
/// disk, and two spellings of one id must compare equal.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(from = "String")]
pub(crate) struct WriteId(String);

impl From<String> for WriteId {
    fn from(mut value: String) -> Self {
        value.make_ascii_lowercase();
        Self(value)
    }
}

impl WriteId {
    pub(crate) fn generate() -> Self {
        Self::from(crate::ulid::new_ulid())
    }

    pub(crate) fn from_string(value: String) -> Self {
        Self::from(value)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WriteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A store revision: the version of one key. Each backend counts its own.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub(crate) struct Rev(pub u64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_keys_round_trip_and_refuse_bad_parts() {
        let key: TaskKey = "task.ac-mbm5.tk-01j0".parse().unwrap();
        assert_eq!(key.host.as_str(), "ac-mbm5");
        assert_eq!(key.id.as_str(), "tk-01j0");
        assert_eq!(key.to_string(), "task.ac-mbm5.tk-01j0");
        let json = serde_json::to_string(&key).unwrap();
        assert_eq!(json, "\"task.ac-mbm5.tk-01j0\"");
        assert_eq!(serde_json::from_str::<TaskKey>(&json).unwrap(), key);

        for bad in [
            "run.m4m.tk-1",
            "task.m4m",
            "task.M4M.tk-1",
            "task.m4m.tk-1.extra",
            "task.m4m.1",
            "task..tk-1",
            "task.m4m.tk-",
            "task.m4m.tk-ABC",
        ] {
            assert!(bad.parse::<TaskKey>().is_err(), "{bad}");
        }
        assert!(HostId::new("m4m-2").is_some());
        assert!(HostId::new("-m4m").is_none());
        assert!(HostId::new(&"a".repeat(64)).is_none());
    }

    #[test]
    fn generated_ids_are_valid_and_unique() {
        let first = TaskId::generate();
        assert!(TaskId::new(first.as_str()).is_some(), "{first}");
        assert_ne!(first, TaskId::generate());
        assert_ne!(WriteId::generate(), WriteId::generate());
    }

    #[test]
    fn a_write_id_has_one_spelling_however_it_arrives() {
        let made = WriteId::generate();
        assert_eq!(made.as_str(), made.as_str().to_ascii_lowercase());
        let shouted: WriteId = serde_json::from_str("\"01J0ABCDEF\"").unwrap();
        assert_eq!(shouted, WriteId::from_string("01j0abcdef".into()));
        assert_eq!(serde_json::to_string(&shouted).unwrap(), "\"01j0abcdef\"");
    }
}
