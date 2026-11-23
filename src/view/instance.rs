//! Which far program a view's pane is bound to (security review M3): the
//! far herdr's launch label and the program's pid, as core reported them
//! at the pane's first attach. A far core reset can give the same address
//! (`t2`) to another program, so the address alone names no program: the
//! stream's reattach takes only this one, and a structural action on the
//! pane is sent only while core still shows this one there (E.3 review,
//! item 1).

use serde_json::Value;

/// The label the far herdr puts on each terminal it starts, one value per
/// launch.
pub(crate) const LAUNCH_LABEL: &str = "herdr.launch";

/// What tells one far terminal from another at the same address. Either
/// part missing is not compared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FarInstance {
    launch: Value,
    pid: u32,
}

impl FarInstance {
    /// The instance a core terminal record (`terminal.get`, an attach's
    /// `terminal`) describes.
    pub(crate) fn of(record: &Value) -> Self {
        Self {
            launch: record["labels"][LAUNCH_LABEL].clone(),
            pid: pid(&record["pid"]),
        }
    }

    /// An instance core told nothing of.
    pub(crate) fn unknown() -> Self {
        Self {
            launch: Value::Null,
            pid: 0,
        }
    }

    /// Whether `record` is this terminal: the same launch label and pid
    /// where the first attach had them.
    pub(crate) fn matches(&self, record: &Value) -> bool {
        (self.launch.is_null() || record["labels"][LAUNCH_LABEL] == self.launch)
            && (self.pid == 0 || pid(&record["pid"]) == self.pid)
    }

    /// Whether this tells a program apart at all: an instance with neither
    /// part matches every record, so it vouches for none.
    pub(crate) fn is_known(&self) -> bool {
        !self.launch.is_null() || self.pid != 0
    }
}

/// A pid as core reports it, 0 for none or one out of range.
fn pid(value: &Value) -> u32 {
    value
        .as_u64()
        .filter(|pid| (1..=i32::MAX as u64).contains(pid))
        .unwrap_or(0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_instance_is_its_launch_label_and_pid() {
        let first = FarInstance::of(&json!({"pid": 100, "labels": {LAUNCH_LABEL: "l1"}}));
        assert!(first.is_known());
        assert!(first.matches(&json!({"pid": 100, "labels": {LAUNCH_LABEL: "l1"}})));
        assert!(!first.matches(&json!({"pid": 200, "labels": {LAUNCH_LABEL: "l1"}})));
        assert!(!first.matches(&json!({"pid": 100, "labels": {LAUNCH_LABEL: "l2"}})));
        let unknown = FarInstance::of(&json!({"state": "live"}));
        assert_eq!(unknown, FarInstance::unknown());
        assert!(!unknown.is_known(), "it would match any program");
        assert!(unknown.matches(&json!({"pid": 5})));
    }
}
