//! Starting a view server on demand (core-phase5 D6 Lifetime): the endpoint
//! transport and `herdr --remote core:<machine>` reach a view through its
//! client socket and start one when none listens there. It runs in the
//! background like the primary server, exits 10 min after its last client,
//! and dials nothing by starting: its first client's lease decides that.

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
        return Ok(socket);
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
