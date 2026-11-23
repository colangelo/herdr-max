//! A TUI client's lease on its primary's core machines (core-phase5 D3,
//! task B.2): human input keeps the machines' links allowed for 10 min.
//! The client loop only stamps the time of each input; at most once a
//! minute that stamp goes to a writer thread, which writes
//! `leases/client-<pid>` in the folder of each machine this client's
//! primary core lists and then posts a wake to that machine's dialer
//! (D3a), so the lease always comes before the connect. Starting a client
//! writes nothing: opening herdr dials no machine (D3). Nothing here runs
//! on the input, render or PTY path but an `Instant` comparison and a
//! `try_send`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, Instant};

use super::command::{linked_machines_in, listed_by_core};
use super::dialer::{self, Dialer, PrimaryCore};
use super::name::MachineName;
use super::state::{now_unix, passwd_machines_root, LeaseHolder, MachineDir};

/// A client's lease lasts this long after the input that wrote it: #260's
/// `REMOTE_DORMANT_AFTER`, so the link sleeps when the client does.
pub(crate) const CLIENT_LEASE_SECS: f64 =
    crate::client::endpoint::REMOTE_DORMANT_AFTER.as_secs() as f64;
/// At most one lease write per client per this long (V24).
pub(crate) const WRITE_EVERY: Duration = Duration::from_secs(60);
/// Set (to anything) in a client's environment, the client writes no lease
/// at all. Test harnesses and live scripts set it for every client that is
/// not itself the subject of a lease test, so no test client allows a link
/// (core-phase5 H7). Unset, as for every person's client, leases are written.
pub(crate) const NO_LEASES_ENV: &str = "HERDR_NO_MACHINE_LEASES";

/// When the next lease write is due: the first input, then the first input
/// a minute or more after the last write. Pure, so V24 is a unit test.
#[derive(Debug, Default)]
pub(crate) struct LeaseClock {
    last_write: Option<Instant>,
}

impl LeaseClock {
    /// Whether this input writes the lease.
    pub(crate) fn input(&mut self, now: Instant) -> bool {
        let due = self
            .last_write
            .is_none_or(|last| now.saturating_duration_since(last) >= WRITE_EVERY);
        if due {
            self.last_write = Some(now);
        }
        due
    }
}

/// The client's side: a clock and the writer thread's channel.
pub(crate) struct ClientPresence {
    clock: LeaseClock,
    writes: Option<SyncSender<f64>>,
}

impl ClientPresence {
    /// The client's lease writer. It writes on input only: a client's start,
    /// a person's command included, is no input, so opening herdr leaves
    /// every dormant machine dormant (D3, review M3).
    pub(crate) fn start() -> Self {
        // Unit tests that run a client loop, and every client whose
        // environment opts out, write no lease into the account's machine
        // folders.
        if cfg!(test) || std::env::var_os(NO_LEASES_ENV).is_some() {
            return Self {
                clock: LeaseClock::default(),
                writes: None,
            };
        }
        let (writes, requests) = mpsc::sync_channel::<f64>(1);
        let writes = match std::thread::Builder::new()
            .name("herdr-machine-lease".into())
            .spawn(move || write_leases(requests, std::process::id()))
        {
            Ok(_) => Some(writes),
            Err(err) => {
                tracing::warn!(%err, "no core machine lease writer");
                None
            }
        };
        Self {
            clock: LeaseClock::default(),
            writes,
        }
    }

    /// A key, text, paste or mouse input (#260's `InputClock` events).
    pub(crate) fn input(&mut self, now: Instant) {
        if self.clock.input(now) {
            if let Some(writes) = &self.writes {
                // Full: a write is pending and carries a fresh enough time.
                let _ = writes.try_send(now_unix());
            }
        }
    }
}

/// The writer thread: one pass over the machines per input stamp.
fn write_leases(requests: Receiver<f64>, pid: u32) {
    let holder = LeaseHolder::client(pid);
    let mut dialers: HashMap<MachineName, Dialer> = HashMap::new();
    while let Ok(input_at) = requests.recv() {
        let Ok(root) = passwd_machines_root() else {
            continue;
        };
        for name in lease_pass(&root, primary_listing, &holder, input_at) {
            let dir = MachineDir::at(&root, &name);
            let dialer = match dialers.entry(name.clone()) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    match Dialer::spawn(
                        name.clone(),
                        dir.connect_lock_path(),
                        PrimaryCore,
                        dialer::Timing::default(),
                        |machine, outcome| {
                            tracing::info!(%machine, ?outcome, "core machine wake");
                        },
                    ) {
                        Ok(dialer) => entry.insert(dialer),
                        Err(err) => {
                            tracing::warn!(machine = %name, %err, "no dialer");
                            continue;
                        }
                    }
                }
            };
            dialer.wake();
        }
    }
}

/// This client's primary core's `machine.list`, `None` when no primary
/// core runs or it does not answer within a second. Never starts a core.
fn primary_listing() -> Option<serde_json::Value> {
    crate::pane::core_machines::call_running_primary(
        "machine.list",
        serde_json::json!({}),
        Duration::from_secs(1),
    )
    .ok()
    .flatten()
}

/// Writes `holder`'s lease into the folder of each machine herdr links
/// under `root` that this client's own primary core lists (`listing`, asked
/// only when some machine is linked), and returns the ones written, to be
/// woken. A folder of another session's core, a scratch core's or a test's
/// is not this client's to allow (review H7).
fn lease_pass(
    root: &Path,
    listing: impl FnOnce() -> Option<serde_json::Value>,
    holder: &LeaseHolder,
    input_at: f64,
) -> Vec<MachineName> {
    let linked = linked_machines_in(root, now_unix());
    if linked.is_empty() {
        return Vec::new();
    }
    let Some(listed) = listing() else {
        return Vec::new();
    };
    let machines = listed_by_core(linked, &listed);
    let own_pid = holder.pid().unwrap_or(0);
    let mut written = Vec::new();
    for machine in &machines {
        let dir = MachineDir::at(root, &machine.name);
        if dir.check().is_err() || !dir.target_path().is_file() {
            continue;
        }
        if let Err(err) = dir.write_lease(holder, input_at + CLIENT_LEASE_SECS, input_at) {
            tracing::warn!(machine = %machine.name, %err, "could not write the client's machine lease");
            continue;
        }
        prune_dead_clients(&dir, own_pid);
        written.push(machine.name.clone());
    }
    written
}

/// Removes the expired lease files (`client-` and `cli-`) of processes that no longer run, so the
/// folder does not fill with them. A live client's file is never touched:
/// only its own process writes it (D3).
fn prune_dead_clients(dir: &MachineDir, own_pid: u32) {
    let now = now_unix();
    for lease in dir.leases() {
        let Some(pid) = lease.holder.pid() else {
            continue;
        };
        if pid != own_pid && !lease.in_force(now) && !crate::platform::core_link::process_alive(pid)
        {
            let _ = dir.remove_lease(&lease.holder);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// V24's count: inputs every 100 ms for ten minutes write ten leases,
    /// one a minute; the first input writes at once.
    #[test]
    fn at_most_one_lease_write_a_minute() {
        let start = Instant::now();
        let mut clock = LeaseClock::default();
        let writes: Vec<Duration> = (0..6_000)
            .map(|step| start + Duration::from_millis(step * 100))
            .filter(|&at| clock.input(at))
            .map(|at| at - start)
            .collect();
        assert_eq!(writes.len(), 10, "{writes:?}");
        assert_eq!(writes[0], Duration::ZERO);
        for pair in writes.windows(2) {
            assert!(pair[1] - pair[0] >= WRITE_EVERY, "{pair:?}");
        }
    }

    #[test]
    fn the_first_input_after_a_long_idle_writes_at_once() {
        let start = Instant::now();
        let mut clock = LeaseClock::default();
        assert!(clock.input(start));
        assert!(!clock.input(start + Duration::from_secs(59)));
        assert!(clock.input(start + Duration::from_secs(11 * 60)));
    }

    #[test]
    fn the_lease_lasts_as_long_as_remote_dormancy_waits() {
        assert_eq!(CLIENT_LEASE_SECS, 600.0);
    }

    fn linked(root: &Path, name: &str) -> MachineDir {
        let dir = MachineDir::at(root, &MachineName::parse(name).expect("name"));
        dir.create().expect("create");
        dir.write_target(&crate::core_machine::state::Target {
            target: format!("ac@{name}"),
            session: "far".into(),
            ssh: "/usr/bin/ssh".into(),
            ssh_config: None,
            ssh_auth_sock: None,
        })
        .expect("target");
        dir
    }

    // H7: a client writes its lease only into the folders of the machines
    // its own primary core lists. A folder herdr links that the primary
    // does not list (another session's core, a scratch or test core's)
    // gets nothing, however the client's input comes.
    #[test]
    fn a_client_leases_only_machines_its_primary_lists() {
        let root = crate::core_machine::state::tests::scratch("primary-only");
        let listed = linked(&root, "listed");
        let other = linked(&root, "other-session");
        let primary = json!({"machines": [{"name": "listed", "state": "connected"}]});
        let holder = LeaseHolder::client(std::process::id());
        let written = lease_pass(&root, || Some(primary), &holder, now_unix());
        assert_eq!(
            written,
            [MachineName::parse("listed").expect("name")],
            "only the listed machine is woken"
        );
        let holders = |dir: &MachineDir| -> Vec<String> {
            dir.leases()
                .into_iter()
                .map(|lease| lease.holder.as_str().to_string())
                .collect()
        };
        assert_eq!(holders(&listed), [holder.as_str().to_string()]);
        assert!(holders(&other).is_empty(), "{:?}", holders(&other));
        // No primary core answering lists nothing: no lease anywhere.
        let _ = listed.remove_lease(&holder);
        assert!(lease_pass(&root, || None, &holder, now_unix()).is_empty());
        assert!(lease_pass(&root, || Some(json!({})), &holder, now_unix()).is_empty());
        assert!(holders(&listed).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn only_dead_clients_expired_leases_are_pruned() {
        let root = crate::core_machine::state::tests::scratch("prune");
        let dir = MachineDir::at(&root, &MachineName::parse("loop").expect("name"));
        dir.create().expect("create");
        let me = std::process::id();
        // A pid that runs (this process's parent) and one that cannot.
        let parent = crate::platform::core_link::parent_pid();
        let dead = 999_999_u32;
        dir.write_lease(&LeaseHolder::client(dead), 1.0, 0.0)
            .expect("dead");
        dir.write_lease(&LeaseHolder::client(parent), 1.0, 0.0)
            .expect("live");
        dir.write_lease(&LeaseHolder::client(me), 1.0, 0.0)
            .expect("own");
        dir.write_lease(&LeaseHolder::cli(dead), 1.0, 0.0)
            .expect("cli");
        prune_dead_clients(&dir, me);
        let left: Vec<String> = dir
            .leases()
            .into_iter()
            .map(|lease| lease.holder.as_str().to_string())
            .collect();
        assert!(!left.contains(&format!("client-{dead}")), "{left:?}");
        assert!(left.contains(&format!("client-{parent}")), "{left:?}");
        assert!(left.contains(&format!("client-{me}")), "{left:?}");
        assert!(!left.contains(&format!("cli-{dead}")), "{left:?}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
