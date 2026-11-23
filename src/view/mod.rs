//! The view server (openspec/changes/core-phase5, D6): the herdr binary in a
//! second role, one process per viewed core machine, that shows and drives
//! another Mac's herdr-max panes through the primary core's machine link.
//!
//! Its layout is the far side's `herdr-app` document, read through the
//! primary core (`doc.get {on}`); its panes are the far terminals, attached
//! so that the view never becomes their active viewer (D6 Size). It saves
//! nothing, publishes nothing, serves no JSON API and starts no program; a
//! TUI client connects to its own client socket as to any herdr server.
//! Unix only, as the core backend is.

pub(crate) mod doc;
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
    use std::os::unix::fs::OpenOptionsExt as _;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)?;
    if !crate::platform::core_link::lock_exclusive_within(&file, std::time::Duration::ZERO)? {
        return Ok(None);
    }
    let record = ViewRecord {
        version: crate::build_info::version(),
        pid: std::process::id(),
    };
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

/// The record of the view whose lock is at `path`, while one holds it.
pub(crate) fn running_view_at(path: &std::path::Path) -> Option<ViewRecord> {
    let file = std::fs::File::open(path).ok()?;
    if crate::platform::core_link::lock_exclusive_within(&file, std::time::Duration::ZERO)
        .unwrap_or(false)
    {
        // Nobody holds it: no view runs (the lock goes with the file).
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
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
        assert_eq!(
            running_view_at(&path),
            Some(ViewRecord {
                version: crate::build_info::version(),
                pid: std::process::id(),
            })
        );
        drop(held);
        assert_eq!(running_view_at(&path), None, "released: no view runs");
        assert!(take_lock_at(&path).expect("lock").is_some());
        let _ = std::fs::remove_dir_all(&dir);
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
