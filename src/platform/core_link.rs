//! The process calls of `herdr core-link` (openspec/changes/core-phase5, D2):
//! the passwd home, the dial lock, the watcher's fork and the exec of ssh.
//! Unix only; the link's own logic lives in `crate::core_machine`.

use std::ffi::{CStr, OsString};
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::time::Duration;

/// The account this process runs as, from the passwd database: its home and
/// login name. Never `HOME` or `USER` from the environment, which are those
/// of whoever started the core that runs the link (F6).
pub(crate) struct PasswdAccount {
    pub home: PathBuf,
    pub name: String,
}

pub(crate) fn passwd_account() -> io::Result<PasswdAccount> {
    let uid = unsafe { libc::getuid() };
    let mut buf = vec![0_u8; 4096];
    loop {
        let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
        let mut found: *mut libc::passwd = std::ptr::null_mut();
        // SAFETY: entry, buf and found outlive the call; buf's length is passed.
        let code = unsafe {
            libc::getpwuid_r(
                uid,
                &mut entry,
                buf.as_mut_ptr().cast(),
                buf.len(),
                &mut found,
            )
        };
        if code == libc::ERANGE && buf.len() < 1 << 20 {
            buf.resize(buf.len() * 2, 0);
            continue;
        }
        if code != 0 {
            return Err(io::Error::from_raw_os_error(code));
        }
        if found.is_null() || entry.pw_dir.is_null() || entry.pw_name.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("uid {uid} has no passwd entry"),
            ));
        }
        // SAFETY: getpwuid_r succeeded, so both point into buf as C strings.
        let (home, name) = unsafe { (CStr::from_ptr(entry.pw_dir), CStr::from_ptr(entry.pw_name)) };
        let home = PathBuf::from(OsString::from_vec(home.to_bytes().to_vec()));
        if !home.is_absolute() {
            return Err(io::Error::other("the passwd home is not an absolute path"));
        }
        return Ok(PasswdAccount {
            home,
            name: name.to_string_lossy().into_owned(),
        });
    }
}

/// Takes an exclusive `flock` on `file`, trying for at most `wait`. False
/// when another process holds it all that time.
pub(crate) fn lock_exclusive_within(file: &File, wait: Duration) -> io::Result<bool> {
    let deadline = std::time::Instant::now() + wait;
    loop {
        // SAFETY: the descriptor is owned by `file`, which outlives the call.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(true);
        }
        let err = io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::EWOULDBLOCK) {
            return Err(err);
        }
        if std::time::Instant::now() >= deadline {
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Lets `fd` survive an exec (clears `FD_CLOEXEC`), so the dial lock lasts
/// as long as the ssh that inherits it.
pub(crate) fn keep_across_exec(fd: RawFd) -> io::Result<()> {
    // SAFETY: fcntl on a descriptor this process owns.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(crate) enum Forked {
    /// In the new process.
    Child,
    /// In the caller, with the child's pid.
    Parent,
}

/// `fork(2)`. Call it only while the process has a single thread.
pub(crate) fn fork() -> io::Result<Forked> {
    // SAFETY: core-link forks before it starts any thread.
    match unsafe { libc::fork() } {
        -1 => Err(io::Error::last_os_error()),
        0 => Ok(Forked::Child),
        _ => Ok(Forked::Parent),
    }
}

/// Detaches the watcher from core's streams and session: stdin, stdout and
/// stderr become `/dev/null`, a new session starts, and every other
/// descriptor except `keep` is closed, so the watcher holds nothing of
/// core's and no copy of the dial lock.
pub(crate) fn detach_watcher(keep: &[RawFd]) -> io::Result<()> {
    let null = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/null")?;
    for target in 0..3 {
        // SAFETY: dup2 onto the standard descriptors of this process.
        if unsafe { libc::dup2(null.as_raw_fd(), target) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    let null_fd = null.as_raw_fd();
    // SAFETY: setsid in the forked child, which is not a group leader.
    unsafe { libc::setsid() };
    let limit = match unsafe { libc::sysconf(libc::_SC_OPEN_MAX) } {
        n if n > 0 => n.min(65_536) as RawFd,
        _ => 1024,
    };
    for fd in 3..limit {
        if fd != null_fd && !keep.contains(&fd) {
            // SAFETY: closing a descriptor number this process may hold;
            // EBADF for one it does not is harmless.
            unsafe { libc::close(fd) };
        }
    }
    Ok(())
}

/// Puts `fd` on this process's stderr (the new descriptor survives exec).
pub(crate) fn stderr_to(fd: OwnedFd) -> io::Result<()> {
    // SAFETY: dup2 of an owned descriptor onto stderr.
    if unsafe { libc::dup2(fd.as_raw_fd(), 2) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Replaces this process with `program`, `args` and exactly `env`; returns
/// only on failure. Inherited descriptors stay: core's stdin and stdout, the
/// watcher's pipe on stderr, the dial lock.
pub(crate) fn exec_with_env(
    program: &std::path::Path,
    arg0: &str,
    args: &[OsString],
    env: &[(String, OsString)],
) -> io::Error {
    let mut command = std::process::Command::new(program);
    command
        .arg0(arg0)
        .args(args)
        .env_clear()
        .envs(env.iter().map(|(key, value)| (key, value)));
    command.exec()
}

/// This process's parent pid.
pub(crate) fn parent_pid() -> u32 {
    // SAFETY: getppid cannot fail.
    unsafe { libc::getppid() as u32 }
}

pub(crate) enum Signal {
    Term,
    Kill,
}

/// Sends `signal` to `pid`.
pub(crate) fn signal(pid: u32, signal: Signal) {
    let number = match signal {
        Signal::Term => libc::SIGTERM,
        Signal::Kill => libc::SIGKILL,
    };
    // SAFETY: kill(2) with a pid the watcher checked is its parent.
    unsafe { libc::kill(pid as libc::pid_t, number) };
}

/// Reads what is ready on `fd` within `timeout`: `None` on timeout,
/// `Some(0)` at end of file.
pub(crate) fn read_within(
    fd: RawFd,
    buf: &mut [u8],
    timeout: Duration,
) -> io::Result<Option<usize>> {
    let mut poll = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let millis = timeout.as_millis().min(i32::MAX as u128) as libc::c_int;
    // SAFETY: one pollfd that outlives the call.
    let ready = unsafe { libc::poll(&mut poll, 1, millis) };
    if ready < 0 {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::Interrupted {
            return Ok(None);
        }
        return Err(err);
    }
    if ready == 0 {
        return Ok(None);
    }
    // SAFETY: buf is valid for buf.len() bytes.
    let read = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
    if read < 0 {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::Interrupted {
            return Ok(None);
        }
        return Err(err);
    }
    Ok(Some(read as usize))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_passwd_home_is_absolute_and_named() {
        let account = passwd_account().expect("passwd entry");
        assert!(account.home.is_absolute());
        assert!(!account.name.is_empty());
    }

    #[test]
    fn a_held_lock_refuses_a_second_holder_in_time() {
        let dir =
            std::path::Path::new("/tmp").join(format!("herdr-cl-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let path = dir.join("dial.lock");
        let first = File::create(&path).expect("create");
        assert!(lock_exclusive_within(&first, Duration::ZERO).expect("lock"));
        let second = File::open(&path).expect("open");
        let started = std::time::Instant::now();
        assert!(!lock_exclusive_within(&second, Duration::from_millis(200)).expect("try"));
        assert!(started.elapsed() >= Duration::from_millis(200));
        drop(first);
        assert!(lock_exclusive_within(&second, Duration::from_millis(200)).expect("relock"));
        drop(second);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
