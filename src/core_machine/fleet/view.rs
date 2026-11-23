//! A view's structural actions (core-phase5 D7, E.3): new tab, split, close
//! pane and rename in `herdr server --view <m>` go to the far herdr as one
//! API request each, through the fleet CLI's reviewed path: `machine.exec`
//! of D5's `env -u` wrapper and `<far herdr> --session S remote-api-bridge
//! --request`, with the request line as its input. The argv is fixed; every
//! name travels inside the JSON request as one string, so none reaches a
//! shell or an argument parser there (V20, D11). The far herdr changes its
//! layout and republishes it; the view follows its document.
//!
//! A person's action in a view is a person's act on that machine (D3), but
//! the action itself is no dial: on a machine that is not connected it is
//! refused at once, with no lease written and nothing sent or dialled for
//! it (D7's rule for a call without `--dial`); a key in one of the view's
//! panes is what wakes it (D6). The chord's own keys are a person's input
//! in a Herdr window like any other, so they renew that client's lease
//! (B.2), and while it is in force Herdr Core's own retry may connect the
//! machine (E.3 review, item 11). On a connected one the view's own
//! `cli-<pid>` keep lease holds the link for the call and admits no dial,
//! so a drop during it is not redialled on its account. A refusal or a far
//! error is the action's own answer: the client shows it as a notice and
//! the view stays connected.
//!
//! Time (review M3): the client waits 60 s for an endpoint command, holding
//! its command lane meanwhile (one command in flight per endpoint, by the
//! lane's design), and a client that gave up could do the action again. So
//! an action has one deadline, `VIEW_ACTION_BUDGET` (45 s) from the moment
//! the view took it, and every step fits in it: no wait for another action
//! (a second one at once is refused, nothing sent), core's `machine.list`
//! (5 s), the far herdr probe when its answer is stale (12 s and core's 5),
//! the check that the far pane is still the one aimed at (a `doc.get` and a
//! `terminal.get`: 10 s, however long its connection to core waits, and
//! never past the deadline less the shortest request), and the request,
//! whose run is what is left, 15 s at most and 5 s at least, else nothing
//! is sent.

use std::io;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex, TryLockError};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{
    bridge_argv, check_request_line, far_herdr_session, far_herdr_within, CoreAt, FarRoute,
    CALL_MARGIN_SECS,
};
use crate::core_machine::state::{now_unix, passwd_machines_root, LeaseHolder, MachineDir};
use crate::core_machine::MachineName;

/// The whole of one view action, from the moment the view took it to its
/// answer: well inside the client's own 60 s wait for an endpoint command.
pub(crate) const VIEW_ACTION_BUDGET: Duration = Duration::from_secs(45);
/// The far herdr probe's run, when its kept answer is stale.
const VIEW_PROBE_SECS: u64 = 12;
/// The aimed-at check: the far document's read and the far terminal's
/// (`view_link`'s calls, 5 s each).
const VIEW_CHECK_SECS: u64 = 10;
/// The request's run: what the budget leaves, at most this.
const VIEW_REQUEST_SECS: u64 = 15;
/// Less than this left for the request: it is not sent.
const VIEW_REQUEST_MIN_SECS: u64 = 5;

// The view answers before the client gives up, with room for the client's
// own queue and the socket (review M3).
const _: () =
    assert!(VIEW_ACTION_BUDGET.as_secs() + 10 <= crate::client::ENDPOINT_COMMAND_TIMEOUT.as_secs());

/// The view's keep lease lives this long past each write: a CLI lease's
/// 60 s, longer than the whole action.
const VIEW_LEASE_SECS: f64 = super::LEASE_LIFE_SECS;

/// Reads the far `herdr-app` document (`doc.get {on}`), bounded.
pub(crate) type LayoutReader = Arc<dyn Fn(&MachineName) -> Result<Value, String> + Send + Sync>;
/// Reads core's record of a far terminal (`terminal.get` of
/// `<machine>/t7`), bounded.
pub(crate) type TerminalReader = Arc<dyn Fn(&str) -> Result<Value, String> + Send + Sync>;

/// The check run just before the request (`send`).
pub(crate) type AimCheck = Box<dyn FnOnce() -> Result<(), String> + Send>;

/// What happens at the point the request is sent, after every step that can
/// still refuse it: the person's one-use confirmation is spent here (fork
/// issue 316, review item 2). `Err` sends nothing.
pub(crate) type Commit = Box<dyn FnOnce() -> Result<(), String> + Send>;

/// Where a view's actions go: the primary core, herdr's machine state, the
/// view's lease holder and the far document's and terminals' readers. A
/// test points them at a fake core and a scratch folder. Clones share one
/// lock: one action at a time per view, since they share its one lease
/// file; a second is refused, never queued (review M3).
#[derive(Clone)]
pub(crate) struct ViewRoute {
    core: CoreAt,
    root: Result<PathBuf, String>,
    holder: LeaseHolder,
    layout: LayoutReader,
    terminal: TerminalReader,
    busy: Arc<Mutex<()>>,
    leases: Arc<RunningLeases>,
}

impl ViewRoute {
    pub(crate) fn primary() -> Self {
        Self {
            core: CoreAt::primary(),
            root: passwd_machines_root().map_err(|err| err.to_string()),
            holder: LeaseHolder::cli(std::process::id()),
            layout: Arc::new(|machine| {
                crate::pane::view_link::layout_document(&crate::pane::view_connector(), machine)
                    .map_err(|err| err.to_string())
            }),
            terminal: Arc::new(|address| {
                crate::pane::view_link::terminal_record(&crate::pane::view_connector(), address)
                    .map_err(|err| err.to_string())
            }),
            busy: Arc::default(),
            leases: Arc::default(),
        }
    }

    /// A route to the core at `socket` and the machine state under `root`,
    /// whose far terminals answer nothing until `with_terminal` says how.
    #[cfg(test)]
    pub(crate) fn at(socket: &std::path::Path, root: PathBuf, layout: LayoutReader) -> Self {
        Self {
            core: CoreAt::at(socket),
            root: Ok(root),
            holder: LeaseHolder::cli(std::process::id()),
            layout,
            terminal: Arc::new(|_| Err("no far terminal in this test".into())),
            busy: Arc::default(),
            leases: Arc::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_terminal(mut self, terminal: TerminalReader) -> Self {
        self.terminal = terminal;
        self
    }

    /// The view server's graceful end: removes the lease of every action
    /// still running of this route, and no action of it writes one again
    /// (fork issue 316, M7 and review item 5). Those actions' threads do not
    /// outlive the process to remove their own.
    pub(crate) fn end(&self) {
        let mut book = self.leases.book();
        book.ended = true;
        for (dir, holder) in book.running.drain(..) {
            if let Err(err) = dir.remove_lease(&holder) {
                tracing::warn!(%err, "could not remove a running view action's lease");
            }
        }
    }

    /// The far document as core has it now.
    pub(crate) fn far_layout(&self, machine: &MachineName) -> Result<Value, String> {
        (self.layout)(machine)
    }

    /// Core's record of the far terminal `address` (`<machine>/t7`) now.
    pub(crate) fn far_terminal(&self, address: &str) -> Result<Value, String> {
        (self.terminal)(address)
    }
}

/// Sends one API request line (`{"id", "method", "params"}` and a newline)
/// to the far herdr of `name` and returns its one reply line, or why there
/// is none: refused unsent, or failed after sending (then the message says
/// that it may have run there). `check` runs just before the request: the
/// far pane the person aimed at is still that one (review M2), else nothing
/// is sent; it gets `VIEW_CHECK_SECS`, never more than the deadline leaves
/// less the shortest request, and one that has not answered by then sends
/// nothing (E.3 review, item 7). `commit` runs last of all, once the lease
/// is written and the run is known: it spends what the request spends, so
/// anything that refuses before spends nothing.
/// Everything ends by `deadline`.
pub(crate) fn send(
    route: &ViewRoute,
    name: &MachineName,
    line: &[u8],
    deadline: Instant,
    check: AimCheck,
    commit: Commit,
) -> Result<Vec<u8>, String> {
    let not_sent = |why: String| format!("{why}; nothing was sent to {name}");
    let _one = match route.busy.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => {
            let why = format!("another action on {name} from this view is still running");
            return Err(not_sent(format!("{why}; do this again when it ends")));
        }
    };
    let root = route
        .root
        .clone()
        .map_err(|err| not_sent(format!("cannot find herdr's machine state: {err}")))?;
    let dir = MachineDir::at(&root, name);
    if !dir.target_path().exists() {
        return Err(not_sent(format!("herdr here did not add {name}")));
    }
    dir.check()
        .map_err(|err| not_sent(format!("{name}: {err}")))?;
    let target = dir
        .read_target()
        .map_err(|err| not_sent(format!("{name}: {err}")))?;
    let session = far_herdr_session(&target.session).ok_or_else(|| {
        not_sent(format!(
            "{name} links to Herdr Core session {}, which is no herdr session's own",
            target.session
        ))
    })?;
    // Every call to core ends by the action's deadline: the connect, the
    // hello and each call's wait (fork issue 316, M7).
    let core = &route.core.by(deadline);
    let record = match core.record(name) {
        Ok(Some(record)) => record,
        Ok(None) => {
            return Err(not_sent(format!(
                "herdr-core here does not list {name}, or is not running"
            )))
        }
        Err(err) => {
            return Err(not_sent(format!(
                "herdr-core did not list its machines: {err}"
            )))
        }
    };
    // The same route check as the fleet CLI's (security review M5): core's
    // machine of that name must be the one herdr added.
    if !crate::core_machine::command::runs_herdr_link(&record, name, &target) {
        return Err(not_sent(format!(
            "herdr-core's machine {name} is not the one herdr added"
        )));
    }
    let state = record["state"].as_str().unwrap_or("unknown");
    if state != "connected" {
        return Err(not_connected(name, state));
    }
    check_request_line(name, line)?;
    let out_of_time = || not_sent(format!("{name} took too long to answer before the request"));
    // The probe (when stale), the check and the shortest request must fit.
    let before_probe = (VIEW_PROBE_SECS + CALL_MARGIN_SECS)
        + VIEW_CHECK_SECS
        + (VIEW_REQUEST_MIN_SECS + CALL_MARGIN_SECS);
    if left(deadline) < Duration::from_secs(before_probe) {
        return Err(out_of_time());
    }
    let lease = KeepLease::write(&dir, &route.holder, &route.leases)
        .map_err(|err| not_sent(format!("{name}: could not write the lease: {err}")))?;
    let far = far_herdr_within(core, name, &dir, &session, &target.session, VIEW_PROBE_SECS)?;
    if left(deadline)
        < Duration::from_secs(VIEW_CHECK_SECS + VIEW_REQUEST_MIN_SECS + CALL_MARGIN_SECS)
    {
        return Err(out_of_time());
    }
    // The check gets its own share and never more than the deadline leaves
    // the shortest request, however long its connection to core waits.
    let request_room = Duration::from_secs(VIEW_REQUEST_MIN_SECS + CALL_MARGIN_SECS);
    let check_room =
        Duration::from_secs(VIEW_CHECK_SECS).min(left(deadline).saturating_sub(request_room));
    within(check_room, check).map_err(|why| {
        not_sent(why.unwrap_or_else(|| {
            format!("Herdr Core took too long to show the pane's terminal on {name}")
        }))
    })?;
    // Written again for the request: the probe may have taken some of it.
    lease
        .write_again()
        .map_err(|err| not_sent(format!("{name}: could not write the lease: {err}")))?;
    let run = request_secs(left(deadline)).ok_or_else(out_of_time)?;
    commit().map_err(not_sent)?;
    let argv = bridge_argv(&far, &session);
    FarRoute {
        core,
        name,
        dir: &dir,
        argv: &argv,
        far_path: &far.path,
    }
    .send(line, run)
}

fn left(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}

/// Runs `check` on a thread of its own and waits `wait` at most for it.
/// `Err(None)` when it did not answer in time (it ends on its own calls'
/// timeouts, unwaited), `Err(Some(why))` when it said no.
fn within(wait: Duration, check: AimCheck) -> Result<(), Option<String>> {
    let (done, answer) = mpsc::channel();
    std::thread::Builder::new()
        .name("herdr-view-check".into())
        .spawn(move || {
            let _ = done.send(check());
        })
        .map_err(|err| Some(format!("could not start the check of the pane: {err}")))?;
    match answer.recv_timeout(wait) {
        Ok(result) => result.map_err(Some),
        Err(_) => Err(None),
    }
}

/// The request's run with `left` of the budget: what core's answer leaves,
/// at most `VIEW_REQUEST_SECS`; `None` below `VIEW_REQUEST_MIN_SECS`.
fn request_secs(left: Duration) -> Option<u64> {
    let run = left
        .as_secs()
        .saturating_sub(CALL_MARGIN_SECS)
        .min(VIEW_REQUEST_SECS);
    (run >= VIEW_REQUEST_MIN_SECS).then_some(run)
}

/// Why an action on a machine that is not connected is refused: the action
/// wrote no lease and sent or dialled nothing (D7, F7). The person's keys,
/// the chord's included, renew their window's lease as any input does, so
/// Herdr Core's own retry may connect the machine meanwhile (E.3 review,
/// item 11): the notice says so rather than "nothing dialled".
fn not_connected(name: &MachineName, state: &str) -> String {
    let what = match state {
        "needs_login" => format!("{name} needs a login (herdr machine login {name})"),
        "connecting" => format!("{name} is still connecting"),
        "dormant" | "unreachable" | "unknown" => {
            format!("{name} is dormant: press a key in one of its panes to wake it")
        }
        other => format!("{name} is not connected ({other})"),
    };
    format!(
        "{what}, then do this again; this action sent nothing and dialled nothing (keys in this window may still let Herdr Core's own retry connect it)"
    )
}

/// The view's `cli-<pid>` keep lease for one action: written before the
/// probe and again before the request, removed when the action ends, and
/// by the view server's graceful end while an action still runs
/// (`ViewRoute::end`, fork issue 316 M7). No renewer runs: the
/// whole action ends within its 45 s (an aim check unwaited past its share
/// ends on its own 5 s calls), and a view killed during one leaves a lease
/// that keeps an up link a minute at most and admits no dial. No
/// termination handler is put in the view server for it (the fleet CLI's
/// would end the server on SIGHUP).
struct KeepLease<'a> {
    dir: &'a MachineDir,
    holder: &'a LeaseHolder,
    leases: &'a RunningLeases,
}

/// The leases of the actions running now, and whether the view server has
/// ended: after that no lease is written. One per route (so one per view
/// server), never a process-wide list.
#[derive(Default)]
struct RunningLeases(Mutex<LeaseBook>);

#[derive(Default)]
struct LeaseBook {
    running: Vec<(MachineDir, LeaseHolder)>,
    ended: bool,
}

impl RunningLeases {
    fn book(&self) -> std::sync::MutexGuard<'_, LeaseBook> {
        match self.0.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl<'a> KeepLease<'a> {
    fn write(
        dir: &'a MachineDir,
        holder: &'a LeaseHolder,
        leases: &'a RunningLeases,
    ) -> io::Result<Self> {
        let lease = Self {
            dir,
            holder,
            leases,
        };
        lease
            .leases
            .book()
            .running
            .push((dir.clone(), holder.clone()));
        lease.write_again()?;
        Ok(lease)
    }

    /// Written under the list's lock, so a server end that removed the
    /// leases cannot be followed by a write (review item 5).
    fn write_again(&self) -> io::Result<()> {
        let book = self.leases.book();
        if book.ended {
            return Err(io::Error::other("the view server is ending"));
        }
        self.dir
            .write_keep_lease(self.holder, now_unix() + VIEW_LEASE_SECS)
    }
}

impl Drop for KeepLease<'_> {
    fn drop(&mut self) {
        {
            let mut book = self.leases.book();
            if let Some(at) = book
                .running
                .iter()
                .position(|(dir, holder)| dir.path() == self.dir.path() && holder == self.holder)
            {
                book.running.remove(at);
            }
        }
        if let Err(err) = self.dir.remove_lease(self.holder) {
            tracing::warn!(%err, "could not remove the view action's lease");
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
