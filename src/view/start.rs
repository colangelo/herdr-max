//! Starting a view server on demand (core-phase5 D6 Lifetime): the endpoint
//! transport and `herdr --remote core:<machine>` reach a view through its
//! client socket and start one when none listens there. It runs in the
//! background like the primary server, exits 10 min after its last client,
//! and dials nothing by starting, nor on a client's attach: a person's input
//! in it does (D3, review M3). A client of a newer build restarts it (D6
//! Lifetime: it keeps no state).

use std::io;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::core_machine::MachineName;

/// How long a starting view gets to bind its socket.
const START_TIMEOUT: Duration = Duration::from_secs(15);

/// `herdr --remote core:<machine>` (D4, D8): a TUI client on the view of
/// `machine`, started first when none runs. The tab ac uses today for the
/// old remote, through the core machine link instead of herdr's own ssh.
pub(crate) fn run_view_client(
    machine: &MachineName,
    keybindings: crate::remote::RemoteKeybindings,
) -> io::Result<()> {
    let socket = ensure_running(machine)?;
    let session = crate::session::active_name()
        .unwrap_or_else(|| crate::session::DEFAULT_SESSION_NAME.to_string());
    let program = std::env::args().next().unwrap_or_else(|| "herdr".into());
    let target = format!("core:{machine}");
    let mut reattach = format!("{program} --remote {target}");
    if crate::session::active_name().is_some() {
        reattach.push_str(&format!(" --session {session}"));
    }
    let preferences = serde_json::to_string(&(&target, &session)).map_err(io::Error::other)?;
    let status = Command::new(std::env::current_exe()?)
        .arg("client")
        .env(
            crate::server::socket_paths::CLIENT_SOCKET_PATH_ENV_VAR,
            &socket,
        )
        .env(crate::remote::REMOTE_PREFERENCES_ENV_VAR, preferences)
        .env(crate::remote::REATTACH_COMMAND_ENV_VAR, reattach)
        .env(
            crate::remote::REMOTE_KEYBINDINGS_ENV_VAR,
            keybindings.as_str(),
        )
        .env_remove(crate::api::SOCKET_PATH_ENV_VAR)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            format!("the view client exited with {status}"),
        ))
    }
}

/// The machine of a `--remote` target `core:<machine>`; `None` for any
/// other target, which the old remote takes (D8).
pub(crate) fn core_target(target: &str) -> Option<Result<MachineName, String>> {
    let name = target.strip_prefix("core:")?;
    Some(MachineName::parse(name).map_err(|err| err.to_string()))
}

/// Whether a view server for `machine` answers on its socket now.
pub(crate) fn listening(machine: &MachineName) -> bool {
    let socket = super::socket_path(machine);
    socket.exists() && crate::ipc::connect_local_stream(&socket).is_ok()
}

/// The view's socket, after starting a view server for `machine` when none
/// listens there. Any machine the primary core lists can be viewed (one
/// added with `hc machine add` too); a name core does not list shows the
/// view's notice saying so. Starting a view dials nothing (D3).
pub(crate) fn ensure_running(machine: &MachineName) -> io::Result<PathBuf> {
    let socket = super::socket_path(machine);
    if listening(machine) {
        let lock = super::lock_path(machine);
        let Some(running) = stale_view(&lock, &crate::build_info::version()) else {
            return Ok(socket);
        };
        // D6 Lifetime: the view keeps no state, so a client of a newer
        // build restarts it instead of a handoff.
        tracing::info!(%machine, old = %running.version, pid = running.pid, "restarting the view server on this build");
        stop_view(&lock, &running)?;
    }
    let exe = std::env::current_exe()?;
    let mut command = Command::new(exe);
    if let Some(session) = crate::session::active_name() {
        command.arg("--session").arg(session);
    }
    command
        .arg("server")
        .arg("--view")
        .arg(machine.as_str())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // A pane's own server socket must not leak into the view: it binds
        // its own.
        .env_remove(crate::api::SOCKET_PATH_ENV_VAR)
        .env_remove(crate::server::socket_paths::CLIENT_SOCKET_PATH_ENV_VAR);
    crate::platform::detach_server_daemon_command(&mut command);
    command.env_remove(crate::platform::daemon_reaper::SPAWNED_DAEMONS_ENV_VAR);
    let pid = crate::platform::launch_server_daemon_command(&mut command)?;
    tracing::info!(%machine, pid, "started a view server");
    crate::server::autodetect::wait_for_server_socket(&socket, START_TIMEOUT)?;
    Ok(socket)
}

/// The running view whose lock is at `lock`, when a client of build `mine`
/// restarts it (`replaces`); `None` to use it as it is.
fn stale_view(lock: &std::path::Path, mine: &str) -> Option<super::ViewRecord> {
    super::running_view_at(lock).filter(|running| replaces(mine, &running.version))
}

/// Whether a client of build `mine` restarts a view running build
/// `running` (D6 Lifetime, review M2): when they differ and `mine` is not
/// the older one, so two clients of two builds do not restart one view in
/// turn. Versions are `<major>.<minor>.<patch>[-<channel>.<build>]`; a build
/// number counts within one base version, and when the two cannot be
/// ordered the client's build wins.
pub(crate) fn replaces(mine: &str, running: &str) -> bool {
    if mine == running {
        return false;
    }
    match (version_key(mine), version_key(running)) {
        (Some(mine), Some(running)) => mine >= running,
        _ => true,
    }
}

/// A version's order: its base, then the leading number of its build id
/// (`161` of `0.9.3-ac-beta.161-rabiot`), 0 without one.
fn version_key(version: &str) -> Option<((u64, u64, u64), u64)> {
    let (base, rest) = version.split_once('-').unwrap_or((version, ""));
    let mut parts = base.split('.').map(|part| part.parse::<u64>().ok());
    let base = (parts.next()??, parts.next()??, parts.next()??);
    if parts.next().is_some() {
        return None;
    }
    let build = rest
        .rsplit_once('.')
        .map(|(_, build)| build)
        .unwrap_or("")
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|digits| digits.parse().ok())
        .unwrap_or(0);
    Some((base, build))
}

/// `herdr server --view <machine> --restart` (PR D's second read, item 4):
/// ends the view of `machine` that runs, of any build, before this process
/// takes its place. Nothing to do when none runs.
pub(crate) fn stop_running(machine: &MachineName) -> io::Result<()> {
    let lock = super::lock_path(machine);
    match super::running_view_at(&lock) {
        Some(running) => stop_view(&lock, &running),
        None => Ok(()),
    }
}

/// Ends the view whose lock is at `lock`: SIGTERM, then SIGKILL after 5 s;
/// it is gone when the lock is free. Each signal goes only while the record
/// still names the process that wrote it (review M8: its start token), so
/// a pid the system gave again is never signalled. A view's end detaches
/// its panes and changes nothing on the machine (D6).
fn stop_view(lock: &std::path::Path, running: &super::ViewRecord) -> io::Result<()> {
    use crate::platform::core_link::{signal, Signal};
    let still_it = || {
        super::running_view_at(lock).is_some_and(|now| now.pid == running.pid)
            && running.names_its_process()
    };
    if !still_it() {
        return Err(io::Error::other(format!(
            "the view server recorded as pid {} is not that process any more; nothing was signalled",
            running.pid
        )));
    }
    let gone_within = |wait: std::time::Duration| {
        let deadline = std::time::Instant::now() + wait;
        loop {
            if super::running_view_at(lock).is_none() {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    };
    signal(running.pid, Signal::Term);
    if gone_within(std::time::Duration::from_secs(5)) {
        return Ok(());
    }
    if still_it() {
        signal(running.pid, Signal::Kill);
    }
    if gone_within(std::time::Duration::from_secs(2)) {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "the view server {} did not exit for its restart",
            running.pid
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // M2: a client of a newer build restarts the view; one of the same or
    // an older build uses it, so two builds never restart a view in turn.
    #[test]
    fn a_newer_client_build_replaces_the_running_view() {
        assert!(!replaces("0.9.5", "0.9.5"));
        assert!(replaces("0.9.6", "0.9.5"));
        assert!(!replaces("0.9.5", "0.9.6"));
        assert!(replaces(
            "0.9.5-ac-beta.162-pippo",
            "0.9.5-ac-beta.161-rabiot"
        ));
        assert!(!replaces(
            "0.9.5-ac-beta.161-rabiot",
            "0.9.5-ac-beta.162-pippo"
        ));
        assert!(replaces("0.10.0", "0.9.9-ac-beta.900"));
        assert!(
            replaces("0.9.5-ac-beta.161-b", "0.9.5-ac-beta.161-a"),
            "same order, other build: the client's"
        );
        assert!(replaces("weird", "0.9.5"), "unordered: the client's");
    }

    // M2: a running view of an older build is the one a client restarts;
    // one of the client's build, or none running, is not.
    #[test]
    fn a_client_restarts_only_an_older_running_view() {
        let dir = std::env::temp_dir().join(format!("herdr-view-stale-{}", std::process::id()));
        let lock = dir.join("herdr-view-loop.lock");
        assert_eq!(stale_view(&lock, "0.9.5"), None, "no view runs");
        let held = super::super::take_lock_at(&lock)
            .expect("lock")
            .expect("held");
        let mine = crate::build_info::version();
        assert_eq!(stale_view(&lock, &mine), None, "the same build is used");
        std::fs::write(&lock, r#"{"version":"0.0.1","pid":4000000}"#).expect("an old record");
        assert_eq!(
            stale_view(&lock, &mine).map(|running| running.version),
            Some("0.0.1".to_string()),
            "an older view is restarted"
        );
        drop(held);
        assert_eq!(stale_view(&lock, &mine), None, "it ended");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // M8 (security review): a restart signals the view's pid only while
    // that pid is still the process that wrote the record (its start
    // token). A record whose pid now runs another process is refused, and
    // that process is never signalled.
    #[test]
    fn a_restart_never_signals_a_process_that_reused_the_views_pid() {
        let dir = std::env::temp_dir().join(format!("herdr-view-signal-{}", std::process::id()));
        let lock = dir.join("herdr-view-loop.lock");
        let held = super::super::take_lock_at(&lock)
            .expect("lock")
            .expect("held");
        let mut bystander = Command::new("/bin/sleep").arg("30").spawn().expect("sleep");
        let started = match crate::platform::process_start(bystander.id()) {
            crate::platform::ProcessStart::Started(token) => token,
            other => panic!("no start token: {other:?}"),
        };
        let reused = super::super::ViewRecord {
            version: "0.0.1".into(),
            pid: bystander.id(),
            start: Some(started.wrapping_add(1)),
        };
        std::fs::write(&lock, serde_json::to_string(&reused).unwrap()).expect("record");
        assert!(stop_view(&lock, &reused).is_err());
        std::thread::sleep(Duration::from_millis(100));
        assert!(
            bystander.try_wait().expect("wait").is_none(),
            "the bystander was signalled"
        );
        let _ = bystander.kill();
        let _ = bystander.wait();
        drop(held);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // V21's routing part (D8): only `core:<machine>` is a view; every other
    // `--remote` target is the old remote's, unchanged, and starts no view.
    #[test]
    fn only_a_core_target_is_a_view() {
        assert_eq!(
            core_target("core:ac-mbm5"),
            Some(Ok(MachineName::parse("ac-mbm5").unwrap()))
        );
        assert!(matches!(core_target("core:../x"), Some(Err(_))));
        assert!(matches!(core_target("core:"), Some(Err(_))));
        for old in [
            "ac-mbm5",
            "ac@ac-mbm5",
            "core",
            "ssh://core:22",
            "ac@core:x",
        ] {
            assert_eq!(core_target(old), None, "{old}");
        }
    }
}
