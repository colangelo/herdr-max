//! Which far program a view's pane is bound to (security review M3): the
//! far herdr's launch label and the program's pid, as core reported them
//! at the pane's first attach. A far core reset can give the same address
//! (`t2`) to another program, so the address alone names no program: the
//! stream's reattach takes only this one, and a structural action on the
//! pane is sent only while core still shows this one there (E.3 review,
//! item 1), and only with both parts known, for the far herdr to check
//! again (fork issue 316, M1).

use serde_json::Value;

/// The label the far herdr puts on each terminal it starts, one value per
/// launch.
pub(crate) const LAUNCH_LABEL: &str = "herdr.launch";

/// What tells one far terminal from another at the same address. Either
/// part missing is not compared by the stream's reattach; a structural
/// action needs both (`expectation`).
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

    /// Whether this tells a program apart for a structural action: both
    /// the launch label and the pid known (security delta re-check of PR
    /// 100, item 1). With one part only, `matches` compares that part, so
    /// another program with the same pid, or another run's terminal that
    /// kept the label, would pass.
    pub(crate) fn is_known(&self) -> bool {
        self.launch.is_string() && self.pid != 0
    }

    /// Whether core knows the run (it has a launch label) but reports no
    /// process for it: the far program ended before the view attached to it.
    pub(crate) fn has_ended(&self) -> bool {
        self.launch.is_string() && self.pid == 0
    }

    /// What a guarded far pane action names for the far herdr to check
    /// (`pane.close_if_terminal` and the others, fork issue 316): the far
    /// core's address of the terminal, the pid and the launch label. `None`
    /// unless both parts are known.
    pub(crate) fn expectation(
        &self,
        terminal: &str,
    ) -> Option<crate::api::schema::PaneTerminalExpectation> {
        let launch = self.launch.as_str().filter(|_| self.pid != 0)?;
        Some(crate::api::schema::PaneTerminalExpectation {
            terminal: terminal.to_string(),
            pid: self.pid,
            launch: launch.to_string(),
        })
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
        assert_eq!(unknown.expectation("t1"), None);
        let expect = first.expectation("t7").expect("both parts");
        assert_eq!(
            (expect.terminal.as_str(), expect.pid, expect.launch.as_str()),
            ("t7", 100, "l1")
        );
    }

    // Security delta re-check of PR 100, item 1: an attach that learned only
    // the label or only the pid vouches for no program. Either half alone
    // matches another program that shares it, so neither is known, and no
    // structural action names it.
    #[test]
    fn a_one_sided_instance_is_not_known() {
        let label_only = FarInstance::of(&json!({"labels": {LAUNCH_LABEL: "l1"}}));
        let pid_only = FarInstance::of(&json!({"pid": 100}));
        let not_a_label = FarInstance::of(&json!({"pid": 100, "labels": {LAUNCH_LABEL: 7}}));
        for one_sided in [label_only, pid_only, not_a_label] {
            assert!(!one_sided.is_known(), "{one_sided:?}");
            assert_eq!(one_sided.expectation("t1"), None, "{one_sided:?}");
        }
    }
}
