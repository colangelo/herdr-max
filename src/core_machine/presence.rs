//! A TUI client's lease on its primary's core machines (core-phase5 D3,
//! task B.2): a person's input keeps the machines' links allowed for 10
//! min. The client loop only stamps the time of each input; at most once a
//! minute that stamp goes to a writer thread, which writes
//! `leases/client-<pid>` in the folder of each machine this client's
//! primary core lists and added. A lease write dials nothing (PR D's second
//! read, item 1): input in a pane on a machine, or in a view of one, wakes
//! that one machine, through the process that got it. Starting a client
//! writes nothing: opening herdr dials no machine (D3). Nothing here runs
//! on the input, render or PTY path but an `Instant` comparison and a
//! `try_send`.

use std::path::Path;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, Instant};

use super::command::{linked_machines_in, listed_by_core, runs_herdr_link};
use super::name::MachineName;
use super::state::{now_unix, passwd_machines_root, LeaseHolder, MachineDir, Target};

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
        // Unit tests that run a client loop, every client whose environment
        // opts out, and the old remote's local client write no lease into
        // the account's machine folders.
        let opted_out = std::env::var_os(NO_LEASES_ENV).is_some();
        if opted_out {
            tracing::info!("{NO_LEASES_ENV} is set: this client writes no core machine lease");
        }
        if cfg!(test) || !writes_leases(opted_out, remote_target().as_deref()) {
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

/// The `--remote` target of a remote client process (`ac@host`, or
/// `core:<machine>` for a view), `None` for a client of this Mac's own
/// server.
fn remote_target() -> Option<String> {
    std::env::var_os(crate::remote::REMOTE_KEYBINDINGS_ENV_VAR)?;
    let preferences = std::env::var(crate::remote::REMOTE_PREFERENCES_ENV_VAR).unwrap_or_default();
    let (target, _session): (String, String) =
        serde_json::from_str(&preferences).unwrap_or_default();
    Some(target)
}

/// Whether a client writes leases: not when its environment opts out, and
/// not as the old remote's local client (`herdr --remote <host>`, D8,
/// PR D's second read, item 2), which reaches another Mac through herdr's
/// own ssh and has no core machine to allow. A view's client (`--remote
/// core:<machine>`) and a client of this Mac's server do.
pub(crate) fn writes_leases(opted_out: bool, remote_target: Option<&str>) -> bool {
    !opted_out && remote_target.is_none_or(|target| target.starts_with("core:"))
}

/// The writer thread: one pass over the machines per input stamp. It dials
/// nothing (item 1): a key in a pane of this Mac allows every linked
/// machine for 10 min (D3) but wakes none of them.
fn write_leases(requests: Receiver<f64>, pid: u32) {
    let holder = LeaseHolder::client(pid);
    let own_core = crate::pane::core_machines::primary_session();
    while let Ok(input_at) = requests.recv() {
        let Ok(root) = passwd_machines_root() else {
            continue;
        };
        let _ = lease_pass(&root, primary_listing, &own_core, &holder, input_at);
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
/// under `root` that this client's own primary core (`own_core`) lists as a
/// herdr-managed link (`listing`, asked only when some machine is linked)
/// and added, and returns the ones written. A folder of another session's
/// core, a scratch core's or a test's is not this client's to allow (review
/// H7, M7, item 5): a same-named machine that core lists with another link,
/// or a folder another core session added, gets nothing.
fn lease_pass(
    root: &Path,
    listing: impl FnOnce() -> Option<serde_json::Value>,
    own_core: &str,
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
        let Ok(target) = dir.read_target() else {
            continue;
        };
        if !managed_by_herdr(&listed, &machine.name, &target) {
            tracing::debug!(machine = %machine.name, "the primary lists this name with another link; no lease");
            continue;
        }
        if dir.read_primary().is_some_and(|core| core != own_core) {
            tracing::debug!(machine = %machine.name, "another core session added this machine; no lease");
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

/// Whether `listed` (a primary's `machine.list`) has `name` as the link
/// herdr's `machine add --core` made (review M7, Sol D-08): core's record
/// carries the `via` and session `target` (`target.json`) recorded, as
/// `machine status` and the fleet bridge check it (`runs_herdr_link`, #310),
/// not a direct or scratch machine of the same name.
fn managed_by_herdr(listed: &serde_json::Value, name: &MachineName, target: &Target) -> bool {
    listed["machines"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|record| record["name"] == name.as_str())
        .any(|record| runs_herdr_link(record, name, target))
}

/// Removes the expired lease files (`client-` and `cli-`) of processes that
/// no longer run, and the files of pids the system gave again to a later
/// process (review M7: such a lease is no one's), so the folder does not
/// fill with them. A live holder's file is never touched: only its own
/// process writes it (D3).
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
    for holder in dir.reused_pid_holders() {
        if holder.pid() != Some(own_pid) {
            let _ = dir.remove_lease(&holder);
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
            via: None,
        })
        .expect("target");
        dir
    }

    const OWN_CORE: &str = "herdrdev-presence-test";

    /// A primary's record of a machine herdr links (`via` its core-link).
    fn managed(name: &str) -> serde_json::Value {
        json!({"name": name, "state": "connected", "via": true,
            "target": format!("/opt/herdr core-link {name}"), "session": "far"})
    }

    // M7 and item 5 of PR D's second read: a name the primary lists with
    // another link (a direct or scratch machine of the same name), and a
    // folder another core session added, get no lease; a folder this
    // session's primary added, or one an older build added (no record of
    // its core), does.
    #[test]
    fn a_client_leases_only_its_own_primarys_herdr_links() {
        let root = crate::core_machine::state::tests::scratch("own-links");
        let direct = linked(&root, "direct");
        let theirs = linked(&root, "theirs");
        theirs.write_primary("herdrdev-another").expect("primary");
        let ours = linked(&root, "ours");
        ours.write_primary(OWN_CORE).expect("primary");
        let older = linked(&root, "older");
        // Core's record of herdr's link in another hc session than the one
        // `target.json` linked (#310's `runs_herdr_link`).
        let elsewhere = linked(&root, "elsewhere");
        let mut other_session = managed("elsewhere");
        other_session["session"] = json!("other");
        let primary = json!({"machines": [
            {"name": "direct", "state": "connected", "via": false, "target": "direct"},
            managed("theirs"), managed("ours"), managed("older"), other_session,
        ]});
        let holder = LeaseHolder::client(std::process::id());
        let mut written = lease_pass(&root, || Some(primary), OWN_CORE, &holder, now_unix());
        written.sort();
        assert_eq!(
            written,
            [
                MachineName::parse("older").expect("name"),
                MachineName::parse("ours").expect("name")
            ]
        );
        assert!(direct.leases().is_empty() && theirs.leases().is_empty());
        assert!(elsewhere.leases().is_empty());
        let _ = (ours, older);
        let _ = std::fs::remove_dir_all(&root);
    }

    // Item 2 of PR D's second read: the old remote's local client (`herdr
    // --remote <host>`) writes no lease, so its keys never reach core; a
    // view's client (`--remote core:<machine>`) and a local client do,
    // unless the environment opts out.
    #[test]
    fn only_local_and_view_clients_write_leases() {
        assert!(writes_leases(false, None));
        assert!(writes_leases(false, Some("core:ac-mbm5")));
        assert!(!writes_leases(false, Some("ac@ac-mbm5")));
        assert!(
            !writes_leases(false, Some("")),
            "a remote client of no known target"
        );
        assert!(!writes_leases(true, None));
        assert!(!writes_leases(true, Some("core:ac-mbm5")));
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
        let primary = json!({"machines": [managed("listed")]});
        let holder = LeaseHolder::client(std::process::id());
        let written = lease_pass(&root, || Some(primary), OWN_CORE, &holder, now_unix());
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
        assert!(lease_pass(&root, || None, OWN_CORE, &holder, now_unix()).is_empty());
        assert!(lease_pass(&root, || Some(json!({})), OWN_CORE, &holder, now_unix()).is_empty());
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
        // M7: a live pid's file written before that process started is a
        // reused pid's, no one's lease: pruned, in force or not.
        dir.write_lease(&LeaseHolder::cli(parent), now_unix() + 30.0, now_unix())
            .expect("reused");
        let reused = dir.leases_dir().join(format!("cli-{parent}"));
        let file = std::fs::File::options()
            .write(true)
            .open(&reused)
            .expect("open");
        file.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1_000_000))
            .expect("mtime");
        drop(file);
        prune_dead_clients(&dir, me);
        assert!(!reused.exists(), "a reused pid's file is pruned");
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
