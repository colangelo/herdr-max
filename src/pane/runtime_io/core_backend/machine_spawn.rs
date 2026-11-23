//! A pane herdr starts on a core machine (core-phase5 D5): what goes to
//! `terminal.run {on}`.
//!
//! A terminal on a machine inherits the far core's environment, and
//! `terminal.run`'s `env` can override a variable but not unset it (R6). The
//! far core may carry a herdr pane's integration variables (one started by a
//! bare `hc` inside a pane there), and the `herdr` on the far `PATH` is that
//! Mac's own server: a hook report with them would land on one of its panes.
//! So every argv on a machine starts with `env -u` for each of them, and
//! none is passed.

use serde_json::{Map, Value};

use super::pane_io::CoreSpawn;
use crate::core_machine::MachineName;
use crate::layout::PaneId;

/// The herdr integration variables a program on a machine never sees.
pub(crate) const MACHINE_UNSET_VARS: [&str; 6] = [
    "HERDR_ENV",
    "HERDR_PANE_ID",
    "HERDR_TAB_ID",
    "HERDR_WORKSPACE_ID",
    "HERDR_SOCKET_PATH",
    "HERDR_CLIENT_SOCKET_PATH",
];

/// The variables of herdr's own a pane on a machine gets: its terminal's
/// identity. The rest of this Mac's environment (its `PATH`, `HOME`, the
/// server's tokens) describes this Mac, not the far one.
const MACHINE_TERMINAL_VARS: [&str; 4] =
    ["TERM", "COLORTERM", "TERM_PROGRAM", "TERM_PROGRAM_VERSION"];

/// The machine's own login shell, through `sh` because its path is the
/// far side's to know (D5).
const MACHINE_SHELL: &str = r#"exec "${SHELL:-/bin/sh}" -l"#;

/// What a pane on a machine runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MachineProgram {
    /// The machine's login shell.
    Shell,
    /// A shell line, run by the machine's `sh`.
    Line(String),
    /// A program and its arguments, found on the machine's `PATH`.
    Argv(Vec<String>),
}

/// Where and what: the pane's machine, its program, and what the request
/// named for it. Nothing of this Mac's own (its folders, its shell) goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MachineSpawn {
    pub machine: MachineName,
    pub program: MachineProgram,
    /// The folder on the machine, when the request named one; else the far
    /// core's own.
    pub cwd: Option<String>,
    /// The request's own variables (`pane.split {env}`), less any herdr
    /// integration variable.
    pub env: Vec<(String, String)>,
}

/// Herdr Core's own variables (`HC_SESSION`, `HC_SOCKET`, ...): on the far
/// side they would point an `hc` there at a session of the request's
/// choosing, so a request's env never carries one to a machine (security
/// review M8). The far core sets its own for its terminals.
fn is_core_variable(key: &str) -> bool {
    key.starts_with("HC_")
}

/// `env -u NAME ... --`, the wrapper every argv on a machine starts with.
pub(crate) fn machine_env_wrapper() -> Vec<String> {
    let mut argv = vec!["/usr/bin/env".to_string()];
    for name in MACHINE_UNSET_VARS {
        argv.push("-u".into());
        argv.push(name.into());
    }
    argv.push("--".into());
    argv
}

/// The argv `terminal.run` gets for `program` on a machine (D5).
pub(crate) fn machine_argv(program: &MachineProgram) -> Vec<String> {
    let mut argv = machine_env_wrapper();
    match program {
        MachineProgram::Shell => {
            argv.extend(["/bin/sh".into(), "-c".into(), MACHINE_SHELL.into()]);
        }
        MachineProgram::Line(line) => {
            argv.extend(["/bin/sh".into(), "-c".into(), line.clone()]);
        }
        MachineProgram::Argv(program) => argv.extend(program.iter().cloned()),
    }
    argv
}

/// The spawn for a pane on `spawn.machine`. Its labels name the pane as a
/// local one would; the core backend makes `herdr.session` the owner value
/// once it knows the core's id. `None` for an empty argv.
pub(crate) fn machine_spawn_from(
    pane_id: PaneId,
    command: &portable_pty::CommandBuilder,
    spawn: &MachineSpawn,
    rows: u16,
    cols: u16,
) -> Option<CoreSpawn> {
    if matches!(&spawn.program, MachineProgram::Argv(argv) if argv.is_empty()) {
        return None;
    }
    let mut env = Map::new();
    for (key, value) in command.iter_full_env_as_str() {
        if MACHINE_TERMINAL_VARS.contains(&key) {
            env.insert(key.to_string(), Value::String(value.to_string()));
        }
    }
    for (key, value) in &spawn.env {
        if !MACHINE_UNSET_VARS.contains(&key.as_str()) && !is_core_variable(key) {
            env.insert(key.clone(), Value::String(value.clone()));
        }
    }
    let public_pane_id = command
        .get_env(crate::integration::HERDR_PANE_ID_ENV_VAR)
        .and_then(|value| value.to_str());
    let mut labels = super::pane_labels(public_pane_id);
    if super::labels::enabled() {
        labels.insert("herdr.seen".into(), Value::String("0".into()));
    }
    Some(CoreSpawn {
        pane_id,
        session: super::core_session_name(),
        on: Some(spawn.machine.clone()),
        argv: machine_argv(&spawn.program),
        cwd: spawn.cwd.clone(),
        env,
        labels,
        rows,
        cols,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawn_of(program: MachineProgram) -> MachineSpawn {
        MachineSpawn {
            machine: MachineName::parse("loop").expect("name"),
            program,
            cwd: None,
            env: vec![
                ("HERDR_RUN_ID".into(), "r1".into()),
                ("HERDR_PANE_ID".into(), "forged".into()),
                ("HC_SESSION".into(), "another-session".into()),
                ("HC_SOCKET".into(), "/tmp/another.sock".into()),
            ],
        }
    }

    /// A command as herdr builds one for a local pane: its whole
    /// environment, with the integration variables set.
    fn local_command() -> portable_pty::CommandBuilder {
        let mut command = portable_pty::CommandBuilder::new("/bin/zsh");
        command.env_clear();
        command.env("PATH", "/opt/homebrew/bin:/usr/bin");
        command.env("HOME", "/Users/here");
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("HERDR_ENV", "1");
        command.env("HERDR_PANE_ID", "w1:p2");
        command.env("HERDR_SOCKET_PATH", "/tmp/herdr.sock");
        command
    }

    #[test]
    fn the_integration_variable_names_are_herdrs_own() {
        assert_eq!(MACHINE_UNSET_VARS[0], crate::HERDR_ENV_VAR);
        assert!(MACHINE_UNSET_VARS.contains(&crate::integration::HERDR_PANE_ID_ENV_VAR));
        assert!(MACHINE_UNSET_VARS.contains(&crate::integration::HERDR_TAB_ID_ENV_VAR));
        assert!(MACHINE_UNSET_VARS.contains(&crate::integration::HERDR_WORKSPACE_ID_ENV_VAR));
    }

    // V9 (D5, F6): every form starts with the wrapper, which unsets each of
    // the six names, and ends the options before the program.
    #[test]
    fn every_program_on_a_machine_starts_with_the_env_wrapper() {
        let wrapper = machine_env_wrapper();
        assert_eq!(
            wrapper,
            [
                "/usr/bin/env",
                "-u",
                "HERDR_ENV",
                "-u",
                "HERDR_PANE_ID",
                "-u",
                "HERDR_TAB_ID",
                "-u",
                "HERDR_WORKSPACE_ID",
                "-u",
                "HERDR_SOCKET_PATH",
                "-u",
                "HERDR_CLIENT_SOCKET_PATH",
                "--"
            ]
        );
        let cases = [
            (
                MachineProgram::Shell,
                vec!["/bin/sh", "-c", r#"exec "${SHELL:-/bin/sh}" -l"#],
            ),
            (
                MachineProgram::Line("echo $HOME; ls".into()),
                vec!["/bin/sh", "-c", "echo $HOME; ls"],
            ),
            (
                MachineProgram::Argv(vec!["claude".into(), "--name".into(), "a b".into()]),
                vec!["claude", "--name", "a b"],
            ),
        ];
        for (program, tail) in cases {
            let argv = machine_argv(&program);
            assert_eq!(argv[..wrapper.len()], wrapper[..], "{program:?}");
            assert_eq!(argv[wrapper.len()..], tail[..], "{program:?}");
        }
    }

    // V9: no herdr integration variable and nothing of this Mac's
    // environment goes; the request's own variables and the terminal's
    // identity do, less Herdr Core's own (security review M8); the labels
    // name the pane.
    #[test]
    fn a_machine_spawn_passes_no_integration_variable_and_no_local_environment() {
        let spawn = machine_spawn_from(
            PaneId::from_raw(3),
            &local_command(),
            &spawn_of(MachineProgram::Shell),
            24,
            80,
        )
        .expect("spawn");
        assert_eq!(spawn.on.as_ref().map(MachineName::as_str), Some("loop"));
        let mut names: Vec<&str> = spawn.env.keys().map(String::as_str).collect();
        names.sort_unstable();
        assert_eq!(names, ["COLORTERM", "HERDR_RUN_ID", "TERM"]);
        assert_eq!(spawn.cwd, None, "no folder of this Mac goes");
        assert_eq!(
            spawn.labels.get("herdr.pane_id"),
            Some(&Value::String("w1:p2".into()))
        );
        assert!(spawn.labels.contains_key("herdr.session"));
    }

    #[test]
    fn the_owner_label_is_the_session_label_at_the_core_id() {
        let mut labels = super::super::pane_labels(Some("w1:p2"));
        let plain = labels["herdr.session"].as_str().expect("label").to_string();
        super::super::pane_io::machine_owner_label(&mut labels, "c0ffee");
        assert_eq!(labels["herdr.session"], format!("{plain}@c0ffee"));
        assert_eq!(labels["herdr.pane_id"], "w1:p2");
    }

    // R6 on this Mac: the wrapper unsets the names even when the parent
    // has them, and passes the rest.
    #[test]
    fn the_wrapper_unsets_the_names_from_the_environment_it_inherits() {
        let mut argv = machine_env_wrapper();
        argv.extend(["/usr/bin/env".into()]);
        let mut command = std::process::Command::new(&argv[0]);
        command.args(&argv[1..]).env_clear().env("KEPT", "1");
        for name in MACHINE_UNSET_VARS {
            command.env(name, "fake");
        }
        let output = command.output().expect("run the wrapper");
        assert!(output.status.success(), "{output:?}");
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(!text.contains("HERDR_"), "{text}");
        assert!(text.contains("KEPT=1"), "{text}");
    }
}
