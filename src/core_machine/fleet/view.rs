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
//! it is no dial: on a machine that is not connected it is refused at once,
//! with nothing written, sent or dialled (D7's rule for a call without
//! `--dial`); a key in one of the view's panes is what wakes it (D6). On a
//! connected one the view's own `cli-<pid>` keep lease holds the link for
//! the call and admits no dial, so a drop during it is not redialled on its
//! account. A refusal or a far error is the action's own answer: the client
//! shows it as a notice and the view stays connected.
//!
//! Time (review M3): the client waits 60 s for an endpoint command, holding
//! its command lane meanwhile (one command in flight per endpoint, by the
//! lane's design), and a client that gave up could do the action again. So
//! an action has one deadline, `VIEW_ACTION_BUDGET` (45 s) from the moment
//! the view took it, and every step fits in it: no wait for another action
//! (a second one at once is refused, nothing sent), core's `machine.list`
//! (5 s), the far herdr probe when its answer is stale (12 s and core's 5),
//! the check that the far pane is still the one aimed at (one `doc.get`,
//! 5 s), and the request, whose run is what is left, 15 s at most and 5 s
//! at least, else nothing is sent.

use std::io;
use std::path::PathBuf;
use std::sync::{Mutex, TryLockError};
use std::time::{Duration, Instant};

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
/// The far document's read for the aimed-at check (`view_link`'s call).
const VIEW_CHECK_SECS: u64 = 5;
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

/// One action at a time in a view process: they share its one lease file.
/// A second is refused, never queued (review M3).
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// Where a view's actions go: the primary core, herdr's machine state and
/// the view's lease holder. A test points them at a fake core and a
/// scratch folder.
#[derive(Clone)]
pub(crate) struct ViewRoute {
    core: CoreAt,
    root: Result<PathBuf, String>,
    holder: LeaseHolder,
}

impl ViewRoute {
    pub(crate) fn primary() -> Self {
        Self {
            core: CoreAt::primary(),
            root: passwd_machines_root().map_err(|err| err.to_string()),
            holder: LeaseHolder::cli(std::process::id()),
        }
    }

    #[cfg(test)]
    pub(crate) fn at(socket: &std::path::Path, root: PathBuf) -> Self {
        Self {
            core: CoreAt::at(socket),
            root: Ok(root),
            holder: LeaseHolder::cli(std::process::id()),
        }
    }
}

/// Sends one API request line (`{"id", "method", "params"}` and a newline)
/// to the far herdr of `name` and returns its one reply line, or why there
/// is none: refused unsent, or failed after sending (then the message says
/// that it may have run there). `check` runs just before the request: the
/// far pane the person aimed at is still that one (review M2), else nothing
/// is sent. Everything ends by `deadline`.
pub(crate) fn send(
    route: &ViewRoute,
    name: &MachineName,
    line: &[u8],
    deadline: Instant,
    check: &dyn Fn() -> Result<(), String>,
) -> Result<Vec<u8>, String> {
    let not_sent = |why: String| format!("{why}; nothing was sent to {name}");
    let _one = match ONE_AT_A_TIME.try_lock() {
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
    let core = &route.core;
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
    let lease = KeepLease::write(&dir, &route.holder)
        .map_err(|err| not_sent(format!("{name}: could not write the lease: {err}")))?;
    let far = far_herdr_within(core, name, &dir, &session, &target.session, VIEW_PROBE_SECS)?;
    if left(deadline)
        < Duration::from_secs(VIEW_CHECK_SECS + VIEW_REQUEST_MIN_SECS + CALL_MARGIN_SECS)
    {
        return Err(out_of_time());
    }
    check().map_err(not_sent)?;
    // Written again for the request: the probe may have taken some of it.
    lease
        .write_again()
        .map_err(|err| not_sent(format!("{name}: could not write the lease: {err}")))?;
    let run = request_secs(left(deadline)).ok_or_else(out_of_time)?;
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

/// The request's run with `left` of the budget: what core's answer leaves,
/// at most `VIEW_REQUEST_SECS`; `None` below `VIEW_REQUEST_MIN_SECS`.
fn request_secs(left: Duration) -> Option<u64> {
    let run = left
        .as_secs()
        .saturating_sub(CALL_MARGIN_SECS)
        .min(VIEW_REQUEST_SECS);
    (run >= VIEW_REQUEST_MIN_SECS).then_some(run)
}

/// Why an action on a machine that is not connected is refused: nothing
/// was written, sent or dialled (D7, F7).
fn not_connected(name: &MachineName, state: &str) -> String {
    let what = match state {
        "needs_login" => format!("{name} needs a login (herdr machine login {name})"),
        "connecting" => format!("{name} is still connecting"),
        "dormant" | "unreachable" | "unknown" => {
            format!("{name} is dormant: press a key in one of its panes to wake it")
        }
        other => format!("{name} is not connected ({other})"),
    };
    format!("{what}, then do this again; nothing was sent and nothing dialled")
}

/// The view's `cli-<pid>` keep lease for one action: written before the
/// probe and again before the request, removed when the action ends. No
/// renewer runs: the whole action ends within its 45 s, and a view killed
/// during one leaves a lease that keeps an up link a minute at most and
/// admits no dial. No termination handler is put in the view server for it
/// (the fleet CLI's would end the server on SIGHUP).
struct KeepLease<'a> {
    dir: &'a MachineDir,
    holder: &'a LeaseHolder,
}

impl<'a> KeepLease<'a> {
    fn write(dir: &'a MachineDir, holder: &'a LeaseHolder) -> io::Result<Self> {
        let lease = Self { dir, holder };
        lease.write_again()?;
        Ok(lease)
    }

    fn write_again(&self) -> io::Result<()> {
        self.dir
            .write_keep_lease(self.holder, now_unix() + VIEW_LEASE_SECS)
    }
}

impl Drop for KeepLease<'_> {
    fn drop(&mut self) {
        if let Err(err) = self.dir.remove_lease(self.holder) {
            tracing::warn!(%err, "could not remove the view action's lease");
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
