//! Reaping the server daemons this process started (fork issue 179).
//!
//! A client starts its server with `setsid()`, but the server stays the
//! client's child. Upstream clients exit on a live handoff, so launchd reaps the
//! old server; the fork's client stays attached (and re-execs onto the new
//! build, issue 165), so nothing ever `wait()`ed for it and every handoff left
//! one `<defunct>` server behind until the client exited.
//!
//! Each spawned server gets a thread that blocks in `waitpid`. An `exec` keeps
//! the process but drops its threads, so the pids still alive travel across the
//! re-exec in [`SPAWNED_DAEMONS_ENV_VAR`] and the new image reaps them again.
//! The server itself never inherits the variable.

use std::sync::Mutex;

/// Pids (comma separated) of server daemons this client started and has not yet
/// seen exit, handed to the re-exec'd client.
pub(crate) const SPAWNED_DAEMONS_ENV_VAR: &str = "HERDR_SPAWNED_SERVER_PIDS";

static LIVE: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// Block until `pid` exits and is reaped. Returns at once when it is not our
/// child (already reaped, or never ours).
fn wait_for_exit(pid: u32) {
    loop {
        let mut status = 0;
        // SAFETY: waitpid only writes the status integer we own.
        let waited = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, 0) };
        if waited >= 0 {
            return;
        }
        if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
            return;
        }
    }
}

/// Reap `pid` from a thread of its own once it exits.
pub(crate) fn reap_when_it_exits(pid: u32) {
    let Ok(mut live) = LIVE.lock() else {
        return;
    };
    if live.contains(&pid) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("herdr-daemon-reaper".into())
        .spawn(move || {
            wait_for_exit(pid);
            if let Ok(mut live) = LIVE.lock() {
                live.retain(|live_pid| *live_pid != pid);
            }
        });
    match spawned {
        Ok(_) => live.push(pid),
        Err(err) => tracing::warn!(pid, error = %err, "could not start the server reaper thread"),
    }
}

/// The daemons still running, as the value for [`SPAWNED_DAEMONS_ENV_VAR`].
pub(crate) fn live_pids_env() -> Option<String> {
    let live = LIVE.lock().ok()?;
    (!live.is_empty()).then(|| {
        live.iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",")
    })
}

/// Start reaping the daemons a previous image of this process started.
pub(crate) fn adopt_inherited(value: Option<&str>) {
    for pid in value
        .unwrap_or_default()
        .split(',')
        .filter_map(|part| part.trim().parse::<u32>().ok())
    {
        reap_when_it_exits(pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gone(pid: u32) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            // kill(pid, 0) succeeds for a zombie and fails with ESRCH once reaped.
            if unsafe { libc::kill(pid as libc::pid_t, 0) } != 0 {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn a_daemon_a_previous_image_started_is_reaped_after_adoption() {
        // The re-exec'd client never spawned this child, but it is still its child.
        let child = std::process::Command::new("sh")
            .args(["-c", "exit 0"])
            .spawn()
            .expect("spawn the stand-in daemon");
        let pid = child.id();
        std::mem::forget(child);
        adopt_inherited(Some(&format!("not-a-pid, {pid}")));
        assert!(gone(pid), "the inherited daemon {pid} was left as a zombie");
    }

    #[test]
    fn live_daemons_are_exported_for_the_reexec_and_dropped_once_reaped() {
        let child = std::process::Command::new("sh")
            .args(["-c", "sleep 30"])
            .spawn()
            .expect("spawn the stand-in daemon");
        let pid = child.id();
        reap_when_it_exits(pid);
        let exported = live_pids_env().unwrap_or_default();
        assert!(
            exported.split(',').any(|part| part == pid.to_string()),
            "{exported:?} should list {pid}"
        );
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        assert!(gone(pid));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while live_pids_env()
            .unwrap_or_default()
            .split(',')
            .any(|part| part == pid.to_string())
        {
            assert!(std::time::Instant::now() < deadline, "{pid} stayed listed");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        std::mem::forget(child);
    }
}
