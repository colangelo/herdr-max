use super::*;

/// Runs the thin client and enters the main event loop. After a live handoff
/// the app client waits for the new server and attaches again (fork issue 94).
pub fn run_client() -> io::Result<()> {
    handoff::run_following_handoffs(
        |reconnecting| {
            let log_message = if reconnecting {
                "reconnecting to server after a live update"
            } else {
                "connecting to server"
            };
            run_client_with_mode(None, None, log_message, reconnecting)
        },
        handoff::Reconnect::wait_for_server,
    )
}

#[cfg(unix)]
pub fn run_terminal_attach(terminal_id: String, takeover: bool) -> io::Result<()> {
    run_client_with_mode(
        Some((terminal_id, takeover)),
        Some(AttachEscapeState::default()),
        "attaching to terminal",
        false,
    )
    .map(|_| ())
}

#[cfg(windows)]
pub fn run_terminal_attach(_terminal_id: String, _takeover: bool) -> io::Result<()> {
    debug_assert!(!crate::platform::capabilities().direct_terminal_attach);
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "direct terminal attach is not supported on Windows yet",
    ))
}
