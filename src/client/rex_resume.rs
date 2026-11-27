//! Program Resume Protocol (OSC 7577) reports, so a terminal that supports it
//! (Rex) can reattach this client after the terminal restarts.
//!
//! The terminal puts a per-pane secret in `OSC7577_TOKEN`. The client proves it
//! knows the secret with a mac and never sends the secret. Spec:
//! <https://www.superlogical.com/rex/docs/build/program-resume>.
//!
//! - [`capture_token_from_env`] runs first in `main`. It moves the token out of
//!   the process environment, so nothing the client spawns (a server, ssh, a
//!   pane helper) inherits it.
//! - [`report_attach`] writes one report: `<program> session attach <name>`.
//! - [`report_remote_launch`] writes the report of a `--remote` launcher:
//!   `<program> --remote <target> [--remote-keybindings=server] --session <name>`.
//!   The client that launcher starts for the screen never reports, so a remote
//!   tab has one record, owned by the launcher.
//! - [`note_user_detach`] marks a deliberate detach. Only then does [`finish`]
//!   send `clear=1`. SIGHUP, SIGTERM and a lost terminal are what a terminal
//!   restart looks like to the client, and the record must survive them.

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use sha2::{Digest, Sha256};

const TOKEN_ENV_VAR: &str = "OSC7577_TOKEN";

struct Reporter {
    token: Option<String>,
    reported: AtomicBool,
    user_detached: AtomicBool,
}

static REPORTER: OnceLock<Reporter> = OnceLock::new();

impl Reporter {
    fn new(token: Option<String>) -> Self {
        Self {
            token,
            reported: AtomicBool::new(false),
            user_detached: AtomicBool::new(false),
        }
    }

    fn report(&self, out: &mut impl Write, pid: u32, time: u64, argv: &[String]) -> io::Result<()> {
        let Some(token) = &self.token else {
            return Ok(());
        };
        let Some(bytes) = report_bytes(token, pid, time, argv) else {
            return Ok(());
        };
        out.write_all(&bytes)?;
        out.flush()?;
        self.reported.store(true, Ordering::Release);
        Ok(())
    }

    /// `clear=1`, only after a report and a deliberate detach.
    fn finish(&self, out: &mut impl Write, pid: u32, time: u64) -> io::Result<()> {
        let Some(token) = &self.token else {
            return Ok(());
        };
        if !self.reported.load(Ordering::Acquire) || !self.user_detached.load(Ordering::Acquire) {
            return Ok(());
        }
        out.write_all(&clear_bytes(token, pid, time))?;
        out.flush()
    }
}

/// The body's keys end with `mac`, the SHA-256 of the body, `:` and the token.
fn sequence(token: &str, body: &str) -> Vec<u8> {
    let mut hash = Sha256::new();
    hash.update(body.as_bytes());
    hash.update(b":");
    hash.update(token.as_bytes());
    let mac: String = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("\x1b]7577;{body}:mac={mac}\x1b\\").into_bytes()
}

/// `None` when the command cannot be reported: empty, a first element that
/// starts with `-`, or an element with a control character.
fn report_bytes(token: &str, pid: u32, time: u64, argv: &[String]) -> Option<Vec<u8>> {
    let first = argv.first()?;
    if first.is_empty()
        || first.starts_with('-')
        || argv.iter().any(|arg| arg.chars().any(char::is_control))
    {
        return None;
    }
    let argv = argv
        .iter()
        .map(|arg| STANDARD.encode(arg))
        .collect::<Vec<_>>()
        .join(",");
    Some(sequence(
        token,
        &format!("pid={pid}:time={time}:argv={argv}"),
    ))
}

fn clear_bytes(token: &str, pid: u32, time: u64) -> Vec<u8> {
    sequence(token, &format!("pid={pid}:time={time}:clear=1"))
}

fn take_env_token() -> Option<String> {
    let token = std::env::var(TOKEN_ENV_VAR)
        .ok()
        .filter(|token| !token.is_empty());
    std::env::remove_var(TOKEN_ENV_VAR);
    token
}

/// Moves `OSC7577_TOKEN` out of the process environment. Call once, first in
/// `main`, before anything spawns a child.
pub fn capture_token_from_env() {
    let _ = REPORTER.set(Reporter::new(take_env_token()));
}

/// Whether the terminal offered a token.
#[cfg(unix)]
pub(super) fn active() -> bool {
    REPORTER
        .get()
        .is_some_and(|reporter| reporter.token.is_some())
}

/// The token for a re-exec of this same client, which must keep reporting.
#[cfg(unix)]
pub(super) fn token_for_reexec() -> Option<&'static str> {
    REPORTER.get()?.token.as_deref()
}

fn unix_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// The program's bare name as it was run, so `herdr-beta` stays `herdr-beta`.
fn program_name() -> Option<String> {
    let program = std::env::args_os().next()?;
    Some(
        std::path::Path::new(&program)
            .file_name()?
            .to_str()?
            .to_owned(),
    )
}

/// What a resumed client runs: the program's bare name, then the session named
/// explicitly (the protocol carries no environment). `None` when this client
/// does not attach by name: a socket path override, an odd program name, or
/// the client a `--remote` launcher starts, whose launcher reports instead.
fn attach_argv() -> Option<Vec<String>> {
    if crate::config::path_env(crate::api::SOCKET_PATH_ENV_VAR).is_some()
        || crate::config::path_env(crate::server::socket_paths::CLIENT_SOCKET_PATH_ENV_VAR)
            .is_some()
        || std::env::var_os(crate::remote::REMOTE_KEYBINDINGS_ENV_VAR).is_some()
    {
        return None;
    }
    let session = crate::session::active_name()
        .unwrap_or_else(|| crate::session::DEFAULT_SESSION_NAME.to_owned());
    Some(vec![
        program_name()?,
        "session".to_owned(),
        "attach".to_owned(),
        session,
    ])
}

/// What a resumed `--remote` launcher runs: the same target as given, the
/// keybinding source when it is not the default, and the far session named
/// explicitly. Never `--handoff`: a resume must not hand off the far server.
fn remote_launch_argv(
    program: String,
    target: &str,
    keybindings: crate::remote::RemoteKeybindings,
    session: &str,
) -> Vec<String> {
    let mut argv = vec![program, "--remote".to_owned(), target.to_owned()];
    if keybindings != crate::remote::RemoteKeybindings::Local {
        argv.push(format!("--remote-keybindings={}", keybindings.as_str()));
    }
    argv.push("--session".to_owned());
    argv.push(session.to_owned());
    argv
}

/// Reports this client to the terminal. Silent without a token.
pub(super) fn report_attach(out: &mut impl Write) {
    let Some(reporter) = REPORTER.get().filter(|reporter| reporter.token.is_some()) else {
        return;
    };
    let Some(argv) = attach_argv() else {
        return;
    };
    if let Err(error) = reporter.report(out, std::process::id(), unix_time(), &argv) {
        tracing::debug!(%error, "could not write the resume report");
    }
}

/// Reports a `--remote` launcher to the terminal, with the launcher's pid.
/// Call it once the far side is ready and before the launcher starts the
/// client that draws the screen, so the report is the only output on the
/// terminal at that moment. Silent without a token.
///
/// The launcher never sends `clear=1`: it cannot tell a detach from a lost
/// connection, and the terminal drops the record when the launcher exits.
pub(crate) fn report_remote_launch(
    out: &mut impl Write,
    target: &str,
    keybindings: crate::remote::RemoteKeybindings,
    session: &str,
) {
    let Some(reporter) = REPORTER.get().filter(|reporter| reporter.token.is_some()) else {
        return;
    };
    let Some(program) = program_name() else {
        return;
    };
    let argv = remote_launch_argv(program, target, keybindings, session);
    if let Err(error) = reporter.report(out, std::process::id(), unix_time(), &argv) {
        tracing::debug!(%error, "could not write the remote resume report");
    }
}

/// The person detached on purpose; the record should go away.
pub(super) fn note_user_detach() {
    if let Some(reporter) = REPORTER.get() {
        reporter.user_detached.store(true, Ordering::Release);
    }
}

/// Call once the terminal is restored and the session is over.
pub(super) fn finish(out: &mut impl Write) {
    if let Some(reporter) = REPORTER.get() {
        if let Err(error) = reporter.finish(out, std::process::id(), unix_time()) {
            tracing::debug!(%error, "could not write the resume clear");
        }
    }
}

/// Reports again after the client is continued from a stop (SIGCONT). The
/// event goes to the client loop, which owns the terminal output.
#[cfg(unix)]
pub(super) fn watch_continue(
    tx: tokio::sync::mpsc::Sender<super::ClientLoopEvent>,
    should_quit: std::sync::Arc<AtomicBool>,
) {
    crate::platform::watch_terminal_continue_signal();
    while !should_quit.load(Ordering::Acquire) {
        std::thread::sleep(std::time::Duration::from_millis(250));
        if crate::platform::take_terminal_continue_signal()
            && tx
                .blocking_send(super::ClientLoopEvent::TerminalRegained)
                .is_err()
        {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "test-token-0123456789abcd";

    fn argv() -> Vec<String> {
        ["herdr-beta", "session", "attach", "rex_a"]
            .map(String::from)
            .to_vec()
    }

    // Computed by hand with python hashlib and base64, not by this code.
    const REPORT: &str = "\x1b]7577;pid=4242:time=1795543200:argv=aGVyZHItYmV0YQ==,c2Vzc2lvbg==,YXR0YWNo,cmV4X2E=:mac=cba111a073df5899a1a198fdd4f2bae2514c3b085ccbdaf05e79d65733a1b760\x1b\\";
    const CLEAR: &str = "\x1b]7577;pid=4242:time=1795543200:clear=1:mac=357129e8a7721a1663d085ba948d4e209234c1c4afda1ccdc5974afc5ee0b7bf\x1b\\";

    #[test]
    fn the_report_matches_a_hand_computed_mac_and_encoding() {
        let reporter = Reporter::new(Some(TOKEN.to_owned()));
        let mut out = Vec::new();
        reporter
            .report(&mut out, 4242, 1795543200, &argv())
            .unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), REPORT);
    }

    #[test]
    fn no_token_writes_nothing() {
        let reporter = Reporter::new(None);
        reporter.user_detached.store(true, Ordering::Release);
        let mut out = Vec::new();
        reporter
            .report(&mut out, 4242, 1795543200, &argv())
            .unwrap();
        reporter.finish(&mut out, 4242, 1795543200).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn a_terminal_that_ends_leaves_the_record_alone() {
        // SIGHUP, SIGTERM and a lost terminal end the session without a
        // deliberate detach.
        let reporter = Reporter::new(Some(TOKEN.to_owned()));
        let mut out = Vec::new();
        reporter
            .report(&mut out, 4242, 1795543200, &argv())
            .unwrap();
        let reported = out.len();
        reporter.finish(&mut out, 4242, 1795543200).unwrap();
        assert_eq!(out.len(), reported);
    }

    #[test]
    fn a_user_detach_sends_clear() {
        let reporter = Reporter::new(Some(TOKEN.to_owned()));
        let mut out = Vec::new();
        reporter
            .report(&mut out, 4242, 1795543200, &argv())
            .unwrap();
        reporter.user_detached.store(true, Ordering::Release);
        out.clear();
        reporter.finish(&mut out, 4242, 1795543200).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), CLEAR);
    }

    #[test]
    fn a_detach_before_any_report_clears_nothing() {
        let reporter = Reporter::new(Some(TOKEN.to_owned()));
        reporter.user_detached.store(true, Ordering::Release);
        let mut out = Vec::new();
        reporter.finish(&mut out, 4242, 1795543200).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn a_report_after_regaining_the_terminal_carries_the_new_time() {
        let reporter = Reporter::new(Some(TOKEN.to_owned()));
        let mut out = Vec::new();
        reporter
            .report(&mut out, 4242, 1795543200, &argv())
            .unwrap();
        reporter
            .report(&mut out, 4242, 1795543260, &argv())
            .unwrap();
        let text = String::from_utf8(out).unwrap();
        let reports: Vec<&str> = text
            .split("\x1b\\")
            .filter(|part| !part.is_empty())
            .collect();
        assert_eq!(reports.len(), 2);
        assert!(reports[1].contains("time=1795543260:"));
        assert_ne!(reports[0], reports[1]);
    }

    // Computed by hand with python hashlib and base64, not by this code.
    const REMOTE_REPORT: &str = "\x1b]7577;pid=4242:time=1795543200:argv=aGVyZHItbWF4,LS1yZW1vdGU=,YWMtbWJtNS5sb2NhbA==,LS1yZW1vdGUta2V5YmluZGluZ3M9c2VydmVy,LS1zZXNzaW9u,ZGVmYXVsdA==:mac=afccdb88934eb832edd986d30d4c0d0b348efcb6528f6724f7a6958d7319718a\x1b\\";

    #[test]
    fn the_remote_launcher_reports_its_target_keybindings_and_session() {
        let reporter = Reporter::new(Some(TOKEN.to_owned()));
        let argv = remote_launch_argv(
            "herdr-max".to_owned(),
            "ac-mbm5.local",
            crate::remote::RemoteKeybindings::Server,
            "default",
        );
        let mut out = Vec::new();
        reporter.report(&mut out, 4242, 1795543200, &argv).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), REMOTE_REPORT);
    }

    #[test]
    fn the_remote_launcher_leaves_out_default_keybindings_and_never_hands_off() {
        let argv = remote_launch_argv(
            "herdr-beta".to_owned(),
            "m4m.local",
            crate::remote::RemoteKeybindings::Local,
            "agents",
        );
        assert_eq!(
            argv,
            ["herdr-beta", "--remote", "m4m.local", "--session", "agents"].map(String::from)
        );
    }

    #[test]
    fn a_command_that_cannot_be_reported_is_not() {
        assert!(report_bytes(TOKEN, 1, 1, &[]).is_none());
        assert!(report_bytes(TOKEN, 1, 1, &["-x".to_owned()]).is_none());
        assert!(report_bytes(TOKEN, 1, 1, &["a".to_owned(), "b\nc".to_owned()]).is_none());
    }
}

#[cfg(all(test, unix))]
mod unix_tests {
    use super::*;

    #[test]
    fn the_client_a_remote_launcher_starts_never_reports() {
        for name in [
            crate::api::SOCKET_PATH_ENV_VAR,
            crate::server::socket_paths::CLIENT_SOCKET_PATH_ENV_VAR,
            crate::remote::REMOTE_KEYBINDINGS_ENV_VAR,
        ] {
            std::env::remove_var(name);
        }
        assert!(attach_argv().is_some(), "a local client reports");
        // What the launcher sets for its client (configure_remote_client_environment).
        std::env::set_var(crate::remote::REMOTE_KEYBINDINGS_ENV_VAR, "server");
        assert!(attach_argv().is_none(), "the launcher's client reported");
    }

    #[test]
    fn a_spawned_child_does_not_inherit_the_token() {
        std::env::set_var(TOKEN_ENV_VAR, "test-token-0123456789abcd");
        assert_eq!(
            take_env_token().as_deref(),
            Some("test-token-0123456789abcd")
        );
        let status = std::process::Command::new("sh")
            .args(["-c", "test -z \"${OSC7577_TOKEN+set}\""])
            .status()
            .unwrap();
        assert!(status.success(), "the child saw OSC7577_TOKEN");
    }
}
