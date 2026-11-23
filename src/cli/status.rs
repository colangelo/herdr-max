use serde::Serialize;

use crate::api;
use crate::api::client::ApiClientError;
use crate::remote::visible_line;

pub(super) fn run_status_command(args: &[String]) -> std::io::Result<i32> {
    let Some((scope, json)) = parse_status_args(args) else {
        return Ok(2);
    };

    match scope {
        StatusScope::Full => print_full_status(json),
        StatusScope::Server => print_server_status(json),
        StatusScope::Client => {
            print_client_status(json)?;
            Ok(0)
        }
        StatusScope::Help => {
            print_status_help();
            Ok(0)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatusScope {
    Full,
    Server,
    Client,
    Help,
}

fn parse_status_args(args: &[String]) -> Option<(StatusScope, bool)> {
    match args.first().map(|arg| arg.as_str()) {
        None => Some((StatusScope::Full, false)),
        Some("--json") if args.len() == 1 => Some((StatusScope::Full, true)),
        Some("server") => {
            parse_status_scope_args(args, StatusScope::Server, "herdr status server [--json]")
        }
        Some("client") => {
            parse_status_scope_args(args, StatusScope::Client, "herdr status client [--json]")
        }
        Some("help" | "--help" | "-h") => {
            if args.len() > 1 {
                print_status_help();
                return None;
            }
            Some((StatusScope::Help, false))
        }
        Some(_) => {
            print_status_help();
            None
        }
    }
}

fn parse_status_scope_args(
    args: &[String],
    scope: StatusScope,
    usage: &str,
) -> Option<(StatusScope, bool)> {
    match args.get(1).map(|arg| arg.as_str()) {
        None => Some((scope, false)),
        Some("--json") if args.len() == 2 => Some((scope, true)),
        _ => {
            eprintln!("usage: {usage}");
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ServerRuntimeStatus {
    Running {
        version: Option<String>,
        protocol: Option<u32>,
        capabilities: Option<crate::api::schema::ServerCapabilities>,
        macos_services: Option<crate::api::schema::MacosServicesInfo>,
        core: Option<String>,
    },
    NotRunning,
}

fn print_full_status(json: bool) -> std::io::Result<i32> {
    let server = read_server_runtime_status()?;

    if json {
        print_json(&FullStatusJson {
            client: client_status_json(),
            server: server_status_json(&server),
            update: update_status_json(&server),
        })?;
        return Ok(0);
    }

    println!("client:");
    println!("  version: {}", crate::build_info::version());
    println!(
        "  channel: {}",
        crate::config::Config::load().config.update.channel.as_str()
    );
    println!("  protocol: {}", crate::protocol::PROTOCOL_VERSION);
    println!(
        "  endpoint_protocol_generation: {}",
        crate::protocol::endpoint::ENDPOINT_PROTOCOL_GENERATION
    );
    println!();
    println!("server:");
    print_server_status_body(&server, "  ");
    println!();
    println!("update:");
    println!("  restart_needed: {}", restart_needed_label(&server));
    println!(
        "  server_binary_stale: {}",
        server_binary_stale_label(&server)
    );

    Ok(0)
}

fn print_server_status(json: bool) -> std::io::Result<i32> {
    let server = read_server_runtime_status()?;
    if json {
        print_json(&server_status_json(&server))?;
        return Ok(0);
    }
    print_server_status_body(&server, "");
    Ok(0)
}

fn print_client_status(json: bool) -> std::io::Result<()> {
    if json {
        print_json(&client_status_json())?;
        return Ok(());
    }

    println!("version: {}", crate::build_info::version());
    println!(
        "channel: {}",
        crate::config::Config::load().config.update.channel.as_str()
    );
    println!("protocol: {}", crate::protocol::PROTOCOL_VERSION);
    println!(
        "endpoint_protocol_generation: {}",
        crate::protocol::endpoint::ENDPOINT_PROTOCOL_GENERATION
    );
    println!("binary: {}", current_exe_label());
    Ok(())
}

fn print_server_status_body(server: &ServerRuntimeStatus, indent: &str) {
    for line in server_status_lines(server, &super::target::socket_label()) {
        println!("{indent}{line}");
    }
}

/// The human-readable server status, one line each. A `--machine` status
/// shows the far server's own strings, so they are shown with their
/// controls escaped (security review 2026-11-23 M1).
fn server_status_lines(server: &ServerRuntimeStatus, socket: &str) -> Vec<String> {
    match server {
        ServerRuntimeStatus::Running {
            version,
            protocol,
            capabilities,
            macos_services,
            core,
        } => {
            let mut lines = vec![
                "status: running".to_string(),
                format!(
                    "version: {}",
                    visible_line(option_label(version.as_deref()))
                ),
                format!(
                    "endpoint_compatible: {}",
                    endpoint_compatibility_label(capabilities.as_ref())
                ),
                format!("private_protocol: {}", protocol_label(*protocol)),
                format!(
                    "private_protocol_compatible: {}",
                    compatibility_label(*protocol)
                ),
                format!("socket: {}", visible_line(socket)),
            ];
            if let Some(services) = macos_services {
                lines.push(format!(
                    "macos_services: {}",
                    macos_services_label(services)
                ));
            }
            if let Some(core) = core {
                lines.push(format!("core: {}", visible_line(core)));
            }
            lines
        }
        ServerRuntimeStatus::NotRunning => vec![
            "status: not running".to_string(),
            format!("socket: {}", visible_line(socket)),
        ],
    }
}

fn read_server_runtime_status() -> std::io::Result<ServerRuntimeStatus> {
    match super::target::server_status(&super::target::api_client()?) {
        Ok(status) => Ok(ServerRuntimeStatus::Running {
            version: status.version,
            protocol: status.protocol,
            capabilities: status.capabilities,
            macos_services: status.macos_services,
            core: status.core,
        }),
        Err(err) if super::target::is_remote() => Err(super::target::remote_error(
            super::api_client_error_to_io(err),
        )),
        Err(ApiClientError::Io(err)) if super::server_not_running_error(&err) => {
            Ok(ServerRuntimeStatus::NotRunning)
        }
        Err(err) => Err(super::api_client_error_to_io(err)),
    }
}

/// `user context, reachable` reads as healthy; a `session` context dies with
/// its login, and `unreachable` means it already has (fork issue 228).
pub(super) fn macos_services_label(services: &crate::api::schema::MacosServicesInfo) -> String {
    format!(
        "{} context, {}",
        visible_line(&services.context),
        if services.reachable {
            "reachable"
        } else {
            "unreachable: run `herdr server restart` from Terminal.app"
        }
    )
}

fn option_label(value: Option<&str>) -> &str {
    value.unwrap_or("unknown")
}

fn protocol_label(protocol: Option<u32>) -> String {
    protocol
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn compatibility_label(protocol: Option<u32>) -> &'static str {
    match protocol {
        Some(protocol) if protocol == crate::protocol::PROTOCOL_VERSION => "yes",
        Some(_) => "no",
        None => "unknown",
    }
}

fn endpoint_compatibility_label(
    capabilities: Option<&crate::api::schema::ServerCapabilities>,
) -> &'static str {
    match capabilities.and_then(|value| value.endpoint_protocol_generation) {
        Some(generation)
            if generation == crate::protocol::endpoint::ENDPOINT_PROTOCOL_GENERATION =>
        {
            "yes"
        }
        Some(_) => "no",
        None => "unknown",
    }
}

fn restart_needed_label(server: &ServerRuntimeStatus) -> &'static str {
    match restart_needed_bool(server) {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unknown",
    }
}

fn server_binary_stale_label(server: &ServerRuntimeStatus) -> &'static str {
    match server_binary_stale_bool(server) {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unknown",
    }
}

#[derive(Serialize)]
struct FullStatusJson {
    client: ClientStatusJson,
    server: ServerStatusJson,
    update: UpdateStatusJson,
}

#[derive(Serialize)]
struct ClientStatusJson {
    version: String,
    channel: &'static str,
    protocol: u32,
    endpoint_protocol_generation: u32,
    endpoint_capabilities: Vec<&'static str>,
    remote_host_bridge: bool,
    remote_bridge_idle_timeout: bool,
    binary: String,
    session: Option<String>,
    /// The Herdr Core session this herdr session's panes run in, so a fleet
    /// CLI on another Mac can check that it reaches the herdr whose core it
    /// was linked to (core-phase5 D7). Absent without a core backend.
    #[serde(skip_serializing_if = "Option::is_none")]
    core_session: Option<String>,
}

#[derive(Serialize)]
struct ServerStatusJson {
    status: &'static str,
    running: bool,
    version: Option<String>,
    protocol: Option<u32>,
    capabilities: Option<ServerCapabilitiesJson>,
    compatible: Option<bool>,
    endpoint_compatible: Option<bool>,
    socket: String,
    session: Option<String>,
    restart_needed: Option<bool>,
    server_binary_stale: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    macos_services: Option<crate::api::schema::MacosServicesInfo>,
    /// `gave_up` when the server stopped using Herdr Core (core-phase3 D3a).
    #[serde(skip_serializing_if = "Option::is_none")]
    core: Option<String>,
}

#[derive(Serialize)]
struct ServerCapabilitiesJson {
    live_handoff: bool,
    detached_server_daemon: bool,
    endpoint_protocol_generation: Option<u32>,
    surface_interest: bool,
    health_check: bool,
    ssh_agent_registration: bool,
}

#[derive(Serialize)]
struct UpdateStatusJson {
    restart_needed: Option<bool>,
    server_binary_stale: Option<bool>,
}

fn client_status_json() -> ClientStatusJson {
    ClientStatusJson {
        version: crate::build_info::version(),
        channel: crate::config::Config::load().config.update.channel.as_str(),
        protocol: crate::protocol::PROTOCOL_VERSION,
        endpoint_protocol_generation: crate::protocol::endpoint::ENDPOINT_PROTOCOL_GENERATION,
        endpoint_capabilities: vec![
            crate::protocol::endpoint::SURFACE_INTEREST_CAPABILITY,
            crate::protocol::endpoint::PRESENTATION_EFFECTS_FENCE_CAPABILITY,
            crate::protocol::endpoint::HEALTH_CHECK_CAPABILITY,
        ],
        remote_host_bridge: true,
        remote_bridge_idle_timeout: crate::platform::REMOTE_BRIDGE_IDLE_TIMEOUT_SUPPORTED,
        binary: current_exe_label(),
        session: crate::session::active_name(),
        core_session: Some(crate::pane::core_session_name()).filter(|name| !name.is_empty()),
    }
}

fn server_status_json(server: &ServerRuntimeStatus) -> ServerStatusJson {
    let mut status = match server {
        ServerRuntimeStatus::Running {
            version,
            protocol,
            capabilities,
            macos_services,
            core,
        } => ServerStatusJson {
            status: "running",
            running: true,
            version: version.clone(),
            protocol: *protocol,
            capabilities: capabilities
                .as_ref()
                .map(|capabilities| ServerCapabilitiesJson {
                    live_handoff: capabilities.live_handoff,
                    detached_server_daemon: capabilities.detached_server_daemon,
                    endpoint_protocol_generation: capabilities.endpoint_protocol_generation,
                    surface_interest: capabilities.surface_interest,
                    health_check: capabilities.health_check,
                    ssh_agent_registration: capabilities.ssh_agent_registration,
                }),
            compatible: protocol.map(|value| value == crate::protocol::PROTOCOL_VERSION),
            endpoint_compatible: capabilities.as_ref().and_then(|capabilities| {
                capabilities.endpoint_protocol_generation.map(|generation| {
                    generation == crate::protocol::endpoint::ENDPOINT_PROTOCOL_GENERATION
                })
            }),
            socket: api::socket_path().display().to_string(),
            session: crate::session::active_name(),
            restart_needed: restart_needed_bool(server),
            server_binary_stale: server_binary_stale_bool(server),
            macos_services: macos_services.clone(),
            core: core.clone(),
        },
        ServerRuntimeStatus::NotRunning => ServerStatusJson {
            status: "not_running",
            running: false,
            version: None,
            protocol: None,
            capabilities: None,
            compatible: None,
            endpoint_compatible: None,
            socket: api::socket_path().display().to_string(),
            session: crate::session::active_name(),
            restart_needed: Some(false),
            server_binary_stale: Some(false),
            macos_services: None,
            core: None,
        },
    };
    if let Some((_, session)) = super::target::remote_identity() {
        status.socket = super::target::socket_label();
        status.session = Some(session);
        status.server_binary_stale = None;
    }
    status
}

fn update_status_json(server: &ServerRuntimeStatus) -> UpdateStatusJson {
    UpdateStatusJson {
        restart_needed: restart_needed_bool(server),
        server_binary_stale: server_binary_stale_bool(server),
    }
}

fn restart_needed_bool(server: &ServerRuntimeStatus) -> Option<bool> {
    match server {
        ServerRuntimeStatus::Running { capabilities, .. } => Some(
            capabilities
                .as_ref()
                .and_then(|value| value.endpoint_protocol_generation)
                != Some(crate::protocol::endpoint::ENDPOINT_PROTOCOL_GENERATION),
        ),
        ServerRuntimeStatus::NotRunning => Some(false),
    }
}

fn server_binary_stale_bool(server: &ServerRuntimeStatus) -> Option<bool> {
    match server {
        ServerRuntimeStatus::Running { version, .. } => version
            .as_deref()
            .map(|version| version != crate::build_info::version()),
        ServerRuntimeStatus::NotRunning => Some(false),
    }
}

fn print_json(value: &impl Serialize) -> std::io::Result<()> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}

fn current_exe_label() -> String {
    std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|err| format!("unknown ({err})"))
}

fn print_status_help() {
    eprintln!("herdr status commands:");
    eprintln!("  herdr status [--json]         show local client and running server status");
    eprintln!("  herdr status server [--json]  show running server status");
    eprintln!("  herdr status client [--json]  show local client binary status");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running_server(
        version: Option<&str>,
        endpoint_generation: Option<u32>,
    ) -> ServerRuntimeStatus {
        ServerRuntimeStatus::Running {
            version: version.map(str::to_owned),
            protocol: Some(crate::protocol::PROTOCOL_VERSION),
            capabilities: Some(crate::api::schema::ServerCapabilities {
                live_handoff: true,
                detached_server_daemon: true,
                endpoint_protocol_generation: endpoint_generation,
                surface_interest: true,
                health_check: true,
                ssh_agent_registration: false,
                spawn_on_machine: false,
            }),
            macos_services: None,
            core: None,
        }
    }

    /// security review 2026-11-23 M1: a far server's version, core state
    /// and macOS services context show their controls as escapes.
    #[test]
    fn far_status_strings_print_their_controls_escaped() {
        let osc52 = "\x1b]52;c;aGk=\x07";
        let mut server = running_server(Some(&format!("0.9.3{osc52}")), None);
        if let ServerRuntimeStatus::Running {
            macos_services,
            core,
            ..
        } = &mut server
        {
            *macos_services = Some(crate::api::schema::MacosServicesInfo {
                context: "user\x1b]0;title\x1b\\".into(),
                reachable: true,
            });
            *core = Some("gave_up\x1b[2K\x1b[1A".into());
        }
        let lines = server_status_lines(&server, "machine:core:loop/default");
        for line in &lines {
            assert!(!line.chars().any(char::is_control), "{line:?}");
        }
        assert!(lines.contains(&"version: 0.9.3\\x1b]52;c;aGk=\\x07".to_string()));
        assert!(lines
            .contains(&"macos_services: user\\x1b]0;title\\x1b\\ context, reachable".to_string()));
        assert!(lines.contains(&"core: gave_up\\x1b[2K\\x1b[1A".to_string()));
    }

    #[test]
    fn status_exposes_ssh_agent_registration() {
        let server = running_server(Some("test"), None);
        let value = serde_json::to_value(server_status_json(&server)).unwrap();
        assert_eq!(value["capabilities"]["ssh_agent_registration"], false);
    }

    #[test]
    fn stale_compatible_server_does_not_require_restart() {
        let server = running_server(
            Some("0.0.0-old"),
            Some(crate::protocol::endpoint::ENDPOINT_PROTOCOL_GENERATION),
        );

        assert_eq!(restart_needed_bool(&server), Some(false));
        assert_eq!(server_binary_stale_bool(&server), Some(true));
    }

    #[test]
    fn server_without_endpoint_baseline_requires_restart() {
        let server = running_server(Some(crate::build_info::version().as_str()), None);

        assert_eq!(restart_needed_bool(&server), Some(true));
        assert_eq!(server_binary_stale_bool(&server), Some(false));
    }
}
