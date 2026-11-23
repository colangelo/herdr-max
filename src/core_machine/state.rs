//! A core machine's link state on this Mac (core-phase5 D2): one private
//! folder per machine under the account's passwd home, holding
//! `target.json`, `gate.json`, `leases/`, `dial.lock`, `connect.lock`,
//! `dial.json`, `far_herdr.json`, the link's `ssh_config` and `link.log`.
//! Every file is parsed here, at the boundary; a file that does not parse is
//! refused, never guessed at.

use std::fs;
use std::io::{self, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::name::{is_plain_name, MachineName};

/// Seconds since the Unix epoch, as every state file records time.
pub(crate) fn now_unix() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

/// `<home>/.local/state/herdr/core-machines` (`herdr-dev` for a debug
/// build, as herdr's other folders), where `home` is the passwd home.
pub(crate) fn machines_root(home: &Path) -> PathBuf {
    home.join(".local/state")
        .join(crate::config::app_dir_name())
        .join("core-machines")
}

/// The root for the account this process runs as, from the passwd
/// database, never from `HOME` or `XDG_STATE_HOME` (F6).
pub(crate) fn passwd_machines_root() -> io::Result<PathBuf> {
    Ok(machines_root(
        &crate::platform::core_link::passwd_account()?.home,
    ))
}

/// One machine's folder.
#[derive(Debug, Clone)]
pub(crate) struct MachineDir {
    path: PathBuf,
}

const LOG_LIMIT: u64 = 1 << 20;

impl MachineDir {
    pub(crate) fn at(root: &Path, name: &MachineName) -> Self {
        Self {
            path: root.join(name.as_str()),
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Creates the folder and `leases/`, private (0700), with the root.
    pub(crate) fn create(&self) -> io::Result<()> {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder.create(self.path.join("leases"))?;
        self.check()
    }

    pub(crate) fn exists(&self) -> bool {
        self.path.is_dir()
    }

    /// Refuses a folder this account does not own or that others can
    /// write: its files decide whether ssh runs.
    pub(crate) fn check(&self) -> io::Result<()> {
        let meta = fs::symlink_metadata(&self.path)?;
        let uid = unsafe { libc::getuid() };
        if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o022 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "{} is not a private folder of this account",
                    self.path.display()
                ),
            ));
        }
        Ok(())
    }

    pub(crate) fn target_path(&self) -> PathBuf {
        self.path.join("target.json")
    }

    pub(crate) fn gate_path(&self) -> PathBuf {
        self.path.join("gate.json")
    }

    pub(crate) fn leases_dir(&self) -> PathBuf {
        self.path.join("leases")
    }

    pub(crate) fn dial_lock_path(&self) -> PathBuf {
        self.path.join("dial.lock")
    }

    /// Held by a herdr dialer around its `machine.connect` (D3a), so herdr
    /// makes one connect per wake across its processes.
    pub(crate) fn connect_lock_path(&self) -> PathBuf {
        self.path.join("connect.lock")
    }

    pub(crate) fn dial_record_path(&self) -> PathBuf {
        self.path.join("dial.json")
    }

    pub(crate) fn ssh_config_path(&self) -> PathBuf {
        self.path.join("ssh_config")
    }

    pub(crate) fn log_path(&self) -> PathBuf {
        self.path.join("link.log")
    }

    /// The far herdr the fleet CLI found over the current link (D7).
    pub(crate) fn far_herdr_path(&self) -> PathBuf {
        self.path.join("far_herdr.json")
    }

    /// Appends one line to `link.log`, keeping one older log of 1 MiB.
    pub(crate) fn log(&self, line: &str) {
        let path = self.log_path();
        if fs::metadata(&path).is_ok_and(|meta| meta.len() > LOG_LIMIT) {
            let _ = fs::rename(&path, self.path.join("link.log.1"));
        }
        let opened = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&path);
        if let Ok(mut file) = opened {
            let _ = writeln!(file, "{:.3} pid={} {line}", now_unix(), std::process::id());
        }
    }

    pub(crate) fn read_target(&self) -> io::Result<Target> {
        Target::parse(&fs::read(self.target_path())?)
    }

    pub(crate) fn write_target(&self, target: &Target) -> io::Result<()> {
        write_atomic(&self.target_path(), &to_json(target)?)
    }

    /// The file naming the herdr core session that added this machine:
    /// machine folders are keyed by name under the passwd home and shared
    /// by every core session of the account, so a client leases only the
    /// folders its own primary added (PR D's second read, item 5). A file
    /// of its own, so `target.json` keeps its fields for older builds.
    pub(crate) fn primary_path(&self) -> PathBuf {
        self.path.join("primary.json")
    }

    /// The core session that added this machine; `None` for a folder an
    /// older build added, or one whose file is unreadable.
    pub(crate) fn read_primary(&self) -> Option<String> {
        let bytes = fs::read(self.primary_path()).ok()?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        value["core"].as_str().map(str::to_string)
    }

    pub(crate) fn write_primary(&self, core: &str) -> io::Result<()> {
        write_atomic(
            &self.primary_path(),
            serde_json::json!({"core": core}).to_string().as_bytes(),
        )
    }

    /// The gate as recorded, `None` when no failure is recorded.
    pub(crate) fn read_gate(&self) -> io::Result<Option<Gate>> {
        match fs::read(self.gate_path()) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|err| invalid(format!("gate.json: {err}"))),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err),
        }
    }

    pub(crate) fn write_gate(&self, gate: &Gate) -> io::Result<()> {
        write_atomic(&self.gate_path(), &to_json(gate)?)
    }

    pub(crate) fn clear_gate(&self) -> io::Result<()> {
        match fs::remove_file(self.gate_path()) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
            _ => Ok(()),
        }
    }

    pub(crate) fn write_dial_record(&self, record: &DialRecord) -> io::Result<()> {
        write_atomic(&self.dial_record_path(), &to_json(record)?)
    }

    pub(crate) fn read_dial_record(&self) -> Option<DialRecord> {
        serde_json::from_slice(&fs::read(self.dial_record_path()).ok()?).ok()
    }

    /// Writes `leases/<holder>` with its expiry and the time of the human
    /// act behind it (write, then rename): each holder writes only its own
    /// file (D3).
    /// A `cli-<pid>` lease names its kind, `dial` (security review
    /// 2026-11-23 M4); the other holders keep the two-field form an older
    /// core-link reads.
    pub(crate) fn write_lease(
        &self,
        holder: &LeaseHolder,
        expires_at: f64,
        input_at: f64,
    ) -> io::Result<()> {
        let kind = if holder.is_cli() { " dial" } else { "" };
        write_atomic(
            &self.leases_dir().join(holder.as_str()),
            format!("{expires_at:.3} {input_at:.3}{kind}\n").as_bytes(),
        )
    }

    /// Writes `leases/<holder>` as a lease that holds a link while it is up
    /// and admits no dial (`<expires_at> 0 keep`): a fleet CLI call on a
    /// connected machine (D7), so a drop during the call is never redialled
    /// on its account.
    pub(crate) fn write_keep_lease(&self, holder: &LeaseHolder, expires_at: f64) -> io::Result<()> {
        write_atomic(
            &self.leases_dir().join(holder.as_str()),
            format!("{expires_at:.3} 0 keep\n").as_bytes(),
        )
    }

    /// Turns every fleet CLI lease in force that admits a dial into a keep
    /// lease with the same expiry: core-link does this under `dial.lock`
    /// for the dial it admitted, so one `--dial` grant gives one ssh start,
    /// however core retries (security review 2026-11-23 M4).
    pub(crate) fn consume_cli_grants(&self, now: f64) -> io::Result<usize> {
        let mut consumed = 0;
        for lease in self.leases() {
            if lease.holder.is_cli() && lease.dials && lease.in_force(now) {
                self.write_keep_lease(&lease.holder, lease.expires_at)?;
                consumed += 1;
            }
        }
        Ok(consumed)
    }

    /// Removes a holder's own lease (a CLI call's, after its reply).
    pub(crate) fn remove_lease(&self, holder: &LeaseHolder) -> io::Result<()> {
        match fs::remove_file(self.leases_dir().join(holder.as_str())) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
            _ => Ok(()),
        }
    }

    /// The holders of lease files whose pid now runs a process that started
    /// after the file was last written (review M7): `leases` lists them not,
    /// as they are no one's, so the client's prune finds them here. Read
    /// under the same cap as `leases`.
    pub(crate) fn reused_pid_holders(&self) -> Vec<LeaseHolder> {
        let Ok(entries) = fs::read_dir(self.leases_dir()) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| {
                let holder = LeaseHolder::parse(entry.file_name().to_str()?).ok()?;
                Some((entry, holder))
            })
            .take(LEASE_FILES_READ)
            .filter(|(entry, holder)| {
                let modified = entry
                    .metadata()
                    .and_then(|meta| meta.modified())
                    .ok()
                    .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
                    .map(|at| at.as_secs_f64());
                holder.reused_since(modified)
            })
            .map(|(_, holder)| holder)
            .collect()
    }

    /// Every lease file that parses, in force or not; `.new` files are
    /// writes in progress, and a file that does not parse, or is not one
    /// of its holder's (`Lease::parse`), is no lease. At most
    /// `LEASE_FILES_READ` holder-named files of at most `LEASE_FILE_BYTES` are read (M4).
    pub(crate) fn leases(&self) -> Vec<Lease> {
        use std::io::Read as _;
        let Ok(entries) = fs::read_dir(self.leases_dir()) else {
            return Vec::new();
        };
        let now = now_unix();
        let mut leases: Vec<Lease> = entries
            .flatten()
            .filter_map(|entry| {
                let holder = LeaseHolder::parse(entry.file_name().to_str()?).ok()?;
                Some((entry, holder))
            })
            // The cap counts holder-named files only: temporaries and
            // strays must not push a live lease out of the read.
            .take(LEASE_FILES_READ)
            .filter_map(|(entry, holder)| {
                let mut text = String::new();
                fs::File::open(entry.path())
                    .ok()?
                    .take(LEASE_FILE_BYTES + 1)
                    .read_to_string(&mut text)
                    .ok()?;
                if text.len() as u64 > LEASE_FILE_BYTES {
                    return None;
                }
                let modified = || {
                    entry
                        .metadata()
                        .ok()?
                        .modified()
                        .ok()?
                        .duration_since(UNIX_EPOCH)
                        .ok()
                        .map(|at| at.as_secs_f64())
                };
                // A pid the system gave again to a process that started after
                // the lease was last written is not its holder (review M7):
                // that lease is no one's.
                if holder.reused_since(modified()) {
                    return None;
                }
                Lease::parse(holder, &text, modified, now)
            })
            .collect();
        leases.sort_by(|a, b| a.holder.cmp(&b.holder));
        leases
    }
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn to_json(value: &impl Serialize) -> io::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Counts this process's `write_atomic` calls, for their temporary names.
static WRITE_SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// `<path>.<pid>-<n>.new`: a temporary name of this process and write alone.
fn temporary_path(path: &Path, n: u32) -> PathBuf {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".{}-{n}.new", std::process::id()));
    PathBuf::from(temporary)
}

/// Writes `<path>.<pid>-<n>.new` (0600), then renames it over `path`, so a
/// reader sees the old file or the new one, never half of one. The name is
/// this process's and this write's alone, so two processes writing one file
/// (two fleet CLIs keeping `far_herdr.json`) never write into each other's.
/// One that is there already was left by a writer killed before its rename
/// that had this pid: no live writer can hold it, so it is removed and the
/// write made again (#310).
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = temporary_path(
        path,
        WRITE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    );
    let open = || {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
    };
    let written = (|| {
        let mut file = match open() {
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                fs::remove_file(&temporary)?;
                open()?
            }
            opened => opened?,
        };
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

/// How ssh reaches the machine, written by `herdr machine add --core` and
/// read by every dial (D2, D11). The ssh options themselves are fixed in
/// code (D10), so no file can turn the master or the prompts back on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Target {
    /// The ssh destination: a host, `user@host`, or a `Host` of the ssh
    /// config.
    pub target: String,
    /// The core session on the machine (`default` unless chosen).
    pub session: String,
    /// The ssh program, by absolute path, found when the machine was added.
    pub ssh: PathBuf,
    /// An ssh config used instead of the managed one (`-F`), when chosen.
    #[serde(default)]
    pub ssh_config: Option<PathBuf>,
    /// The adding terminal's `SSH_AUTH_SOCK`: a core someone else started
    /// passes theirs, and herdr's own core start drops it.
    #[serde(default)]
    pub ssh_auth_sock: Option<PathBuf>,
    /// The `--via` command line `machine add --core` gave core: this
    /// herdr's own path and `core-link <name>`. Core's record of the machine
    /// must carry exactly it (#310, M5). Absent in a target written before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
}

/// An ssh destination herdr passes as one argument: no option (`-...`),
/// nothing blank, no control or shell characters.
pub(crate) fn is_ssh_destination(raw: &str) -> bool {
    !raw.is_empty()
        && raw.len() <= 255
        && !raw.starts_with('-')
        && raw.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'.' | b'_' | b'-' | b'@' | b':' | b'[' | b']' | b'%' | b'/'
                )
        })
}

impl Target {
    pub(crate) fn parse(bytes: &[u8]) -> io::Result<Self> {
        let target: Self =
            serde_json::from_slice(bytes).map_err(|err| invalid(format!("target.json: {err}")))?;
        target.validate()?;
        Ok(target)
    }

    pub(crate) fn validate(&self) -> io::Result<()> {
        if !is_ssh_destination(&self.target) {
            return Err(invalid(format!(
                "target.json: {:?} is not an ssh destination herdr passes",
                self.target
            )));
        }
        if !is_plain_name(&self.session) {
            return Err(invalid(format!(
                "target.json: session {:?} is not a plain name",
                self.session
            )));
        }
        let absolute = |path: &Option<PathBuf>| path.as_ref().is_none_or(|path| path.is_absolute());
        if !self.ssh.is_absolute() || !absolute(&self.ssh_config) || !absolute(&self.ssh_auth_sock)
        {
            return Err(invalid("target.json: paths must be absolute".into()));
        }
        if self.via.as_ref().is_some_and(|via| {
            via.is_empty() || via.len() > 4096 || via.chars().any(char::is_control)
        }) {
            return Err(invalid("target.json: via is not a command line".into()));
        }
        Ok(())
    }
}

/// Why the gate is closed (D10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GateClass {
    /// A Tailscale SSH check asked for a browser sign-in.
    NeedsCheck,
    /// No answer from the far side 15 s after the start: an unanswered
    /// prompt on this Mac.
    AuthStalled,
    /// Authentication or the host key failed.
    AuthFailed,
    /// The far core does not run; reopens by itself after 5 min.
    FarCoreDown,
    /// One dial ended without a link, or a link dropped (D2 step 9). It
    /// closes nothing: core's next dial is the one retry.
    DialFailed,
    /// The retry failed too: closed until a human act gives a lease newer
    /// than the failure (D2 step 9).
    RetrySpent,
}

impl GateClass {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NeedsCheck => "needs_check",
            Self::AuthStalled => "auth_stalled",
            Self::AuthFailed => "auth_failed",
            Self::FarCoreDown => "far_core_down",
            Self::DialFailed => "dial_failed",
            Self::RetrySpent => "retry_spent",
        }
    }
}

/// How long `far_core_down` keeps the gate closed (D10).
pub(crate) const FAR_CORE_DOWN_SECS: f64 = 300.0;

/// The last failure that closed the gate, and when.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Gate {
    pub class: GateClass,
    pub at: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Gate {
    /// Whether this record still closes the gate at `now`, given the time
    /// of the newest human act behind a lease in force (`human_at`): the
    /// login classes until a person logs in, `far_core_down` for 5 min,
    /// `retry_spent` until a lease newer than the failure (D2 step 9, D10).
    pub(crate) fn closes_at(&self, now: f64, human_at: Option<f64>) -> bool {
        match self.class {
            GateClass::NeedsCheck | GateClass::AuthStalled | GateClass::AuthFailed => true,
            GateClass::FarCoreDown => now < self.at + FAR_CORE_DOWN_SECS,
            GateClass::DialFailed => false,
            GateClass::RetrySpent => !human_at.is_some_and(|at| at > self.at),
        }
    }
}

/// How a dial ended without a link, as the watcher saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Failure {
    /// One of D10's classes, read on ssh's stderr or from the deadline.
    Class(GateClass),
    /// ssh ended with no class: refused, cut, timed out, or a link that
    /// dropped (D2 step 9).
    Ended(&'static str),
}

/// The gate a failure leaves (D2 step 9 over D10): the login classes stand
/// as they are; any other failure is the first (`dial_failed`, or a
/// timed `far_core_down`) when no failure is recorded, else `retry_spent`.
/// `current` is the gate as recorded when the dial ended: a link that
/// settled cleared it, so a drop after that is a first failure again.
pub(crate) fn next_gate(current: Option<&Gate>, failure: Failure, now: f64) -> Gate {
    let first = current.is_none();
    let (class, detail) = match failure {
        Failure::Class(
            class @ (GateClass::NeedsCheck | GateClass::AuthStalled | GateClass::AuthFailed),
        ) => (class, None),
        Failure::Class(GateClass::FarCoreDown) if first => (GateClass::FarCoreDown, None),
        Failure::Class(class) => (GateClass::RetrySpent, Some(class.as_str().to_string())),
        Failure::Ended(why) if first => (GateClass::DialFailed, Some(why.to_string())),
        Failure::Ended(why) => (GateClass::RetrySpent, Some(why.to_string())),
    };
    Gate {
        class,
        at: now,
        detail,
    }
}

/// When a dial last started, and its pid (the ssh pid core holds).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct DialRecord {
    pub at: f64,
    pub pid: u32,
}

/// A lease holder's file name (D3): `manual`, `client-<pid>`, `cli-<pid>`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct LeaseHolder(String);

impl LeaseHolder {
    pub(crate) fn manual() -> Self {
        Self("manual".into())
    }

    /// A TUI client's lease.
    pub(crate) fn client(pid: u32) -> Self {
        Self(format!("client-{pid}"))
    }

    /// A fleet CLI call's lease (D7).
    pub(crate) fn cli(pid: u32) -> Self {
        Self(format!("cli-{pid}"))
    }

    pub(crate) fn parse(raw: &str) -> Result<Self, String> {
        // A positive `pid_t`: digits that overflow it are no holder, never a
        // pid-less one (review M7).
        let pid = |rest: &str| {
            !rest.is_empty()
                && rest.len() <= 10
                && rest.bytes().all(|byte| byte.is_ascii_digit())
                && !rest.starts_with('0')
                && rest.parse::<i32>().is_ok_and(|pid| pid > 0)
        };
        let ok = raw == "manual"
            || raw.strip_prefix("client-").is_some_and(pid)
            || raw.strip_prefix("cli-").is_some_and(pid);
        ok.then(|| Self(raw.to_string()))
            .ok_or_else(|| format!("{raw:?} is not a lease holder"))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// A fleet CLI call's holder, `cli-<pid>`.
    pub(crate) fn is_cli(&self) -> bool {
        self.0.starts_with("cli-")
    }

    /// The longest a lease of this holder lives past its write: a fleet
    /// CLI's minute, a client's #260 dormancy, `machine wake`'s 12 h.
    fn max_life(&self) -> f64 {
        if self.is_cli() {
            CLI_LEASE_SECS
        } else if self.0.starts_with("client-") {
            super::presence::CLIENT_LEASE_SECS
        } else {
            MAX_WAKE_SECS
        }
    }

    /// The pid of a `client-<pid>` or `cli-<pid>` holder.
    pub(crate) fn pid(&self) -> Option<u32> {
        let rest = self
            .0
            .strip_prefix("client-")
            .or_else(|| self.0.strip_prefix("cli-"))?;
        rest.parse().ok()
    }

    /// Whether the holder can still hold its lease: `manual` always, a
    /// client or a CLI call while its process runs (review H7). A pid the
    /// system gave again to another process is caught by `reused_since`, not
    /// here.
    fn alive(&self) -> bool {
        self.pid()
            .is_none_or(crate::platform::core_link::process_alive)
    }

    /// Whether this holder's pid now runs a process that started after the
    /// lease was last written (`written_at`, unix seconds), so the pid was
    /// given again and the lease is not that process's (review M7). False
    /// for `manual`, a pid that runs nothing, or a start time that cannot be
    /// read; a second of slack covers the file time's granularity.
    pub(crate) fn reused_since(&self, written_at: Option<f64>) -> bool {
        let (Some(pid), Some(written_at)) = (self.pid(), written_at) else {
            return false;
        };
        crate::platform::process_started_at(pid).is_some_and(|started| started > written_at + 1.0)
    }
}

/// One lease: in force until `expires_at`; `input_at` is the time of the
/// human act that wrote it (an input, a person's command), which is what
/// reopens `retry_spent` (D2 step 9). A renewal that is no new act keeps
/// its `input_at`. A lease that does not `dial` only holds a link that is
/// up (D7): core-link admits no dial on it, the watcher keeps the link.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Lease {
    pub holder: LeaseHolder,
    pub expires_at: f64,
    pub input_at: f64,
    pub dials: bool,
}

impl Lease {
    /// `<expires_at> <input_at>`, or with a kind, `<expires_at> <input_at>
    /// dial` or `keep` for a lease that admits no dial; a file with the
    /// expiry alone (PR A's form) takes its modification time as the act.
    /// Refused as no lease (security review 2026-11-23 M4): a `cli-<pid>`
    /// lease with no kind and an act in the future (5 s for clocks). An
    /// expiry further than its holder's longest life from `now` is clamped
    /// to that life, not refused (a clock stepped back must not kill it).
    fn parse(
        holder: LeaseHolder,
        text: &str,
        modified: impl FnOnce() -> Option<f64>,
        now: f64,
    ) -> Option<Self> {
        let mut fields = text.split_ascii_whitespace();
        let number = |raw: &str| raw.parse::<f64>().ok().filter(|at| at.is_finite());
        let expires_at = number(fields.next()?)?;
        let input_at = match fields.next() {
            Some(raw) => number(raw)?,
            None => modified()?,
        };
        let dials = match fields.next() {
            None if holder.is_cli() => return None,
            None | Some("dial") => true,
            Some("keep") => false,
            Some(_) => return None,
        };
        if fields.next().is_some() {
            return None;
        }
        if input_at > now + LEASE_CLOCK_SLACK {
            return None;
        }
        // A wall clock stepped back (after a wake) leaves an expiry past
        // the holder's longest life: keep the lease, cap it.
        let expires_at = expires_at.min(now + holder.max_life());
        Some(Self {
            holder,
            expires_at,
            input_at,
            dials,
        })
    }

    /// Before its expiry and while its holder runs: a client that quit or
    /// crashed, or a CLI call killed before it removed its lease, allows
    /// the link no longer (review H7).
    pub(crate) fn in_force(&self, now: f64) -> bool {
        self.expires_at > now && self.holder.alive()
    }
}

/// The time of the newest human act behind a lease in force that admits a
/// dial, `None` when no such lease is in force (dormant: core-link dials
/// nothing).
pub(crate) fn newest_act(leases: &[Lease], now: f64) -> Option<f64> {
    leases
        .iter()
        .filter(|lease| lease.dials && lease.in_force(now))
        .map(|lease| lease.input_at)
        .reduce(f64::max)
}

/// Whether any lease is in force, one that only keeps a link included:
/// what keeps an up link up (D2 step 7).
pub(crate) fn holds_link(leases: &[Lease], now: f64) -> bool {
    leases.iter().any(|lease| lease.in_force(now))
}

/// How long a fleet CLI call's `cli-<pid>` lease lives past each write
/// (D7, E.2a 2.1): written again every 30 s while the CLI runs.
pub(crate) const CLI_LEASE_SECS: f64 = 60.0;
/// What a lease's times may be off by, for clocks and the write itself.
const LEASE_CLOCK_SLACK: f64 = 5.0;
/// The most lease files read, and the most bytes of each: a lease is one
/// short line, and a machine has a handful of holders (M4).
const LEASE_FILES_READ: usize = 256;
const LEASE_FILE_BYTES: u64 = 128;

/// How long a `manual` lease from `machine add --core` or `machine login`
/// lasts, and `machine wake`'s default (D3).
pub(crate) const MANUAL_LEASE_SECS: f64 = 30.0 * 60.0;
/// The longest `machine wake --for` (D3).
pub(crate) const MAX_WAKE_SECS: f64 = 12.0 * 3600.0;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A fresh private folder under `/tmp` for one test.
    pub(crate) fn scratch(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = PathBuf::from("/tmp").join(format!(
            "herdr-cm-{}-{tag}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("scratch");
        path
    }

    fn target() -> Target {
        Target {
            target: "ac@ac-mbm5".into(),
            session: "default".into(),
            ssh: "/usr/bin/ssh".into(),
            ssh_config: None,
            ssh_auth_sock: Some("/tmp/agent.sock".into()),
            via: None,
        }
    }

    #[test]
    fn the_root_is_under_the_given_home() {
        let root = machines_root(Path::new("/home/u"));
        assert_eq!(
            root,
            PathBuf::from("/home/u/.local/state")
                .join(crate::config::app_dir_name())
                .join("core-machines")
        );
    }

    #[test]
    fn the_root_follows_the_passwd_home_not_home_or_xdg() {
        let account = crate::platform::core_link::passwd_account().expect("passwd");
        assert_eq!(
            passwd_machines_root().expect("root"),
            machines_root(&account.home)
        );
    }

    #[test]
    fn target_round_trips_and_refuses_what_it_must() {
        let root = scratch("target");
        let dir = MachineDir::at(&root, &MachineName::parse("loop").expect("name"));
        dir.create().expect("create");
        dir.write_target(&target()).expect("write");
        assert_eq!(dir.read_target().expect("read"), target());

        for bad in [
            r#"{"target":"-oProxyCommand=x","session":"default","ssh":"/usr/bin/ssh"}"#,
            r#"{"target":"a b","session":"default","ssh":"/usr/bin/ssh"}"#,
            r#"{"target":"h","session":"x'y","ssh":"/usr/bin/ssh"}"#,
            r#"{"target":"h","session":"default","ssh":"ssh"}"#,
            r#"{"target":"h","session":"default","ssh":"/usr/bin/ssh","extra":1}"#,
            r#"{"target":"h","session":"default","ssh":"/usr/bin/ssh","ssh_auth_sock":"rel"}"#,
        ] {
            assert!(Target::parse(bad.as_bytes()).is_err(), "{bad} was accepted");
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_others_can_write_is_refused() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = scratch("mode");
        let dir = MachineDir::at(&root, &MachineName::parse("loop").expect("name"));
        dir.create().expect("create");
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o777)).expect("chmod");
        assert!(dir.check().is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn gate_classes_close_and_far_core_down_reopens_after_five_minutes() {
        let at = 1_000.0;
        for class in [
            GateClass::NeedsCheck,
            GateClass::AuthStalled,
            GateClass::AuthFailed,
        ] {
            let gate = Gate {
                class,
                at,
                detail: None,
            };
            assert!(
                gate.closes_at(at + 86_400.0, Some(at + 86_000.0)),
                "{class:?}"
            );
        }
        let down = Gate {
            class: GateClass::FarCoreDown,
            at,
            detail: None,
        };
        assert!(down.closes_at(at + 299.0, Some(at + 200.0)));
        assert!(!down.closes_at(at + 300.0, None));
    }

    #[test]
    fn a_first_failure_closes_nothing_and_retry_spent_waits_for_a_newer_act() {
        let failed = Gate {
            class: GateClass::DialFailed,
            at: 1_000.0,
            detail: None,
        };
        assert!(!failed.closes_at(1_001.0, None));
        let spent = Gate {
            class: GateClass::RetrySpent,
            at: 1_000.0,
            detail: None,
        };
        assert!(spent.closes_at(1_001.0, None));
        assert!(
            spent.closes_at(1_001.0, Some(999.0)),
            "a renewal with no new act"
        );
        assert!(spent.closes_at(1_001.0, Some(1_000.0)));
        assert!(
            !spent.closes_at(1_001.0, Some(1_000.5)),
            "an act after the failure"
        );
    }

    /// D2 step 9 as a sequence: a first failure, the one retry, then
    /// `retry_spent` until an act newer than it; a link that settled
    /// cleared the gate, so its drop is a first failure again.
    #[test]
    fn the_retry_budget_is_one_dial_per_act() {
        let refused = Failure::Ended("ssh ended before the far side answered");
        let first = next_gate(None, refused, 10.0);
        assert_eq!(first.class, GateClass::DialFailed);
        let spent = next_gate(Some(&first), refused, 20.0);
        assert_eq!(spent.class, GateClass::RetrySpent);
        assert_eq!(
            spent.detail.as_deref(),
            Some("ssh ended before the far side answered")
        );
        // An act at 30 reopens; its dial fails at 40: spent again, newer.
        assert!(!spent.closes_at(31.0, Some(30.0)));
        let again = next_gate(Some(&spent), refused, 40.0);
        assert_eq!((again.class, again.at), (GateClass::RetrySpent, 40.0));
        assert!(again.closes_at(41.0, Some(30.0)));

        // far_core_down: timed the first time, spent the second.
        let down = next_gate(None, Failure::Class(GateClass::FarCoreDown), 10.0);
        assert_eq!(down.class, GateClass::FarCoreDown);
        let down_again = next_gate(Some(&down), Failure::Class(GateClass::FarCoreDown), 400.0);
        assert_eq!(down_again.class, GateClass::RetrySpent);
        assert_eq!(down_again.detail.as_deref(), Some("far_core_down"));

        // The login classes stand whatever was recorded before.
        for class in [
            GateClass::NeedsCheck,
            GateClass::AuthStalled,
            GateClass::AuthFailed,
        ] {
            assert_eq!(next_gate(None, Failure::Class(class), 1.0).class, class);
            assert_eq!(
                next_gate(Some(&spent), Failure::Class(class), 1.0).class,
                class
            );
        }
    }

    #[test]
    fn gate_file_round_trips_and_clears() {
        let root = scratch("gate");
        let dir = MachineDir::at(&root, &MachineName::parse("loop").expect("name"));
        dir.create().expect("create");
        assert_eq!(dir.read_gate().expect("none"), None);
        // A nanosecond clock reading whose shortest form has 17 digits: serde_json's default
        // float parser read it back one ulp off, so the gate came back a different time
        // (https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/300).
        let gate = Gate {
            class: GateClass::AuthStalled,
            at: 1_795_385_661.720_020_5,
            detail: Some("15 s".into()),
        };
        dir.write_gate(&gate).expect("write");
        assert_eq!(dir.read_gate().expect("read"), Some(gate));
        let text = fs::read_to_string(dir.gate_path()).expect("text");
        assert!(text.contains("\"auth_stalled\""), "{text}");
        dir.clear_gate().expect("clear");
        dir.clear_gate().expect("clear twice");
        assert_eq!(dir.read_gate().expect("none again"), None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn leases_are_one_file_per_holder() {
        let root = scratch("lease");
        let dir = MachineDir::at(&root, &MachineName::parse("loop").expect("name"));
        dir.create().expect("create");
        dir.write_lease(&LeaseHolder::manual(), 2_000.0, 100.0)
            .expect("manual");
        let me = std::process::id();
        dir.write_lease(&LeaseHolder::cli(me), 3_000.0, 200.0)
            .expect("cli");
        fs::write(dir.leases_dir().join("manual.new"), "9 9").expect("in progress");
        fs::write(dir.leases_dir().join("Bad Name"), "9 9").expect("bad");
        fs::write(dir.leases_dir().join("client-7"), "9 9 9").expect("three fields");
        fs::write(dir.leases_dir().join("cli-7.12-0.new"), "9 9").expect("a write in progress");
        let leases = dir.leases();
        assert_eq!(
            leases,
            vec![
                Lease {
                    holder: LeaseHolder::cli(me),
                    expires_at: 3_000.0,
                    input_at: 200.0,
                    dials: true,
                },
                Lease {
                    holder: LeaseHolder::manual(),
                    expires_at: 2_000.0,
                    input_at: 100.0,
                    dials: true,
                },
            ]
        );
        assert_eq!(newest_act(&leases, 1_000.0), Some(200.0));
        assert_eq!(newest_act(&leases, 2_500.0), Some(200.0));
        assert_eq!(newest_act(&leases, 3_000.0), None, "both expired");
        dir.remove_lease(&LeaseHolder::cli(me)).expect("remove");
        dir.remove_lease(&LeaseHolder::cli(me))
            .expect("remove twice");
        assert_eq!(dir.leases().len(), 1);
        let _ = fs::remove_dir_all(&root);
    }

    /// D7 (2.2): a keep lease holds an up link but admits no dial, so a
    /// drop during a fleet CLI call is never redialled on its account.
    #[test]
    fn a_keep_lease_holds_the_link_and_admits_no_dial() {
        let root = scratch("keep");
        let dir = MachineDir::at(&root, &MachineName::parse("loop").expect("name"));
        dir.create().expect("create");
        // A running holder: a gone one's lease is not in force (H7).
        let me = std::process::id();
        dir.write_keep_lease(&LeaseHolder::cli(me), 3_000.0)
            .expect("keep");
        let leases = dir.leases();
        assert_eq!(
            leases,
            vec![Lease {
                holder: LeaseHolder::cli(me),
                expires_at: 3_000.0,
                input_at: 0.0,
                dials: false,
            }]
        );
        assert_eq!(
            newest_act(&leases, 1_000.0),
            None,
            "no dial on a keep lease"
        );
        assert!(holds_link(&leases, 1_000.0));
        assert!(!holds_link(&leases, 3_000.0), "expired");
        fs::write(dir.leases_dir().join("cli-43"), "9e9 0 maybe").expect("unknown kind");
        assert_eq!(dir.leases().len(), 1, "an unknown kind is no lease");
        let _ = fs::remove_dir_all(&root);
    }

    /// M4: a `cli-<pid>` lease names its kind; any lease whose expiry is
    /// beyond its holder's longest life, or whose act is in the future, is
    /// no lease; a lease file over 128 bytes neither.
    #[test]
    fn leases_are_bounded_by_their_holders_kind() {
        let root = scratch("lease-bounds");
        let dir = MachineDir::at(&root, &MachineName::parse("loop").expect("name"));
        dir.create().expect("create");
        let now = now_unix();
        let lease = |name: &str, text: String| {
            fs::write(dir.leases_dir().join(name), text).expect("write");
            let found = dir
                .leases()
                .into_iter()
                .find(|lease| lease.holder.as_str() == name);
            let _ = fs::remove_file(dir.leases_dir().join(name));
            found
        };
        // Written by herdr: a CLI's grant names `dial`.
        dir.write_lease(&LeaseHolder::cli(42), now + 60.0, now)
            .expect("grant");
        assert!(fs::read_to_string(dir.leases_dir().join("cli-42"))
            .expect("read")
            .trim_end()
            .ends_with(" dial"));
        assert!(dir.leases()[0].dials);
        dir.remove_lease(&LeaseHolder::cli(42)).expect("remove");
        assert!(
            lease("cli-43", format!("{:.3} {now:.3}", now + 60.0)).is_none(),
            "no kind"
        );
        assert!(
            lease("cli-43", format!("{:.3}", now + 60.0)).is_none(),
            "PR A's form"
        );
        assert!(lease("cli-43", format!("{:.3} {now:.3} dial", now + 60.0)).is_some());
        assert!(lease("client-7", format!("{:.3} {now:.3}", now + 600.0)).is_some());
        assert!(lease("manual", format!("{:.3} {now:.3}", now + MAX_WAKE_SECS)).is_some());
        assert!(
            lease("manual", format!("{:.3} {:.3}", now + 60.0, now + 3_600.0)).is_none(),
            "a future act"
        );
        assert!(
            lease(
                "manual",
                format!("{:.3} {now:.3}{}", now + 60.0, " ".repeat(200))
            )
            .is_none(),
            "200 bytes"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// #310: a wall clock stepped back leaves an expiry past the holder's
    /// longest life; the lease is kept with its expiry clamped to
    /// `now + max_life`, not refused. A future act is still refused.
    #[test]
    fn an_expiry_past_the_longest_life_is_clamped_not_refused() {
        let root = scratch("clamp");
        let dir = MachineDir::at(&root, &MachineName::parse("loop").expect("name"));
        dir.create().expect("create");
        let now = now_unix();
        let lease = |name: &str, text: String| {
            fs::write(dir.leases_dir().join(name), text).expect("write");
            let found = dir
                .leases()
                .into_iter()
                .find(|lease| lease.holder.as_str() == name);
            let _ = fs::remove_file(dir.leases_dir().join(name));
            found
        };
        // Live holders: a lease of a process that no longer runs is no lease (PR D, H7).
        let pid = std::process::id();
        let (cli, client) = (format!("cli-{pid}"), format!("client-{pid}"));
        for (name, life, text_kind) in [
            (cli.as_str(), CLI_LEASE_SECS, " dial"),
            (
                client.as_str(),
                super::super::presence::CLIENT_LEASE_SECS,
                "",
            ),
            ("manual", MAX_WAKE_SECS, ""),
            ("manual", MAX_WAKE_SECS, " keep"),
        ] {
            let found = lease(name, format!("{:.3} {now:.3}{text_kind}", now + 3.0 * life))
                .unwrap_or_else(|| panic!("{name} refused"));
            assert!(
                found.expires_at <= now_unix() + life,
                "{name}: {}",
                found.expires_at
            );
            assert!(found.in_force(now), "{name}");
        }
        assert!(
            lease("manual", format!("{:.3} {:.3}", now + 60.0, now + 3_600.0)).is_none(),
            "a future act"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// #310: temporaries, unparseable names and expired leases do not
    /// count toward the read cap, so a live lease among them is found.
    #[test]
    fn strays_do_not_push_a_live_lease_out_of_the_read() {
        let root = scratch("cap");
        let dir = MachineDir::at(&root, &MachineName::parse("loop").expect("name"));
        dir.create().expect("create");
        for n in 0..(LEASE_FILES_READ * 16) {
            fs::write(dir.leases_dir().join(format!("stray-{n:04}.new")), "x").expect("stray");
        }
        dir.write_lease(&LeaseHolder::manual(), now_unix() + 60.0, now_unix())
            .expect("live");
        let holders: Vec<_> = dir.leases().into_iter().map(|l| l.holder).collect();
        assert_eq!(holders, vec![LeaseHolder::manual()]);
        let _ = fs::remove_dir_all(&root);
    }

    /// 7.4: a write never takes over another writer's temporary file.
    #[test]
    fn an_atomic_write_leaves_another_writers_temporary_alone() {
        let root = scratch("atomic");
        let path = root.join("far_herdr.json");
        let theirs = root.join("far_herdr.json.new");
        fs::write(&theirs, b"another writer, mid-write").expect("theirs");
        write_atomic(&path, b"mine").expect("write");
        assert_eq!(fs::read(&path).expect("mine"), b"mine");
        assert_eq!(
            fs::read(&theirs).expect("theirs"),
            b"another writer, mid-write"
        );
        let left: Vec<_> = fs::read_dir(&root)
            .expect("list")
            .flatten()
            .map(|entry| entry.file_name())
            .collect();
        assert_eq!(left.len(), 2, "no temporary of mine is left: {left:?}");
        let _ = fs::remove_dir_all(&root);
    }

    /// #310: a temporary file with this pid and the next write's number,
    /// left by a killed writer, does not fail the write.
    #[test]
    fn an_atomic_write_replaces_a_stale_temporary_of_its_own_name() {
        use std::sync::atomic::Ordering;
        let root = scratch("atomic-stale");
        let path = root.join("far_herdr.json");
        let next = WRITE_SEQ.load(Ordering::Relaxed);
        for n in next..next + 4 {
            fs::write(temporary_path(&path, n), b"left by a killed writer").expect("stale");
        }
        write_atomic(&path, b"mine").expect("write");
        assert_eq!(fs::read(&path).expect("mine"), b"mine");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_one_field_lease_takes_its_mtime_as_the_act() {
        let root = scratch("lease1");
        let dir = MachineDir::at(&root, &MachineName::parse("loop").expect("name"));
        dir.create().expect("create");
        let before = now_unix() - 1.0;
        fs::write(dir.leases_dir().join("manual"), "5000.0\n").expect("write");
        let lease = &dir.leases()[0];
        assert_eq!(lease.expires_at, 5_000.0);
        assert!(lease.input_at >= before && lease.input_at <= now_unix() + 1.0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn holders_are_the_three_kinds() {
        for good in ["manual", "client-1", "cli-4242"] {
            assert!(LeaseHolder::parse(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            "client-",
            "client-01",
            "cli-x",
            "manual-1",
            "other",
            "client-1.new",
            // M7: digits a pid_t cannot hold are no holder, not a pid-less
            // one that would hold its lease as `manual` does.
            "client-4294967296",
            "cli-9999999999",
            "client-2147483648",
        ] {
            assert!(LeaseHolder::parse(bad).is_err(), "{bad}");
        }
        assert!(LeaseHolder::parse("client-2147483647").is_ok());
        assert_eq!(LeaseHolder::client(7).pid(), Some(7));
        assert_eq!(LeaseHolder::cli(7).pid(), Some(7));
        assert_eq!(LeaseHolder::manual().pid(), None);
    }

    /// V5: two clients and a CLI writer, each writing only its own lease
    /// over and over while a reader lists them: every listing sees whole
    /// files, never half of one; an expired lease is not in force.
    #[test]
    fn no_holder_writes_another_holders_lease() {
        let root = scratch("lease-threads");
        let dir = MachineDir::at(&root, &MachineName::parse("loop").expect("name"));
        dir.create().expect("create");
        let holders = [
            LeaseHolder::client(11),
            LeaseHolder::client(12),
            LeaseHolder::cli(13),
        ];
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader = {
            let dir = dir.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut reads = 0_u32;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    for lease in dir.leases() {
                        // Each holder writes expiry = act + 600 with its own
                        // act base, so a mixed or torn file shows here.
                        let base = match lease.holder.as_str() {
                            "client-11" => 1_000.0,
                            "client-12" => 2_000.0,
                            "cli-13" => 3_000.0,
                            other => panic!("unexpected holder {other}"),
                        };
                        assert_eq!(lease.expires_at, lease.input_at + 600.0, "{lease:?}");
                        assert!(
                            lease.input_at >= base && lease.input_at < base + 1_000.0,
                            "{lease:?}"
                        );
                    }
                    reads += 1;
                }
                reads
            })
        };
        let writers: Vec<_> = holders
            .iter()
            .cloned()
            .zip([1_000.0, 2_000.0, 3_000.0])
            .map(|(holder, base)| {
                let dir = dir.clone();
                std::thread::spawn(move || {
                    for step in 0..200 {
                        let act = base + f64::from(step);
                        dir.write_lease(&holder, act + 600.0, act).expect("write");
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().expect("writer");
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(reader.join().expect("reader") > 0);
        let leases = dir.leases();
        assert_eq!(leases.len(), 3);
        for lease in &leases {
            assert_eq!(lease.input_at.fract(), 0.0);
            assert_eq!(lease.expires_at, lease.input_at + 600.0);
        }
        assert_eq!(newest_act(&leases, 3_199.0 + 600.0), None);
        let _ = fs::remove_dir_all(&root);
    }

    // M7: a lease whose pid now runs a process that started after the lease
    // was last written is not that process's: the pid was given again, and
    // the lease is not listed (so not in force). A lease this process wrote
    // after it started is.
    #[test]
    fn a_reused_pids_lease_is_no_ones() {
        let root = scratch("lease-reused");
        let dir = MachineDir::at(&root, &MachineName::parse("loop").expect("name"));
        dir.create().expect("create");
        let me = std::process::id();
        dir.write_lease(&LeaseHolder::client(me), now_unix() + 600.0, now_unix())
            .expect("write");
        assert_eq!(dir.leases().len(), 1, "written by this process");
        // As if written long before this process started.
        let file = fs::File::options()
            .write(true)
            .open(dir.leases_dir().join(format!("client-{me}")))
            .expect("open");
        file.set_modified(UNIX_EPOCH + std::time::Duration::from_secs(1_000_000))
            .expect("mtime");
        drop(file);
        assert!(dir.leases().is_empty(), "{:?}", dir.leases());
        let _ = fs::remove_dir_all(&root);
    }

    // H7: a lease is in force before its expiry and while its holder runs.
    // A client or CLI call that is gone allows the link no longer; `manual`
    // has no process and ends by its expiry alone.
    #[test]
    fn a_gone_holders_lease_is_not_in_force() {
        let root = scratch("lease-holder");
        let dir = MachineDir::at(&root, &MachineName::parse("loop").expect("name"));
        dir.create().expect("create");
        let me = std::process::id();
        // A pid that cannot be a process here (above macOS's and Linux's
        // pid limits).
        let gone = 4_000_000_u32;
        assert!(!crate::platform::core_link::process_alive(gone));
        for holder in [
            LeaseHolder::client(gone),
            LeaseHolder::cli(gone),
            LeaseHolder::client(me),
            LeaseHolder::cli(me),
            LeaseHolder::manual(),
        ] {
            dir.write_lease(&holder, 2_000.0, 100.0).expect("write");
        }
        let in_force: Vec<String> = dir
            .leases()
            .into_iter()
            .filter(|lease| lease.in_force(1_000.0))
            .map(|lease| lease.holder.as_str().to_string())
            .collect();
        assert_eq!(
            in_force,
            [
                format!("cli-{me}"),
                format!("client-{me}"),
                "manual".to_string()
            ]
        );
        assert!(dir.leases().iter().all(|lease| !lease.in_force(2_000.0)));
        dir.remove_lease(&LeaseHolder::client(me)).expect("remove");
        dir.remove_lease(&LeaseHolder::cli(me)).expect("remove");
        dir.remove_lease(&LeaseHolder::manual()).expect("remove");
        assert_eq!(
            newest_act(&dir.leases(), 1_000.0),
            None,
            "only gone holders left"
        );
        let _ = fs::remove_dir_all(&root);
    }
}
