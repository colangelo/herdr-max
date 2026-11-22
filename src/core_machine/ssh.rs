//! What `core-link` runs: the ssh argv and environment (D10, F6, F9, F11)
//! and the far-side guard (D2 step 5, F10).

use std::ffi::OsString;
use std::path::Path;

use super::state::Target;

/// What the guard prints on stderr once the far core answers: the link is
/// up from here on (D2 step 6).
pub(crate) const LINK_UP_MARK: &str = "HERDR-LINK-UP";
/// What the guard prints when no far core answers, before it exits 69.
pub(crate) const FAR_CORE_DOWN_TEXT: &str = "herdr-link: herdr-core is not running there";
/// The guard's exit status when no far core answers (`EX_UNAVAILABLE`).
pub(crate) const FAR_CORE_DOWN_EXIT: i32 = 69;

/// The options of every link ssh after the shared non-interactive ones: no
/// master, so nothing authenticated outlives the link (F9), keepalives, so a
/// frozen far side ends the link in about 15 s (F11), and `LogLevel=INFO`,
/// because OpenSSH prints an auth banner (a Tailscale SSH check, task A.1)
/// only at INFO or above, and a user config may set less.
pub(crate) const LINK_SSH_OPTIONS: [&str; 5] = [
    "ControlMaster=no",
    "ControlPath=none",
    "ServerAliveInterval=5",
    "ServerAliveCountMax=2",
    "LogLevel=INFO",
];

/// Core's POSIX line is `sh -c '...'`; its Windows fallback (`cmd /d /c
/// ...`, R3) and anything else is not dialled (D2 step 1).
pub(crate) fn is_posix_line(line: &str) -> bool {
    line.starts_with("sh -c '") && line.ends_with('\'')
}

/// `ssh -C -F <config> <non-interactive> <link options> -T <target>
/// <guard && line>`. The managed config, or the one the machine was added
/// with, is the only config ssh reads (D10).
pub(crate) fn ssh_args(target: &Target, managed_config: &Path, line: &str) -> Vec<OsString> {
    let config = target.ssh_config.as_deref().unwrap_or(managed_config);
    let mut args: Vec<OsString> = vec!["-C".into(), "-F".into(), config.into()];
    for option in crate::remote::NONINTERACTIVE_SSH_OPTIONS
        .iter()
        .chain(LINK_SSH_OPTIONS.iter())
    {
        args.push("-o".into());
        args.push((*option).into());
    }
    args.push("-T".into());
    args.push(target.target.as_str().into());
    args.push(remote_command(&target.session, line).into());
    args
}

/// The ssh of `herdr machine login` (D10): in the person's terminal, so a
/// key, host key or Tailscale check is answered there; still no master.
pub(crate) fn login_ssh_args(target: &Target, managed_config: &Path) -> Vec<OsString> {
    let config = target.ssh_config.as_deref().unwrap_or(managed_config);
    let mut args: Vec<OsString> = vec!["-C".into(), "-F".into(), config.into()];
    for option in ["ControlMaster=no", "ControlPath=none", "ConnectTimeout=10"] {
        args.push("-o".into());
        args.push(option.into());
    }
    args.extend(["-T".into(), target.target.as_str().into(), "exit".into()]);
    args
}

/// Writes the managed ssh config (an `Include` of the user's config under
/// the passwd home and of the system one) into the machine's folder.
pub(crate) fn write_managed_config(
    dir: &super::state::MachineDir,
    home: &Path,
) -> std::io::Result<std::path::PathBuf> {
    let path = dir.ssh_config_path();
    let contents = crate::remote::managed_ssh_config_contents(
        Some(&home.join(".ssh/config")),
        crate::platform::remote_ssh_config_paths()
            .system_config
            .as_deref(),
    );
    super::state::write_atomic(&path, contents.as_bytes())?;
    Ok(path)
}

/// The variable names ssh may get from the environment core passed (D2):
/// the locale and the temporary folder. `HOME`, `USER`, `LOGNAME`, `PATH`
/// and `SSH_AUTH_SOCK` are set from the passwd entry, a fixed list and
/// `target.json`.
fn inherited_name_allowed(name: &str) -> bool {
    matches!(name, "LANG" | "TMPDIR") || name.starts_with("LC_")
}

/// The fixed `PATH` of a link ssh.
pub(crate) const SSH_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// ssh's whole environment: the allowlist above plus the passwd home and
/// name and the recorded `SSH_AUTH_SOCK` (D2, F6). Nothing else of the
/// environment core passed reaches ssh.
pub(crate) fn ssh_env(
    home: &Path,
    user: &str,
    target: &Target,
    inherited: impl IntoIterator<Item = (OsString, OsString)>,
) -> Vec<(String, OsString)> {
    let mut env: Vec<(String, OsString)> = inherited
        .into_iter()
        .filter_map(|(key, value)| {
            let key = key.into_string().ok()?;
            inherited_name_allowed(&key).then_some((key, value))
        })
        .collect();
    env.push(("HOME".into(), home.into()));
    env.push(("USER".into(), user.into()));
    env.push(("LOGNAME".into(), user.into()));
    env.push(("PATH".into(), SSH_PATH.into()));
    if let Some(sock) = &target.ssh_auth_sock {
        env.push(("SSH_AUTH_SOCK".into(), sock.into()));
    }
    env.sort();
    env
}

/// `sh -c '<guard>' && <core's line>`.
pub(crate) fn remote_command(session: &str, line: &str) -> String {
    format!("sh -c '{}' && {line}", far_guard(session))
}

/// The far guard (D2 step 5): finds the far session's socket by core's
/// endpoint rules and tests it with `nc -U`, which tells a live core from a
/// missing or stale socket without starting one (R5). Where `nc` has no
/// `-U` it falls back to a socket-file test, which a stale socket passes
/// (accepted for VMs). `session` is a plain name (no quote can occur); the
/// guard holds no single quote, so it goes inside `sh -c '...'`.
pub(crate) fn far_guard(session: &str) -> String {
    format!(
        concat!(
            r#"d=${{XDG_RUNTIME_DIR:+$XDG_RUNTIME_DIR/herdr-core}}; "#,
            r#"[ -n "$d" ] || {{ [ -d /run/user/$(id -u) ] && d=/run/user/$(id -u)/herdr-core; }}; "#,
            r#"[ -n "$d" ] || d=${{XDG_STATE_HOME:-$HOME/.local/state}}/herdr-core/{session}; "#,
            r#"s="$d/{session}.sock"; "#,
            r#"if command -v nc >/dev/null 2>&1; then "#,
            r#"e=$(nc -U -w 2 "$s" </dev/null 2>&1 >/dev/null); r=$?; "#,
            r#"case "$e" in *nvalid?option*|*llegal?option*|*nrecognized?option*|*sage:*) [ -S "$s" ]; r=$?;; esac; "#,
            r#"else [ -S "$s" ]; r=$?; fi; "#,
            r#"[ "$r" -eq 0 ] || {{ echo "{down}" >&2; exit {code}; }}; "#,
            r#"echo {mark} >&2"#,
        ),
        session = session,
        down = FAR_CORE_DOWN_TEXT,
        code = FAR_CORE_DOWN_EXIT,
        mark = LINK_UP_MARK,
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::process::Command;

    use super::*;

    /// Core's POSIX line as hc 0.1.21 sends it (P1, R3), and its Windows
    /// fallback (R3). V35's pin: a core whose line changes shape fails here.
    const CORE_POSIX_LINE: &str = r#"sh -c 'p=$(command -v herdr-core) || p=$HOME/.local/bin/herdr-core; [ -x "$p" ] || { echo "herdr-core is not installed here" >&2; exit 127; }; HC_SESSION=hcmx-far exec "$p" connect'"#;
    const CORE_WINDOWS_LINE: &str = r#"cmd /d /c "if exist .\AppData\Local\herdr-core\herdr-core.exe (.\AppData\Local\herdr-core\herdr-core.exe connect) else (echo herdr-core is not installed here 1>&2 & exit /b 127)""#;

    fn target() -> Target {
        Target {
            target: "ac@ac-mbm5".into(),
            session: "default".into(),
            ssh: "/usr/bin/ssh".into(),
            ssh_config: None,
            ssh_auth_sock: Some("/tmp/recorded.sock".into()),
        }
    }

    #[test]
    fn only_cores_posix_line_is_dialled() {
        assert!(is_posix_line(CORE_POSIX_LINE));
        assert!(CORE_POSIX_LINE.ends_with(r#"exec "$p" connect'"#));
        assert!(!is_posix_line(CORE_WINDOWS_LINE));
        assert!(!is_posix_line("sh -c"));
        assert!(!is_posix_line("bash -c 'x'"));
    }

    #[test]
    fn the_argv_has_no_master_keepalives_and_the_guard_first() {
        let args = ssh_args(&target(), Path::new("/st/loop/ssh_config"), CORE_POSIX_LINE);
        let args: Vec<String> = args.into_iter().map(|a| a.into_string().unwrap()).collect();
        let expected_head = [
            "-C",
            "-F",
            "/st/loop/ssh_config",
            "-o",
            "BatchMode=yes",
            "-o",
            "NumberOfPasswordPrompts=0",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            "ConnectTimeout=10",
            "-o",
            "ConnectionAttempts=1",
            "-o",
            "ControlMaster=no",
            "-o",
            "ControlPath=none",
            "-o",
            "ServerAliveInterval=5",
            "-o",
            "ServerAliveCountMax=2",
            "-o",
            "LogLevel=INFO",
            "-T",
            "ac@ac-mbm5",
        ];
        assert_eq!(&args[..expected_head.len()], expected_head);
        let remote = args.last().unwrap();
        assert_eq!(args.len(), expected_head.len() + 1);
        assert!(
            remote.starts_with("sh -c 'd=${XDG_RUNTIME_DIR:+"),
            "{remote}"
        );
        assert!(
            remote.ends_with(&format!("' && {CORE_POSIX_LINE}")),
            "{remote}"
        );
        assert!(!args
            .iter()
            .any(|a| a.contains("ControlPersist") || a == "-S"));
    }

    #[test]
    fn login_prompts_in_the_terminal_without_a_master() {
        let args = login_ssh_args(&target(), Path::new("/st/loop/ssh_config"));
        let args: Vec<String> = args.into_iter().map(|a| a.into_string().unwrap()).collect();
        assert_eq!(
            args,
            [
                "-C",
                "-F",
                "/st/loop/ssh_config",
                "-o",
                "ControlMaster=no",
                "-o",
                "ControlPath=none",
                "-o",
                "ConnectTimeout=10",
                "-T",
                "ac@ac-mbm5",
                "exit"
            ]
        );
    }

    #[test]
    fn a_chosen_ssh_config_replaces_the_managed_one() {
        let mut target = target();
        target.ssh_config = Some(PathBuf::from("/cfg/test_ssh_config"));
        let args = ssh_args(&target, Path::new("/st/loop/ssh_config"), CORE_POSIX_LINE);
        assert_eq!(args[2], OsString::from("/cfg/test_ssh_config"));
    }

    #[test]
    fn ssh_gets_only_allowlisted_names_and_the_recorded_agent() {
        let inherited = [
            ("HOME", "/elsewhere"),
            ("XDG_STATE_HOME", "/elsewhere/state"),
            ("SSH_AUTH_SOCK", "/elsewhere/agent"),
            ("PATH", "/evil/bin:/usr/bin"),
            ("HERDR_PANE_ID", "w1:p1"),
            ("HC_SESSION", "x"),
            ("LANG", "en_US.UTF-8"),
            ("LC_CTYPE", "UTF-8"),
            ("TMPDIR", "/tmp/x"),
            ("USER", "someone"),
        ]
        .map(|(k, v)| (OsString::from(k), OsString::from(v)));
        let env = ssh_env(Path::new("/Users/ac"), "ac", &target(), inherited);
        let names: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            names,
            [
                "HOME",
                "LANG",
                "LC_CTYPE",
                "LOGNAME",
                "PATH",
                "SSH_AUTH_SOCK",
                "TMPDIR",
                "USER"
            ]
        );
        let value = |name: &str| env.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
        assert_eq!(value("HOME"), Some("/Users/ac".into()));
        assert_eq!(value("USER"), Some("ac".into()));
        assert_eq!(value("PATH"), Some(SSH_PATH.into()));
        assert_eq!(value("SSH_AUTH_SOCK"), Some("/tmp/recorded.sock".into()));
    }

    #[test]
    fn no_recorded_agent_means_no_agent() {
        let mut target = target();
        target.ssh_auth_sock = None;
        let inherited = [(
            OsString::from("SSH_AUTH_SOCK"),
            OsString::from("/elsewhere"),
        )];
        let env = ssh_env(Path::new("/h"), "u", &target, inherited);
        assert!(!env.iter().any(|(k, _)| k == "SSH_AUTH_SOCK"));
    }

    #[test]
    fn the_guard_has_no_single_quote() {
        assert!(!far_guard("default").contains('\''));
    }

    /// The guard against a live socket, a missing one and a stale one, run
    /// by this Mac's `sh` and `nc` (macOS and Linux both take `-U`, or the
    /// guard falls back to the socket-file test).
    #[test]
    fn the_guard_tells_a_live_core_from_none() {
        let root = super::super::state::tests::scratch("guard");
        let runtime = root.join("run");
        std::fs::create_dir_all(runtime.join("herdr-core")).expect("run dir");
        let socket = runtime.join("herdr-core/far.sock");
        let run = || {
            Command::new("/bin/sh")
                .arg("-c")
                .arg(far_guard("far"))
                .env_clear()
                .env("PATH", SSH_PATH)
                .env("HOME", &root)
                .env("XDG_RUNTIME_DIR", &runtime)
                .output()
                .expect("guard")
        };

        let missing = run();
        assert_eq!(missing.status.code(), Some(FAR_CORE_DOWN_EXIT));
        assert!(String::from_utf8_lossy(&missing.stderr).contains(FAR_CORE_DOWN_TEXT));

        // A live core accepts; this one then hangs up, so no nc waits out
        // its -w on an idle connection.
        let listener = std::os::unix::net::UnixListener::bind(&socket).expect("bind");
        let accepting = listener.try_clone().expect("clone");
        let server = std::thread::spawn(move || {
            if let Ok((connection, _)) = accepting.accept() {
                drop(connection);
            }
        });
        let live = run();
        assert_eq!(live.status.code(), Some(0), "{live:?}");
        assert_eq!(String::from_utf8_lossy(&live.stderr).trim(), LINK_UP_MARK);
        drop(listener);
        // Without nc the guard tests the file and nothing connects.
        let _ = std::os::unix::net::UnixStream::connect(&socket);
        let _ = server.join();

        // The socket file stays after its listener is gone, as a killed
        // core leaves it. macOS's nc refuses it; a Linux image without
        // nc -U cannot tell (D2 step 5's accepted limit).
        if cfg!(target_os = "macos") {
            let stale = run();
            assert_eq!(stale.status.code(), Some(FAR_CORE_DOWN_EXIT), "{stale:?}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
