//! `herdr --machine <name>` to a Herdr Core machine (core-phase5 D7, E.1,
//! E.2): the fleet CLI's requests go to the far side's own herdr through
//! `machine.exec` of `<far herdr> --session S remote-api-bridge`, with the
//! request line as its input. Those are the bytes `remote-api-bridge` reads
//! from ssh on the old path, so the reply is the far side's own, with its
//! pane ids.
//!
//! - `machine.exec` never dials (F7). On a machine that is not connected the
//!   call is refused with exit 75 and nothing is written or dialled, unless
//!   the caller passed `--dial` (or `HERDR_MACHINE_DIAL=1`): then the
//!   `cli-<pid>` lease comes first and one connect goes through the dialer's
//!   rules (D3a). A connected machine gets the lease and the exec only.
//! - Every argv starts with D5's `env -u` wrapper (F6): the far core's own
//!   environment may carry a herdr pane's `HERDR_SOCKET_PATH`.
//! - A request is sent once (no retry after send); a reply cut at 8 MiB or
//!   timed out is an error with none of its output printed (F12); a request
//!   over 8 MiB is refused before it is sent.
//! - The far herdr's path and capabilities are read once per link
//!   establishment (the link's dial record) and kept in `far_herdr.json`.
//!
//! The CLI reaches this through a local socket that speaks herdr's JSON API,
//! one request per connection as the API does, so every CLI verb that works
//! with `--machine` today works here unchanged.

use std::io::{self, BufRead as _, BufReader, Read as _, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::dialer::{self, CoreLink, Outcome};
use super::name::MachineName;
use super::state::{
    newest_act, now_unix, passwd_machines_root, write_atomic, Gate, GateClass, LeaseHolder,
    MachineDir,
};
use crate::pane::core_machines::{self as core, CallError};

/// The exit code of a refusal on a machine that is not connected (D7, F7):
/// `EX_TEMPFAIL`, as core-link's own refusals.
pub(crate) const EXIT_DORMANT: i32 = 75;
/// `HERDR_MACHINE_DIAL=1` is `--dial` (D7).
pub(crate) const DIAL_ENV: &str = "HERDR_MACHINE_DIAL";
/// The most `machine.exec` takes as input, raw (core's own limit is 8 MB).
pub(crate) const INPUT_LIMIT: usize = 8 * 1024 * 1024;
/// `machine.exec`'s timeout: core's default, longer by the request's own
/// `timeout_ms`, at most core's 600 s.
const EXEC_DEFAULT_SECS: u64 = 60;
const EXEC_MAX_SECS: u64 = 600;
/// How long past a call the `cli-<pid>` lease lasts (D3): 120 s, longer by
/// the call's own timeout, so a crashed CLI's lease ends by itself.
const LEASE_SECS: f64 = 120.0;
/// The probe of the far herdr: a few local programs there.
const PROBE_SECS: u64 = 30;
const PROBE_MARK: &str = "herdr-probe-v1";

/// Finds the far herdr that answers `remote-api-bridge --check`, by the
/// names given as arguments, on the far `PATH` and in the usual install
/// folders, and prints it with its `status client --json`. A fixed script:
/// names come as arguments, never as script text.
const PROBE_SCRIPT: &str = r#"set -f
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
        printf 'herdr-probe-v1\n%s\n' "$candidate"
        "$candidate" status client --json </dev/null 2>/dev/null
        exit 0
    done
done
echo 'no herdr there answers remote-api-bridge' >&2
exit 2"#;

/// Whether this call may dial: `--dial`, or `HERDR_MACHINE_DIAL=1`.
pub(crate) fn dial_requested(flag: bool) -> bool {
    flag || std::env::var(DIAL_ENV).is_ok_and(|value| value == "1")
}

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
    let root = match passwd_machines_root() {
        Ok(root) => root,
        Err(err) => return refused(1, format!("cannot find herdr's machine state: {err}")),
    };
    let core = CoreAt::primary();
    let dir = MachineDir::at(&root, &name);
    if !dir.target_path().exists() {
        return match core.state(&name) {
            Ok(Some(_)) => refused(
                2,
                format!(
                    "{name} is a Herdr Core machine herdr did not add, so herdr keeps no lease or gate for it; \
                     --machine reaches the machines added with `herdr machine add --core`"
                ),
            ),
            _ => Lookup::NotCore,
        };
    }
    prepare(&core, &name, &dir, dial)
}

/// The far herdr session whose primary core is `core_session` on the far
/// side (the inverse of herdr's own session-to-core mapping): `default`
/// for core's default session (or a debug build's default), `X` for
/// `herdr-X` and `herdrdev-X`. `None` for a core session no herdr session
/// uses as its primary.
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
        // S4: the call rides the link; it is no act that reopens a gate.
        LeaseKeeper::new(dir.clone(), holder, 0.0)
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
        let lease = LeaseKeeper::new(dir.clone(), holder, now);
        if let Err(err) = lease.refresh(dialer::CONNECT_TIMEOUT.as_secs_f64()) {
            return refused(1, format!("{name}: could not write the lease: {err}"));
        }
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
        lease
    };
    if let Err(err) = lease.refresh(PROBE_SECS as f64) {
        return refused(1, format!("{name}: could not write the lease: {err}"));
    }
    let far = match far_herdr(core, name, dir) {
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
/// login reopens, or the far core was found down within 5 min (D10).
/// `retry_spent` is reopened by the `--dial` lease itself (D2 step 9).
fn dial_refusal(name: &MachineName, gate: io::Result<Option<Gate>>, now: f64) -> Option<String> {
    let gate = match gate {
        Ok(gate) => gate?,
        Err(err) => {
            return Some(format!(
                "{name}: unreadable gate.json ({err}); a person runs: herdr machine login {name}"
            ))
        }
    };
    match gate.class {
        GateClass::NeedsCheck | GateClass::AuthStalled | GateClass::AuthFailed => Some(format!(
            "{name}: the gate is closed ({}); --dial cannot reopen it. A person runs: herdr machine login {name}",
            gate.class.as_str()
        )),
        GateClass::FarCoreDown if gate.closes_at(now, None) => Some(format!(
            "{name}: Herdr Core was not running there {:.0} s ago (far_core_down); start Herdr there, then run herdr machine wake {name}",
            now - gate.at
        )),
        _ => None,
    }
}

/// The `cli-<pid>` lease (D3): written before the first call, again only
/// when what is left of it would not cover the next call and a minute
/// more, so a run of calls writes it at most about once a minute; removed
/// when the CLI is done.
pub(crate) struct LeaseKeeper {
    dir: MachineDir,
    holder: LeaseHolder,
    /// The act behind it: `--dial`'s time, else 0 (no act).
    act: f64,
    expires: Mutex<Option<f64>>,
}

impl LeaseKeeper {
    fn new(dir: MachineDir, holder: LeaseHolder, act: f64) -> Self {
        Self {
            dir,
            holder,
            act,
            expires: Mutex::new(None),
        }
    }

    /// Makes the lease cover a call of `call_secs` from now.
    fn refresh(&self, call_secs: f64) -> io::Result<()> {
        let now = now_unix();
        let mut expires = lock(&self.expires);
        if expires.is_some_and(|at| at - now >= call_secs + LEASE_SECS / 2.0) {
            return Ok(());
        }
        let at = now + LEASE_SECS + call_secs;
        self.dir.write_lease(&self.holder, at, self.act)?;
        *expires = Some(at);
        Ok(())
    }
}

impl Drop for LeaseKeeper {
    fn drop(&mut self) {
        if lock(&self.expires).is_some() {
            if let Err(err) = self.dir.remove_lease(&self.holder) {
                tracing::warn!(%err, "could not remove the fleet CLI's lease");
            }
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
    /// Its absolute path there.
    pub path: String,
    #[serde(default)]
    pub version: Option<String>,
    /// Its private protocol, from `status client --json`.
    #[serde(default)]
    pub protocol: Option<u32>,
    #[serde(default)]
    pub endpoint_capabilities: Vec<String>,
}

/// The far herdr found over the machine's current link, if one was.
pub(crate) fn cached_far_herdr(dir: &MachineDir) -> Option<FarHerdr> {
    let link = dir.read_dial_record()?;
    let cached: FarHerdr =
        serde_json::from_slice(&std::fs::read(dir.far_herdr_path()).ok()?).ok()?;
    (cached.link_at == link.at && cached.link_pid == link.pid && is_far_path(&cached.path))
        .then_some(cached)
}

/// The far herdr, read once per link establishment (D7).
fn far_herdr(core: &CoreAt, name: &MachineName, dir: &MachineDir) -> Result<FarHerdr, String> {
    if let Some(cached) = cached_far_herdr(dir) {
        return Ok(cached);
    }
    let link = dir.read_dial_record();
    let mut argv = core::machine_env_wrapper();
    argv.extend(
        ["/bin/sh", "-c", PROBE_SCRIPT, "herdr-probe"]
            .into_iter()
            .map(String::from),
    );
    argv.extend(binary_names());
    let done = exec(core, name, &argv, None, PROBE_SECS)?;
    if done.code != Some(0) {
        return Err(format!(
            "no herdr on {name} answers remote-api-bridge (looked for {} on its PATH and in ~/.local/bin, \
             /opt/homebrew/bin, /usr/local/bin): {}; install Herdr there",
            binary_names().join(", "),
            done.stderr_tail()
        ));
    }
    let (path, status) = parse_probe(&done.stdout)
        .ok_or_else(|| format!("the herdr probe on {name} answered something else"))?;
    let far = FarHerdr {
        link_at: link.as_ref().map_or(0.0, |link| link.at),
        link_pid: link.as_ref().map_or(0, |link| link.pid),
        path,
        version: status.version,
        protocol: status.protocol,
        endpoint_capabilities: status.endpoint_capabilities,
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
    /// The last line it wrote to stderr, for a message.
    fn stderr_tail(&self) -> String {
        let text = String::from_utf8_lossy(&self.stderr);
        text.lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("no message")
            .chars()
            .take(400)
            .collect()
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

/// One `machine.exec`, sent once. Refusals, a lost answer, a cut reply and
/// a timed-out run are errors with their message; a reply's output is
/// returned only when it is whole (F12).
fn exec(
    core: &CoreAt,
    name: &MachineName,
    argv: &[String],
    input: Option<&[u8]>,
    timeout_secs: u64,
) -> Result<Ran, String> {
    if input.is_some_and(|input| input.len() > INPUT_LIMIT) {
        return Err(format!(
            "the request is over 8 MiB, so it was not sent to {name}"
        ));
    }
    let mut params = json!({"on": name.as_str(), "argv": argv, "timeout": timeout_secs});
    if let Some(input) = input {
        params["input"] = Value::String(base64::engine::general_purpose::STANDARD.encode(input));
    }
    let reply = match core.call(
        "machine.exec",
        params,
        Duration::from_secs(timeout_secs + 10),
    ) {
        Ok(Some(reply)) => reply,
        Ok(None) => return Err("herdr-core is not running here".into()),
        Err(CallError::Rpc { kind, .. }) if kind == "unavailable" => {
            return Err(format!(
                "{name} is not connected, so the request was not sent there"
            ))
        }
        Err(CallError::Rpc { kind, message }) => {
            return Err(format!(
                "herdr-core refused the request for {name}: {kind}: {message}"
            ))
        }
        Err(CallError::NoAnswer(why)) => {
            return Err(format!(
                "no answer from {name} after the request was sent ({why}); it may have run there, and it is not sent again"
            ))
        }
        Err(CallError::NotSent(why)) => {
            return Err(format!("herdr-core here is not reachable: {why}"))
        }
    };
    let result: ExecResult = serde_json::from_value(reply)
        .map_err(|err| format!("herdr-core's answer from {name} does not parse: {err}"))?;
    if result.timed_out {
        return Err(format!(
            "the reply from {name} timed out after {timeout_secs} s"
        ));
    }
    if result.truncated {
        return Err(format!("the reply from {name} was cut at 8 MiB"));
    }
    let decode = |field: &str, raw: &str| {
        base64::engine::general_purpose::STANDARD
            .decode(raw)
            .map_err(|err| format!("the {field} of the reply from {name} is not base64: {err}"))
    };
    Ok(Ran {
        code: result.code,
        stdout: decode("stdout", &result.stdout)?,
        stderr: decode("stderr", &result.stderr)?,
    })
}

/// `machine.exec`'s timeout for one API request: the request's own
/// `timeout_ms` and 10 s more, at least core's default, at most 600 s.
fn exec_timeout(request: &[u8]) -> u64 {
    let own = serde_json::from_slice::<Value>(request)
        .ok()
        .and_then(|request| request["params"]["timeout_ms"].as_u64())
        .map(|ms| ms.div_ceil(1000).saturating_add(10));
    own.unwrap_or(EXEC_DEFAULT_SECS)
        .clamp(EXEC_DEFAULT_SECS, EXEC_MAX_SECS)
}

/// The argv that carries one request (D7, F6).
fn bridge_argv(far: &FarHerdr, session: &str) -> Vec<String> {
    let mut argv = core::machine_env_wrapper();
    argv.extend([
        far.path.clone(),
        "--session".into(),
        session.to_string(),
        "remote-api-bridge".into(),
    ]);
    argv
}

/// What the bridge thread needs for each request.
struct Bridge {
    core: CoreAt,
    name: MachineName,
    dir: MachineDir,
    lease: Arc<LeaseKeeper>,
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
    fn serve(&self, connection: UnixStream) {
        if let Err(err) = connection.set_read_timeout(Some(Duration::from_secs(30))) {
            tracing::debug!(%err, "fleet bridge read timeout unavailable");
        }
        let Ok(reader) = connection.try_clone() else {
            return;
        };
        let mut line = Vec::new();
        let read = BufReader::new(reader)
            .take(INPUT_LIMIT as u64 + 1)
            .read_until(b'\n', &mut line);
        if let Err(err) = read {
            self.fail(format!("the request could not be read: {err}"));
            return;
        }
        if line.len() > INPUT_LIMIT {
            self.fail(format!(
                "the request is over 8 MiB, so it was not sent to {}",
                self.name
            ));
            return;
        }
        if line.is_empty() {
            return;
        }
        let timeout = exec_timeout(&line);
        if let Err(err) = self.lease.refresh(timeout as f64) {
            self.fail(format!("{}: could not write the lease: {err}", self.name));
            return;
        }
        match exec(&self.core, &self.name, &self.argv, Some(&line), timeout) {
            Ok(ran) if ran.code == Some(0) => {
                let mut connection = connection;
                if let Err(err) = connection.write_all(&ran.stdout) {
                    tracing::debug!(%err, "the fleet CLI left before its reply");
                }
            }
            Ok(ran) => {
                // The path no longer runs there (moved, uninstalled): the
                // next call probes again.
                if matches!(ran.code, Some(126 | 127)) {
                    forget_far_herdr(&self.dir);
                }
                let code = ran
                    .code
                    .map_or_else(|| "a signal".to_string(), |code| format!("exit {code}"));
                self.fail(format!(
                    "herdr on {} ({}) ended with {code}: {}",
                    self.name,
                    self.far_path,
                    ran.stderr_tail()
                ));
            }
            Err(message) => self.fail(message),
        }
    }
}

/// A local socket that carries the fleet CLI's requests to a core machine.
/// Dropping it removes the socket and the `cli-<pid>` lease.
pub(crate) struct FleetLink {
    name: MachineName,
    session: String,
    far: FarHerdr,
    folder: PathBuf,
    socket: PathBuf,
    failure: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// Held to its drop, which removes the `cli-<pid>` lease once the
    /// thread's own handle is gone.
    _lease: Arc<LeaseKeeper>,
}

impl FleetLink {
    fn start(
        core: CoreAt,
        name: MachineName,
        dir: MachineDir,
        lease: LeaseKeeper,
        far: FarHerdr,
        session: String,
    ) -> io::Result<Self> {
        let folder = private_folder()?;
        let socket = folder.join("api.sock");
        let listener = UnixListener::bind(&socket).inspect_err(|_| {
            let _ = std::fs::remove_dir_all(&folder);
        })?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        let lease = Arc::new(lease);
        let failure = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let bridge = Bridge {
            core,
            name: name.clone(),
            dir,
            lease: Arc::clone(&lease),
            argv: bridge_argv(&far, &session),
            far_path: far.path.clone(),
            failure: Arc::clone(&failure),
        };
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name(format!("herdr-fleet-{name}"))
            .spawn(move || {
                for connection in listener.incoming() {
                    if thread_stop.load(Ordering::Acquire) {
                        break;
                    }
                    match connection {
                        Ok(connection) => bridge.serve(connection),
                        Err(err) => {
                            tracing::debug!(%err, "fleet bridge accept failed");
                            break;
                        }
                    }
                }
            })?;
        Ok(Self {
            name,
            session,
            far,
            folder,
            socket,
            failure,
            stop,
            thread: Some(thread),
            _lease: lease,
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
        self.stop.store(true, Ordering::Release);
        // Wakes the accept; the thread sees the stop and ends.
        let _ = UnixStream::connect(&self.socket);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_dir_all(&self.folder);
        tracing::debug!(machine = %self.name, far = %self.far.path, "fleet bridge closed");
    }
}

/// A fresh 0700 folder for the bridge socket, short enough for a Unix
/// socket path.
fn private_folder() -> io::Result<PathBuf> {
    use std::sync::atomic::AtomicU32;
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let leaf = format!(
        "herdr-fleet-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let temp = std::env::temp_dir();
    let base = if temp.join(&leaf).join("api.sock").as_os_str().len() < 100 {
        temp
    } else {
        PathBuf::from("/tmp")
    };
    let folder = base.join(leaf);
    std::fs::DirBuilder::new().mode(0o700).create(&folder)?;
    Ok(folder)
}

#[cfg(test)]
mod tests;
