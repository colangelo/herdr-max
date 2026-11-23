//! What a herdr server knows of the core machines its panes run on
//! (core-phase5 D5, D3a). Core sends no event when a machine's link drops
//! (P5), so the server reads `machine.list` while a lease allows a link,
//! and marks each machine here; a pane on a machine that is not connected
//! sends no input and posts a wake to this process's dialer instead.
//!
//! The input path reads one atomic per batch and, at most once per
//! `WAKE_EVERY`, does a `try_send` to the wake thread; the passwd lookup,
//! the folder checks, the dialer's own thread and every core call happen on
//! that thread, the dialer's or the server's probe thread.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use serde_json::json;

use super::dialer::{self, Dialer, PrimaryCore};
use super::name::MachineName;
use super::state::{now_unix, passwd_machines_root, MachineDir};
use crate::pane::core_machines as core;

/// The most wakes one machine's panes post, so a person typing into a pane
/// whose machine is away starts one dial, not one per key.
const WAKE_EVERY: Duration = Duration::from_secs(5);
/// Wakes waiting for the wake thread, which never blocks on one: a full
/// queue drops the wake and leaves the next input free to post again.
const WAKE_QUEUE: usize = 16;
/// How long a wake waits for the person's lease (PR D's second read, item
/// 1): the person's client writes it for the same input that reached the
/// pane, off its own thread and after one `machine.list` of at most 1 s, so
/// the wake can arrive first. Input that no person's client leases (an
/// agent's, the API's) still dials nothing.
const LEASE_GRACE: Duration = Duration::from_secs(3);
/// How often the wake thread looks again for the lease of a waiting wake.
const LEASE_POLL: Duration = Duration::from_millis(100);

/// One machine as this process sees it.
pub(crate) struct MachineLink {
    name: MachineName,
    connected: AtomicBool,
    last_wake: Mutex<Option<Instant>>,
    /// Made and used on the wake thread only.
    dialer: Mutex<Option<Dialer>>,
}

static LINKS: Mutex<Vec<Arc<MachineLink>>> = Mutex::new(Vec::new());

/// The thread that turns a posted wake into a dial: started with the
/// first link, never on the input path. `None` when it could not start.
static WAKES: OnceLock<Option<SyncSender<Arc<MachineLink>>>> = OnceLock::new();

fn wake_thread() -> Option<&'static SyncSender<Arc<MachineLink>>> {
    WAKES
        .get_or_init(|| {
            let (wakes, requests) = mpsc::sync_channel::<Arc<MachineLink>>(WAKE_QUEUE);
            let started = std::thread::Builder::new()
                .name("herdr-machine-wake".into())
                .spawn(move || {
                    run_wakes(
                        &requests,
                        || passwd_machines_root().ok(),
                        LEASE_GRACE,
                        |link, root| link.dial_now(root),
                    );
                });
            match started {
                Ok(_) => Some(wakes),
                Err(err) => {
                    tracing::warn!(%err, "no wake thread for core machine panes");
                    None
                }
            }
        })
        .as_ref()
}

/// The process's record of `name`, made on first use, not connected: a
/// pane's attach marks it connected once core answers it, and the poll as
/// core lists it (D5).
pub(crate) fn link(name: &MachineName) -> Arc<MachineLink> {
    // Before the first link exists, so no key ever starts the thread.
    let _ = wake_thread();
    let mut links = LINKS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(found) = links.iter().find(|link| &link.name == name) {
        return Arc::clone(found);
    }
    let made = Arc::new(MachineLink {
        name: name.clone(),
        connected: AtomicBool::new(false),
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
    /// posts a wake to the wake thread and returns at once, at most once
    /// per `WAKE_EVERY`. True when one was posted.
    pub(crate) fn wake(self: &Arc<Self>) -> bool {
        let Some(wakes) = wake_thread() else {
            return false;
        };
        self.post_wake(wakes)
    }

    /// Posts a wake on `wakes` unless one went within `WAKE_EVERY`. A wake
    /// the queue could not take starts no interval, so the next input posts
    /// again (#312 follow-up: a person's first key after a burst of agent
    /// input is never held off for 5 s).
    fn post_wake(self: &Arc<Self>, wakes: &SyncSender<Arc<MachineLink>>) -> bool {
        let now = Instant::now();
        let mut last = self
            .last_wake
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if last.is_some_and(|at| now.saturating_duration_since(at) < WAKE_EVERY) {
            return false;
        }
        match wakes.try_send(Arc::clone(self)) {
            Ok(()) => {
                *last = Some(now);
                true
            }
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => false,
        }
    }

    /// On the wake thread, once a lease allows the link: the machine's
    /// dialer, made on first use, gets the wake. Only a machine herdr links
    /// is dialled (its folder holds `target.json`), and only while a lease
    /// is in force (`run_wakes`): leases are a person's input, which the
    /// person's client writes; input through the API, from an agent or a
    /// script, writes none, so it never dials a dormant machine, even where
    /// `core-link` would refuse (security review M7).
    fn dial_now(&self, root: &Path) {
        let mut dialer = self.dialer.lock().unwrap_or_else(PoisonError::into_inner);
        if dialer.is_none() {
            *dialer = self.spawn_dialer(root);
        }
        if let Some(dialer) = dialer.as_ref() {
            dialer.wake();
        }
    }

    fn spawn_dialer(&self, root: &Path) -> Option<Dialer> {
        let dir = MachineDir::at(root, &self.name);
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
                let probe = wake_probe(machine, &outcome, !dormant(machine));
                core::tell_server(crate::events::AppEvent::CoreMachines {
                    probes: vec![probe],
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

/// What a wake found, as a probe: the state the dial ended in and whether
/// a lease allows the link now, read after the dial (a wake refused for no
/// lease leaves the machine dormant, not unreachable). A failed wake, or
/// one with no core to ask, read nothing.
fn wake_probe(machine: &MachineName, outcome: &dialer::Outcome, leased: bool) -> Probe {
    let state = outcome.state().map(str::to_string);
    Probe {
        machine: machine.clone(),
        read: state.is_some(),
        state,
        leased,
    }
}

/// What the server read of one machine (D5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Probe {
    pub machine: MachineName,
    /// Its state in `machine.list`; `None` when it was not read or core
    /// does not list it.
    pub state: Option<String>,
    /// A lease allows its link now, or herdr does not gate it (a machine
    /// added by hand, whose link core keeps by itself).
    pub leased: bool,
    /// Whether this says anything about the machine: core answered
    /// `machine.list` for it, or it is dormant (no lease, so no link). A
    /// probe that was not read leaves the machine and its panes as they
    /// are: a failed `machine.list` is not a link down.
    pub read: bool,
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

/// The wake thread's loop: a wake whose machine a lease allows dials at
/// once (`dial`); one with no lease yet waits as the machine's one pending
/// wake (a newer wake of the machine takes its place and deadline), looked
/// at again every `LEASE_POLL` until `grace` ends, when it goes undialled.
/// The thread never sleeps on one wake, so a wake waiting for a lease (an
/// agent's input, which no person leases) never holds up another
/// machine's, and the queue does not fill behind it (#312 follow-up).
fn run_wakes(
    requests: &Receiver<Arc<MachineLink>>,
    root: impl Fn() -> Option<PathBuf>,
    grace: Duration,
    mut dial: impl FnMut(&MachineLink, &Path),
) {
    let mut pending: Vec<(Arc<MachineLink>, Instant)> = Vec::new();
    loop {
        let received = if pending.is_empty() {
            match requests.recv() {
                Ok(link) => Some(link),
                Err(_) => return,
            }
        } else {
            match requests.recv_timeout(LEASE_POLL) {
                Ok(link) => Some(link),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        };
        let now = Instant::now();
        if let Some(link) = received {
            pending.retain(|(waiting, _)| waiting.name != link.name);
            pending.push((link, now + grace));
        }
        let Some(root) = root() else {
            pending.clear();
            continue;
        };
        pending.retain(|(link, deadline)| {
            if leased(Some(&root), &link.name) {
                dial(link, &root);
                return false;
            }
            if now >= *deadline {
                tracing::debug!(machine = %link.name, "no lease for the machine: input does not wake it");
                return false;
            }
            true
        });
    }
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
        .map(|machine| {
            let leased = leased(root.as_deref(), machine);
            Probe {
                machine: machine.clone(),
                state: None,
                leased,
                read: !leased,
            }
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
        probe.read = true;
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

/// What a failed attach to a machine's terminal makes of the pane
/// (core-phase5 D5, positive evidence): ended only on a finished record
/// with the pane's labels; waiting, with its terminal missing, when core
/// has the machine connected but not the pane's terminal; `None` keeps the
/// pane's reason (core did not answer, or has the machine away).
pub(crate) fn machine_wait_reason(
    machine: &MachineName,
    failure: crate::pane::CoreAttachFailure,
) -> Option<crate::terminal::state::CoreWaitReason> {
    use crate::pane::CoreAttachFailure;
    use crate::terminal::state::CoreWaitReason;
    match failure {
        CoreAttachFailure::Unreachable => None,
        CoreAttachFailure::Gone => Some(CoreWaitReason::MachineEnded {
            machine: machine.to_string(),
            notice: format!("Its program on {machine} has ended."),
        }),
        // `NotOurs` is this Mac's answer for a live terminal of another
        // pane; on a machine that is `Missing`, and neither is final.
        CoreAttachFailure::Missing | CoreAttachFailure::NotOurs => {
            Some(CoreWaitReason::MachineMissing {
                machine: machine.to_string(),
            })
        }
    }
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

    /// A herdr-linked machine folder under `root`, with no lease.
    fn gated(root: &Path, raw: &str) -> MachineDir {
        let dir = MachineDir::at(root, &name(raw));
        dir.create().expect("create");
        std::fs::write(dir.target_path(), b"{}").expect("target");
        dir
    }

    fn lease_now(dir: &MachineDir) {
        let now = now_unix();
        dir.write_lease(&LeaseHolder::client(std::process::id()), now + 60.0, now)
            .expect("lease");
    }

    /// The wake loop on a thread of its own, recording what it dials.
    struct WakeLoop {
        wakes: SyncSender<Arc<MachineLink>>,
        dials: Receiver<(MachineName, Instant)>,
        thread: std::thread::JoinHandle<()>,
    }

    impl WakeLoop {
        /// Over `root`, with `grace` for a lease.
        fn start(root: &Path, grace: Duration) -> Self {
            let (wakes, requests) = mpsc::sync_channel::<Arc<MachineLink>>(WAKE_QUEUE);
            let (dialled, dials) = mpsc::channel();
            let root = root.to_path_buf();
            let thread = std::thread::spawn(move || {
                run_wakes(
                    &requests,
                    || Some(root.clone()),
                    grace,
                    |link, _| {
                        let _ = dialled.send((link.name.clone(), Instant::now()));
                    },
                );
            });
            Self {
                wakes,
                dials,
                thread,
            }
        }

        fn stop(self) {
            drop(self.wakes);
            self.thread.join().expect("loop");
        }
    }

    // Item 1 of PR D's second read: a lease write wakes nothing, so input in
    // a pane on the machine wakes it, and that wake can come before the
    // person's client wrote the lease for the same input. It waits a short
    // grace for that lease; input no client leases (an agent's) never dials.
    #[test]
    fn a_wake_waits_briefly_for_the_persons_lease() {
        let root = crate::core_machine::state::tests::scratch("panes-grace");
        let late = gated(&root, "grace-late");
        let _never = gated(&root, "grace-never");
        let wakes = WakeLoop::start(&root, Duration::from_millis(600));
        let posted = Instant::now();
        wakes.wakes.send(link(&name("grace-never"))).expect("post");
        wakes.wakes.send(link(&name("grace-late"))).expect("post");
        std::thread::sleep(Duration::from_millis(150));
        lease_now(&late);
        let (dialled, at) = wakes
            .dials
            .recv_timeout(Duration::from_secs(3))
            .expect("a dial");
        assert_eq!(dialled, name("grace-late"));
        assert!(at - posted >= Duration::from_millis(150), "it waited");
        assert!(
            wakes
                .dials
                .recv_timeout(Duration::from_millis(900))
                .is_err(),
            "input no person leases never dials"
        );
        wakes.stop();
        let _ = std::fs::remove_dir_all(&root);
    }

    // #312 follow-up: a wake waiting for a lease never holds up another
    // machine's. Agent input into panes on dormant machines leaves them
    // pending, and a person's wake behind them dials at once, not after
    // their grace.
    #[test]
    fn a_wake_without_a_lease_never_holds_up_another_machines() {
        let root = crate::core_machine::state::tests::scratch("panes-no-block");
        for agent in ["agent-a", "agent-b", "agent-c"] {
            gated(&root, agent);
        }
        let person = gated(&root, "person");
        lease_now(&person);
        let wakes = WakeLoop::start(&root, Duration::from_secs(3));
        for agent in ["agent-a", "agent-b", "agent-c"] {
            wakes.wakes.send(link(&name(agent))).expect("post");
        }
        let posted = Instant::now();
        wakes.wakes.send(link(&name("person"))).expect("post");
        let (dialled, at) = wakes
            .dials
            .recv_timeout(Duration::from_secs(10))
            .expect("a dial");
        assert_eq!(dialled, name("person"));
        assert!(
            at - posted < Duration::from_secs(1),
            "the person's wake waited {:?} behind agents' wakes",
            at - posted
        );
        wakes.stop();
        let _ = std::fs::remove_dir_all(&root);
    }

    // #312 follow-up: a wake the queue could not take starts no interval,
    // so the next input posts at once instead of 5 s later.
    #[test]
    fn a_dropped_wake_does_not_hold_off_the_next() {
        let link = link(&name("wake-dropped"));
        let (full, _never_read) = mpsc::sync_channel::<Arc<MachineLink>>(0);
        assert!(!link.post_wake(&full), "the queue took nothing");
        let (open, posted) = mpsc::sync_channel::<Arc<MachineLink>>(1);
        assert!(link.post_wake(&open), "the next input posts");
        assert!(posted.try_recv().is_ok());
        assert!(
            !link.post_wake(&open),
            "within the interval: nothing posted"
        );
    }

    // C4 (D5): a `machine.list` that failed reads nothing for the machines
    // that may be up; a dormant one is known without asking.
    #[test]
    fn a_failed_machine_list_reads_nothing() {
        let core =
            crate::pane::FakeCore::start_with(|config| config.failing = vec!["machine.list"]);
        std::env::set_var("HC_SOCKET", &core.socket);
        let probes = probe(&[name("byhand-unread")]);
        assert_eq!(
            probes,
            [Probe {
                machine: name("byhand-unread"),
                state: None,
                leased: true,
                read: false,
            }]
        );
        let answered = crate::pane::FakeCore::start();
        answered.set_machine("byhand-unread", "unreachable");
        std::env::set_var("HC_SOCKET", &answered.socket);
        let probes = probe(&[name("byhand-unread")]);
        assert!(probes[0].read);
        assert_eq!(probes[0].state.as_deref(), Some("unreachable"));
    }

    // C9 (D5): a wake reports what it found: a dial `core-link` refused for
    // want of a lease leaves the machine dormant, and a failed one reads
    // nothing.
    #[test]
    fn a_wake_reports_its_real_outcome() {
        let machine = name("wake-outcome");
        let refused = wake_probe(
            &machine,
            &dialer::Outcome::Connected("unreachable".into()),
            false,
        );
        assert!(refused.read && !refused.leased && !refused.connected());
        let up = wake_probe(&machine, &dialer::Outcome::AlreadyConnected, true);
        assert!(up.read && up.leased && up.connected());
        let failed = wake_probe(&machine, &dialer::Outcome::Failed("no core".into()), true);
        assert!(!failed.read);
        assert!(!wake_probe(&machine, &dialer::Outcome::NotListed, true).read);
    }

    // C8 (D5): a key posts at most one wake per `WAKE_EVERY`; the dial
    // itself runs on the wake thread.
    #[test]
    fn a_wake_is_posted_once_per_interval() {
        let link = link(&name("wake-once"));
        assert!(link.wake(), "posted");
        assert!(!link.wake(), "within the interval: nothing posted");
    }

    // Security review M7: input without a lease, as API input is, does not
    // dial a machine herdr gates: no dialer is made, no connect posted.
    #[test]
    fn input_without_a_lease_does_not_dial_a_gated_machine() {
        // Whatever happens, no dial reaches a real core.
        let core = crate::pane::FakeCore::start();
        std::env::set_var("HC_SOCKET", &core.socket);
        let root = crate::core_machine::state::tests::scratch("panes-no-lease-wake");
        gated(&root, "no-lease-wake");
        let link = link(&name("no-lease-wake"));
        let (wakes, requests) = mpsc::sync_channel::<Arc<MachineLink>>(1);
        let thread = {
            let root = root.clone();
            std::thread::spawn(move || {
                run_wakes(
                    &requests,
                    || Some(root.clone()),
                    Duration::from_millis(200),
                    |link, root| link.dial_now(root),
                );
            })
        };
        wakes.send(Arc::clone(&link)).expect("post");
        std::thread::sleep(Duration::from_millis(500));
        drop(wakes);
        thread.join().expect("loop");
        assert!(
            link.dialer
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_none(),
            "no dialer, no connect"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // C10 (D5): a machine is connected only once core answered for it.
    #[test]
    fn a_link_starts_not_connected_and_says_when_it_changes() {
        let link = link(&name("panes-test-link"));
        assert!(!link.connected(), "nothing answered for it yet");
        assert!(!link.set_connected(false));
        assert!(link.set_connected(true));
        assert!(link.connected());
        assert!(super::link(&name("panes-test-link")).set_connected(false));
    }
}
