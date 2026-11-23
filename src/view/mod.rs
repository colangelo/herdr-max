//! The view server (openspec/changes/core-phase5, D6): the herdr binary in a
//! second role, one process per viewed core machine, that shows and drives
//! another Mac's herdr-max panes through the primary core's machine link.
//!
//! Its layout is the far side's `herdr-app` document, read through the
//! primary core (`doc.get {on}`); its panes are the far terminals, attached
//! so that the view never becomes their active viewer (D6 Size). It saves
//! nothing, publishes nothing, serves no JSON API and starts no program; a
//! TUI client connects to its own client socket as to any herdr server.
//! Unix only, as the core backend is.

pub(crate) mod doc;
pub(crate) mod lifetime;
pub(crate) mod start;
pub(crate) mod watch;

use std::path::PathBuf;
use std::sync::OnceLock;

use crate::core_machine::MachineName;

/// What a herdr server process is (D6). Read at the boundaries where the
/// two differ: socket, layout source, persistence, publishers, API, input,
/// close and lifetime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ServerRole {
    Primary,
    View(MachineName),
}

static ROLE: OnceLock<ServerRole> = OnceLock::new();

/// This process's role; `Primary` unless `herdr server --view` set it.
pub(crate) fn role() -> &'static ServerRole {
    ROLE.get_or_init(|| ServerRole::Primary)
}

/// The machine this process views, `None` for a primary server or a client.
pub(crate) fn viewed_machine() -> Option<&'static MachineName> {
    match role() {
        ServerRole::Primary => None,
        ServerRole::View(machine) => Some(machine),
    }
}

/// Sets the role once, at the start of `herdr server --view`; false when it
/// was already read or set.
pub(crate) fn set_role(role: ServerRole) -> bool {
    ROLE.set(role).is_ok()
}

/// The view server's client socket for `machine`, beside the herdr
/// session's own: one per viewed machine, so two views never share one.
pub(crate) fn socket_path(machine: &MachineName) -> PathBuf {
    crate::session::data_dir().join(format!("herdr-view-{machine}.sock"))
}

/// The log of the view server for `machine`, beside the session's own.
pub(crate) fn log_file_name(machine: &MachineName) -> String {
    format!("herdr-view-{machine}.log")
}

/// The configuration a view server runs with: the user's, with every core
/// publisher off (D6: no spaces, extras, status mirror or label writes; the
/// far herdr owns those documents and labels).
pub(crate) fn view_config(config: &mut crate::config::Config) {
    let experimental = &mut config.experimental;
    experimental.core_spaces_doc = false;
    experimental.core_extras_doc = false;
    experimental.core_status_mirror = false;
    experimental.core_label_sync = false;
}

/// `--view <machine>` after `server`: the machine, `Ok(None)` without the
/// flag.
pub(crate) fn parse_view_flag(args: &[String]) -> Result<Option<MachineName>, String> {
    match args {
        [flag, machine, ..] if flag == "--view" => MachineName::parse(machine)
            .map(Some)
            .map_err(|err| err.to_string()),
        [flag] if flag == "--view" => Err("usage: herdr server --view <machine>".into()),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(raw: &[&str]) -> Vec<String> {
        raw.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn the_view_flag_names_a_machine() {
        assert_eq!(
            parse_view_flag(&args(&["--view", "ac-mbm5"])).unwrap(),
            Some(MachineName::parse("ac-mbm5").unwrap())
        );
        assert_eq!(parse_view_flag(&args(&[])).unwrap(), None);
        assert_eq!(parse_view_flag(&args(&["--handoff-import"])).unwrap(), None);
        assert!(parse_view_flag(&args(&["--view"])).is_err());
        assert!(parse_view_flag(&args(&["--view", "../x"])).is_err());
    }

    #[test]
    fn each_machine_has_its_own_socket_and_log() {
        let a = MachineName::parse("ac-mbm5").unwrap();
        let b = MachineName::parse("m4m").unwrap();
        assert_ne!(socket_path(&a), socket_path(&b));
        assert!(socket_path(&a).ends_with("herdr-view-ac-mbm5.sock"));
        assert_eq!(log_file_name(&b), "herdr-view-m4m.log");
        assert_ne!(
            socket_path(&a),
            crate::server::socket_paths::client_socket_path()
        );
    }
}
