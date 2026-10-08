//! Staying attached across a live handoff (fork issue 94).
//!
//! On a live handoff the server tells its clients to reconnect once the new
//! server is up. A local app client restores the terminal, waits for the new
//! server's socket, and attaches again at the terminal's current size; a
//! detach, a normal server stop and a direct terminal attach still exit.
//! A federated client does not come through here: its Local endpoint
//! supervisor reconnects while the shell keeps running.

use super::*;
use std::sync::Mutex;

/// How an app client session ended.
pub(super) enum ClientExit {
    Done,
    /// The server handed off to a new one; reconnect to it.
    HandedOff,
    /// A reconnect attempt found the old server still answering, refusing, or
    /// the new one not ready; wait and try again.
    Retry,
}

/// How long a client waits for the new server after a live handoff.
const RECONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Whether the client ended because the server handed off to a new build.
pub(super) fn is_live_handoff_shutdown(err: &ClientError) -> bool {
    matches!(
        err,
        ClientError::ServerShutdown { reason: Some(reason) }
            if reason == crate::protocol::LIVE_HANDOFF_SHUTDOWN_REASON
    )
}

/// Whether a session that ended with `err` follows the server to its successor
/// instead of exiting: only a live handoff, and only for an app client. A
/// direct terminal attach has no session to follow; a detach and a normal
/// server stop end the client.
pub(super) fn follows_handoff(err: &ClientError, app_client: bool) -> bool {
    app_client && is_live_handoff_shutdown(err)
}

/// Runs `session` until it ends for good. After a handoff, `wait` blocks until
/// the new server answers and the session runs again; its argument says
/// whether it is a reconnect.
pub(super) fn run_following_handoffs(
    mut session: impl FnMut(bool) -> io::Result<ClientExit>,
    mut wait: impl FnMut(&Reconnect),
) -> io::Result<()> {
    let mut reconnect: Option<Reconnect> = None;
    loop {
        match session(reconnect.is_some())? {
            ClientExit::Done => return Ok(()),
            ClientExit::HandedOff => reconnect = Some(Reconnect::begin()),
            ClientExit::Retry => {}
        }
        if let Some(reconnect) = &reconnect {
            wait(reconnect);
        }
    }
}

/// The client error a failed attach carries: the handshake error keeps its
/// [`ClientError`] as the `io::Error` source.
fn attach_failure(err: &io::Error) -> Option<&ClientError> {
    err.get_ref()
        .and_then(|source| source.downcast_ref::<ClientError>())
}

/// Whether a failed attach is a live handoff in progress.
pub(super) fn is_live_handoff_attach_failure(err: &io::Error) -> bool {
    attach_failure(err).is_some_and(is_live_handoff_shutdown)
}

/// Whether a failed attach, while reconnecting, may succeed once the new server
/// is ready: the old server still refusing, or the new one closing the
/// connection before it answers. A rejected or malformed handshake will not.
pub(super) fn is_transient_attach_failure(err: &io::Error) -> bool {
    matches!(
        attach_failure(err),
        Some(
            ClientError::ServerShutdown { .. }
                | ClientError::ConnectionLost(_)
                | ClientError::ConnectionFailed(_)
        )
    )
}

/// One reconnect after a live handoff: a window in which refused or failed
/// connects are retried.
pub(super) struct Reconnect {
    deadline: std::time::Instant,
}

impl Reconnect {
    pub(super) fn begin() -> Self {
        eprintln!("herdr: live update in progress; reconnecting…");
        Self {
            deadline: std::time::Instant::now() + RECONNECT_TIMEOUT,
        }
    }

    /// Block until the new server's socket accepts, then decide whether this
    /// client should re-exec onto the server's build (fork issue 165). Exits the
    /// process when the server does not come back in time.
    pub(super) fn wait_for_server(&self) {
        // The old server's socket can still answer, refusing, for a moment
        // after it hands off; give the new one room to bind.
        std::thread::sleep(Duration::from_millis(200));
        let remaining = self
            .deadline
            .saturating_duration_since(std::time::Instant::now());
        // A reattach that keeps failing against a reachable socket ends here too.
        if remaining.is_zero()
            || !wait_for_socket(&client_socket_path(), remaining, quit_signalled())
        {
            eprintln!(
                "herdr: the updated server did not come back; run `{}` to reattach",
                crate::session::local_attach_command()
            );
            std::process::exit(1);
        }
        reexec::reexec_onto_server_build();
    }
}

/// Wait until the client socket accepts a connection, up to `timeout`, or
/// until `quit` is raised. True when a server is there.
fn wait_for_socket(path: &std::path::Path, timeout: Duration, quit: &AtomicBool) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if quit.load(Ordering::Acquire) {
            return false;
        }
        if crate::ipc::connect_local_stream(path).is_ok() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Raised by SIGINT, SIGTERM and SIGHUP, and never lowered.
static QUIT_SIGNALLED: AtomicBool = AtomicBool::new(false);
/// The running session's quit flag, which the signal handler raises as well.
static SESSION_QUIT: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);

pub(super) fn quit_signalled() -> &'static AtomicBool {
    &QUIT_SIGNALLED
}

/// The quit flag of a new session. The termination handler can be installed
/// only once per process, and a client that reconnects after a handoff runs
/// several sessions, so the handler raises whichever session is current. A
/// session raises its own flag when it ends, which stops the reader threads it
/// started (a stale stdin reader would take the next session's input).
pub(super) fn new_session_quit_flag() -> Arc<AtomicBool> {
    static HANDLER: std::sync::Once = std::sync::Once::new();
    HANDLER.call_once(|| {
        // ctrlc's "termination" feature also catches SIGTERM/SIGHUP so direct
        // termination signals still run the quit path and TerminalGuard::Drop.
        if let Err(err) = ctrlc::set_handler(|| {
            QUIT_SIGNALLED.store(true, Ordering::Release);
            if let Ok(current) = SESSION_QUIT.lock() {
                if let Some(flag) = current.as_ref() {
                    flag.store(true, Ordering::Release);
                }
            }
        }) {
            warn!(%err, "failed to install termination handler; terminal restore relies on TerminalGuard::Drop and the panic hook");
        }
    });
    let flag = Arc::new(AtomicBool::new(false));
    if let Ok(mut current) = SESSION_QUIT.lock() {
        *current = Some(flag.clone());
    }
    // A signal that arrived before this session registered still applies.
    if QUIT_SIGNALLED.load(Ordering::Acquire) {
        flag.store(true, Ordering::Release);
    }
    flag
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_live_handoff_shutdown_reason_counts_as_a_handoff() {
        let handoff = ClientError::ServerShutdown {
            reason: Some(crate::protocol::LIVE_HANDOFF_SHUTDOWN_REASON.to_owned()),
        };
        let detached = ClientError::ServerShutdown {
            reason: Some("detached".to_owned()),
        };
        let stopped = ClientError::ServerShutdown { reason: None };
        let lost = ClientError::ConnectionLost(io::Error::other("gone"));
        assert!(is_live_handoff_shutdown(&handoff));
        assert!(!is_live_handoff_shutdown(&detached));
        assert!(!is_live_handoff_shutdown(&stopped));
        assert!(!is_live_handoff_shutdown(&lost));
    }

    #[test]
    fn a_handshake_refused_by_a_handing_off_server_is_a_handoff_not_an_error() {
        let handoff = io::Error::other(ClientError::ServerShutdown {
            reason: Some(crate::protocol::LIVE_HANDOFF_SHUTDOWN_REASON.to_owned()),
        });
        let stopped = io::Error::other(ClientError::ServerShutdown { reason: None });
        assert!(is_live_handoff_attach_failure(&handoff));
        assert!(!is_live_handoff_attach_failure(&stopped));
        assert!(!is_live_handoff_attach_failure(&io::Error::other("plain")));
        // While reconnecting, a refusal or a dropped connection is retried; a
        // rejected handshake is not.
        assert!(is_transient_attach_failure(&handoff));
        assert!(is_transient_attach_failure(&stopped));
        assert!(is_transient_attach_failure(&io::Error::other(
            ClientError::ConnectionLost(io::Error::other("eof"))
        )));
        assert!(!is_transient_attach_failure(&io::Error::other(
            ClientError::HandshakeRejected {
                version: 1,
                error: "no".into()
            }
        )));
        assert!(!is_transient_attach_failure(&io::Error::other("plain")));
        // The text a user sees is unchanged by keeping the typed source.
        assert_eq!(
            handoff.to_string(),
            "server shut down: live update in progress; reconnect after handoff completes"
        );
    }

    #[test]
    fn a_normal_stop_a_detach_and_a_direct_attach_do_not_follow_a_handoff() {
        let handoff = ClientError::ServerShutdown {
            reason: Some(crate::protocol::LIVE_HANDOFF_SHUTDOWN_REASON.to_owned()),
        };
        let detached = ClientError::ServerShutdown {
            reason: Some("detached".to_owned()),
        };
        let stopped = ClientError::ServerShutdown { reason: None };
        assert!(follows_handoff(&handoff, true));
        assert!(!follows_handoff(&handoff, false));
        assert!(!follows_handoff(&detached, true));
        assert!(!follows_handoff(&stopped, true));
    }

    #[test]
    fn a_session_that_ends_normally_does_not_wait_or_rerun() {
        let mut runs = Vec::new();
        let mut waits = 0;
        run_following_handoffs(
            |reconnecting| {
                runs.push(reconnecting);
                Ok(ClientExit::Done)
            },
            |_| waits += 1,
        )
        .expect("done");
        assert_eq!(runs, [false]);
        assert_eq!(waits, 0);
    }

    #[test]
    fn a_handoff_waits_then_reattaches_and_retries_until_the_session_runs() {
        let mut exits = vec![ClientExit::Done, ClientExit::Retry, ClientExit::HandedOff];
        let mut runs = Vec::new();
        let mut waits = 0;
        run_following_handoffs(
            |reconnecting| {
                runs.push(reconnecting);
                Ok(exits.pop().expect("an exit per run"))
            },
            |_| waits += 1,
        )
        .expect("done");
        // First run ends in a handoff, the reconnect is retried once, then runs.
        assert_eq!(runs, [false, true, true]);
        assert_eq!(waits, 2);
    }

    #[test]
    fn a_session_error_ends_the_client_without_waiting() {
        let mut waits = 0;
        let err = run_following_handoffs(|_| Err(io::Error::other("rejected")), |_| waits += 1)
            .expect_err("error");
        assert_eq!(err.to_string(), "rejected");
        assert_eq!(waits, 0);
    }

    #[cfg(unix)]
    #[test]
    fn a_handshake_with_a_handing_off_server_reports_the_handoff() {
        use crate::ipc::{bind_local_listener, connect_local_stream};
        use interprocess::local_socket::traits::Listener as _;
        let dir = std::env::temp_dir().join(format!("hhs-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("client.sock");
        let _ = std::fs::remove_file(&path);
        let listener = bind_local_listener(&path).expect("bind");
        // The server keeps its end open until the client is done: macOS refuses
        // socket options on a connection whose peer has already closed.
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let mut stream = listener.accept().expect("accept");
            let _hello: ClientMessage =
                protocol::read_message(&mut stream, protocol::MAX_FRAME_SIZE).expect("hello");
            protocol::write_message(
                &mut stream,
                &ServerMessage::ServerShutdown {
                    reason: Some(crate::protocol::LIVE_HANDOFF_SHUTDOWN_REASON.to_owned()),
                },
            )
            .expect("shutdown");
            let _ = done_rx.recv();
        });
        let mut stream = connect_local_stream(&path).expect("connect");
        // The client shell's endpoint hello, as an app client sends it.
        let err = do_handshake(
            &mut stream,
            80,
            24,
            0,
            0,
            false,
            Some(crate::protocol::ClientSurfaceSize { cols: 80, rows: 24 }),
            false,
            false,
            true,
            true,
        )
        .map(|_| ())
        .expect_err("a handing-off server refuses");
        let _ = done_tx.send(());
        server.join().expect("server thread");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(is_live_handoff_shutdown(&err), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn waiting_for_the_server_ends_when_its_socket_accepts() {
        let dir = std::env::temp_dir().join(format!("hcw-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("client.sock");
        let _ = std::fs::remove_file(&path);
        let bind_path = path.clone();
        let binder = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            std::os::unix::net::UnixListener::bind(&bind_path).expect("bind")
        });
        let quit = AtomicBool::new(false);

        assert!(wait_for_socket(&path, Duration::from_secs(5), &quit));
        drop(binder.join());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn waiting_for_the_server_gives_up_on_timeout_or_quit() {
        let path = std::env::temp_dir().join(format!("hcw-none-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let quit = AtomicBool::new(false);
        assert!(!wait_for_socket(&path, Duration::from_millis(200), &quit));

        quit.store(true, Ordering::Release);
        assert!(!wait_for_socket(&path, Duration::from_secs(30), &quit));
    }
}
