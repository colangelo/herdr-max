//! A core machine's link state on this Mac (core-phase5 D2): one private
//! folder per machine under the account's passwd home, holding
//! `target.json`, `gate.json`, `leases/`, `dial.lock`, `dial.json`, the
//! managed `ssh_config` and `link.log`. Every file is parsed here, at the
//! boundary; a file that does not parse is refused, never guessed at.

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

    pub(crate) fn dial_record_path(&self) -> PathBuf {
        self.path.join("dial.json")
    }

    pub(crate) fn ssh_config_path(&self) -> PathBuf {
        self.path.join("ssh_config")
    }

    pub(crate) fn log_path(&self) -> PathBuf {
        self.path.join("link.log")
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

    /// Writes `leases/<holder>` with its expiry (write, then rename): each
    /// holder writes only its own file (D3).
    pub(crate) fn write_lease(&self, holder: &LeaseHolder, expires_at: f64) -> io::Result<()> {
        write_atomic(
            &self.leases_dir().join(holder.as_str()),
            format!("{expires_at:.3}\n").as_bytes(),
        )
    }

    /// Every lease file that parses, in force or not; `.new` files are
    /// writes in progress.
    pub(crate) fn leases(&self) -> Vec<Lease> {
        let Ok(entries) = fs::read_dir(self.leases_dir()) else {
            return Vec::new();
        };
        let mut leases: Vec<Lease> = entries
            .flatten()
            .filter_map(|entry| {
                let holder = LeaseHolder::parse(entry.file_name().to_str()?).ok()?;
                let text = fs::read_to_string(entry.path()).ok()?;
                let expires_at = text
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|at| at.is_finite())?;
                Some(Lease { holder, expires_at })
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

/// Writes `<path>.new` (0600), then renames it over `path`, so a reader sees
/// the old file or the new one, never half of one.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".new");
    let temporary = PathBuf::from(temporary);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, path)
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
}

impl GateClass {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NeedsCheck => "needs_check",
            Self::AuthStalled => "auth_stalled",
            Self::AuthFailed => "auth_failed",
            Self::FarCoreDown => "far_core_down",
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
    pub(crate) fn new(class: GateClass, detail: Option<String>) -> Self {
        Self {
            class,
            at: now_unix(),
            detail,
        }
    }

    /// Whether this failure still closes the gate at `now`: every class
    /// until a person logs in, `far_core_down` for 5 min.
    pub(crate) fn closes_at(&self, now: f64) -> bool {
        match self.class {
            GateClass::FarCoreDown => now < self.at + FAR_CORE_DOWN_SECS,
            _ => true,
        }
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

    pub(crate) fn parse(raw: &str) -> Result<Self, String> {
        let ok = !raw.is_empty()
            && raw.len() <= 64
            && raw
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        ok.then(|| Self(raw.to_string()))
            .ok_or_else(|| format!("{raw:?} is not a lease holder"))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Lease {
    pub holder: LeaseHolder,
    pub expires_at: f64,
}

/// How long a `manual` lease from `machine add --core` or `machine login`
/// lasts (D3).
pub(crate) const MANUAL_LEASE_SECS: f64 = 30.0 * 60.0;

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
            assert!(gate.closes_at(at + 86_400.0), "{class:?}");
        }
        let down = Gate {
            class: GateClass::FarCoreDown,
            at,
            detail: None,
        };
        assert!(down.closes_at(at + 299.0));
        assert!(!down.closes_at(at + 300.0));
    }

    #[test]
    fn gate_file_round_trips_and_clears() {
        let root = scratch("gate");
        let dir = MachineDir::at(&root, &MachineName::parse("loop").expect("name"));
        dir.create().expect("create");
        assert_eq!(dir.read_gate().expect("none"), None);
        let gate = Gate::new(GateClass::AuthStalled, Some("15 s".into()));
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
        dir.write_lease(&LeaseHolder::manual(), 2_000.0)
            .expect("manual");
        dir.write_lease(&LeaseHolder::parse("cli-42").expect("cli"), 3_000.0)
            .expect("cli");
        fs::write(dir.leases_dir().join("manual.new"), "9").expect("in progress");
        fs::write(dir.leases_dir().join("Bad Name"), "9").expect("bad");
        let leases = dir.leases();
        assert_eq!(
            leases,
            vec![
                Lease {
                    holder: LeaseHolder::parse("cli-42").expect("cli"),
                    expires_at: 3_000.0
                },
                Lease {
                    holder: LeaseHolder::manual(),
                    expires_at: 2_000.0
                },
            ]
        );
        let _ = fs::remove_dir_all(&root);
    }
}
