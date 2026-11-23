//! One herdr server per session (fork issue 319,
//! https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/319).
//!
//! A server holds an exclusive `flock` on `herdr-server.lock` in its session's
//! data directory for its whole life; the kernel lets it go when the process
//! dies, however it dies. A plain server that cannot take it exits before it
//! binds or unlinks a socket, and a client that finds it held waits for that
//! server instead of starting another one.
//!
//! A live handoff passes the lock, it never lets go of it: the old server
//! sends its lock descriptor to the importer with the pane descriptors. Both
//! descriptors are the same open file, so the lock is held as long as either
//! process lives: a rollback kills the importer and the old server keeps it,
//! a commit ends the old server and the importer keeps it.

use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::fs::{FileExt as _, MetadataExt as _};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use tracing::warn;

const LOCK_FILE: &str = "herdr-server.lock";

/// How long a starting server waits for the lock: a server stopped for a
/// restart closes its sockets a moment before its process ends.
pub(crate) const START_WAIT: Duration = Duration::from_secs(5);

/// The most of the owner record read (a pid).
const RECORD_MAX: usize = 64;

/// The session lock's path for the active session.
pub(crate) fn lock_path() -> PathBuf {
    crate::session::data_dir().join(LOCK_FILE)
}

/// The lock a server holds while it runs.
pub(crate) struct ServerLock {
    file: File,
}

impl ServerLock {
    /// Writes this process's pid into the lock file, for the clients and
    /// servers that find the lock held.
    fn record_owner(&self) -> io::Result<()> {
        self.file.set_len(0)?;
        self.file
            .write_all_at(std::process::id().to_string().as_bytes(), 0)
    }
}

static HELD: OnceLock<ServerLock> = OnceLock::new();

/// What a server that tried to take the lock found.
pub(crate) enum Start {
    /// It holds the lock now.
    Held,
    /// Another server holds it.
    Busy(Holder),
    /// The lock cannot be taken here (a filesystem without `flock`, a lock
    /// file that is not this account's); the server runs without one, as
    /// before the lock existed.
    Unavailable(io::Error),
}

/// The server that holds a session's lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Holder {
    pub(crate) pid: Option<u32>,
}

impl std::fmt::Display for Holder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.pid {
            Some(pid) => write!(f, "a herdr server (pid {pid})"),
            None => f.write_str("a herdr server"),
        }
    }
}

/// Takes the lock at `path`, trying for at most `wait`; `Ok(None)` when
/// another process holds it all that time.
pub(crate) fn take_at(path: &Path, wait: Duration) -> io::Result<Option<ServerLock>> {
    lock_and_record(crate::view::open_lock(path, true)?, path, wait)
}

/// Locks `file` and records this process in it. The record is a diagnostic:
/// a lock whose pid cannot be written (a full disk) is kept all the same, so
/// it still keeps every other server out.
fn lock_and_record(file: File, path: &Path, wait: Duration) -> io::Result<Option<ServerLock>> {
    if !crate::platform::core_link::lock_exclusive_within(&file, wait)? {
        return Ok(None);
    }
    let lock = ServerLock { file };
    if let Err(err) = lock.record_owner() {
        warn!(path = %path.display(), %err, "holding the session's server lock without recording this server in it");
    }
    Ok(Some(lock))
}

/// The server holding the lock at `path`, if one does. A lock that cannot
/// be probed counts as free, so clients behave as they did before it.
pub(crate) fn holder_at(path: &Path) -> Option<Holder> {
    let file = match crate::view::open_lock(path, false) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return None,
        Err(err) => {
            warn!(path = %path.display(), %err, "cannot read the session's server lock");
            return None;
        }
    };
    match crate::platform::core_link::lock_exclusive_within(&file, Duration::ZERO) {
        // Nobody holds it; dropping the file lets it go again.
        Ok(true) => None,
        Ok(false) => Some(Holder {
            pid: read_owner(&file),
        }),
        Err(err) => {
            warn!(path = %path.display(), %err, "cannot probe the session's server lock");
            None
        }
    }
}

fn read_owner(file: &File) -> Option<u32> {
    let mut buf = [0u8; RECORD_MAX];
    let read = file.read_at(&mut buf, 0).ok()?;
    std::str::from_utf8(&buf[..read]).ok()?.trim().parse().ok()
}

/// Takes the active session's lock for a plain server start and keeps it
/// for the process's life.
pub(crate) fn hold_for_start() -> Start {
    let path = lock_path();
    match take_at(&path, START_WAIT) {
        Ok(Some(lock)) => keep(lock),
        Ok(None) => Start::Busy(holder_at(&path).unwrap_or(Holder { pid: None })),
        Err(err) => Start::Unavailable(err),
    }
}

/// Takes the lock for a handoff importer whose old server passed none: an
/// older build, which holds no lock. Another server holding it fails the
/// import before it commits, so the old server rolls back.
pub(crate) fn hold_for_import() -> io::Result<()> {
    let path = lock_path();
    match take_at(&path, Duration::ZERO) {
        Ok(Some(lock)) => match keep(lock) {
            Start::Held => Ok(()),
            _ => Err(io::Error::other("the server lock was already set")),
        },
        Ok(None) => Err(io::Error::other(format!(
            "{} already runs on this session",
            holder_at(&path).unwrap_or(Holder { pid: None })
        ))),
        Err(err) => {
            warn!(path = %path.display(), %err, "running without the session's server lock");
            Ok(())
        }
    }
}

/// Keeps the lock the old server passed in a live handoff. It must be this
/// session's lock file, already locked (the same open file the old server
/// holds), or the import fails before it commits.
pub(crate) fn adopt_passed(fd: OwnedFd) -> io::Result<()> {
    set_cloexec(fd.as_raw_fd())?;
    let file = File::from(fd);
    let passed = file.metadata()?;
    let ours = std::fs::metadata(lock_path())?;
    if passed.dev() != ours.dev() || passed.ino() != ours.ino() {
        return Err(io::Error::other(
            "the lock the old server passed is not this session's server lock",
        ));
    }
    // A lock already held through this open file is taken again at once.
    if !crate::platform::core_link::lock_exclusive_within(&file, Duration::ZERO)? {
        return Err(io::Error::other(
            "the lock the old server passed is not held through the file it sent",
        ));
    }
    HELD.set(ServerLock { file })
        .map_err(|_| io::Error::other("the server lock was already set"))
}

/// Records this process as the lock's owner, once a handoff committed to it.
pub(crate) fn record_owner() {
    if let Some(lock) = HELD.get() {
        if let Err(err) = lock.record_owner() {
            warn!(%err, "could not record this server in the session's server lock");
        }
    }
}

/// The descriptor of the lock this server holds, to pass in a live handoff.
pub(crate) fn held_fd() -> Option<RawFd> {
    HELD.get().map(|lock| lock.file.as_raw_fd())
}

/// Marks `fd` close-on-exec, so no pane program inherits the lock: one that
/// outlived the server would keep every later server out.
pub(crate) fn set_cloexec(fd: RawFd) -> io::Result<()> {
    // SAFETY: fcntl on a descriptor this process owns.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: as above.
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn keep(lock: ServerLock) -> Start {
    match HELD.set(lock) {
        Ok(()) => Start::Held,
        Err(_) => Start::Unavailable(io::Error::other("the server lock was already set")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "herdr-server-lock-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_second_server_cannot_take_the_lock_while_the_first_holds_it() {
        let dir = lock_dir("second");
        let path = dir.join(LOCK_FILE);
        let first = take_at(&path, Duration::ZERO)
            .unwrap()
            .expect("first server");
        assert!(
            take_at(&path, Duration::from_millis(100))
                .unwrap()
                .is_none(),
            "a second server must not take a held lock"
        );
        assert_eq!(
            holder_at(&path),
            Some(Holder {
                pid: Some(std::process::id())
            }),
            "a client sees the first server holds it, and its pid"
        );
        drop(first);
        assert_eq!(holder_at(&path), None, "a lock goes with its holder");
        assert!(take_at(&path, Duration::ZERO).unwrap().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A pid that cannot be written leaves the lock held, not dropped.
    #[test]
    fn a_lock_whose_pid_cannot_be_written_is_kept() {
        let dir = lock_dir("unrecorded");
        let path = dir.join(LOCK_FILE);
        std::fs::write(&path, b"").unwrap();
        // Read-only, so the pid write fails after the flock succeeded.
        let read_only = File::open(&path).unwrap();
        let held = lock_and_record(read_only, &path, Duration::ZERO)
            .unwrap()
            .expect("the lock is kept");
        assert!(
            take_at(&path, Duration::from_millis(100))
                .unwrap()
                .is_none(),
            "a second server is still kept out"
        );
        drop(held);
        assert!(take_at(&path, Duration::ZERO).unwrap().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_lock_file_has_no_holder() {
        let dir = lock_dir("missing");
        assert_eq!(holder_at(&dir.join(LOCK_FILE)), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
