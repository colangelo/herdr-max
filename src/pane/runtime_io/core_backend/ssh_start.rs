//! Whether a core herdr connects to was started over SSH
//! (openspec/changes/archive/2026-11-22-core-phase3, D7).
//!
//! The release default herdr session shares core session `default` with the
//! Herdr apps and the phone, so its core may have been started by any of
//! them. Only positive evidence counts: a parent pid of 1 says nothing, since
//! a desktop start and a direct child whose launcher exited both have it.

/// The process facts the check reads, so a test can pass its own table.
pub(crate) trait ProcessTable {
    /// The session `pid` belongs to (`getsid`).
    fn session(&self, pid: u32) -> Option<u32>;
    fn name_and_parent(&self, pid: u32) -> Option<(String, u32)>;
}

/// The running system's processes.
pub(crate) struct LiveProcesses;

impl ProcessTable for LiveProcesses {
    fn session(&self, pid: u32) -> Option<u32> {
        crate::platform::process_session(pid)
    }

    fn name_and_parent(&self, pid: u32) -> Option<(String, u32)> {
        Some((
            crate::platform::process_name(pid)?,
            crate::platform::parent_pid(pid)?,
        ))
    }
}

/// The names an SSH login runs under: `sshd`, and `sshd-session` since
/// OpenSSH 9.8.
const SSH_LOGIN: [&str; 2] = ["sshd", "sshd-session"];

/// Bounds the walk up the parents, should a table ever loop.
const MAX_ANCESTORS: usize = 64;

/// True when the session leader of the core process `core_pid` is an SSH
/// login or has one as an ancestor. A leader that has exited proves nothing.
pub(crate) fn started_over_ssh(core_pid: u32, table: &impl ProcessTable) -> bool {
    let Some(mut pid) = table.session(core_pid) else {
        return false;
    };
    for _ in 0..MAX_ANCESTORS {
        if pid <= 1 {
            return false;
        }
        let Some((name, parent)) = table.name_and_parent(pid) else {
            return false;
        };
        if SSH_LOGIN.contains(&name.as_str()) {
            return true;
        }
        pid = parent;
    }
    false
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// pid -> (session, name, parent).
    struct FakeTable(HashMap<u32, (u32, &'static str, u32)>);

    impl FakeTable {
        fn new(rows: &[(u32, u32, &'static str, u32)]) -> Self {
            Self(
                rows.iter()
                    .map(|&(pid, session, name, parent)| (pid, (session, name, parent)))
                    .collect(),
            )
        }
    }

    impl ProcessTable for FakeTable {
        fn session(&self, pid: u32) -> Option<u32> {
            self.0.get(&pid).map(|row| row.0)
        }

        fn name_and_parent(&self, pid: u32) -> Option<(String, u32)> {
            self.0.get(&pid).map(|row| (row.1.to_string(), row.2))
        }
    }

    const LAUNCHD: (u32, u32, &str, u32) = (1, 1, "launchd", 0);

    #[test]
    fn a_core_in_an_ssh_login_session_was_started_over_ssh() {
        // The phone's SSH login: sshd-session leads the session the core
        // still belongs to, with its launcher gone (parent 1).
        let leader = FakeTable::new(&[LAUNCHD, (300, 300, "sshd-session", 1), (700, 300, "hc", 1)]);
        assert!(started_over_ssh(700, &leader));

        // The login shell leads the session; sshd-session is its parent.
        let shell = FakeTable::new(&[
            LAUNCHD,
            (290, 290, "sshd-session", 1),
            (300, 300, "zsh", 290),
            (700, 300, "hc", 1),
        ]);
        assert!(started_over_ssh(700, &shell));
    }

    #[test]
    fn parent_pid_one_alone_is_no_evidence() {
        // A desktop start: a one-shot launchd job, its own session leader.
        let desktop = FakeTable::new(&[LAUNCHD, (700, 700, "hc", 1)]);
        assert!(!started_over_ssh(700, &desktop));

        // A start from a desktop terminal whose shell has exited.
        let gone = FakeTable::new(&[LAUNCHD, (700, 410, "hc", 1)]);
        assert!(!started_over_ssh(700, &gone));

        // A start from a herdr pane: the pane's shell leads, herdr is its
        // parent, launchd runs herdr.
        let pane = FakeTable::new(&[
            LAUNCHD,
            (200, 200, "herdr", 1),
            (300, 300, "zsh", 200),
            (700, 300, "hc", 1),
        ]);
        assert!(!started_over_ssh(700, &pane));

        assert!(!started_over_ssh(700, &FakeTable::new(&[])));
    }

    #[test]
    fn a_looping_table_ends_the_walk() {
        let looped = FakeTable::new(&[(300, 300, "zsh", 301), (301, 301, "zsh", 300)]);
        assert!(!started_over_ssh(300, &looped));
    }
}
