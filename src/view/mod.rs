//! The view server (openspec/changes/core-phase5, D6): the herdr binary in a
//! second role, one process per viewed core machine, that shows and drives
//! another Mac's herdr-max panes through the primary core's machine link.
//!
//! Its layout is the far side's `herdr-app` document, read through the
//! primary core (`doc.get {on}`); its panes are the far terminals, attached
//! so that the view never becomes their active viewer (D6 Size). It saves
//! nothing, publishes nothing, serves no JSON API and starts no program on
//! this Mac; its structural actions go to the far herdr (D7, E.3), where a
//! new tab or a split starts a far shell and a pane's close ends the far
//! program. A TUI client connects to its own client socket as to any herdr
//! server.
//! Unix only, as the core backend is.

pub(crate) mod doc;
pub(crate) mod instance;
pub(crate) mod lifetime;
pub(crate) mod policy;
pub(crate) mod start;
pub(crate) mod watch;

use std::path::PathBuf;
use std::sync::OnceLock;

use crate::core_machine::MachineName;

/// What a herdr server process is (D6). Read at the boundaries where the
/// two differ: socket, layout source, persistence, publishers, API, input,
/// close and lifetime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ServerRole {
    Primary,
    View(MachineName),
}

static ROLE: OnceLock<ServerRole> = OnceLock::new();

/// This process's role; `Primary` unless `herdr server --view` set it.
pub(crate) fn role() -> &'static ServerRole {
    ROLE.get_or_init(|| ServerRole::Primary)
}

/// The machine this process views, `None` for a primary server or a client.
pub(crate) fn viewed_machine() -> Option<&'static MachineName> {
    match role() {
        ServerRole::Primary => None,
        ServerRole::View(machine) => Some(machine),
    }
}

/// Sets the role once, at the start of `herdr server --view`; false when it
/// was already read or set.
pub(crate) fn set_role(role: ServerRole) -> bool {
    ROLE.set(role).is_ok()
}

/// The largest far grid a view builds (security review M4): a far size is
/// checked against these before a grid is built at it or resized to it. A
/// 6K display at a 6x12 px font is about 1000 columns by 280 rows, so a
/// real terminal fits with room; 300 000 cells is a few MB of grid, and the
/// document caps a view at 256 panes. A size over any of them is refused,
/// never clamped: a clamped grid would draw the far screen wrongly, and its
/// size would reach core as the view's RESIZE (D6 Size). The pane keeps its
/// last good size; an attach at such a size fails and says why.
pub(crate) const MAX_FAR_ROWS: u16 = 1000;
pub(crate) const MAX_FAR_COLS: u16 = 1000;
pub(crate) const MAX_FAR_CELLS: u32 = 300_000;

/// Whether a far size (rows, cols) is one a view builds a grid for.
pub(crate) fn far_size_fits(rows: u16, cols: u16) -> bool {
    rows > 0
        && cols > 0
        && rows <= MAX_FAR_ROWS
        && cols <= MAX_FAR_COLS
        && u32::from(rows) * u32::from(cols) <= MAX_FAR_CELLS
}

/// The view server's client socket for `machine`, beside the herdr
/// session's own: one per viewed machine, so two views never share one.
pub(crate) fn socket_path(machine: &MachineName) -> PathBuf {
    crate::session::data_dir().join(format!("herdr-view-{machine}.sock"))
}

/// The log of the view server for `machine`, beside the session's own.
pub(crate) fn log_file_name(machine: &MachineName) -> String {
    format!("herdr-view-{machine}.log")
}

/// A view's warning line for its clients: the config diagnostic and the
/// view's notice, both, one per line (neither replaces the other).
pub(crate) fn with_notice(config: Option<String>, notice: Option<&str>) -> Option<String> {
    match (config, notice) {
        (Some(config), Some(notice)) => Some(format!("{config}\n{notice}")),
        (config, None) => config,
        (None, Some(notice)) => Some(notice.to_owned()),
    }
}

/// The view server's lock and record for `machine`, beside its socket: the
/// running view holds an exclusive `flock` on it for its whole life and
/// writes its version and pid into it (`ViewRecord`).
pub(crate) fn lock_path(machine: &MachineName) -> PathBuf {
    crate::session::data_dir().join(format!("herdr-view-{machine}.lock"))
}

/// What a running view says of itself in its lock file (D6 Lifetime): the
/// build it runs, so a client of another build restarts it, and its pid,
/// for that restart.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ViewRecord {
    pub version: String,
    pub pid: u32,
    /// The view process's start token (`platform::process_start`), so a
    /// restart signals that process and no other that got its pid (review
    /// M8). `None` in a record of an older build.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<u64>,
}

impl ViewRecord {
    /// This process's record.
    fn own() -> Self {
        let pid = std::process::id();
        Self {
            version: crate::build_info::version(),
            pid,
            start: match crate::platform::process_start(pid) {
                crate::platform::ProcessStart::Started(token) => Some(token),
                _ => None,
            },
        }
    }

    /// Whether `pid` is still the process that wrote this record (review
    /// M8): its start token matches. A record with none (an older build's,
    /// or one whose start could not be read) names no process: a live pid
    /// alone may be another program's, so such a view is never signalled.
    pub(crate) fn names_its_process(&self) -> bool {
        self.start.is_some_and(|token| {
            crate::platform::process_start(self.pid)
                == crate::platform::ProcessStart::Started(token)
        })
    }
}

/// The most of a lock record read (a version and two numbers).
const RECORD_MAX: u64 = 4096;

/// Opens the view's lock file at `path`, never through a symlink, and
/// only as a regular file of this account in a folder of this account that
/// not everyone can write (review M8): a planted link cannot point the
/// lock, or its truncation, at another file. The folder is checked, and the
/// file opened, at its real path, so a symlinked config folder is its
/// target's; a group-writable one (umask 002) is this account's still. The
/// open never waits (`O_NONBLOCK`): a FIFO planted as the lock is refused,
/// not waited on.
fn open_lock(path: &std::path::Path, create: bool) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
    let uid = unsafe { libc::getuid() };
    let not_ours = |what: &std::path::Path, why: &str| {
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("{} is not {why}", what.display()),
        )
    };
    let (Some(dir), Some(file_name)) = (path.parent(), path.file_name()) else {
        return Err(not_ours(path, "a lock file path"));
    };
    if create {
        std::fs::create_dir_all(dir)?;
    }
    let dir = std::fs::canonicalize(dir)?;
    let meta = std::fs::metadata(&dir)?;
    if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o002 != 0 {
        return Err(not_ours(
            &dir,
            "a folder of this account that not everyone can write",
        ));
    }
    let path = dir.join(file_name);
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(create)
        .create(create)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&path)?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.uid() != uid {
        return Err(not_ours(&path, "a regular file of this account"));
    }
    Ok(file)
}

/// Takes the exclusive lock on `file` now; an `flock` the filesystem does
/// not support fails at once and says so (PR D's second read, item 6).
fn lock_now(file: &std::fs::File, path: &std::path::Path) -> std::io::Result<bool> {
    crate::platform::core_link::lock_exclusive_within(file, std::time::Duration::ZERO).map_err(
        |err| {
            if err.raw_os_error() == Some(libc::ENOTSUP)
                || err.raw_os_error() == Some(libc::EOPNOTSUPP)
            {
                std::io::Error::new(
                    err.kind(),
                    format!(
                        "{} is on a filesystem without flock, so a view cannot hold its lock there: {err}",
                        path.display()
                    ),
                )
            } else {
                err
            }
        },
    )
}

/// The lock a view server holds while it runs; dropped with the process.
pub(crate) struct ViewLock {
    _file: std::fs::File,
}

static HELD: OnceLock<ViewLock> = OnceLock::new();

/// Takes the view's lock for `machine` at `path`, before anything else in
/// the process writes to the view's log or socket (review L3.4): two views
/// started at once (two clients racing to start one) are one view and one
/// that exits at once, touching neither. `Ok(None)` when another view holds
/// it. The record is written once the lock is taken.
pub(crate) fn take_lock_at(path: &std::path::Path) -> std::io::Result<Option<ViewLock>> {
    use std::io::Write as _;
    let mut file = open_lock(path, true)?;
    if !lock_now(&file, path)? {
        return Ok(None);
    }
    let record = ViewRecord::own();
    file.set_len(0)?;
    file.write_all(
        serde_json::to_string(&record)
            .map_err(std::io::Error::other)?
            .as_bytes(),
    )?;
    Ok(Some(ViewLock { _file: file }))
}

/// Takes and keeps the lock of the view of `machine` for this process's
/// life; false when another view of it runs.
pub(crate) fn hold_lock(machine: &MachineName) -> std::io::Result<bool> {
    match take_lock_at(&lock_path(machine))? {
        Some(lock) => Ok(HELD.set(lock).is_ok()),
        None => Ok(false),
    }
}

/// The record of the view whose lock is at `path`, while one holds it,
/// read from the inode it locks (review M8), at most `RECORD_MAX` bytes.
/// `None` when no view holds it, or the lock cannot be probed (said once in
/// the log: a filesystem without flock, a file that is no private regular
/// file).
pub(crate) fn running_view_at(path: &std::path::Path) -> Option<ViewRecord> {
    use std::io::Read as _;
    let file = match open_lock(path, false) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
        Err(err) => {
            tracing::warn!(path = %path.display(), %err, "cannot read the view's lock");
            return None;
        }
    };
    match lock_now(&file, path) {
        // Nobody holds it: no view runs (the lock goes with the file).
        Ok(true) => return None,
        Ok(false) => {}
        Err(err) => {
            tracing::warn!(path = %path.display(), %err, "cannot probe the view's lock");
            return None;
        }
    }
    let mut text = String::new();
    (&file).take(RECORD_MAX).read_to_string(&mut text).ok()?;
    serde_json::from_str(&text).ok()
}

/// The configuration a view server runs with: the user's, with every core
/// publisher off (D6: no spaces, extras, status mirror or label writes; the
/// far herdr owns those documents and labels).
pub(crate) fn view_config(config: &mut crate::config::Config) {
    let experimental = &mut config.experimental;
    experimental.core_spaces_doc = false;
    experimental.core_extras_doc = false;
    experimental.core_status_mirror = false;
    experimental.core_label_sync = false;
}

/// The machine `herdr server --view <machine>` asks for, from the arguments
/// as main parsed them (`herdr server ...`, `--session` already taken out:
/// review M6, a session named `server` is never the server's word).
/// `Ok(None)` for a primary server.
pub(crate) fn requested_view(parsed_args: &[String]) -> Result<Option<ViewRequest>, String> {
    match parsed_args.get(1).map(String::as_str) {
        Some("server") => {
            let rest = &parsed_args[2..];
            let Some(machine) = parse_view_flag(rest)? else {
                return Ok(None);
            };
            let restart = match &rest[2..] {
                [] => false,
                [flag] if flag == "--restart" => true,
                _ => return Err("usage: herdr server --view <machine> [--restart]".into()),
            };
            Ok(Some(ViewRequest { machine, restart }))
        }
        _ => Ok(None),
    }
}

/// `herdr server --view <machine> [--restart]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ViewRequest {
    pub machine: MachineName,
    /// Stop a view of the machine that runs, whatever its build, and take
    /// its place (PR D's second read, item 4: after a rollback, a newer
    /// view is never restarted by an older client).
    pub restart: bool,
}

/// `--view <machine>` after `server`: the machine, `Ok(None)` without the
/// flag.
pub(crate) fn parse_view_flag(args: &[String]) -> Result<Option<MachineName>, String> {
    match args {
        [flag, machine, ..] if flag == "--view" => MachineName::parse(machine)
            .map(Some)
            .map_err(|err| err.to_string()),
        [flag] if flag == "--view" => Err("usage: herdr server --view <machine>".into()),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(raw: &[&str]) -> Vec<String> {
        raw.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn the_view_flag_names_a_machine() {
        assert_eq!(
            parse_view_flag(&args(&["--view", "ac-mbm5"])).unwrap(),
            Some(MachineName::parse("ac-mbm5").unwrap())
        );
        assert_eq!(parse_view_flag(&args(&[])).unwrap(), None);
        assert_eq!(parse_view_flag(&args(&["--handoff-import"])).unwrap(), None);
        assert!(parse_view_flag(&args(&["--view"])).is_err());
        assert!(parse_view_flag(&args(&["--view", "../x"])).is_err());
    }

    // M6 (security review): a view in a session named `server` is still a
    // view, and that session's primary is still a primary: the role comes
    // from the arguments as main parsed them, the session's name taken out.
    #[test]
    fn a_session_named_server_does_not_change_the_role() {
        let parsed = |raw: &[&str]| crate::session::configure_from_args(&args(raw)).unwrap();
        let view = |restart| {
            Some(ViewRequest {
                machine: MachineName::parse("loop").unwrap(),
                restart,
            })
        };
        assert_eq!(
            requested_view(&parsed(&[
                "herdr",
                "--session",
                "server",
                "server",
                "--view",
                "loop"
            ]))
            .unwrap(),
            view(false)
        );
        assert_eq!(
            requested_view(&parsed(&[
                "herdr",
                "--session=server",
                "server",
                "--view",
                "loop"
            ]))
            .unwrap(),
            view(false)
        );
        assert_eq!(
            requested_view(&parsed(&["herdr", "server", "--view", "loop", "--restart"])).unwrap(),
            view(true)
        );
        assert!(requested_view(&parsed(&["herdr", "server", "--view", "loop", "--x"])).is_err());
        assert_eq!(
            requested_view(&parsed(&["herdr", "--session", "server", "server"])).unwrap(),
            None
        );
        assert!(requested_view(&parsed(&[
            "herdr",
            "--session",
            "server",
            "server",
            "--view"
        ]))
        .is_err());
        assert_eq!(requested_view(&args(&["herdr", "client"])).unwrap(), None);
    }

    // L3.4, M2: one view per machine holds the lock; a second start gets
    // none (and so writes no log and binds no socket); the holder's record
    // names its build and pid while it holds the lock, nothing after.
    #[test]
    fn one_view_holds_the_lock_and_records_its_build() {
        let dir = std::env::temp_dir().join(format!(
            "herdr-view-lock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos())
        ));
        let path = dir.join("herdr-view-loop.lock");
        assert_eq!(running_view_at(&path), None, "no file: no view");
        let held = take_lock_at(&path).expect("lock").expect("first takes it");
        assert!(
            take_lock_at(&path).expect("lock").is_none(),
            "a racing second start gets no lock"
        );
        let running = running_view_at(&path).expect("a view runs");
        assert_eq!(running, ViewRecord::own());
        assert!(
            running.start.is_some(),
            "the record names its process start"
        );
        assert!(running.names_its_process());
        drop(held);
        assert_eq!(running_view_at(&path), None, "released: no view runs");
        assert!(take_lock_at(&path).expect("lock").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // M8 (security review): the lock is never opened through a symlink: a
    // planted link is refused and its target is left as it was; a lock in
    // a folder others can write to is refused too.
    #[test]
    fn a_symlinked_or_shared_lock_is_refused() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!(
            "herdr-view-link-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos())
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("precious");
        std::fs::write(&target, "keep me").unwrap();
        let lock = dir.join("herdr-view-loop.lock");
        std::os::unix::fs::symlink(&target, &lock).unwrap();
        assert!(take_lock_at(&lock).is_err(), "a symlinked lock is refused");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep me");
        assert_eq!(running_view_at(&lock), None);
        std::fs::remove_file(&lock).unwrap();

        let shared = dir.join("shared");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(take_lock_at(&shared.join("herdr-view-loop.lock")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // #312 follow-up: a folder only everyone may not write. A group-writable
    // one (an umask of 002) is this account's, and a symlinked folder (a
    // linked `~/.config/herdr`) is its target's, with the lock in the target.
    #[test]
    fn a_group_writable_or_linked_folder_of_this_account_holds_the_lock() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!(
            "herdr-view-group-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos())
        ));
        let group = dir.join("group");
        std::fs::create_dir_all(&group).unwrap();
        std::fs::set_permissions(&group, std::fs::Permissions::from_mode(0o775)).unwrap();
        let held = take_lock_at(&group.join("herdr-view-loop.lock"))
            .expect("a group-writable folder of this account")
            .expect("held");
        assert!(running_view_at(&group.join("herdr-view-loop.lock")).is_some());
        drop(held);

        let real = dir.join("real");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o700)).unwrap();
        let linked = dir.join("linked");
        std::os::unix::fs::symlink(&real, &linked).unwrap();
        let held = take_lock_at(&linked.join("herdr-view-loop.lock"))
            .expect("a linked folder")
            .expect("held");
        assert!(real.join("herdr-view-loop.lock").is_file(), "in the target");
        assert!(running_view_at(&linked.join("herdr-view-loop.lock")).is_some());
        drop(held);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // #312 follow-up: a FIFO planted as the lock is refused at once, never
    // waited on (the probe opens without blocking).
    #[test]
    fn a_fifo_lock_is_refused_without_waiting() {
        let dir = std::env::temp_dir().join(format!(
            "herdr-view-fifo-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos())
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let lock = dir.join("herdr-view-loop.lock");
        let raw = std::ffi::CString::new(lock.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(raw.as_ptr(), 0o600) }, 0, "mkfifo");
        let (done, finished) = std::sync::mpsc::channel();
        let probe = lock.clone();
        std::thread::spawn(move || {
            let _ = done.send((running_view_at(&probe), take_lock_at(&probe).is_err()));
        });
        let (running, refused) = finished
            .recv_timeout(std::time::Duration::from_secs(3))
            .expect("the probe waited on a FIFO");
        assert_eq!(running, None);
        assert!(refused, "a FIFO is no lock");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // M8: a record whose pid now runs another process (its start differs)
    // does not name that process, so a restart signals nothing.
    #[test]
    fn a_record_names_only_the_process_that_wrote_it() {
        let own = ViewRecord::own();
        assert!(own.names_its_process());
        let reused = ViewRecord {
            start: own.start.map(|token| token.wrapping_add(1)),
            ..own.clone()
        };
        assert!(!reused.names_its_process());
        // #312 follow-up: no start token names no process, alive or not.
        let unnamed = ViewRecord {
            start: None,
            ..own.clone()
        };
        assert!(!unnamed.names_its_process());
        let gone = ViewRecord {
            pid: 4_000_000,
            ..own
        };
        assert!(!gone.names_its_process());
    }

    #[test]
    fn each_machine_has_its_own_socket_and_log() {
        let a = MachineName::parse("ac-mbm5").unwrap();
        let b = MachineName::parse("m4m").unwrap();
        assert_ne!(socket_path(&a), socket_path(&b));
        assert!(socket_path(&a).ends_with("herdr-view-ac-mbm5.sock"));
        assert_eq!(log_file_name(&b), "herdr-view-m4m.log");
        assert_ne!(
            socket_path(&a),
            crate::server::socket_paths::client_socket_path()
        );
    }
}
