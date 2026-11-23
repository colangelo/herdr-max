//! `herdr --machine <name>` to a Herdr Core machine (core-phase5 D7, E.1,
//! E.2): the fleet CLI's requests go to the far side's own herdr through
//! `machine.exec` of `<far herdr> --session S remote-api-bridge --request`, with the
//! request line as its input. Those are the bytes `remote-api-bridge` reads
//! from ssh on the old path, so the reply is the far side's own, with its
//! pane ids.
//!
//! - `machine.exec` never dials (F7). On a machine that is not connected the
//!   call is refused with exit 75 and nothing is written or dialled, unless
//!   the caller passed `--dial`: then a `cli-<pid>` lease that admits a dial
//!   comes first and one connect goes through the dialer's rules (D3a). Once
//!   the machine is connected, and from the start on a connected one, the
//!   lease only keeps the link up: a drop during the call is never redialled
//!   on its account.
//! - The lease lives 60 s past its last write and is written again every
//!   30 s while the CLI runs, so a killed CLI's lease ends within a minute;
//!   SIGINT, SIGTERM and SIGHUP remove it and the bridge folder at once.
//! - Every argv starts with D5's `env -u` wrapper (F6): the far core's own
//!   environment may carry a herdr pane's `HERDR_SOCKET_PATH`.
//! - A request is sent once (no retry after send); a reply cut at 8 MiB,
//!   timed out, or not one whole reply line with the request's id is an
//!   error with none of its output printed (F12); a request that is not one
//!   line within the far API's 1 MiB limit is refused before it is sent.
//!   Every failure after sending says that the request may have run.
//! - The far herdr's path and capabilities are read once per link
//!   establishment (the link's dial record), kept in `far_herdr.json` for at
//!   most 5 min, and forgotten when a request through it fails there.
//!
//! The CLI reaches this through a local socket that speaks herdr's JSON API,
//! one request per connection as the API does, so every CLI verb that works
//! with `--machine` today works here unchanged.

use std::io::{self, Read as _, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::Duration;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::dialer::{self, CoreLink, Outcome};
use super::name::MachineName;
use super::state::{
    newest_act, now_unix, passwd_machines_root, write_atomic, Gate, GateClass, LeaseHolder,
    MachineDir, FAR_CORE_DOWN_SECS,
};
use crate::pane::core_machines::{self as core, CallError};
use crate::remote::visible_line;

/// The exit code of a refusal on a machine that is not connected (D7, F7):
/// `EX_TEMPFAIL`, as core-link's own refusals.
pub(crate) const EXIT_DORMANT: i32 = 75;
/// The most `machine.exec` takes as input, raw: 8 MiB. Core's schema says
/// "8 MB"; measured on hc 0.1.21, 8 388 608 bytes run and 8 388 609 are
/// refused as `invalid_params`.
pub(crate) const INPUT_LIMIT: usize = 8 * 1024 * 1024;
/// The longest request line the bridge sends, its newline aside: the far
/// API's own limit, so a longer one is refused here, unsent (M6).
const REQUEST_LIMIT: usize = crate::api::MAX_REQUEST_LINE;
/// `machine.exec`'s longest run: core ends a program after 600 s at most.
const EXEC_MAX_SECS: u64 = 600;
/// A request with its own `timeout_ms` runs that long and this much more,
/// for the far herdr's start and its answer.
const REQUEST_SLACK_SECS: u64 = 10;
/// The shortest run given to a request with its own timeout (core's
/// default).
const EXEC_MIN_SECS: u64 = 60;
/// The status probe (`ping`): its run and the core call around it end
/// before the CLI's own probe gives up (`cli::target`, 15 s), so the bridge
/// is idle when the CLI leaves.
pub(crate) const PING_EXEC_SECS: u64 = 9;
/// How long a core call waits past its run's own timeout for core's answer.
pub(crate) const CALL_MARGIN_SECS: u64 = 5;
/// The `cli-<pid>` lease (D3): in force this long past its last write,
/// written again this often while the CLI runs.
const LEASE_LIFE_SECS: f64 = 60.0;
const LEASE_RENEW: Duration = Duration::from_secs(30);
/// How often the lease's renewer reads the wall clock. Its waits are on
/// the monotonic clock, which stops while the Mac sleeps; the lease's
/// expiry is wall-clock time, which does not. So it looks this often and
/// renews once the wall clock says a renewal is due: after a sleep the
/// lease is written within this long of the wake (#310).
const LEASE_TICK: Duration = Duration::from_secs(1);
/// The probe of the far herdr: a few local programs there.
const PROBE_SECS: u64 = 30;
const PROBE_MARK: &str = "herdr-probe-v1";
/// How long a probe's answer is trusted on the same link, so a far herdr
/// upgraded in place is read again within minutes.
const FAR_HERDR_TTL_SECS: f64 = 300.0;
/// How long dropping the bridge waits for a request still in flight.
const BRIDGE_JOIN_WAIT: Duration = Duration::from_millis(500);

/// Finds the far herdr that answers `remote-api-bridge --check` and
/// `remote-api-bridge --request --check` (the one-request bridge, which a
/// waiting request needs: `machine.exec` closes stdin once the request is
/// in, and the stream bridge passes that end on to the API), by the
/// names given as arguments after the far herdr session, on the far `PATH`
/// and in the usual install folders, and prints it with its `status client
/// --json` for that session. A fixed script: names come as arguments, never
/// as script text.
const PROBE_SCRIPT: &str = r#"set -f
session=$1
shift
for name do
    for candidate in "$(command -v "$name" 2>/dev/null)" "${HOME:-/nonexistent}/.local/bin/$name" "/opt/homebrew/bin/$name" "/usr/local/bin/$name" "/home/linuxbrew/.linuxbrew/bin/$name"; do
        case "$candidate" in
            */mise/shims/*) continue ;;
            /*) ;;
            *) continue ;;
        esac
        [ -x "$candidate" ] || continue
        capability=$("$candidate" remote-api-bridge --check </dev/null 2>/dev/null) || continue
        [ "$capability" = herdr-api-bridge-v1 ] || continue
        request=$("$candidate" remote-api-bridge --request --check </dev/null 2>/dev/null) || continue
        [ "$request" = herdr-api-request-v1 ] || continue
        printf 'herdr-probe-v1\n%s\n' "$candidate"
        "$candidate" --session "$session" status client --json </dev/null 2>/dev/null
        exit 0
    done
done
echo 'no herdr there answers remote-api-bridge --request' >&2
exit 2"#;

/// What `herdr --machine <selector>` found among core machines.
pub(crate) enum Lookup {
    /// Not a core machine here; the caller reports the selector unknown.
    NotCore,
    /// Refused before any request was sent.
    Refused {
        message: String,
        code: i32,
    },
    Ready(Box<FleetLink>),
}

fn refused(code: i32, message: impl Into<String>) -> Lookup {
    Lookup::Refused {
        message: message.into(),
        code,
    }
}

/// The primary core, or a test's fake, by its socket. Never starts a core.
#[derive(Debug, Clone)]
pub(crate) struct CoreAt(PathBuf);

impl CoreAt {
    pub(crate) fn primary() -> Self {
        Self(core::primary_socket())
    }

    #[cfg(test)]
    pub(crate) fn at(socket: &Path) -> Self {
        Self(socket.to_path_buf())
    }

    fn call(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Option<Value>, CallError> {
        core::call_running_at(&self.0, method, params, timeout)
    }
}

impl CoreLink for CoreAt {
    fn state(&self, machine: &MachineName) -> Result<Option<String>, String> {
        let Some(reply) = self
            .call("machine.list", json!({}), Duration::from_secs(5))
            .map_err(|err| err.to_string())?
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
        match self
            .call(
                "machine.connect",
                json!({"machine": machine.as_str()}),
                timeout,
            )
            .map_err(|err| err.to_string())?
        {
            Some(reply) => Ok(reply["state"].as_str().unwrap_or("unknown").to_string()),
            None => Err("herdr-core is not running".into()),
        }
    }
}

/// Resolves `selector` as a core machine herdr added and makes it ready
/// for requests, or says why not.
pub(crate) fn open(selector: &str, dial: bool) -> Lookup {
    let Ok(name) = MachineName::parse(selector) else {
        return Lookup::NotCore;
    };
    open_at(&CoreAt::primary(), passwd_machines_root(), &name, dial)
}

fn open_at(core: &CoreAt, root: io::Result<PathBuf>, name: &MachineName, dial: bool) -> Lookup {
    let root = match root {
        Ok(root) => root,
        // Without herdr's state no machine is herdr's; a name core does not
        // list either is an unknown selector, as a typo is (exit 2).
        Err(err) => {
            return match core.state(name) {
                Ok(Some(_)) => refused(
                    1,
                    format!("{name}: cannot find herdr's machine state: {err}"),
                ),
                _ => Lookup::NotCore,
            }
        }
    };
    let dir = MachineDir::at(&root, name);
    // A name herdr did not add is no core machine of herdr's: refused as an
    // unknown selector, whose message names `machine status --core`, with
    // no core call, so a mistyped name costs no `machine.list` (#310).
    if !dir.target_path().exists() {
        return Lookup::NotCore;
    }
    prepare(core, name, &dir, dial)
}

/// The far herdr session whose primary core is `core_session` on the far
/// side (the inverse of herdr's own session-to-core mapping): `default`
/// for core's default session (or a debug build's default), `X` for
/// `herdr-X` and `herdrdev-X`. `None` for a core session no herdr session
/// uses as its primary. Which of the two builds runs there is the far
/// herdr's to say (`check_far_session`).
pub(crate) fn far_herdr_session(core_session: &str) -> Option<String> {
    if matches!(core_session, "default" | "herdr" | "herdrdev") {
        return Some(crate::session::DEFAULT_SESSION_NAME.to_string());
    }
    core_session
        .strip_prefix("herdrdev-")
        .or_else(|| core_session.strip_prefix("herdr-"))
        .filter(|session| crate::session::validate_name(session).is_ok())
        .map(str::to_string)
}

fn prepare(core: &CoreAt, name: &MachineName, dir: &MachineDir, dial: bool) -> Lookup {
    if let Err(err) = dir.check() {
        return refused(1, format!("{name}: {err}"));
    }
    let target = match dir.read_target() {
        Ok(target) => target,
        Err(err) => return refused(1, format!("{name}: {err}")),
    };
    let Some(session) = far_herdr_session(&target.session) else {
        return refused(
            2,
            format!(
                "{name} links to Herdr Core session {}, which is no herdr session's own; \
                 --machine needs a machine added with --remote-session default or herdr-<session>",
                target.session
            ),
        );
    };
    let state = match core.state(name) {
        Ok(Some(state)) => state,
        Ok(None) => {
            return refused(
                1,
                format!(
                    "herdr-core here does not list {name}, or is not running; see herdr machine status --core {name}"
                ),
            )
        }
        Err(err) => return refused(1, format!("herdr-core did not list its machines: {err}")),
    };
    let now = now_unix();
    let holder = LeaseHolder::cli(std::process::id());
    let lease = if state == "connected" {
        // S4: the call rides the link. Its lease keeps the link up and
        // admits no dial, so a drop during the call is not redialled.
        match LeaseKeeper::start(dir.clone(), holder, LeaseUse::Keep) {
            Ok(lease) => lease,
            Err(err) => return refused(1, format!("{name}: could not write the lease: {err}")),
        }
    } else {
        if !dial {
            return refused(EXIT_DORMANT, not_connected_message(name, dir, &state, now));
        }
        if let Some(message) = dial_refusal(name, dir.read_gate(), now) {
            return refused(1, message);
        }
        // D3, F8: the lease before the connect it leads to; `--dial` is a
        // person's or an agent's announced act (S4b), so it reopens
        // `retry_spent` (D2 step 9).
        let lease = match LeaseKeeper::start(dir.clone(), holder, LeaseUse::Dial { act: now }) {
            Ok(lease) => lease,
            Err(err) => return refused(1, format!("{name}: could not write the lease: {err}")),
        };
        dir.log("CLI --dial: lease, then connect");
        let outcome = dialer::dial_once(
            core,
            name,
            &dir.connect_lock_path(),
            dialer::Timing::default(),
        );
        if outcome.state() != Some("connected") {
            return refused(
                1,
                format!(
                    "{name} did not connect ({}); see herdr machine status --core {name}",
                    describe(&outcome)
                ),
            );
        }
        // The one announced dial is done: from here the lease only keeps
        // the link, as on a machine that was connected (D7).
        if let Err(err) = lease.keep_only() {
            return refused(1, format!("{name}: could not write the lease: {err}"));
        }
        lease
    };
    let far = match far_herdr(core, name, dir, &session, &target.session) {
        Ok(far) => far,
        Err(message) => return refused(1, message),
    };
    match FleetLink::start(core.clone(), name.clone(), dir.clone(), lease, far, session) {
        Ok(link) => Lookup::Ready(Box::new(link)),
        Err(err) => refused(
            1,
            format!("{name}: could not start the request bridge: {err}"),
        ),
    }
}

fn describe(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Failed(err) => format!("machine.connect failed: {err}"),
        Outcome::NotListed => "herdr-core does not list it".into(),
        other => other.state().unwrap_or("unknown").to_string(),
    }
}

/// D7's refusal without `--dial` (S4a): nothing was written or dialled.
fn not_connected_message(name: &MachineName, dir: &MachineDir, state: &str, now: f64) -> String {
    let what = if newest_act(&dir.leases(), now).is_none() {
        "is dormant".to_string()
    } else {
        format!("is not connected ({state})")
    };
    format!(
        "{name} {what}; reaching it opens ssh (a Touch ID or Tailscale check can prompt). \
         Announce it, then rerun with --dial, or run herdr machine wake {name}."
    )
}

/// Why `--dial` must not dial: the gate is closed by a class that only a
/// login reopens, or the far core was found down within 5 min, as the
/// first failure (`far_core_down`) or as the retry's (`retry_spent` over
/// `far_core_down`) (D10, D2 step 9). Any other `retry_spent` is reopened
/// by the `--dial` lease itself.
fn dial_refusal(name: &MachineName, gate: io::Result<Option<Gate>>, now: f64) -> Option<String> {
    let gate = match gate {
        Ok(gate) => gate?,
        Err(err) => {
            return Some(format!(
                "{name}: unreadable gate.json ({err}); a person runs: herdr machine login {name}"
            ))
        }
    };
    let far_core_down = match gate.class {
        GateClass::FarCoreDown => true,
        GateClass::RetrySpent => gate.detail.as_deref() == Some(GateClass::FarCoreDown.as_str()),
        _ => false,
    };
    match gate.class {
        GateClass::NeedsCheck | GateClass::AuthStalled | GateClass::AuthFailed => Some(format!(
            "{name}: the gate is closed ({}); --dial cannot reopen it. A person runs: herdr machine login {name}",
            gate.class.as_str()
        )),
        _ if far_core_down && now < gate.at + FAR_CORE_DOWN_SECS => Some(format!(
            "{name}: Herdr Core was not running there {:.0} s ago ({}); start Herdr there, then run herdr machine wake {name}",
            now - gate.at,
            gate.class.as_str()
        )),
        _ => None,
    }
}

/// What a `cli-<pid>` lease lets core-link do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum LeaseUse {
    /// Admit a dial: `--dial`'s own connect, with the time of that act.
    Dial { act: f64 },
    /// Keep an up link up, and admit no dial (D7).
    Keep,
}

/// How long a lease lives past each write, how often it is written, how
/// often the renewer reads the clock, and the wall clock itself.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LeaseTiming {
    pub life: f64,
    pub renew: Duration,
    pub tick: Duration,
    pub clock: fn() -> f64,
}

impl Default for LeaseTiming {
    fn default() -> Self {
        Self {
            life: LEASE_LIFE_SECS,
            renew: LEASE_RENEW,
            tick: LEASE_TICK,
            clock: now_unix,
        }
    }
}

/// The `cli-<pid>` lease (D3): written at the start, again every 30 s
/// while the CLI runs, each write in force for 60 s; removed when the CLI
/// is done, and by SIGINT, SIGTERM or SIGHUP. A CLI killed outright leaves
/// a lease that ends within a minute.
pub(crate) struct LeaseKeeper(Arc<LeaseInner>);

struct LeaseInner {
    dir: MachineDir,
    holder: LeaseHolder,
    timing: LeaseTiming,
    state: Mutex<LeaseState>,
    renew: Condvar,
}

struct LeaseState {
    used_for: LeaseUse,
    released: bool,
}

impl LeaseInner {
    fn path(&self) -> PathBuf {
        self.dir.leases_dir().join(self.holder.as_str())
    }

    /// Writes the lease while it is remembered as a leftover: once the
    /// termination cleanup took it, a renewal racing the exit writes
    /// nothing.
    fn write(&self, used_for: LeaseUse) -> io::Result<()> {
        let leftovers = lock(&LEFTOVERS);
        if !leftovers.paths.contains(&self.path()) {
            return Ok(());
        }
        let expires_at = (self.timing.clock)() + self.timing.life;
        match used_for {
            LeaseUse::Dial { act } => self.dir.write_lease(&self.holder, expires_at, act),
            LeaseUse::Keep => self.dir.write_keep_lease(&self.holder, expires_at),
        }
    }

    fn release(&self) {
        let mut state = lock(&self.state);
        if state.released {
            return;
        }
        state.released = true;
        if let Err(err) = self.dir.remove_lease(&self.holder) {
            tracing::warn!(%err, "could not remove the fleet CLI's lease");
        }
        forget_leftover(&self.path());
        self.renew.notify_all();
    }
}

impl LeaseKeeper {
    fn start(dir: MachineDir, holder: LeaseHolder, used_for: LeaseUse) -> io::Result<Self> {
        Self::start_with(dir, holder, used_for, LeaseTiming::default())
    }

    pub(crate) fn start_with(
        dir: MachineDir,
        holder: LeaseHolder,
        used_for: LeaseUse,
        timing: LeaseTiming,
    ) -> io::Result<Self> {
        let inner = Arc::new(LeaseInner {
            dir,
            holder,
            timing,
            state: Mutex::new(LeaseState {
                used_for,
                released: false,
            }),
            renew: Condvar::new(),
        });
        // Remembered before the write, so a signal between the two still
        // removes it.
        make_leftover(&inner.path(), || Ok(()))?;
        let keeper = Self(inner);
        keeper.0.write(used_for)?;
        let renewer = Arc::clone(&keeper.0);
        std::thread::Builder::new()
            .name("herdr-fleet-lease".into())
            .spawn(move || {
                let timing = renewer.timing;
                let mut written_at = (timing.clock)();
                let mut state = lock(&renewer.state);
                while !state.released {
                    state = match renewer.renew.wait_timeout(state, timing.tick) {
                        Ok((state, _)) => state,
                        Err(poisoned) => poisoned.into_inner().0,
                    };
                    if state.released {
                        break;
                    }
                    // Due by the wall clock, which a sleep moves on (or
                    // set back: then too).
                    let now = (timing.clock)();
                    if now - written_at < timing.renew.as_secs_f64() && now >= written_at {
                        continue;
                    }
                    written_at = now;
                    if let Err(err) = renewer.write(state.used_for) {
                        tracing::warn!(%err, "could not renew the fleet CLI's lease");
                    }
                }
            })?;
        Ok(keeper)
    }

    /// From now on the lease only keeps the link (after `--dial`'s connect).
    pub(crate) fn keep_only(&self) -> io::Result<()> {
        let mut state = lock(&self.0.state);
        state.used_for = LeaseUse::Keep;
        self.0.write(LeaseUse::Keep)
    }

    /// Removes the lease now; it is not written again.
    pub(crate) fn release(&self) {
        self.0.release();
    }
}

impl Drop for LeaseKeeper {
    fn drop(&mut self) {
        self.0.release();
    }
}

/// The files and folders a fleet CLI leaves until it ends: its lease and
/// its bridge folder. The termination signals remove them (D7).
static LEFTOVERS: Mutex<Leftovers> = Mutex::new(Leftovers {
    paths: Vec::new(),
    terminating: false,
});

struct Leftovers {
    paths: Vec<PathBuf>,
    /// Set by the termination cleanup: nothing is made or kept after it.
    terminating: bool,
}

/// Makes a leftover with `make` and remembers it, as one step under the
/// leftovers' lock, so the termination cleanup runs before it or after it
/// is remembered, never between; once that cleanup ran, nothing is made
/// (security review 2026-11-23 M3).
fn make_leftover<T>(path: &Path, make: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    if let Err(err) = crate::platform::core_link::clean_up_on_termination(clean_up_leftovers) {
        tracing::warn!(%err, "no cleanup on SIGINT or SIGTERM; the lease ends by its expiry");
    }
    let mut leftovers = lock(&LEFTOVERS);
    if leftovers.terminating {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "the fleet CLI is ending",
        ));
    }
    let made = make()?;
    leftovers.paths.push(path.to_path_buf());
    Ok(made)
}

fn forget_leftover(path: &Path) {
    lock(&LEFTOVERS).paths.retain(|kept| kept != path);
}

fn clean_up_leftovers() {
    // Held while removing, so nothing new is made meanwhile.
    let mut leftovers = lock(&LEFTOVERS);
    leftovers.terminating = true;
    for path in std::mem::take(&mut leftovers.paths) {
        let removed = match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => remove_bridge_folder(&path),
            Ok(_) => std::fs::remove_file(&path),
            Err(_) => Ok(()),
        };
        if let Err(err) = removed {
            tracing::warn!(%err, path = %path.display(), "could not remove a fleet CLI leftover");
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// The far herdr as the probe found it over one link (E.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct FarHerdr {
    /// The link it was read over: that link's dial record.
    pub link_at: f64,
    pub link_pid: u32,
    /// When it was read; a cache older than 5 min is read again.
    #[serde(default)]
    pub probed_at: f64,
    /// The far herdr session it was asked about.
    #[serde(default)]
    pub session: String,
    /// Its absolute path there.
    pub path: String,
    #[serde(default)]
    pub version: Option<String>,
    /// Its private protocol, from `status client --json`.
    #[serde(default)]
    pub protocol: Option<u32>,
    #[serde(default)]
    pub endpoint_capabilities: Vec<String>,
    /// The Herdr Core session that herdr session's panes run in, as it says
    /// (absent from a herdr older than this field).
    #[serde(default)]
    pub core_session: Option<String>,
    /// It answers `remote-api-bridge --request`: false in a probe kept by
    /// an older build, which is probed again.
    #[serde(default)]
    pub request_bridge: bool,
}

/// The far herdr found over the machine's current link, if one was.
pub(crate) fn cached_far_herdr(dir: &MachineDir) -> Option<FarHerdr> {
    let link = dir.read_dial_record()?;
    let cached: FarHerdr = serde_json::from_slice(&read_private(&dir.far_herdr_path())?).ok()?;
    (cached.link_at == link.at && cached.link_pid == link.pid && is_far_path(&cached.path))
        .then_some(cached)
}

/// The most a kept probe answer can be: a path of 4 KiB and its status.
const FAR_HERDR_FILE_LIMIT: u64 = 64 * 1024;

/// `path` as this account wrote it: a regular file, not a symlink, of this
/// account, that no one else can read or write (0600, as `write_atomic`
/// makes it), at most 64 KiB. `None` for anything else, so a file put
/// there another way names no far program to run (security review
/// 2026-11-23 M2).
fn read_private(path: &Path) -> Option<Vec<u8>> {
    use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    let private = meta.is_file()
        && meta.uid() == crate::platform::core_link::uid()
        && meta.mode() & 0o077 == 0
        && meta.len() <= FAR_HERDR_FILE_LIMIT;
    if !private {
        tracing::warn!(path = %path.display(), "ignoring a far herdr record this account did not keep privately");
        return None;
    }
    let mut bytes = Vec::new();
    file.take(FAR_HERDR_FILE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(bytes)
}

/// The far herdr for `session`, read once per link establishment (D7) and
/// again after 5 min, and checked to run that session in `linked`, the
/// core session the machine was added with.
fn far_herdr(
    core: &CoreAt,
    name: &MachineName,
    dir: &MachineDir,
    session: &str,
    linked: &str,
) -> Result<FarHerdr, String> {
    let now = now_unix();
    let far = match cached_far_herdr(dir).filter(|cached| {
        cached.session == session
            && cached.request_bridge
            && (0.0..FAR_HERDR_TTL_SECS).contains(&(now - cached.probed_at))
    }) {
        Some(cached) => cached,
        None => probe_far_herdr(core, name, dir, session)?,
    };
    check_far_session(name, &far, linked)?;
    Ok(far)
}

fn probe_far_herdr(
    core: &CoreAt,
    name: &MachineName,
    dir: &MachineDir,
    session: &str,
) -> Result<FarHerdr, String> {
    let link = dir.read_dial_record();
    let mut argv = core::machine_env_wrapper();
    argv.extend(
        ["/bin/sh", "-c", PROBE_SCRIPT, "herdr-probe", session]
            .into_iter()
            .map(String::from),
    );
    argv.extend(binary_names());
    // The probe runs before the request is sent: none of its failures may
    // say that the request may have run (#310).
    let done = exec(core, name, &argv, None, PROBE_SECS).map_err(|failure| {
        format!(
            "could not find the herdr on {name}: {}; the request was not sent",
            failure.message()
        )
    })?;
    if done.code != Some(0) {
        return Err(format!(
            "no herdr on {name} answers remote-api-bridge --request (looked for {} on its PATH and in \
             ~/.local/bin, /opt/homebrew/bin, /usr/local/bin): {}; install or update Herdr there",
            binary_names().join(", "),
            done.stderr_tail()
        ));
    }
    let (path, status) = parse_probe(&done.stdout)
        .ok_or_else(|| format!("the herdr probe on {name} answered something else"))?;
    let far = FarHerdr {
        link_at: link.as_ref().map_or(0.0, |link| link.at),
        link_pid: link.as_ref().map_or(0, |link| link.pid),
        probed_at: now_unix(),
        session: session.to_string(),
        path,
        version: status.version,
        protocol: status.protocol,
        endpoint_capabilities: status.endpoint_capabilities,
        core_session: status.core_session,
        request_bridge: true,
    };
    if link.is_some() {
        match serde_json::to_vec_pretty(&far) {
            Ok(bytes) => {
                if let Err(err) = write_atomic(&dir.far_herdr_path(), &bytes) {
                    tracing::warn!(%err, "could not keep the far herdr's probe");
                }
            }
            Err(err) => tracing::warn!(%err, "could not encode the far herdr's probe"),
        }
    }
    dir.log(&format!("FAR herdr {}", far.path));
    Ok(far)
}

/// Whether the far herdr found runs its session in `linked`, the core
/// session the machine was added with, so `--machine` reaches the herdr
/// whose core it is linked to. A herdr too old to say is taken for a
/// release build, whose sessions never use a debug build's `herdrdev`.
fn check_far_session(name: &MachineName, far: &FarHerdr, linked: &str) -> Result<(), String> {
    match far.core_session.as_deref() {
        Some(core_session) if core_session == linked => Ok(()),
        Some(core_session) => {
            let core_session = visible_line(core_session);
            Err(format!(
                "the herdr found on {name} ({}) runs session {} in Herdr Core session {core_session}, \
                 not {linked}, the one {name} was added with; --machine would reach another herdr. \
                 Put the herdr of that core first on its PATH there, or add the machine again with \
                 --remote-session {core_session}",
                far.path, far.session
            ))
        }
        None if linked.starts_with("herdrdev") => Err(format!(
            "{name} links to Herdr Core session {linked}, a debug build's, and the herdr found there ({}) \
             does not say which core session it uses; update Herdr there",
            far.path
        )),
        None => Ok(()),
    }
}

fn forget_far_herdr(dir: &MachineDir) {
    if let Err(err) = std::fs::remove_file(dir.far_herdr_path()) {
        if err.kind() != io::ErrorKind::NotFound {
            tracing::warn!(%err, "could not forget the far herdr's probe");
        }
    }
}

/// Plain names for the far herdr, as the old remote looks for it: the name
/// this herdr runs as (a `herdr-beta` finds the far `herdr-beta`), then
/// `herdr`.
fn binary_names() -> Vec<String> {
    let mut names = Vec::new();
    let invoked = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.file_name()?.to_str().map(str::to_string))
        .filter(|name| is_binary_name(name));
    names.extend(invoked);
    if !names.iter().any(|name| name == "herdr") {
        names.push("herdr".into());
    }
    names
}

fn is_binary_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// An absolute path with no control characters, at most 4 KiB: it goes
/// into an argv, never a shell line.
fn is_far_path(path: &str) -> bool {
    path.starts_with('/') && path.len() <= 4096 && !path.chars().any(char::is_control)
}

#[derive(Debug, Default, Deserialize)]
struct ClientStatus {
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    protocol: Option<u32>,
    #[serde(default)]
    endpoint_capabilities: Vec<String>,
    #[serde(default)]
    core_session: Option<String>,
}

/// The probe's answer: its mark, the path, then the path's `status client
/// --json` (absent from a herdr too old to print it).
fn parse_probe(stdout: &[u8]) -> Option<(String, ClientStatus)> {
    let text = std::str::from_utf8(stdout).ok()?;
    let mut lines = text.lines();
    if lines.next()? != PROBE_MARK {
        return None;
    }
    let path = lines.next()?.to_string();
    if !is_far_path(&path) {
        return None;
    }
    let status = lines
        .rev()
        .filter(|line| !line.trim().is_empty())
        .find_map(|line| serde_json::from_str::<ClientStatus>(line).ok())
        .unwrap_or_default();
    Some((path, status))
}

/// A `machine.exec` that ran to its end: neither cut nor timed out.
#[derive(Debug)]
struct Ran {
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl Ran {
    /// The last line it wrote to stderr, for a message: at most 400
    /// characters, its controls shown as escapes (far text, M1).
    fn stderr_tail(&self) -> String {
        let text = String::from_utf8_lossy(&self.stderr);
        let tail: String = text
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("no message")
            .chars()
            .take(400)
            .collect();
        visible_line(&tail).into_owned()
    }
}

#[derive(Debug, Deserialize)]
struct ExecResult {
    #[serde(default)]
    code: Option<i32>,
    #[serde(default)]
    stdout: String,
    #[serde(default)]
    stderr: String,
    truncated: bool,
    timed_out: bool,
}

/// How every failure after a request may have reached the far side ends.
const MAY_HAVE_RUN: &str = "it may have run there, and it is not sent again";

/// Why one `machine.exec` gave no whole run, by what it says of the
/// program: never started, may have run, or ran.
#[derive(Debug, PartialEq)]
enum ExecFailure {
    NotSent(String),
    MayHaveRun(String),
    Ran(String),
}

impl ExecFailure {
    /// What happened, without what it means for the program.
    fn message(&self) -> &str {
        match self {
            Self::NotSent(message) | Self::MayHaveRun(message) | Self::Ran(message) => message,
        }
    }

    /// The message for an API request carried by the run (F12).
    fn for_request(&self) -> String {
        match self {
            Self::NotSent(message) => message.clone(),
            Self::MayHaveRun(message) => format!("{message}; {MAY_HAVE_RUN}"),
            Self::Ran(message) => format!("{message}; it ran there, and it is not sent again"),
        }
    }
}

/// One `machine.exec`, sent once. Refusals, a lost answer, a cut reply and
/// a timed-out run are failures that say whether the program may have
/// run; a reply's output is returned only when it is whole (F12). Core's
/// messages can quote the far side, so they are shown escaped (M1).
fn exec(
    core: &CoreAt,
    name: &MachineName,
    argv: &[String],
    input: Option<&[u8]>,
    timeout_secs: u64,
) -> Result<Ran, ExecFailure> {
    use ExecFailure::{MayHaveRun, NotSent, Ran as RanThere};
    if input.is_some_and(|input| input.len() > INPUT_LIMIT) {
        return Err(NotSent(format!(
            "the request is over 8 MiB, so it was not sent to {name}"
        )));
    }
    let mut params = json!({"on": name.as_str(), "argv": argv, "timeout": timeout_secs});
    if let Some(input) = input {
        params["input"] = Value::String(base64::engine::general_purpose::STANDARD.encode(input));
    }
    let reply = match core.call(
        "machine.exec",
        params,
        Duration::from_secs(timeout_secs + CALL_MARGIN_SECS),
    ) {
        Ok(Some(reply)) => reply,
        Ok(None) => return Err(NotSent("herdr-core is not running here".into())),
        // Core answers `unavailable` both for a machine that is not
        // connected and for a link lost during the run ("lost the line to
        // <m> during the call", hc 0.1.21), so it may have run.
        Err(CallError::Rpc { kind, message }) if kind == "unavailable" => {
            return Err(MayHaveRun(format!(
                "herdr-core lost {name} (unavailable: {})",
                visible_line(&message)
            )))
        }
        Err(CallError::Rpc { kind, message }) => {
            return Err(NotSent(format!(
                "herdr-core refused the request for {name}: {}: {}",
                visible_line(&kind),
                visible_line(&message)
            )))
        }
        Err(CallError::NoAnswer(why)) => {
            return Err(MayHaveRun(format!(
                "no answer from {name} after the request was sent ({})",
                visible_line(&why)
            )))
        }
        Err(CallError::NotSent(why)) => {
            return Err(NotSent(format!(
                "herdr-core here is not reachable: {}",
                visible_line(&why)
            )))
        }
    };
    let result: ExecResult = serde_json::from_value(reply).map_err(|err| {
        MayHaveRun(format!(
            "herdr-core's answer from {name} does not parse: {}",
            visible_line(&err.to_string())
        ))
    })?;
    if result.timed_out {
        return Err(MayHaveRun(format!(
            "the reply from {name} timed out after {timeout_secs} s"
        )));
    }
    if result.truncated {
        return Err(RanThere(format!("the reply from {name} was cut at 8 MiB")));
    }
    let decode = |field: &str, raw: &str| {
        base64::engine::general_purpose::STANDARD
            .decode(raw)
            .map_err(|err| {
                MayHaveRun(format!(
                    "the {field} of the reply from {name} is not base64: {err}"
                ))
            })
    };
    Ok(Ran {
        code: result.code,
        stdout: decode("stdout", &result.stdout)?,
        stderr: decode("stderr", &result.stderr)?,
    })
}

/// `machine.exec`'s timeout for one API request. The status probe gets
/// `PING_EXEC_SECS`; a request with its own `timeout_ms` that long and
/// 10 s more, at least 60 s; one with none (a wait with no `--timeout`,
/// which the old path waits out however long) core's longest, 600 s. A
/// request whose own timeout needs more than 600 s is refused unsent.
fn exec_timeout(request: &[u8]) -> Result<u64, String> {
    let Ok(request) = serde_json::from_slice::<Value>(request) else {
        return Ok(EXEC_MAX_SECS);
    };
    if request["method"] == "ping" {
        return Ok(PING_EXEC_SECS);
    }
    let Some(ms) = request["params"]["timeout_ms"].as_u64() else {
        return Ok(EXEC_MAX_SECS);
    };
    let secs = ms.div_ceil(1000).saturating_add(REQUEST_SLACK_SECS);
    if secs > EXEC_MAX_SECS {
        return Err(format!(
            "its timeout ({} s) is longer than Herdr Core runs a program on a machine ({} s with {} s \
             for the far herdr), so it was not sent; rerun with --timeout {} or less",
            ms.div_ceil(1000),
            EXEC_MAX_SECS,
            REQUEST_SLACK_SECS,
            (EXEC_MAX_SECS - REQUEST_SLACK_SECS) * 1000
        ));
    }
    Ok(secs.max(EXEC_MIN_SECS))
}

/// The argv that carries one request (D7, F6).
fn bridge_argv(far: &FarHerdr, session: &str) -> Vec<String> {
    let mut argv = core::machine_env_wrapper();
    argv.extend([
        far.path.clone(),
        "--session".into(),
        session.to_string(),
        "remote-api-bridge".into(),
        "--request".into(),
    ]);
    argv
}

/// Whether a far bridge's output is one whole reply line to `request`:
/// JSON with the request's id (M6).
fn whole_reply(request: &[u8], stdout: &[u8]) -> Result<(), &'static str> {
    let line = match stdout {
        [] => return Err("answered with no reply"),
        [line @ .., b'\n'] if !line.contains(&b'\n') => line,
        _ => return Err("answered with no whole reply line"),
    };
    let id = |bytes: &[u8]| {
        serde_json::from_slice::<Value>(bytes)
            .ok()
            .map(|value| value["id"].clone())
    };
    let Some(reply_id) = id(line) else {
        return Err("answered with a line that is no API reply");
    };
    match id(request) {
        Some(request_id) if request_id != reply_id => Err("answered another request"),
        _ => Ok(()),
    }
}

/// How long the bridge waits, in all, for a request line to come in and
/// for its reply to go out (M3: one deadline each, not one per read).
const BRIDGE_IO_WAIT: Duration = Duration::from_secs(30);

/// Reads from `stream` up to and with the first newline, or more than
/// `limit` bytes, or to its end, by `deadline` at the latest.
fn read_line_by(
    mut stream: &UnixStream,
    limit: usize,
    deadline: std::time::Instant,
) -> io::Result<Vec<u8>> {
    let mut line = Vec::new();
    let mut chunk = [0_u8; 8192];
    while line.len() <= limit {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "no whole line in time",
            ));
        }
        stream.set_read_timeout(Some(left))?;
        let read = match stream.read(&mut chunk) {
            Ok(read) => read,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "no whole line in time",
                ))
            }
            Err(err) => return Err(err),
        };
        if read == 0 {
            break;
        }
        match chunk[..read].iter().position(|&byte| byte == b'\n') {
            Some(end) => {
                line.extend_from_slice(&chunk[..=end]);
                break;
            }
            None => line.extend_from_slice(&chunk[..read]),
        }
    }
    Ok(line)
}

/// Writes all of `bytes` to `stream` by `deadline` at the latest.
fn write_all_by(
    mut stream: &UnixStream,
    bytes: &[u8],
    deadline: std::time::Instant,
) -> io::Result<()> {
    let mut rest = bytes;
    while !rest.is_empty() {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the reply was not taken in time",
            ));
        }
        stream.set_write_timeout(Some(left))?;
        match stream.write(rest) {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero)),
            Ok(written) => rest = &rest[written..],
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the reply was not taken in time",
                ))
            }
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

/// What the bridge thread needs for each request.
struct Bridge {
    core: CoreAt,
    name: MachineName,
    dir: MachineDir,
    argv: Vec<String>,
    far_path: String,
    failure: Arc<Mutex<Option<String>>>,
}

impl Bridge {
    fn fail(&self, message: String) {
        *lock(&self.failure) = Some(message);
    }

    /// One API request: one line in, the far side's reply out, or nothing
    /// and a recorded failure.
    /// Serves one connection at a time until `woken` is closed. Only a
    /// peer of this account is served (M3).
    fn accept_until_woken(&self, listener: &UnixListener, woken: &std::io::PipeReader) {
        use crate::platform::core_link as sys;
        use std::os::fd::AsRawFd as _;
        loop {
            match sys::wait_readable(&[listener.as_raw_fd(), woken.as_raw_fd()]) {
                Ok(ready) if ready[1] => return,
                Ok(ready) if !ready[0] => continue,
                Ok(_) => {}
                Err(err) => {
                    tracing::debug!(%err, "fleet bridge wait failed");
                    return;
                }
            }
            let connection = match listener.accept() {
                Ok((connection, _)) => connection,
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => continue,
                Err(err) => {
                    tracing::debug!(%err, "fleet bridge accept failed");
                    return;
                }
            };
            match sys::peer_uid(&connection) {
                Ok(uid) if uid == sys::uid() => {}
                other => {
                    tracing::warn!(peer = ?other, "fleet bridge refused a connection of another account");
                    continue;
                }
            }
            if let Err(err) = connection.set_nonblocking(false) {
                tracing::debug!(%err, "fleet bridge connection stays non-blocking");
                continue;
            }
            self.serve(connection);
        }
    }

    fn serve(&self, connection: UnixStream) {
        // Each request's failure is its own.
        *lock(&self.failure) = None;
        let line = match read_line_by(
            &connection,
            REQUEST_LIMIT + 1,
            std::time::Instant::now() + BRIDGE_IO_WAIT,
        ) {
            Ok(line) => line,
            Err(err) => {
                self.fail(format!("the request could not be read: {err}"));
                return;
            }
        };
        if line.is_empty() {
            return;
        }
        // One whole request line at the far API's own limit, or nothing
        // is sent (M6).
        if line.len() > REQUEST_LIMIT + 1 {
            self.fail(format!(
                "the request is over the Herdr API's 1 MiB line limit, so it was not sent to {}",
                self.name
            ));
            return;
        }
        if !line.ends_with(b"\n") {
            self.fail(format!(
                "the request is not one whole line, so it was not sent to {}",
                self.name
            ));
            return;
        }
        let timeout = match exec_timeout(&line) {
            Ok(timeout) => timeout,
            Err(why) => {
                self.fail(format!("the request for {}: {why}", self.name));
                return;
            }
        };
        match exec(&self.core, &self.name, &self.argv, Some(&line), timeout) {
            Ok(ran) if ran.code == Some(0) => {
                // A far side that ended well with no whole reply line (an
                // API that closed first) may still have run the request
                // (#310): a failure, never an empty success.
                if let Err(why) = whole_reply(&line, &ran.stdout) {
                    forget_far_herdr(&self.dir);
                    self.fail(format!(
                        "herdr on {} ({}) {why}; {MAY_HAVE_RUN}",
                        self.name, self.far_path
                    ));
                    return;
                }
                if let Err(err) = write_all_by(
                    &connection,
                    &ran.stdout,
                    std::time::Instant::now() + BRIDGE_IO_WAIT,
                ) {
                    tracing::debug!(%err, "the fleet CLI did not take its reply");
                }
            }
            Ok(ran) => {
                // Any far exit but 0 forgets the probe, so the next call
                // probes again: the path no longer runs there (moved,
                // uninstalled: 126, 127), what runs there has no
                // `remote-api-bridge` (2) or no `--request` (an older herdr
                // answers that with its usage, exit 1, #310), or it failed
                // for its own reason, which costs one probe.
                forget_far_herdr(&self.dir);
                let code = ran
                    .code
                    .map_or_else(|| "a signal".to_string(), |code| format!("exit {code}"));
                self.fail(format!(
                    "herdr on {} ({}) ended with {code}: {}; {MAY_HAVE_RUN}",
                    self.name,
                    self.far_path,
                    ran.stderr_tail()
                ));
            }
            Err(failure) => self.fail(failure.for_request()),
        }
    }
}

/// A local socket that carries the fleet CLI's requests to a core machine.
/// Dropping it removes the `cli-<pid>` lease and the socket's folder at
/// once, and waits at most half a second for a request still in flight.
pub(crate) struct FleetLink {
    name: MachineName,
    session: String,
    far: FarHerdr,
    folder: PathBuf,
    socket: PathBuf,
    failure: Arc<Mutex<Option<String>>>,
    /// Closed to end the bridge thread's wait: never blocks, unlike a
    /// connect to a listener whose queue is full (M3).
    wake: Option<std::io::PipeWriter>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// Disconnected when the thread ends.
    done: mpsc::Receiver<()>,
    lease: LeaseKeeper,
}

/// Removes a folder when dropped, unless kept: the bridge's folder when its
/// start fails after the folder was made.
struct FolderGuard(Option<PathBuf>);

impl FolderGuard {
    fn keep(mut self) -> PathBuf {
        self.0.take().unwrap_or_default()
    }
}

impl Drop for FolderGuard {
    fn drop(&mut self) {
        if let Some(folder) = self.0.take() {
            let _ = remove_bridge_folder(&folder);
            forget_leftover(&folder);
        }
    }
}

impl FleetLink {
    pub(crate) fn start(
        core: CoreAt,
        name: MachineName,
        dir: MachineDir,
        lease: LeaseKeeper,
        far: FarHerdr,
        session: String,
    ) -> io::Result<Self> {
        let folder = private_folder()?;
        let guard = FolderGuard(Some(folder.clone()));
        let socket = folder.join(SOCKET_NAME);
        let listener = UnixListener::bind(&socket)?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let (woken, wake) = std::io::pipe()?;
        let failure = Arc::new(Mutex::new(None));
        let bridge = Bridge {
            core,
            name: name.clone(),
            dir,
            argv: bridge_argv(&far, &session),
            far_path: far.path.clone(),
            failure: Arc::clone(&failure),
        };
        let (done_tx, done) = mpsc::channel::<()>();
        let thread = std::thread::Builder::new()
            .name(format!("herdr-fleet-{name}"))
            .spawn(move || {
                let _done = done_tx;
                bridge.accept_until_woken(&listener, &woken);
            })?;
        let folder = guard.keep();
        Ok(Self {
            name,
            session,
            far,
            folder,
            socket,
            failure,
            wake: Some(wake),
            thread: Some(thread),
            done,
            lease,
        })
    }

    pub(crate) fn socket_path(&self) -> &Path {
        &self.socket
    }

    pub(crate) fn machine(&self) -> &MachineName {
        &self.name
    }

    /// The far herdr session the requests go to.
    pub(crate) fn far_session(&self) -> &str {
        &self.session
    }

    /// Why the last request got no reply, when it did not.
    pub(crate) fn reported_failure(&self) -> Option<io::Error> {
        lock(&self.failure).clone().map(io::Error::other)
    }
}

impl Drop for FleetLink {
    fn drop(&mut self) {
        // The lease first: a CLI that is leaving holds no link up.
        self.lease.release();
        // Closing the pipe wakes the thread's wait; it ends.
        self.wake.take();
        let _ = remove_bridge_folder(&self.folder);
        forget_leftover(&self.folder);
        if let Some(thread) = self.thread.take() {
            match self.done.recv_timeout(BRIDGE_JOIN_WAIT) {
                // A request still in flight (the CLI gave up on it): left
                // to end with the process, which closes its core call.
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    tracing::debug!(machine = %self.name, "fleet bridge left with a request in flight");
                }
                _ => {
                    let _ = thread.join();
                }
            }
        }
        tracing::debug!(machine = %self.name, far = %self.far.path, "fleet bridge closed");
    }
}

/// A fresh 0700 folder for the bridge socket, short enough for a Unix
/// socket path, under a name no one can guess; the temporary folder, or
/// `/tmp` when that one's path is too long. Folders a killed fleet CLI
/// left there are removed first.
fn private_folder() -> io::Result<PathBuf> {
    let temp = std::env::temp_dir();
    let longest = format!("{FOLDER_PREFIX}4294967295-ffffffffffffffff");
    let fits = |base: &Path| base.join(&longest).join(SOCKET_NAME).as_os_str().len() < 100;
    // A temporary folder another account could replace is passed over
    // for `/tmp` (security review 2026-11-23 E1).
    let base = [temp, PathBuf::from("/tmp")]
        .into_iter()
        .find(|base| fits(base) && is_trusted_base(base))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "neither the temporary folder nor /tmp is safe from other accounts \
                 for the fleet bridge's socket",
            )
        })?;
    remove_dead_folders(&base);
    private_folder_in(&base, unguessable_leaf)
}

/// Whether no other account can replace `base` or anything under it that
/// this account makes: the folder and each folder above it (symlinks
/// resolved) belong to this account or root, and one that others can
/// write is sticky, so they cannot rename or remove what is not theirs.
fn is_trusted_base(base: &Path) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    let Ok(real) = base.canonicalize() else {
        return false;
    };
    let uid = crate::platform::core_link::uid();
    real.ancestors().all(|folder| {
        std::fs::metadata(folder).is_ok_and(|meta| {
            meta.is_dir()
                && (meta.uid() == uid || meta.uid() == 0)
                && (meta.mode() & 0o022 == 0 || meta.mode() & 0o1000 != 0)
        })
    })
}

const FOLDER_PREFIX: &str = "herdr-fleet-";
const SOCKET_NAME: &str = "api.sock";

fn unguessable_leaf() -> String {
    use std::hash::{BuildHasher as _, Hasher as _};
    use std::sync::atomic::AtomicU64;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    // `RandomState` is seeded from the OS per process.
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(NEXT.fetch_add(1, Ordering::Relaxed));
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |at| at.as_nanos()),
    );
    format!(
        "{FOLDER_PREFIX}{}-{:016x}",
        std::process::id(),
        hasher.finish()
    )
}

/// The pid in a name `unguessable_leaf` makes, `herdr-fleet-<pid>-<16
/// hex>`, and `None` for any other name.
fn bridge_leaf_pid(name: &str) -> Option<u32> {
    let (pid, random) = name.strip_prefix(FOLDER_PREFIX)?.split_once('-')?;
    let plain_pid = !pid.is_empty()
        && pid.len() <= 10
        && !pid.starts_with('0')
        && pid.bytes().all(|byte| byte.is_ascii_digit());
    let random_part = random.len() == 16
        && random
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
    (plain_pid && random_part)
        .then(|| pid.parse().ok())
        .flatten()
}

/// Makes `base/<leaf>` with mode 0700, trying fresh leaves while the name
/// is taken (as `platform::fallback` does for its ssh folder), and
/// remembers it for the termination cleanup in the same step (M3).
fn private_folder_in(base: &Path, mut leaf: impl FnMut() -> String) -> io::Result<PathBuf> {
    for _ in 0..100 {
        let folder = base.join(leaf());
        match make_leftover(&folder, || {
            std::fs::DirBuilder::new().mode(0o700).create(&folder)
        }) {
            Ok(()) => return Ok(folder),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not make a private folder for the fleet bridge",
    ))
}

/// Removes a bridge folder: its socket, then the folder if that left it
/// empty. Anything else in it, or a socket that is no socket, stays.
fn remove_bridge_folder(folder: &Path) -> io::Result<()> {
    use std::os::unix::fs::FileTypeExt as _;
    let socket = folder.join(SOCKET_NAME);
    match std::fs::symlink_metadata(&socket) {
        Ok(meta) if meta.file_type().is_socket() => std::fs::remove_file(&socket)?,
        Ok(_) => {
            return Err(io::Error::other(format!(
                "{} is no socket, so its folder stays",
                socket.display()
            )))
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    std::fs::remove_dir(folder)
}

/// Removes this account's bridge folders whose fleet CLI is gone (killed
/// outright, so no signal cleanup ran): only names the bridge makes, only
/// 0700 folders of this account, and only their socket and then the empty
/// folder (security review 2026-11-23 M3).
fn remove_dead_folders(base: &Path) {
    use std::os::unix::fs::MetadataExt as _;
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    let uid = crate::platform::core_link::uid();
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(bridge_leaf_pid) else {
            continue;
        };
        if pid == std::process::id() || crate::platform::core_link::process_alive(pid) {
            continue;
        }
        let path = entry.path();
        let ours = std::fs::symlink_metadata(&path)
            .is_ok_and(|meta| meta.is_dir() && meta.uid() == uid && meta.mode() & 0o777 == 0o700);
        if ours {
            if let Err(err) = remove_bridge_folder(&path) {
                tracing::debug!(%err, path = %path.display(), "a dead fleet CLI's folder stays");
            }
        }
    }
}

#[cfg(test)]
mod tests;
