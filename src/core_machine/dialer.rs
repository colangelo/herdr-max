//! The dialer (core-phase5 D3a): the one place a herdr process calls
//! `machine.connect` for a machine. `machine.connect` blocks its caller for
//! the whole dial (20-22 s on a silent one, F4), and a second call during a
//! dial makes core end that dial and start a new one, each a fresh ssh login
//! (R4). So input, render and API handlers only post a wake (a `try_send` on
//! a channel of one: a full channel means a wake is already pending), and one
//! thread per machine makes the call. Across herdr processes the call is
//! made under `connect.lock`; a dialer that cannot take it at once does not
//! call, and polls `machine.list` until the state leaves `connecting`.
//!
//! The lease that lets core-link dial (D3) is the caller's to write before
//! it posts: this module dials only.

use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::PathBuf;
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::time::{Duration, Instant};

use serde_json::json;

use super::name::MachineName;
use crate::pane::core_machines as core;

/// The call's timeout: core-link's own deadline is 15 s (D2 step 6).
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// How often a dialer that found another process dialling reads
/// `machine.list`, and for how long at most.
const POLL_EVERY: Duration = Duration::from_secs(1);
const POLL_FOR: Duration = Duration::from_secs(20);

/// What the primary core answers, behind a trait so the dialer is tested
/// without a core.
pub(crate) trait CoreLink: Send + 'static {
    /// The machine's state in `machine.list` (`connected`, `connecting`,
    /// `unreachable`, ...); `Ok(None)` when core does not list it or no core
    /// runs.
    fn state(&self, machine: &MachineName) -> Result<Option<String>, String>;
    /// `machine.connect`; the state it answers.
    fn connect(&self, machine: &MachineName, timeout: Duration) -> Result<String, String>;
}

/// The running primary core of this herdr session; never starts one.
pub(crate) struct PrimaryCore;

impl CoreLink for PrimaryCore {
    fn state(&self, machine: &MachineName) -> Result<Option<String>, String> {
        let Some(reply) =
            core::call_running_primary("machine.list", json!({}), Duration::from_secs(5))?
        else {
            return Ok(None);
        };
        Ok(reply["machines"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|record| record["name"] == machine.as_str())
            .and_then(|record| record["state"].as_str())
            .map(str::to_string))
    }

    fn connect(&self, machine: &MachineName, timeout: Duration) -> Result<String, String> {
        match core::call_running_primary(
            "machine.connect",
            json!({"machine": machine.as_str()}),
            timeout,
        )? {
            Some(reply) => Ok(reply["state"].as_str().unwrap_or("unknown").to_string()),
            None => Err("herdr-core is not running".into()),
        }
    }
}

/// How one wake ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// This dialer called `machine.connect`; core answered this state.
    Connected(String),
    /// Already connected: nothing was called.
    AlreadyConnected,
    /// Another dial was under way (core's own, or another herdr process
    /// holding `connect.lock`): this dialer waited on `machine.list` and
    /// sent nothing; the last state it read.
    Waited(Option<String>),
    /// Core does not list the machine, or no core runs: nothing to dial.
    NotListed,
    Failed(String),
}

impl Outcome {
    pub(crate) fn state(&self) -> Option<&str> {
        match self {
            Self::Connected(state) => Some(state),
            Self::AlreadyConnected => Some("connected"),
            Self::Waited(state) => state.as_deref(),
            Self::NotListed | Self::Failed(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Timing {
    pub connect_timeout: Duration,
    pub poll_every: Duration,
    pub poll_for: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            connect_timeout: CONNECT_TIMEOUT,
            poll_every: POLL_EVERY,
            poll_for: POLL_FOR,
        }
    }
}

/// One wake, on the calling thread: the CLI's `machine wake` and `machine
/// login` run it directly (no input thread waits on them), the dialer
/// thread for everyone else.
pub(crate) fn dial_once(
    link: &dyn CoreLink,
    machine: &MachineName,
    connect_lock: &std::path::Path,
    timing: Timing,
) -> Outcome {
    match link.state(machine) {
        Err(err) => return Outcome::Failed(err),
        Ok(None) => return Outcome::NotListed,
        Ok(Some(state)) if state == "connected" => return Outcome::AlreadyConnected,
        // A dial is under way: a connect now would end it and start
        // another (R4).
        Ok(Some(state)) if state == "connecting" => return wait(link, machine, timing),
        Ok(Some(_)) => {}
    }
    let lock = match open_lock(connect_lock) {
        Ok(lock) => lock,
        Err(err) => return Outcome::Failed(format!("connect.lock: {err}")),
    };
    match crate::platform::core_link::lock_exclusive_within(&lock, Duration::ZERO) {
        Ok(true) => {}
        Ok(false) => return wait(link, machine, timing),
        Err(err) => return Outcome::Failed(format!("connect.lock: {err}")),
    }
    let result = link.connect(machine, timing.connect_timeout);
    drop(lock);
    match result {
        Ok(state) => Outcome::Connected(state),
        Err(err) => Outcome::Failed(err),
    }
}

fn open_lock(path: &std::path::Path) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
}

/// Reads `machine.list` until the state leaves `connecting`.
fn wait(link: &dyn CoreLink, machine: &MachineName, timing: Timing) -> Outcome {
    let deadline = Instant::now() + timing.poll_for;
    loop {
        std::thread::sleep(timing.poll_every);
        match link.state(machine) {
            Ok(Some(state)) if state == "connecting" && Instant::now() < deadline => {}
            Ok(state) => return Outcome::Waited(state),
            Err(err) => return Outcome::Failed(err),
        }
    }
}

/// A machine's dialer thread. Dropping it ends the thread once a dial in
/// flight returns.
pub(crate) struct Dialer {
    wakes: SyncSender<()>,
}

impl Dialer {
    /// Starts the thread; `outcome` gets every wake's result, on it.
    pub(crate) fn spawn(
        machine: MachineName,
        connect_lock: PathBuf,
        link: impl CoreLink,
        timing: Timing,
        outcome: impl Fn(&MachineName, Outcome) + Send + 'static,
    ) -> std::io::Result<Self> {
        let (wakes, requests) = mpsc::sync_channel::<()>(1);
        std::thread::Builder::new()
            .name(format!("herdr-dialer-{machine}"))
            .spawn(move || {
                while requests.recv().is_ok() {
                    let result = dial_once(&link, &machine, &connect_lock, timing);
                    outcome(&machine, result);
                }
            })?;
        Ok(Self { wakes })
    }

    /// Posts a wake without waiting. False when one is already pending (or
    /// the thread is gone): that wake covers this one.
    pub(crate) fn wake(&self) -> bool {
        match self.wakes.try_send(()) {
            Ok(()) => true,
            Err(TrySendError::Full(())) | Err(TrySendError::Disconnected(())) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;

    /// A core whose `machine.connect` blocks until the test releases it,
    /// counting every call.
    #[derive(Clone, Default)]
    struct SlowCore {
        connects: Arc<AtomicU32>,
        lists: Arc<AtomicU32>,
        state: Arc<Mutex<String>>,
        release: Arc<(Mutex<bool>, std::sync::Condvar)>,
    }

    impl SlowCore {
        fn new(state: &str) -> Self {
            let core = Self::default();
            *core.state.lock().unwrap() = state.into();
            core
        }

        fn release(&self) {
            let (done, signal) = &*self.release;
            *done.lock().unwrap() = true;
            signal.notify_all();
        }
    }

    impl CoreLink for SlowCore {
        fn state(&self, _: &MachineName) -> Result<Option<String>, String> {
            self.lists.fetch_add(1, Ordering::SeqCst);
            Ok(Some(self.state.lock().unwrap().clone()))
        }

        fn connect(&self, _: &MachineName, timeout: Duration) -> Result<String, String> {
            self.connects.fetch_add(1, Ordering::SeqCst);
            *self.state.lock().unwrap() = "connecting".into();
            let (done, signal) = &*self.release;
            let guard = done.lock().unwrap();
            let (_guard, waited) = signal
                .wait_timeout_while(guard, timeout, |done| !*done)
                .unwrap();
            let state = if waited.timed_out() {
                "unreachable"
            } else {
                "connected"
            };
            *self.state.lock().unwrap() = state.into();
            Ok(state.into())
        }
    }

    fn machine() -> MachineName {
        MachineName::parse("loop").expect("name")
    }

    fn timing() -> Timing {
        Timing {
            connect_timeout: Duration::from_secs(20),
            poll_every: Duration::from_millis(20),
            poll_for: Duration::from_secs(20),
        }
    }

    /// V29: a connect that blocks for the whole dial; 100 keys from two
    /// panes and a view wake post at once and never wait; a second process
    /// (its own descriptor on `connect.lock`) with two CLI wakes sends no
    /// connect and waits on `machine.list`; exactly one connect in all.
    #[test]
    fn one_connect_per_wake_and_no_input_path_waits() {
        let root = crate::core_machine::state::tests::scratch("dialer");
        let lock = root.join("connect.lock");
        let core = SlowCore::new("unreachable");
        let (outcomes, results) = mpsc::channel();
        let first = {
            let outcomes = outcomes.clone();
            Dialer::spawn(
                machine(),
                lock.clone(),
                core.clone(),
                timing(),
                move |_, o| {
                    let _ = outcomes.send(("first", o));
                },
            )
            .expect("spawn")
        };
        assert!(first.wake());
        // Wait until the connect is in flight.
        let deadline = Instant::now() + Duration::from_secs(5);
        while core.connects.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < deadline, "the dial never started");
            std::thread::sleep(Duration::from_millis(5));
        }

        let first = Arc::new(first);
        let panes: Vec<_> = (0..2)
            .map(|_| {
                let dialer = first.clone();
                std::thread::spawn(move || {
                    let mut slowest = Duration::ZERO;
                    for _ in 0..50 {
                        let started = Instant::now();
                        dialer.wake();
                        slowest = slowest.max(started.elapsed());
                    }
                    slowest
                })
            })
            .collect();
        let started = Instant::now();
        first.wake();
        let view_wake = started.elapsed();
        for pane in panes {
            let slowest = pane.join().expect("pane");
            assert!(
                slowest < Duration::from_millis(1),
                "a key waited {slowest:?}"
            );
        }
        assert!(
            view_wake < Duration::from_millis(1),
            "the view waited {view_wake:?}"
        );

        // The second process: the core says connecting, and the lock is held.
        let lists_before = core.lists.load(Ordering::SeqCst);
        let second_core = core.clone();
        let second = {
            let outcomes = outcomes.clone();
            Dialer::spawn(
                machine(),
                lock.clone(),
                second_core,
                timing(),
                move |_, o| {
                    let _ = outcomes.send(("second", o));
                },
            )
            .expect("spawn second")
        };
        assert!(second.wake());
        second.wake();
        // At least two polls, waited for rather than slept on: a loaded runner can
        // let a fixed 100 ms pass with one 20 ms poll.
        let deadline = Instant::now() + Duration::from_secs(5);
        while core.lists.load(Ordering::SeqCst) <= lists_before + 1 {
            assert!(Instant::now() < deadline, "the second process did not poll");
            std::thread::sleep(Duration::from_millis(5));
        }
        core.release();

        let mut seen = Vec::new();
        while let Ok(result) = results.recv_timeout(Duration::from_secs(5)) {
            seen.push(result);
            if seen.iter().filter(|(who, _)| *who == "first").count() >= 1
                && seen.iter().filter(|(who, _)| *who == "second").count() >= 1
            {
                break;
            }
        }
        assert_eq!(core.connects.load(Ordering::SeqCst), 1, "{seen:?}");
        assert!(
            seen.contains(&("first", Outcome::Connected("connected".into()))),
            "{seen:?}"
        );
        assert!(
            seen.iter().any(|(who, o)| *who == "second"
                && matches!(o, Outcome::Waited(Some(state)) if state == "connected")),
            "{seen:?}"
        );
        // The wakes posted during the dial collapse into at most one more
        // pass, which finds the machine connected and calls nothing.
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(core.connects.load(Ordering::SeqCst), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The second process's half alone: another process holds
    /// `connect.lock` while core still reads `unreachable`.
    #[test]
    fn a_held_connect_lock_means_waiting_not_calling() {
        let root = crate::core_machine::state::tests::scratch("dialer-lock");
        let lock = root.join("connect.lock");
        let holder = open_lock(&lock).expect("open");
        assert!(
            crate::platform::core_link::lock_exclusive_within(&holder, Duration::ZERO)
                .expect("lock")
        );
        let core = SlowCore::new("unreachable");
        let outcome = dial_once(&core, &machine(), &lock, timing());
        assert_eq!(outcome, Outcome::Waited(Some("unreachable".into())));
        assert_eq!(core.connects.load(Ordering::SeqCst), 0);
        drop(holder);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_connected_or_unlisted_machine_is_not_dialled() {
        let root = crate::core_machine::state::tests::scratch("dialer-noop");
        let lock = root.join("connect.lock");
        let core = SlowCore::new("connected");
        assert_eq!(
            dial_once(&core, &machine(), &lock, timing()),
            Outcome::AlreadyConnected
        );
        struct NoCore;
        impl CoreLink for NoCore {
            fn state(&self, _: &MachineName) -> Result<Option<String>, String> {
                Ok(None)
            }
            fn connect(&self, _: &MachineName, _: Duration) -> Result<String, String> {
                panic!("dialled a machine core does not list")
            }
        }
        assert_eq!(
            dial_once(&NoCore, &machine(), &lock, timing()),
            Outcome::NotListed
        );
        assert_eq!(core.connects.load(Ordering::SeqCst), 0);
        let _ = std::fs::remove_dir_all(&root);
    }
}
