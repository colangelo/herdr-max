//! What `herdr machine ... --core` needs of the primary core
//! (openspec/changes/core-phase5, D1): its session, its `hc`, and calls on
//! its socket that never start it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use super::client;

/// The primary core session, where the machine is added (D1).
pub(crate) fn primary_session() -> String {
    super::core_session_name()
}

/// `hc` with `HC_SESSION` set to the primary, inheriting the rest of the
/// person's environment and terminal, so hc's own questions are asked there.
pub(crate) fn hc_command() -> Result<std::process::Command, String> {
    let hc = client::hc_binary().map_err(|err| err.to_string())?;
    let mut command = std::process::Command::new(hc);
    command.env("HC_SESSION", primary_session());
    Ok(command)
}

/// One call on the running primary core; `Ok(None)` when no core runs
/// there. Never starts one.
pub(crate) fn call_running_primary(
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<Option<Value>, String> {
    call_running_at(&primary_socket(), method, params, timeout).map_err(|err| err.to_string())
}

/// The primary core's socket.
pub(crate) fn primary_socket() -> PathBuf {
    client::primary_socket_path(&primary_session())
}

/// How a call on a running core failed, for a caller that must tell a
/// request that was never sent from one that may have run (core-phase5 D7:
/// no retry after send).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CallError {
    /// Core could not be reached: nothing was sent.
    NotSent(String),
    /// Core answered with an error of this kind (`unavailable`,
    /// `machine_unknown`, ...).
    Rpc { kind: String, message: String },
    /// No answer came: the connection closed or the call timed out after
    /// the request may have been sent.
    NoAnswer(String),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSent(why) | Self::NoAnswer(why) => f.write_str(why),
            Self::Rpc { kind, message } => write!(f, "herdr-core {kind}: {message}"),
        }
    }
}

/// One call on the core at `socket`; `Ok(None)` when no core runs there.
/// Never starts one.
pub(crate) fn call_running_at(
    socket: &Path,
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<Option<Value>, CallError> {
    let connected = match client::connect_at(socket) {
        Ok((connected, _)) => connected,
        Err(client::CoreError::NoCore(_)) => return Ok(None),
        Err(err) => return Err(CallError::NotSent(err.to_string())),
    };
    connected
        .call(method, params, timeout)
        .map(Some)
        .map_err(|err| match err {
            client::CoreError::Rpc { kind, message } => CallError::Rpc { kind, message },
            err @ (client::CoreError::Closed | client::CoreError::Timeout(_)) => {
                CallError::NoAnswer(err.to_string())
            }
            err @ (client::CoreError::Unavailable(_) | client::CoreError::NoCore(_)) => {
                CallError::NotSent(err.to_string())
            }
        })
}

/// `env -u NAME ... --` for each herdr integration variable: the start of
/// every argv herdr runs on a machine, a pane's or a `machine.exec`'s
/// (core-phase5 D5, D7; F6).
pub(crate) fn machine_env_wrapper() -> Vec<String> {
    super::machine_spawn::machine_env_wrapper()
}

/// The machine of a core address (`ac-mbm5/t7`); `None` for this Mac's
/// own terminals and for an address that does not parse.
pub(crate) fn machine_of_address(address: &str) -> Option<crate::core_machine::MachineName> {
    super::address::CoreAddress::parse(address).ok()?.machine
}

/// Hands `event` to the server loop, when one runs in this process.
pub(crate) fn tell_server(event: crate::events::AppEvent) {
    super::tell_server(event);
}
