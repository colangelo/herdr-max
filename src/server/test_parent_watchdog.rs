//! Ends a test-owned server when the test process that spawned it is gone.
//!
//! The integration tests start `herdr server` on a PTY. When the test process
//! dies early (a nextest timeout, the runner's SIGTERM, SIGKILL), nothing
//! reaches the server: it ignores SIGHUP on purpose, so the closed PTY master
//! does not stop it, and a stray SIGTERM can be swallowed once its runtime is
//! no longer polling. The server is then re-parented to launchd or init and
//! lives on, about 31 MB each.
//!
//! A test that wants its server to die with it sets [`TEST_PARENT_PID_ENV_VAR`]
//! to its own pid. The server polls that pid from a plain thread, so it still
//! fires if the async runtime or the shutdown path is wedged, and exits once
//! the pid no longer exists. The pid is watched, not the parent pid: a server
//! started by live handoff or by client autodetect is a child of another
//! server, yet it still belongs to the same test, and the variable is
//! inherited by those children.
//!
//! With the variable unset, which is every production run, this does nothing.

use std::time::Duration;

/// Pid of the test process whose death should end this server.
pub(crate) const TEST_PARENT_PID_ENV_VAR: &str = "HERDR_TEST_PARENT_PID";

const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Starts the watchdog when [`TEST_PARENT_PID_ENV_VAR`] names a pid.
pub(crate) fn spawn_from_env() {
    let Some(raw) = std::env::var_os(TEST_PARENT_PID_ENV_VAR) else {
        return;
    };
    let Some(pid) = parse_pid(raw.to_str()) else {
        tracing::warn!(
            value = ?raw,
            "ignoring {TEST_PARENT_PID_ENV_VAR}: not a process id"
        );
        return;
    };
    let spawned = std::thread::Builder::new()
        .name("herdr-test-parent-watchdog".to_owned())
        .spawn(move || {
            watch_until_gone(pid, POLL_INTERVAL, crate::platform::process_exists);
            tracing::warn!(pid, "test parent process is gone; exiting");
            std::process::exit(1);
        });
    if let Err(err) = spawned {
        tracing::warn!(%err, "failed to start the test parent watchdog");
    }
}

fn parse_pid(raw: Option<&str>) -> Option<u32> {
    raw?.trim().parse::<u32>().ok().filter(|pid| *pid > 1)
}

/// Blocks until `exists(pid)` reports false.
fn watch_until_gone(pid: u32, interval: Duration, exists: impl Fn(u32) -> bool) {
    while exists(pid) {
        std::thread::sleep(interval);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Instant;

    #[test]
    fn only_real_process_ids_are_accepted() {
        assert_eq!(parse_pid(Some("4242")), Some(4242));
        assert_eq!(parse_pid(Some(" 4242\n")), Some(4242));
        for rejected in [
            None,
            Some(""),
            Some("0"),
            Some("1"),
            Some("-5"),
            Some("abc"),
        ] {
            assert_eq!(parse_pid(rejected), None, "{rejected:?}");
        }
    }

    #[test]
    fn watch_returns_once_the_process_is_gone() {
        let polls = AtomicU32::new(0);
        let started = Instant::now();
        watch_until_gone(4242, Duration::from_millis(5), |pid| {
            assert_eq!(pid, 4242);
            polls.fetch_add(1, Ordering::SeqCst) < 3
        });
        assert_eq!(polls.load(Ordering::SeqCst), 4);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_reaped_child_counts_as_gone_and_a_live_process_does_not() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let pid = child.id();
        assert!(crate::platform::process_exists(pid));
        assert!(crate::platform::process_exists(std::process::id()));

        child.kill().unwrap();
        child.wait().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        watch_until_gone(pid, Duration::from_millis(10), |pid| {
            assert!(
                Instant::now() < deadline,
                "reaped child still reported alive"
            );
            crate::platform::process_exists(pid)
        });
    }
}
