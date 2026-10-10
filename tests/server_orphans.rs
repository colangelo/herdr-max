//! A server spawned by a test must not outlive that test.
//!
//! Tests that spawn `herdr server` on a PTY used to leave it running when the
//! test process died early: a nextest timeout, the runner's SIGTERM, or SIGKILL.
//! The server ignores SIGHUP on purpose, so the closed PTY master did not stop
//! it, and it was re-parented to launchd or init. These tests start a helper
//! test process that spawns a server exactly as the other integration tests do,
//! SIGKILL the helper, and require the server to disappear.
//!
//! See https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/194
#![cfg(unix)]

pub mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

/// Set for the helper process; names the directory it works in.
const HELPER_DIR_ENV: &str = "HERDR_ORPHAN_TEST_HELPER_DIR";
/// The pid of the test that started the helper, so a helper whose test died
/// (nextest timeout) does not wait forever either.
const HELPER_OWNER_ENV: &str = "HERDR_ORPHAN_TEST_HELPER_OWNER";
const HELPER_TEST_NAME: &str = "orphan_helper_spawns_a_server_and_waits_to_be_killed";

/// A liveness bound, not a margin: the watchdog polls twice a second, so a
/// correct run takes about a second, and only a server that is really stuck
/// reaches this.
const GONE_TIMEOUT: Duration = Duration::from_secs(30);

fn unique_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    PathBuf::from(format!(
        "/tmp/herdr-orphan-test-{}-{nanos}",
        std::process::id()
    ))
}

fn pid_alive(pid: u32) -> bool {
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Not a test. Runs only as the child of `server_exits_when_its_test_process_is_killed`:
/// spawns a server the way the PTY helpers in the other test files do, reports
/// its pid, then sits there until it is killed.
#[test]
fn orphan_helper_spawns_a_server_and_waits_to_be_killed() {
    let Some(dir) = std::env::var_os(HELPER_DIR_ENV) else {
        return;
    };
    let base = PathBuf::from(dir);
    let config_home = base.join("config");
    let runtime_dir = base.join("runtime");
    let api_socket = runtime_dir.join("herdr.sock");
    fs::create_dir_all(&config_home).unwrap();
    fs::create_dir_all(&runtime_dir).unwrap();
    let config = config_home.join("config.toml");
    fs::write(&config, "onboarding = false\n").unwrap();

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_herdr"));
    support::isolate_herdr_test_process(&mut cmd);
    cmd.arg("server");
    cmd.env("XDG_CONFIG_HOME", &config_home);
    cmd.env("XDG_RUNTIME_DIR", &runtime_dir);
    cmd.env("HERDR_SOCKET_PATH", &api_socket);
    cmd.env("HERDR_CONFIG_PATH", &config);
    cmd.env_remove("HERDR_CLIENT_SOCKET_PATH");
    cmd.env("SHELL", "/bin/sh");
    cmd.env_remove("HERDR_ENV");
    let child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave);

    support::wait_for_socket(&api_socket);
    let pid = child.process_id().expect("spawned server has a pid");
    // Written last: the parent treats this file as "the server is up".
    fs::write(base.join("server.pid"), pid.to_string()).unwrap();

    // Keep the PTY master open and the child handle alive until killed, or
    // until the test that started this helper is gone.
    let _keep = (pair.master, child);
    let owner = std::env::var(HELPER_OWNER_ENV)
        .ok()
        .and_then(|raw| raw.parse::<u32>().ok());
    while owner.is_none_or(pid_alive) {
        thread::sleep(Duration::from_secs(1));
    }
}

fn wait_for_server_pid(base: &Path, helper: &mut std::process::Child) -> u32 {
    let pid_file = base.join("server.pid");
    let deadline = Instant::now() + support::APPEARS_TIMEOUT;
    while Instant::now() < deadline {
        if let Ok(text) = fs::read_to_string(&pid_file) {
            return text.trim().parse().expect("server.pid holds a pid");
        }
        if let Some(status) = helper.try_wait().unwrap() {
            panic!("helper test process exited before its server was up: {status}");
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("helper never reported a server pid in {}", base.display());
}

#[test]
fn server_exits_when_its_test_process_is_killed() {
    let base = unique_dir();
    fs::create_dir_all(&base).unwrap();

    let mut helper = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            HELPER_TEST_NAME,
            "--nocapture",
            "--test-threads=1",
        ])
        .env(HELPER_DIR_ENV, &base)
        .env(HELPER_OWNER_ENV, std::process::id().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let server_pid = wait_for_server_pid(&base, &mut helper);
    assert!(
        pid_alive(server_pid),
        "the helper's server should be running before the helper is killed"
    );

    // SIGKILL: no atexit hook, panic hook or Drop runs in the helper, so only
    // the server's own watchdog can end the server from here.
    helper.kill().unwrap();
    helper.wait().unwrap();

    let deadline = Instant::now() + GONE_TIMEOUT;
    let mut gone = false;
    while Instant::now() < deadline {
        if !pid_alive(server_pid) {
            gone = true;
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    if !gone {
        // Do not add to the very pile of orphans this test guards against.
        unsafe {
            libc::kill(server_pid as libc::pid_t, libc::SIGKILL);
        }
    }
    let _ = fs::remove_dir_all(&base);
    assert!(
        gone,
        "server pid {server_pid} outlived its killed test process by {GONE_TIMEOUT:?}"
    );
}
