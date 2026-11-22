//! What `herdr machine ... --core` needs of the primary core
//! (openspec/changes/core-phase5, D1): its session, its `hc`, and calls on
//! its socket that never start it.

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
    let socket = client::primary_socket_path(&primary_session());
    let connected = match client::connect_at(&socket) {
        Ok((connected, _)) => connected,
        Err(client::CoreError::NoCore(_)) => return Ok(None),
        Err(err) => return Err(err.to_string()),
    };
    connected
        .call(method, params, timeout)
        .map(Some)
        .map_err(|err| err.to_string())
}
