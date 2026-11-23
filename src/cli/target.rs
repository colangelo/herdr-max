use std::cell::RefCell;
use std::io;

use crate::api::client::{ApiClient, ConnectionTarget};
use crate::client::endpoint::{EndpointCatalog, SavedSshEndpoint};

thread_local! {
    // CLI dispatch is synchronous. Scope routing to this command, never the runtime or TUI.
    static TARGET: RefCell<Option<MachineTarget>> = const { RefCell::new(None) };
}

struct MachineTarget {
    route: Route,
    #[cfg(test)]
    client_override: Option<ApiClient>,
}

/// How `--machine` reaches the far herdr: a saved SSH machine (the old
/// remote), or a Herdr Core machine through `machine.exec` (core-phase5 D7).
// One value per CLI call, held in a thread-local: the saved route's size
// costs nothing, so it stays unboxed as it was before the core route.
#[allow(clippy::large_enum_variant)]
enum Route {
    Saved {
        profile: SavedSshEndpoint,
        bridge: Option<crate::remote::SavedSshApiBridge>,
    },
    #[cfg(unix)]
    Core(Box<crate::core_machine::fleet::FleetLink>),
}

impl MachineTarget {
    fn label(&self) -> String {
        match &self.route {
            Route::Saved { profile, .. } => profile.label.clone(),
            #[cfg(unix)]
            Route::Core(link) => link.machine().to_string(),
        }
    }

    fn session(&self) -> String {
        match &self.route {
            Route::Saved { profile, .. } => profile.session.clone(),
            #[cfg(unix)]
            Route::Core(link) => link.far_session().to_string(),
        }
    }

    /// The saved profile's id, or `core:<machine>`.
    fn id(&self) -> String {
        match &self.route {
            Route::Saved { profile, .. } => profile.id.to_string(),
            #[cfg(unix)]
            Route::Core(link) => format!("core:{}", link.machine()),
        }
    }

    /// `text` as printed: escaped for a core route, unchanged for a saved
    /// SSH one.
    fn far_text(&self, text: &str) -> String {
        match &self.route {
            Route::Saved { .. } => text.to_string(),
            #[cfg(unix)]
            Route::Core(_) => crate::remote::far_text(text),
        }
    }

    fn reported_failure(&self) -> Option<io::Error> {
        match &self.route {
            Route::Saved { bridge, .. } => {
                bridge.as_ref().and_then(|bridge| bridge.reported_failure())
            }
            #[cfg(unix)]
            Route::Core(link) => link.reported_failure(),
        }
    }
}

struct TargetScope(Option<MachineTarget>);

impl Drop for TargetScope {
    fn drop(&mut self) {
        TARGET.with(|target| *target.borrow_mut() = self.0.take());
    }
}

pub(super) fn maybe_run(args: &[String]) -> Option<io::Result<super::CommandOutcome>> {
    let MachinePrefix {
        selector,
        dial,
        args,
    } = match parse_machine_prefix(args) {
        Ok(Some(target)) => target,
        Ok(None) => return None,
        Err(error) => return Some(usage_error(error)),
    };
    Some((|| {
        if let Err(error) = validate_machine_command(&args) {
            return usage_error(error);
        }
        if super::spec::print_requested_help(&args)? {
            return Ok(super::CommandOutcome::Handled(0));
        }
        let profiles = EndpointCatalog::load_profiles().map_err(io::Error::other)?;
        #[cfg(unix)]
        let core_machines: Vec<String> = crate::core_machine::command::linked_machines_of_primary()
            .into_iter()
            .map(|machine| machine.name.to_string())
            .collect();
        #[cfg(not(unix))]
        let core_machines: Vec<String> = Vec::new();
        let prefer = crate::config::Config::load().config.remote.prefer;
        // A saved herdr machine first, unchanged (core-phase5 D8), unless a
        // Herdr Core machine shares its name and `[remote] prefer` says
        // core; a name no saved machine has may be a Herdr Core machine
        // (D7).
        let route = match route_machine(&profiles, &core_machines, &selector, prefer) {
            Ok(MachineRoute::Saved(profile)) => Route::Saved {
                profile: profile.clone(),
                bridge: None,
            },
            Ok(MachineRoute::Core(name)) => {
                let unknown = format!("unknown machine '{name}'; use `herdr machine list`");
                match core_route(&selector, dial, unknown) {
                    Ok(route) => route,
                    Err(outcome) => return outcome,
                }
            }
            Err(error) if names_a_saved_machine(&profiles, &selector) => return usage_error(error),
            Err(error) => match core_route(&selector, dial, error) {
                Ok(route) => route,
                Err(outcome) => return outcome,
            },
        };
        let _scope = TARGET.with(|target| {
            TargetScope(target.replace(Some(MachineTarget {
                route,
                #[cfg(test)]
                client_override: None,
            })))
        });
        super::maybe_run(&args)
    })())
}

fn names_a_saved_machine(profiles: &[SavedSshEndpoint], selector: &str) -> bool {
    profiles
        .iter()
        .any(|profile| profile.id.as_str() == selector || profile.label == selector)
}

/// `selector` as a Herdr Core machine: refused (exit 75 on a machine that
/// is not connected, without `--dial`) before any request, or ready.
#[cfg(unix)]
fn core_route(
    selector: &str,
    dial: bool,
    unknown: String,
) -> Result<Route, io::Result<super::CommandOutcome>> {
    use crate::core_machine::fleet::{self, Lookup};
    match fleet::open(selector, dial) {
        Lookup::NotCore => Err(usage_error(format!(
            "{unknown}, or `herdr machine status --core` for the Herdr Core machines added with \
             `herdr machine add --core`"
        ))),
        Lookup::Refused { message, code } => {
            eprintln!("{}", refusal_line(&message));
            Err(Ok(super::CommandOutcome::Handled(code)))
        }
        Lookup::Ready(link) => Ok(Route::Core(link)),
    }
}

/// A core route's refusal as printed. It can carry far text, the probe's
/// stderr or the far herdr's session (security review 2026-11-23 M1).
#[cfg(unix)]
fn refusal_line(message: &str) -> String {
    format!("error: {}", crate::remote::far_line(message))
}

#[cfg(not(unix))]
fn core_route(
    _selector: &str,
    _dial: bool,
    unknown: String,
) -> Result<Route, io::Result<super::CommandOutcome>> {
    Err(usage_error(unknown))
}

fn usage_error(error: String) -> io::Result<super::CommandOutcome> {
    eprintln!("error: {error}");
    Ok(super::CommandOutcome::Handled(2))
}

pub(super) fn is_remote() -> bool {
    TARGET.with(|target| target.borrow().is_some())
}

/// A far string as printed by a human-readable command: escaped when the
/// target is remote (`--machine`), untouched otherwise. JSON output and
/// explicit pane reads do not go through this.
pub(super) fn shown(text: &str) -> std::borrow::Cow<'_, str> {
    if is_remote() {
        crate::remote::visible_line(text)
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

/// `shown` for a multi-line far string: each line escaped, real newlines kept.
pub(super) fn shown_lines(text: &str) -> String {
    text.split('\n').map(shown).collect::<Vec<_>>().join("\n")
}

pub(super) fn api_client() -> io::Result<ApiClient> {
    TARGET.with(|target| {
        let mut target = target.borrow_mut();
        let Some(target) = target.as_mut() else {
            return Ok(ApiClient::local());
        };
        #[cfg(test)]
        if let Some(client) = &target.client_override {
            return Ok(client.clone());
        }
        let (profile, bridge) = match &mut target.route {
            Route::Saved { profile, bridge } => (profile, bridge),
            #[cfg(unix)]
            Route::Core(link) => {
                return Ok(ApiClient::for_target(ConnectionTarget::SocketPath(
                    link.socket_path().to_owned(),
                )))
            }
        };
        if bridge.is_none() {
            *bridge = Some(
                crate::remote::SavedSshApiBridge::start(
                    profile.id.as_str(),
                    &profile.target,
                    &profile.session,
                    true,
                    true,
                )
                .map_err(|error| {
                    io::Error::new(
                        error.kind(),
                        format!("machine '{}': {error}", profile.label),
                    )
                })?,
            );
        }
        let bridge = bridge
            .as_ref()
            .ok_or_else(|| io::Error::other("machine bridge unavailable"))?;
        Ok(ApiClient::for_target(ConnectionTarget::SocketPath(
            bridge.socket_path().to_owned(),
        )))
    })
}

/// How long the status probe of a machine waits for its answer. A core
/// machine's bridge gives the probe's run less than this (D7), so it is
/// done when the probe gives up.
const REMOTE_STATUS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

pub(super) fn server_status(
    client: &ApiClient,
) -> Result<crate::api::RuntimeStatus, crate::api::client::ApiClientError> {
    let probe = || {
        if is_remote() {
            client.status_with_timeout(REMOTE_STATUS_TIMEOUT)
        } else {
            client.status()
        }
    };
    let error = match probe() {
        Ok(status) => return Ok(status),
        Err(error) => error,
    };
    // Only this read-only probe may rediscover and retry. Requests that follow
    // the probe must never be replayed after an ambiguous SSH failure.
    TARGET.with(|target| {
        let mut target = target.borrow_mut();
        let Some(target) = target.as_mut() else {
            return Err(error);
        };
        // A core machine's probe is a request like any other: never sent
        // twice (core-phase5 D7).
        let (profile, saved_bridge) = match &mut target.route {
            Route::Saved { profile, bridge } => (profile, bridge),
            #[cfg(unix)]
            Route::Core(link) => {
                return Err(link.reported_failure().map(Into::into).unwrap_or(error))
            }
        };
        let Some(bridge) = saved_bridge.as_ref() else {
            return Err(error);
        };
        let failure = bridge.reported_failure();
        let stale_metadata = bridge.used_cached_metadata
            && failure
                .as_ref()
                .is_some_and(crate::remote::SavedSshApiBridge::stale_metadata_failure);
        if !bridge.can_retry || (!stale_metadata && !bridge.recover_transport()) {
            return Err(failure.map(Into::into).unwrap_or(error));
        }
        if stale_metadata {
            bridge.invalidate_metadata();
        }
        saved_bridge.take();
        *saved_bridge = Some(crate::remote::SavedSshApiBridge::start(
            profile.id.as_str(),
            &profile.target,
            &profile.session,
            !stale_metadata,
            false,
        )?);
        Ok(())
    })?;
    probe()
}

pub(super) fn remote_error(error: io::Error) -> io::Error {
    TARGET.with(|target| {
        let target = target.borrow();
        let Some(target) = target.as_ref() else {
            return error;
        };
        let error = target.reported_failure().unwrap_or(error);
        // A core machine's far server message and far stderr are far text,
        // escaped here, once, per line (security review 2026-11-23 M1). A
        // saved-SSH machine's error prints as it always did.
        let text = target.far_text(&error.to_string());
        io::Error::new(
            error.kind(),
            format!(
                "machine '{}' (session {}): {text}",
                target.label(),
                target.session()
            ),
        )
    })
}

pub(super) fn restart_guidance() -> String {
    TARGET.with(|target| match target.borrow().as_ref() {
        Some(target) => format!("Update Herdr and restart the server on machine '{}' (session {}). Stopping the server exits its pane processes.", target.label(), target.session()),
        None => crate::session::active_restart_after_update_guidance(),
    })
}

pub(super) fn remote_identity() -> Option<(String, String)> {
    TARGET.with(|target| {
        target
            .borrow()
            .as_ref()
            .map(|target| (target.id(), target.session()))
    })
}

pub(super) fn socket_label() -> String {
    match remote_identity() {
        Some((id, session)) => format!("machine:{id}/{session}"),
        None => crate::api::socket_path().display().to_string(),
    }
}

pub(super) fn remote_path_is_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with('/')
        || path.starts_with("\\\\")
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\'))
}

pub(super) fn caller_pane_id() -> Option<String> {
    if is_remote() {
        return None;
    }
    std::env::var("HERDR_PANE_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

/// `herdr --machine <selector> [--dial] <command>`, split.
#[derive(Debug, PartialEq, Eq)]
struct MachinePrefix {
    selector: String,
    /// `--dial`: a core machine that is not connected may be dialled
    /// (core-phase5 D7). A saved SSH machine dials on every call anyway.
    dial: bool,
    args: Vec<String>,
}

fn parse_machine_prefix(args: &[String]) -> Result<Option<MachinePrefix>, String> {
    let mut index = 1;
    let mut machine = None;
    let mut dial = false;
    let mut other_prefix = false;
    while let Some(arg) = args.get(index) {
        if arg == "--dial" {
            if dial {
                return Err("--dial can only be specified once".into());
            }
            dial = true;
        } else if arg == "--machine" || arg.starts_with("--machine=") {
            if machine.is_some() {
                return Err("--machine can only be specified once".into());
            }
            let value = if let Some(value) = arg.strip_prefix("--machine=") {
                value.to_owned()
            } else {
                index += 1;
                args.get(index)
                    .cloned()
                    .ok_or("missing value for --machine")?
            };
            if value.trim().is_empty() || value.starts_with('-') {
                return Err("--machine requires a saved machine label or profile ID".into());
            }
            machine = Some(value);
        } else if arg.starts_with('-') && arg != "--" {
            other_prefix = true;
            if matches!(
                arg.as_str(),
                "--session" | "--remote" | "--remote-keybindings"
            ) {
                index += 1;
            }
        } else {
            break;
        }
        index += 1;
    }
    let Some(machine) = machine else {
        if dial {
            return Err("--dial applies only with --machine <name>".into());
        }
        return Ok(None);
    };
    if other_prefix {
        return Err("--machine cannot be combined with other launch options; it uses the saved machine's session".into());
    }
    if index >= args.len() || args[index] == "--" {
        return Err("usage: herdr --machine <label-or-id> [--dial] <command>".into());
    }
    let mut cleaned = vec![args[0].clone()];
    cleaned.extend_from_slice(&args[index..]);
    Ok(Some(MachinePrefix {
        selector: machine,
        dial,
        args: cleaned,
    }))
}

/// Which path `--machine <selector>` takes (core-phase5 D7, D8).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum MachineRoute<'a> {
    /// A saved herdr machine, through herdr's own ssh (the old remote).
    Saved(&'a SavedSshEndpoint),
    /// A Herdr Core machine, through the primary core's link.
    Core(String),
}

/// A saved herdr machine is resolved first, unchanged; a name that is only
/// a core machine goes through the core link; when both share the name,
/// `[remote] prefer` decides (D8).
pub(super) fn route_machine<'a>(
    profiles: &'a [SavedSshEndpoint],
    core_machines: &[String],
    selector: &str,
    prefer: crate::config::RemotePrefer,
) -> Result<MachineRoute<'a>, String> {
    let core = core_machines.iter().any(|name| name == selector);
    match (resolve_machine(profiles, selector), core, prefer) {
        (Ok(profile), false, _) | (Ok(profile), true, crate::config::RemotePrefer::Ssh) => {
            Ok(MachineRoute::Saved(profile))
        }
        (Ok(_), true, crate::config::RemotePrefer::Core) | (Err(_), true, _) => {
            Ok(MachineRoute::Core(selector.to_string()))
        }
        (Err(error), false, _) => Err(error),
    }
}

pub(super) fn resolve_machine<'a>(
    profiles: &'a [SavedSshEndpoint],
    selector: &str,
) -> Result<&'a SavedSshEndpoint, String> {
    let profile = if let Some(profile) = profiles
        .iter()
        .find(|profile| profile.id.as_str() == selector)
    {
        profile
    } else {
        let mut matches = profiles.iter().filter(|profile| profile.label == selector);
        let profile = matches
            .next()
            .ok_or_else(|| format!("unknown machine '{selector}'; use `herdr machine list`"))?;
        if matches.next().is_some() {
            return Err(format!(
                "machine label '{selector}' is ambiguous; use its profile ID"
            ));
        }
        profile
    };
    if !profile.enabled {
        return Err(format!("machine '{selector}' is disabled"));
    }
    Ok(profile)
}

fn validate_machine_command(args: &[String]) -> Result<(), String> {
    let command = args.get(1).map(String::as_str).unwrap_or_default();
    let subcommand = args.get(2).map(String::as_str).unwrap_or_default();
    let supported = match command {
        "workspace" | "worktree" | "tab" | "pane" | "notification" | "popup" => true,
        "agent" => {
            subcommand != "attach"
                && !(subcommand == "explain"
                    && args[3..]
                        .iter()
                        .any(|arg| arg == "--file" || arg.starts_with("--file=")))
        }
        "api" => subcommand == "snapshot",
        "client" => subcommand == "list",
        "status" => subcommand == "server",
        "plugin" => matches!(
            subcommand,
            "link" | "unlink" | "enable" | "disable" | "list" | "action" | "log" | "logs" | "pane"
        ),
        "server" => matches!(
            subcommand,
            "stop" | "reload-config" | "agent-manifests" | "reload-agent-manifests"
        ),
        _ => false,
    };
    if supported {
        Ok(())
    } else {
        Err(format!("`{command} {subcommand}` is not an API-backed machine command; --machine does not run local management commands or attach a TUI"))
    }
}

/// A far string with an OSC 52 clipboard write, a CSI, a C1 control and a
/// bidi override, for the tests of every human-readable output (#311).
#[cfg(test)]
pub(super) const FAR_CONTROLS: &str = "x\x1b]52;c;aGk=\x07\x1b[2K\u{9b}1A\u{202e}y";

/// `with_test_client` around `run`, for a remote target.
#[cfg(test)]
pub(super) fn as_remote<T>(run: impl FnOnce() -> T) -> T {
    let client = ApiClient::for_target(ConnectionTarget::SocketPath("/nonexistent".into()));
    with_test_client(client, run)
}

/// The far text as it must print for a remote target, and no control left.
#[cfg(test)]
pub(super) fn assert_escaped(printed: &str) {
    assert!(
        printed.contains(crate::remote::visible_line(FAR_CONTROLS).as_ref()),
        "far text not escaped: {printed:?}"
    );
    assert!(
        !printed
            .chars()
            .any(|ch| (ch.is_control() && ch != '\n') || ch == '\u{202e}'),
        "a control survived: {printed:?}"
    );
}

#[cfg(test)]
pub(super) fn with_test_client<T>(client: ApiClient, run: impl FnOnce() -> T) -> T {
    let _scope = TARGET.with(|target| {
        TargetScope(
            target.replace(Some(MachineTarget {
                route: Route::Saved {
                    profile: SavedSshEndpoint::new("test-machine", "unused", "remote-session")
                        .expect("valid test profile"),
                    bridge: None,
                },
                client_override: Some(client),
            })),
        )
    });
    run()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 3.1: a core machine's status probe runs, and its core call ends,
    /// before the CLI's probe gives up, so the bridge is not left waiting
    /// on a request the CLI abandoned.
    #[cfg(unix)]
    #[test]
    fn a_core_machines_probe_ends_before_the_cli_gives_up() {
        use crate::core_machine::fleet::{CALL_MARGIN_SECS, PING_EXEC_SECS};
        assert!(
            std::time::Duration::from_secs(PING_EXEC_SECS + CALL_MARGIN_SECS)
                < REMOTE_STATUS_TIMEOUT
        );
    }

    /// security review 2026-11-23 M1: far text in a core refusal prints
    /// escaped (#310).
    #[cfg(unix)]
    #[test]
    fn far_errors_print_their_controls_escaped() {
        let far = "boom\x1b]52;c;aGk=\x07\x1b]0;t\x1b\\\x1b[2K\u{9b}1A";
        let shown = "boom\\x1b]52;c;aGk=\\x07\\x1b]0;t\\x1b\\\\\\x1b[2K\\u{9b}1A";
        assert_eq!(refusal_line(far), format!("error: {shown}"));
    }

    /// An old saved-SSH machine's error prints as it always did, its lines
    /// kept (#310).
    #[test]
    fn a_saved_machines_error_prints_as_before() {
        let client = ApiClient::for_target(ConnectionTarget::SocketPath("/nonexistent".into()));
        let ssh = "ssh: connect to host x port 22: refused\r\nsecond line\nthird line";
        let error = with_test_client(client, || remote_error(io::Error::other(ssh)));
        assert_eq!(
            error.to_string(),
            format!("machine 'test-machine' (session remote-session): {ssh}")
        );
    }

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).into()).collect()
    }

    // V22 (core-phase5 D8): a saved machine first, unchanged; a name that is
    // only a core machine goes through the core link; both: `prefer` says.
    #[test]
    fn prefer_decides_only_when_a_saved_and_a_core_machine_share_the_name() {
        use crate::config::RemotePrefer::{Core, Ssh};
        let saved = SavedSshEndpoint::new("ac-mbm5", "ac@ac-mbm5", "default").unwrap();
        let other = SavedSshEndpoint::new("vm1", "ac@vm1", "default").unwrap();
        let profiles = vec![saved.clone(), other.clone()];
        let core = vec!["ac-mbm5".to_string(), "lab".to_string()];
        assert_eq!(
            route_machine(&profiles, &core, "ac-mbm5", Ssh),
            Ok(MachineRoute::Saved(&profiles[0])),
            "the default keeps the old remote"
        );
        assert_eq!(
            route_machine(&profiles, &core, "ac-mbm5", Core),
            Ok(MachineRoute::Core("ac-mbm5".into()))
        );
        for prefer in [Ssh, Core] {
            assert_eq!(
                route_machine(&profiles, &core, "vm1", prefer),
                Ok(MachineRoute::Saved(&profiles[1])),
                "a saved machine only"
            );
            assert_eq!(
                route_machine(&profiles, &core, "lab", prefer),
                Ok(MachineRoute::Core("lab".into())),
                "a core machine only"
            );
            assert!(route_machine(&profiles, &core, "nowhere", prefer).is_err());
        }
        assert_eq!(
            route_machine(&profiles, &core, saved.id.as_str(), Core),
            Ok(MachineRoute::Saved(&profiles[0])),
            "a profile id is no core machine's name"
        );
        assert_eq!(crate::config::Config::default().remote.prefer, Ssh);
        let config: crate::config::Config =
            toml::from_str("[remote]\nprefer = \"core\"\n").unwrap();
        assert_eq!(config.remote.prefer, Core);
    }

    #[test]
    fn machine_prefix_routes_without_consuming_command_payload() {
        for prefix in [args(&["--machine", "mac"]), args(&["--machine=mac"])] {
            let mut input = args(&["herdr"]);
            input.extend(prefix);
            input.extend(args(&["agent", "prompt", "w4:p1", "--machine"]));
            assert_eq!(
                parse_machine_prefix(&input).unwrap(),
                Some(MachinePrefix {
                    selector: "mac".into(),
                    dial: false,
                    args: args(&["herdr", "agent", "prompt", "w4:p1", "--machine"])
                })
            );
        }
        // `--dial` is a prefix flag only: after the command it is the
        // command's own.
        for input in [
            args(&[
                "herdr",
                "--machine",
                "m",
                "--dial",
                "pane",
                "list",
                "--dial",
            ]),
            args(&["herdr", "--dial", "--machine=m", "pane", "list", "--dial"]),
        ] {
            assert_eq!(
                parse_machine_prefix(&input).unwrap(),
                Some(MachinePrefix {
                    selector: "m".into(),
                    dial: true,
                    args: args(&["herdr", "pane", "list", "--dial"])
                })
            );
        }
        assert_eq!(
            parse_machine_prefix(&args(&[
                "herdr",
                "agent",
                "prompt",
                "w4:p1",
                "--machine=mac"
            ]))
            .unwrap(),
            None
        );
    }

    #[test]
    fn machine_prefix_rejects_missing_target_and_conflicting_global_options() {
        for input in [
            args(&["herdr", "--machine"]),
            args(&["herdr", "--machine="]),
            args(&["herdr", "--machine", "--help"]),
            args(&["herdr", "--machine", "mac"]),
            args(&[
                "herdr",
                "--machine",
                "mac",
                "--machine",
                "other",
                "agent",
                "list",
            ]),
            args(&[
                "herdr",
                "--machine",
                "mac",
                "--session",
                "other",
                "agent",
                "list",
            ]),
            args(&[
                "herdr",
                "--session",
                "other",
                "--machine",
                "mac",
                "agent",
                "list",
            ]),
            args(&[
                "herdr",
                "--remote",
                "other",
                "--machine",
                "mac",
                "agent",
                "list",
            ]),
            args(&["herdr", "--dial", "agent", "list"]),
            args(&[
                "herdr",
                "--machine",
                "m",
                "--dial",
                "--dial",
                "agent",
                "list",
            ]),
        ] {
            assert!(parse_machine_prefix(&input).is_err(), "{input:?}");
        }
    }

    #[test]
    fn machine_resolution_requires_a_unique_enabled_saved_machine() {
        let mac = SavedSshEndpoint::new("mac", "mac-ssh", "agents").unwrap();
        let other = SavedSshEndpoint::new("build", "builder", "default").unwrap();
        let profiles = vec![mac.clone(), other];
        assert_eq!(resolve_machine(&profiles, "mac").unwrap(), &mac);
        assert_eq!(resolve_machine(&profiles, mac.id.as_str()).unwrap(), &mac);
        let shadow = SavedSshEndpoint::new(mac.id.as_str(), "shadow", "default").unwrap();
        assert_eq!(
            resolve_machine(&[mac.clone(), shadow], mac.id.as_str()).unwrap(),
            &mac
        );
        assert!(resolve_machine(&profiles, "mac-ssh").is_err());
        assert!(resolve_machine(&profiles, "missing").is_err());
        let duplicate = SavedSshEndpoint::new("mac", "other", "default").unwrap();
        assert!(resolve_machine(&[mac.clone(), duplicate], "mac").is_err());
        let mut disabled = mac;
        disabled.enabled = false;
        assert!(resolve_machine(&[disabled], "mac").is_err());
    }

    #[test]
    fn machine_commands_reject_local_side_effects_and_tui_attach() {
        for command in [
            &["update"][..],
            &["config", "reset-keys"],
            &["machine", "remove", "mac"],
            &["session", "delete", "default"],
            &["server", "live-handoff"],
            &["agent", "attach", "w4:p1"],
            &["terminal", "attach", "w4:p1"],
            &["terminal", "session", "control", "w4:p1"],
            &["plugin", "install", "./plugin"],
            &["integration", "install", "pi"],
            &["api", "schema", "--output", "schema.json"],
            &["status", "client"],
            &["client"],
        ] {
            let mut input = args(&["herdr"]);
            input.extend(args(command));
            assert!(validate_machine_command(&input).is_err(), "{input:?}");
        }
        for command in [
            &["agent", "list"][..],
            &["agent", "wait", "w4:p1"],
            &["pane", "split", "w4:p1", "--direction", "right"],
            &["workspace", "list"],
            &["worktree", "create", "--branch", "feature"],
            &["tab", "list"],
            &["api", "snapshot"],
            &["client", "list"],
            &["server", "stop"],
        ] {
            let mut input = args(&["herdr"]);
            input.extend(args(command));
            assert!(validate_machine_command(&input).is_ok(), "{input:?}");
        }
    }
}
