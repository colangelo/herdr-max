//! What a herdr server knows of the core machines its panes run on
//! (core-phase5 D5, D3a). Core sends no event when a machine's link drops
//! (P5), so the server reads `machine.list` while a lease allows a link,
//! and marks each machine here; a pane on a machine that is not connected
//! sends no input and posts a wake to this process's dialer instead.
//!
//! The input path reads one atomic per batch and, at most once per
//! `WAKE_EVERY`, does a `try_send`; every core call and file read happens on
//! the dialer's thread or the server's probe thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::json;

use super::dialer::{self, Dialer, PrimaryCore};
use super::name::MachineName;
use super::state::{now_unix, passwd_machines_root, MachineDir};
use crate::pane::core_machines as core;

/// The most wakes one machine's panes post, so a person typing into a pane
/// whose machine is away starts one dial, not one per key.
const WAKE_EVERY: Duration = Duration::from_secs(5);

/// One machine as this process sees it.
pub(crate) struct MachineLink {
    name: MachineName,
    connected: AtomicBool,
    last_wake: Mutex<Option<Instant>>,
    dialer: Mutex<Option<Dialer>>,
}

static LINKS: Mutex<Vec<Arc<MachineLink>>> = Mutex::new(Vec::new());

/// The process's record of `name`, made on first use. A machine a pane
/// just started on or attached to is connected: core answered for it.
pub(crate) fn link(name: &MachineName) -> Arc<MachineLink> {
    let mut links = LINKS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(found) = links.iter().find(|link| &link.name == name) {
        return Arc::clone(found);
    }
    let made = Arc::new(MachineLink {
        name: name.clone(),
        connected: AtomicBool::new(true),
        last_wake: Mutex::new(None),
        dialer: Mutex::new(None),
    });
    links.push(Arc::clone(&made));
    made
}

impl MachineLink {
    pub(crate) fn connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    /// Records what core said; true when it changed.
    pub(crate) fn set_connected(&self, connected: bool) -> bool {
        self.connected.swap(connected, Ordering::AcqRel) != connected
    }

    /// Input reached a pane on this machine while it is away (D3, D5):
    /// posts a wake to the dialer and returns at once. Only a machine herdr
    /// links is dialled (its folder holds `target.json`); `core-link` still
    /// refuses unless a lease is in force, which the person's client writes
    /// for its own input.
    pub(crate) fn wake(self: &Arc<Self>) {
        let now = Instant::now();
        {
            let mut last = self
                .last_wake
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if last.is_some_and(|at| now.saturating_duration_since(at) < WAKE_EVERY) {
                return;
            }
            *last = Some(now);
        }
        let mut dialer = self.dialer.lock().unwrap_or_else(PoisonError::into_inner);
        if dialer.is_none() {
            *dialer = self.spawn_dialer();
        }
        if let Some(dialer) = dialer.as_ref() {
            dialer.wake();
        }
    }

    fn spawn_dialer(self: &Arc<Self>) -> Option<Dialer> {
        let root = passwd_machines_root().ok()?;
        let dir = MachineDir::at(&root, &self.name);
        if dir.check().is_err() || !dir.target_path().is_file() {
            tracing::debug!(machine = %self.name, "not a machine herdr links; core redials it itself");
            return None;
        }
        let made = Dialer::spawn(
            self.name.clone(),
            dir.connect_lock_path(),
            PrimaryCore,
            dialer::Timing::default(),
            |machine, outcome| {
                tracing::info!(%machine, ?outcome, "core machine wake for a pane");
                let state = outcome.state().map(str::to_string);
                link(machine).set_connected(state.as_deref() == Some("connected"));
                core::tell_server(crate::events::AppEvent::CoreMachines {
                    probes: vec![Probe {
                        machine: machine.clone(),
                        state,
                        leased: true,
                    }],
                });
            },
        );
        match made {
            Ok(dialer) => Some(dialer),
            Err(err) => {
                tracing::warn!(machine = %self.name, %err, "no dialer for the machine's panes");
                None
            }
        }
    }
}

/// What the server read of one machine (D5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Probe {
    pub machine: MachineName,
    /// Its state in `machine.list`; `None` when it was not read (dormant)
    /// or core does not list it.
    pub state: Option<String>,
    /// A lease allows its link now, or herdr does not gate it (a machine
    /// added by hand, whose link core keeps by itself).
    pub leased: bool,
}

impl Probe {
    pub(crate) fn connected(&self) -> bool {
        self.state.as_deref() == Some("connected")
    }
}

/// Whether `machine`'s link may be up now: a lease in force, or no herdr
/// folder with a `target.json` (herdr does not gate that machine). Reads
/// files: never on the server loop.
pub(crate) fn leased(root: Option<&std::path::Path>, machine: &MachineName) -> bool {
    let Some(root) = root else {
        return true;
    };
    let dir = MachineDir::at(root, machine);
    if !dir.target_path().is_file() {
        return true;
    }
    if dir.check().is_err() {
        return false;
    }
    let now = now_unix();
    dir.leases().iter().any(|lease| lease.in_force(now))
}

/// Whether `machine` is dormant: herdr links it and no lease allows its
/// link now. Reads files.
pub(crate) fn dormant(machine: &MachineName) -> bool {
    !leased(passwd_machines_root().ok().as_deref(), machine)
}

/// One look at `machines`: leases first, then one `machine.list` when any
/// may be up. A dormant machine is not asked about (D5: the server reads
/// `machine.list` only while some lease is in force). Blocks on files and
/// the primary core: call it on a thread of its own.
pub(crate) fn probe(machines: &[MachineName]) -> Vec<Probe> {
    let root = passwd_machines_root().ok();
    let mut probes: Vec<Probe> = machines
        .iter()
        .map(|machine| Probe {
            machine: machine.clone(),
            state: None,
            leased: leased(root.as_deref(), machine),
        })
        .collect();
    if !probes.iter().any(|probe| probe.leased) {
        return probes;
    }
    let listed = match core::call_running_primary("machine.list", json!({}), Duration::from_secs(5))
    {
        Ok(Some(listed)) => listed,
        Ok(None) => return probes,
        Err(err) => {
            tracing::debug!(%err, "machine.list failed; machine panes keep their state");
            return probes;
        }
    };
    for probe in probes.iter_mut().filter(|probe| probe.leased) {
        probe.state = listed["machines"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|record| record["name"] == probe.machine.as_str())
            .and_then(|record| record["state"].as_str())
            .map(str::to_string);
    }
    probes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_machine::state::LeaseHolder;

    fn name(raw: &str) -> MachineName {
        MachineName::parse(raw).expect("name")
    }

    #[test]
    fn a_machine_herdr_does_not_link_is_always_leased() {
        let root = crate::core_machine::state::tests::scratch("panes-ungated");
        assert!(leased(Some(&root), &name("byhand")));
        assert!(leased(None, &name("byhand")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_linked_machine_is_leased_only_while_a_lease_is_in_force() {
        let root = crate::core_machine::state::tests::scratch("panes-gated");
        let machine = name("loop");
        let dir = MachineDir::at(&root, &machine);
        dir.create().expect("create");
        std::fs::write(dir.target_path(), b"{}").expect("target");
        assert!(!leased(Some(&root), &machine), "no lease");
        let now = now_unix();
        dir.write_lease(&LeaseHolder::manual(), now - 1.0, now - 60.0)
            .expect("expired");
        assert!(!leased(Some(&root), &machine), "an expired lease");
        dir.write_lease(&LeaseHolder::client(1), now + 60.0, now)
            .expect("in force");
        assert!(leased(Some(&root), &machine));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_link_starts_connected_and_says_when_it_changes() {
        let link = link(&name("panes-test-link"));
        assert!(link.connected());
        assert!(!link.set_connected(true));
        assert!(link.set_connected(false));
        assert!(!link.connected());
        assert!(super::link(&name("panes-test-link")).set_connected(true));
    }
}
