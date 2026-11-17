//! Spawns a server daemon that is responsible for itself.
//!
//! macOS attributes privacy (TCC) requests to a process's responsible process,
//! which a spawned child inherits from its parent. A live handoff's replacement
//! server, and every pane it starts, would stay attributed to the server it
//! replaced, even after that server and its binary are gone, so an answer to a
//! privacy prompt never sticks (fork issue 240). The private libquarantine call
//! `responsibility_spawnattrs_setdisclaim`, also used by Chromium and LLDB,
//! makes the child its own responsible process.
//!
//! A pane keeper (`pane_job`) starts its pane's program the same way, so the
//! program, not the keeper's herdr binary, owns its privacy answers.

use std::collections::BTreeMap;
use std::ffi::{c_void, CString, OsStr, OsString};
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::process::Command;

use libc::{c_char, c_int, c_short};

type SetDisclaim = unsafe extern "C" fn(*mut libc::posix_spawnattr_t, c_int) -> c_int;

/// From <sys/spawn.h>; the libc crate does not define it.
const POSIX_SPAWN_SETSID: c_int = 0x0400;

/// Starts `command` (program, arguments, environment changes) in a new
/// session with its standard streams on /dev/null and with responsibility
/// disclaimed, and returns its pid. Fails, so the caller can spawn normally,
/// when the private call is missing or the command sets a working directory.
pub(crate) fn spawn_disclaimed_daemon(command: &Command) -> io::Result<u32> {
    let set_disclaim = set_disclaim()?;
    if command.get_current_dir().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "a disclaimed spawn does not carry a working directory",
        ));
    }
    let program = c_string(command.get_program().as_bytes())?;
    let mut args = vec![program.clone()];
    for arg in command.get_args() {
        args.push(c_string(arg.as_bytes())?);
    }
    let env = environment(command)?;
    let argv = null_terminated(&args);
    let envp = null_terminated(&env);

    let mut attrs = SpawnAttrs::new()?;
    let mut actions = FileActions::new()?;
    // Match std::process::Command: an empty signal mask and the default
    // SIGPIPE action in the child.
    let mut empty: libc::sigset_t = 0;
    let mut sigpipe: libc::sigset_t = 0;
    let flags = POSIX_SPAWN_SETSID | libc::POSIX_SPAWN_SETSIGMASK | libc::POSIX_SPAWN_SETSIGDEF;
    // SAFETY: the attribute and action objects are initialized and live until
    // the end of this function; the C strings outlive the posix_spawn call.
    unsafe {
        libc::sigemptyset(&mut empty);
        libc::sigemptyset(&mut sigpipe);
        libc::sigaddset(&mut sigpipe, libc::SIGPIPE);
        check(
            "posix_spawnattr_setsigmask",
            libc::posix_spawnattr_setsigmask(&mut attrs.0, &empty),
        )?;
        check(
            "posix_spawnattr_setsigdefault",
            libc::posix_spawnattr_setsigdefault(&mut attrs.0, &sigpipe),
        )?;
        check(
            "posix_spawnattr_setflags",
            libc::posix_spawnattr_setflags(&mut attrs.0, flags as c_short),
        )?;
        check(
            "responsibility_spawnattrs_setdisclaim",
            set_disclaim(&mut attrs.0, 1),
        )?;
        for fd in 0..=2 {
            check(
                "posix_spawn_file_actions_addopen",
                libc::posix_spawn_file_actions_addopen(
                    &mut actions.0,
                    fd,
                    c"/dev/null".as_ptr(),
                    libc::O_RDWR,
                    0,
                ),
            )?;
        }
        let mut pid: libc::pid_t = 0;
        check(
            "posix_spawn",
            libc::posix_spawn(
                &mut pid,
                program.as_ptr(),
                &actions.0,
                &attrs.0,
                argv.as_ptr(),
                envp.as_ptr(),
            ),
        )?;
        u32::try_from(pid).map_err(io::Error::other)
    }
}

/// Starts `parts` on the terminal `tty` as a session leader with that terminal
/// as fds 0, 1 and 2 (and as its controlling terminal unless `parts` says
/// otherwise), default signal dispositions, no other descriptors, and with
/// responsibility disclaimed. This is the PTY library's own child setup, done
/// by spawn attributes: macOS opens the file actions after the new session
/// exists, so opening the terminal makes it the controlling one.
pub(crate) fn spawn_disclaimed_on_tty(
    parts: &portable_pty::UnixSpawnParts,
    tty: &std::path::Path,
) -> io::Result<u32> {
    let set_disclaim = set_disclaim()?;
    let program = c_string(parts.program.as_bytes())?;
    let args = parts
        .argv
        .iter()
        .map(|arg| c_string(arg.as_bytes()))
        .collect::<io::Result<Vec<_>>>()?;
    let env = parts
        .env
        .iter()
        .map(|(key, value)| {
            let mut entry = key.as_bytes().to_vec();
            entry.push(b'=');
            entry.extend_from_slice(value.as_bytes());
            c_string(&entry)
        })
        .collect::<io::Result<Vec<_>>>()?;
    let cwd = c_string(parts.cwd.as_bytes())?;
    let tty = c_string(tty.as_os_str().as_bytes())?;
    let argv = null_terminated(&args);
    let envp = null_terminated(&env);
    let open_flags = if parts.controlling_tty {
        libc::O_RDWR
    } else {
        libc::O_RDWR | libc::O_NOCTTY
    };

    let mut attrs = SpawnAttrs::new()?;
    let mut actions = FileActions::new()?;
    let mut empty: libc::sigset_t = 0;
    let mut defaults: libc::sigset_t = 0;
    let flags = POSIX_SPAWN_SETSID
        | libc::POSIX_SPAWN_SETSIGMASK
        | libc::POSIX_SPAWN_SETSIGDEF
        | libc::POSIX_SPAWN_CLOEXEC_DEFAULT;
    // SAFETY: the attribute and action objects are initialized and live until
    // the end of this function; the C strings outlive the posix_spawn call.
    unsafe {
        libc::sigemptyset(&mut empty);
        libc::sigemptyset(&mut defaults);
        // The PTY library's list, plus SIGPIPE as std::process::Command does.
        for signal in [
            libc::SIGCHLD,
            libc::SIGHUP,
            libc::SIGINT,
            libc::SIGQUIT,
            libc::SIGTERM,
            libc::SIGALRM,
            libc::SIGPIPE,
        ] {
            libc::sigaddset(&mut defaults, signal);
        }
        check(
            "posix_spawnattr_setsigmask",
            libc::posix_spawnattr_setsigmask(&mut attrs.0, &empty),
        )?;
        check(
            "posix_spawnattr_setsigdefault",
            libc::posix_spawnattr_setsigdefault(&mut attrs.0, &defaults),
        )?;
        check(
            "posix_spawnattr_setflags",
            libc::posix_spawnattr_setflags(&mut attrs.0, flags as c_short),
        )?;
        check(
            "responsibility_spawnattrs_setdisclaim",
            set_disclaim(&mut attrs.0, 1),
        )?;
        check(
            "posix_spawn_file_actions_addchdir_np",
            libc::posix_spawn_file_actions_addchdir_np(&mut actions.0, cwd.as_ptr()),
        )?;
        check(
            "posix_spawn_file_actions_addopen",
            libc::posix_spawn_file_actions_addopen(&mut actions.0, 0, tty.as_ptr(), open_flags, 0),
        )?;
        for fd in 1..=2 {
            check(
                "posix_spawn_file_actions_adddup2",
                libc::posix_spawn_file_actions_adddup2(&mut actions.0, 0, fd),
            )?;
        }
        let mut pid: libc::pid_t = 0;
        check(
            "posix_spawn",
            libc::posix_spawn(
                &mut pid,
                program.as_ptr(),
                &actions.0,
                &attrs.0,
                argv.as_ptr(),
                envp.as_ptr(),
            ),
        )?;
        u32::try_from(pid).map_err(io::Error::other)
    }
}

fn set_disclaim() -> io::Result<SetDisclaim> {
    let name = c"responsibility_spawnattrs_setdisclaim";
    // SAFETY: dlsym with a valid C string; a non-null result is that function.
    let address = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
    if address.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "responsibility_spawnattrs_setdisclaim is unavailable",
        ));
    }
    Ok(unsafe { std::mem::transmute::<*mut c_void, SetDisclaim>(address) })
}

/// The caller's environment with the command's changes applied.
fn environment(command: &Command) -> io::Result<Vec<CString>> {
    let mut vars: BTreeMap<OsString, OsString> = std::env::vars_os().collect();
    for (key, value) in command.get_envs() {
        match value {
            Some(value) => vars.insert(key.to_owned(), value.to_owned()),
            None => vars.remove(key),
        };
    }
    vars.into_iter()
        .map(|(key, value)| {
            let mut entry = key.into_vec();
            entry.push(b'=');
            entry.extend_from_slice(value.as_bytes());
            c_string(&entry)
        })
        .collect()
}

fn c_string(bytes: &[u8]) -> io::Result<CString> {
    CString::new(bytes).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{:?} contains a NUL byte", OsStr::from_bytes(bytes)),
        )
    })
}

fn null_terminated(strings: &[CString]) -> Vec<*mut c_char> {
    strings
        .iter()
        .map(|string| string.as_ptr().cast_mut())
        .chain(std::iter::once(std::ptr::null_mut()))
        .collect()
}

fn check(call: &str, code: c_int) -> io::Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(io::Error::new(
            io::Error::from_raw_os_error(code).kind(),
            format!("{call} failed: {}", io::Error::from_raw_os_error(code)),
        ))
    }
}

struct SpawnAttrs(libc::posix_spawnattr_t);

impl SpawnAttrs {
    fn new() -> io::Result<Self> {
        let mut attrs = std::ptr::null_mut();
        // SAFETY: initializes the opaque attribute pointer.
        check("posix_spawnattr_init", unsafe {
            libc::posix_spawnattr_init(&mut attrs)
        })?;
        Ok(Self(attrs))
    }
}

impl Drop for SpawnAttrs {
    fn drop(&mut self) {
        // SAFETY: initialized in `new`.
        unsafe { libc::posix_spawnattr_destroy(&mut self.0) };
    }
}

struct FileActions(libc::posix_spawn_file_actions_t);

impl FileActions {
    fn new() -> io::Result<Self> {
        let mut actions = std::ptr::null_mut();
        // SAFETY: initializes the opaque file-actions pointer.
        check("posix_spawn_file_actions_init", unsafe {
            libc::posix_spawn_file_actions_init(&mut actions)
        })?;
        Ok(Self(actions))
    }
}

impl Drop for FileActions {
    fn drop(&mut self) {
        // SAFETY: initialized in `new`.
        unsafe { libc::posix_spawn_file_actions_destroy(&mut self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    type ResponsibleFor = unsafe extern "C" fn(libc::pid_t) -> libc::pid_t;

    fn responsible_for(pid: libc::pid_t) -> libc::pid_t {
        let name = c"responsibility_get_pid_responsible_for_pid";
        let address = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
        assert!(!address.is_null(), "responsibility lookup unavailable");
        let lookup = unsafe { std::mem::transmute::<*mut c_void, ResponsibleFor>(address) };
        unsafe { lookup(pid) }
    }

    #[test]
    fn spawned_daemon_owns_its_children_and_session_and_keeps_env_changes() {
        let out = std::env::temp_dir().join(format!("herdr-disclaim-{}", std::process::id()));
        let _ = std::fs::remove_file(&out);
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg(r#"sleep 30 & echo "$! $HERDR_DISCLAIM_SET ${HOME-unset}" > "$0.tmp" && mv "$0.tmp" "$0"; wait"#)
            .arg(&out)
            .env("HERDR_DISCLAIM_SET", "yes")
            .env_remove("HOME");
        let pid = spawn_disclaimed_daemon(&command).expect("spawn") as libc::pid_t;

        let deadline = Instant::now() + Duration::from_secs(10);
        let line = loop {
            if let Ok(line) = std::fs::read_to_string(&out) {
                break line;
            }
            assert!(Instant::now() < deadline, "daemon wrote nothing");
            std::thread::sleep(Duration::from_millis(20));
        };
        let _ = std::fs::remove_file(&out);
        let fields: Vec<&str> = line.split_whitespace().collect();
        let grandchild: libc::pid_t = fields[0].parse().expect("grandchild pid");

        let responsible = responsible_for(grandchild);
        let session = unsafe { libc::getsid(pid) };
        unsafe {
            libc::kill(grandchild, libc::SIGKILL);
            libc::kill(pid, libc::SIGKILL);
            libc::waitpid(pid, std::ptr::null_mut(), 0);
        }
        assert_eq!(fields[1..], ["yes", "unset"]);
        assert_eq!(session, pid, "daemon leads its own session");
        assert_eq!(responsible, pid, "daemon is responsible for its children");
    }

    #[test]
    fn a_program_on_a_tty_leads_its_session_with_that_tty_and_owns_its_privacy() {
        use std::io::Read;

        let pair = portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize::default())
            .expect("pty");
        let tty = pair.master.tty_name().expect("tty name");
        let mut reader = pair.master.try_clone_reader().expect("reader");
        drop(pair.slave);
        let dir = std::env::temp_dir();
        let mut command = portable_pty::CommandBuilder::new("sh");
        command.args([
            "-c",
            r#"sleep 30 & printf 'ready %s %s %s %s\n' "$$" "$(ps -o tty= -p $$)" "$(pwd -P)" "$!"; read -r line; kill $!; echo "got $line""#,
        ]);
        command.cwd(&dir);
        let parts = command.unix_spawn_parts().expect("parts");
        let pid = spawn_disclaimed_on_tty(&parts, &tty).expect("spawn") as libc::pid_t;

        let mut output = String::new();
        let mut buf = [0u8; 512];
        let deadline = Instant::now() + Duration::from_secs(10);
        while !output.contains('\n') {
            assert!(Instant::now() < deadline, "no output: {output:?}");
            let read = reader.read(&mut buf).expect("read");
            output.push_str(&String::from_utf8_lossy(&buf[..read]));
        }
        let line = output
            .lines()
            .find(|line| line.contains("ready"))
            .expect("ready line");
        let fields: Vec<String> = line.split_whitespace().skip(1).map(str::to_owned).collect();
        let grandchild: libc::pid_t = fields[3].parse().expect("grandchild pid");
        // A process this one starts is attributed to it only when it is its
        // own responsible process; this test process may be one too.
        let responsible = responsible_for(grandchild);
        let session = unsafe { libc::getsid(pid) };
        let mut writer = pair.master.take_writer().expect("writer");
        std::io::Write::write_all(&mut writer, b"typed\r").expect("type");
        while !output.contains("got typed") {
            assert!(
                Instant::now() < deadline,
                "input did not arrive: {output:?}"
            );
            let read = reader.read(&mut buf).expect("read");
            output.push_str(&String::from_utf8_lossy(&buf[..read]));
        }
        let mut status = 0;
        unsafe { libc::waitpid(pid, &mut status, 0) };

        let tty_name = tty.file_name().expect("tty file").to_string_lossy();
        let cwd = std::fs::canonicalize(&dir).expect("canonical cwd");
        assert_eq!(fields[0], pid.to_string());
        assert_eq!(fields[1], tty_name, "controlling tty: {fields:?}");
        assert_eq!(session, pid, "leads its own session");
        assert_eq!(fields[2], cwd.to_string_lossy(), "working directory");
        assert_eq!(responsible, pid, "responsible for itself and its children");
        assert_eq!(status, 0);
    }

    #[test]
    fn a_working_directory_falls_back_to_a_normal_spawn() {
        let mut command = Command::new("/usr/bin/true");
        command.current_dir("/");
        let err = spawn_disclaimed_daemon(&command).expect_err("unsupported");
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
    }
}
