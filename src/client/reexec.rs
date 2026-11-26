//! A client left on an older build re-execs onto the server's binary after a
//! live update (fork issue 165).
//!
//! A live handoff moves the server to a new binary and leaves every attached
//! client running the old one: it only reconnects. A client that old parses
//! `config.toml` with an old schema, so client-side settings (toast sounds)
//! fall back to defaults. After a handoff the client asks the server which
//! build it is and, when the builds differ, replaces itself with the server's
//! binary, same arguments, environment and terminal.
//!
//! The decision and the command are assembled here without executing anything,
//! so the unit tests never exec. Exec exists only on Unix: the target and the
//! command are `cfg(unix)`, and on other platforms a mismatched client keeps
//! running and shows the "detach and reattach" toast.

#[cfg(unix)]
use std::ffi::OsString;
#[cfg(unix)]
use std::path::{Path, PathBuf};

/// Set on the re-exec'd client to the server build it exec'd for, so a binary
/// that still disagrees with that same server build is not exec'd again in a
/// loop. A later handoff to a different build is a new decision.
pub(crate) const REEXEC_ENV_VAR: &str = "HERDR_CLIENT_REEXEC";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClientBuild {
    /// Same build as the server.
    Same,
    /// The client is older than the server.
    Older,
    /// The client is newer than the server.
    Newer,
    /// The server did not say which build it is.
    Unknown,
}

/// How the client's build relates to the server's. The base version (semver:
/// herdr-max `0.10.0` sorts above every `0.9.5-ac-beta.N`) orders two builds,
/// then the `-beta.N` build number of one base; anything else that differs is
/// treated as older, since the server is the one that was just updated.
pub(crate) fn compare_builds(client: &str, server: Option<&str>) -> ClientBuild {
    let Some(server) = server else {
        return ClientBuild::Unknown;
    };
    if client == server {
        return ClientBuild::Same;
    }
    match (order_key(client), order_key(server)) {
        (Some(client), Some(server)) if client > server => ClientBuild::Newer,
        _ => ClientBuild::Older,
    }
}

/// What orders a version: its `major.minor.patch`, then the `N` of
/// `...-beta.N...`. A version without a build number is the release of its
/// base and sorts above that base's betas, as semver says.
fn order_key(version: &str) -> Option<(u64, u64, u64, u64)> {
    let (major, minor, patch) = crate::integration::version::extract_version_triple(version)?;
    let build = version
        .split("-beta.")
        .nth(1)
        .map(|rest| {
            rest.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
        })
        .and_then(|digits| digits.parse().ok())
        .unwrap_or(u64::MAX);
    Some((major, minor, patch, build))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReexecDecision {
    Stay,
    Reexec(ClientBuild),
}

/// Re-exec when the builds differ (older or newer: the client should run what
/// the server runs), never when the server did not say, and never twice toward
/// the same server build: `guard` is the server build a previous exec was for
/// ([`REEXEC_ENV_VAR`]), and only a match with the current server blocks.
pub(crate) fn decide(client: &str, server: Option<&str>, guard: Option<&str>) -> ReexecDecision {
    if guard.is_some() && guard == server {
        return ReexecDecision::Stay;
    }
    match compare_builds(client, server) {
        ClientBuild::Same | ClientBuild::Unknown => ReexecDecision::Stay,
        build => ReexecDecision::Reexec(build),
    }
}

/// The binary to exec: the path the server reports when it still exists, else
/// the client's own install path resolved through the Homebrew `opt` link
/// (`<prefix>/Cellar/<formula>/<version>/bin/<name>` becomes
/// `<prefix>/opt/<formula>/bin/<name>`), which survives `brew cleanup` where a
/// versioned Cellar path does not. `None` when neither exists.
#[cfg(unix)]
pub(crate) fn target_binary(
    server_exe: Option<&str>,
    own_exe: &Path,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if let Some(path) = server_exe.map(PathBuf::from).filter(|path| exists(path)) {
        return Some(path);
    }
    let opt = brew_opt_path(own_exe)?;
    exists(&opt).then_some(opt)
}

#[cfg(unix)]
fn brew_opt_path(exe: &Path) -> Option<PathBuf> {
    let parts: Vec<_> = exe.components().collect();
    let cellar = parts.iter().position(|part| part.as_os_str() == "Cellar")?;
    // Cellar / <formula> / <version> / <rest...>
    let formula = parts.get(cellar + 1)?;
    let rest = parts.get(cellar + 3..)?;
    if rest.is_empty() {
        return None;
    }
    let mut path: PathBuf = parts[..cellar].iter().collect();
    path.push("opt");
    path.push(formula);
    path.extend(rest);
    Some(path)
}

/// What to exec: the program and the original argument vector.
#[cfg(unix)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReexecPlan {
    pub program: PathBuf,
    pub argv0: OsString,
    pub args: Vec<OsString>,
}

/// The plan from the running process's own argv. Environment and terminal are
/// inherited by `exec`; only the guard variable is added, by [`ReexecPlan::command`].
#[cfg(unix)]
pub(crate) fn plan(program: PathBuf, argv: &[OsString]) -> ReexecPlan {
    ReexecPlan {
        program: program.clone(),
        argv0: argv
            .first()
            .cloned()
            .unwrap_or_else(|| program.into_os_string()),
        args: argv.iter().skip(1).cloned().collect(),
    }
}

#[cfg(unix)]
impl ReexecPlan {
    /// The command to `exec`. Nothing runs until `exec` is called on it.
    pub(crate) fn command(&self, server_version: &str) -> std::process::Command {
        let mut command = std::process::Command::new(&self.program);
        command.args(&self.args);
        command.env(REEXEC_ENV_VAR, server_version);
        // The re-exec'd image reaps the servers this one started (issue 179).
        if let Some(pids) = crate::platform::daemon_reaper::live_pids_env() {
            command.env(
                crate::platform::daemon_reaper::SPAWNED_DAEMONS_ENV_VAR,
                pids,
            );
        }
        // The same client goes on reporting to its terminal after the exec.
        if let Some(token) = super::rex_resume::token_for_reexec() {
            command.env("OSC7577_TOKEN", token);
        }
        {
            use std::os::unix::process::CommandExt as _;
            command.arg0(&self.argv0);
        }
        command
    }
}

/// What to do about the server's build after a handoff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Follow {
    /// Same build, unknown build, or already exec'd for this server build.
    Stay,
    /// The builds differ. `target` is the binary to exec, `None` when none was
    /// found or this platform cannot exec.
    Restart {
        build: ClientBuild,
        target: Option<std::path::PathBuf>,
    },
}

/// The decision from the server's reported build and binary. Pure: `exists`
/// stands in for the filesystem and nothing is exec'd.
pub(crate) fn follow_server(
    client: &str,
    server_version: Option<&str>,
    server_exe: Option<&str>,
    guard: Option<&str>,
    own_exe: Option<&std::path::Path>,
    exists: impl Fn(&std::path::Path) -> bool,
) -> Follow {
    let ReexecDecision::Reexec(build) = decide(client, server_version, guard) else {
        return Follow::Stay;
    };
    #[cfg(unix)]
    let target = own_exe.and_then(|own| target_binary(server_exe, own, exists));
    #[cfg(not(unix))]
    let target = {
        let _ = (server_exe, own_exe, exists);
        None
    };
    Follow::Restart { build, target }
}

/// After a live update ended this client's session, replace it with the
/// server's binary when the builds differ (fork issue 165): a client left on an
/// old build parses `config.toml` with an old schema and runs on default
/// client-side settings. The terminal is already restored; argv, environment
/// and tty carry over through `exec`. Returns when it stays (same build) or
/// when it could not exec; the client then reattaches in-process and the next
/// session's banner tells the user to detach and reattach (`take_update_notice`).
pub(super) fn reexec_onto_server_build() {
    let Some(status) = read_server_status_after_handoff() else {
        return;
    };
    let client_version = crate::build_info::version();
    let guard = std::env::var(REEXEC_ENV_VAR).ok();
    let own = std::env::current_exe().ok();
    let follow = follow_server(
        &client_version,
        status.version.as_deref(),
        status.exe.as_deref(),
        guard.as_deref(),
        own.as_deref(),
        |path| path.exists(),
    );
    let Follow::Restart { build, target } = follow else {
        return;
    };
    let server_version = status.version.unwrap_or_default();
    #[cfg(unix)]
    if let Some(target) = target {
        let argv: Vec<OsString> = std::env::args_os().collect();
        let plan = plan(target, &argv);
        eprintln!(
            "herdr: the client ({client_version}) is {} than the server ({server_version}); restarting it onto {}",
            if build == ClientBuild::Newer {
                "newer"
            } else {
                "older"
            },
            plan.program.display()
        );
        use std::os::unix::process::CommandExt as _;
        // `exec` returns only when it failed.
        let err = plan.command(&server_version).exec();
        tracing::warn!(error = %err, program = %plan.program.display(), "client re-exec onto the server build failed");
    }
    #[cfg(not(unix))]
    let _ = (build, target);
    let notice = format!(
        "client ({client_version}) is on a different build than the server ({server_version}); detach and reattach to update"
    );
    if let Ok(mut pending) = UPDATE_NOTICE.lock() {
        *pending = Some(notice.clone());
    }
    tracing::warn!(
        client = %client_version,
        server = %server_version,
        "client is on a different build than the server and could not re-exec"
    );
}

/// The banner a client that stays on a different build than the server shows in
/// its next session (a live-handoff reattach is in-process, so the terminal is
/// free for it only once the session starts).
static UPDATE_NOTICE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// `diagnostic` with the pending update notice appended, which is consumed.
pub(super) fn with_update_notice(diagnostic: Option<String>) -> Option<String> {
    let notice = UPDATE_NOTICE
        .lock()
        .ok()
        .and_then(|mut pending| pending.take());
    match (diagnostic, notice) {
        (Some(diagnostic), Some(notice)) => Some(format!("{diagnostic}; {notice}")),
        (diagnostic, notice) => diagnostic.or(notice),
    }
}

/// The new server's version and binary, once its API socket answers (it can
/// lag the client socket by a moment after a handoff).
fn read_server_status_after_handoff() -> Option<crate::api::RuntimeStatus> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Ok(Some(status)) = crate::api::read_runtime_status_at(
            &crate::api::socket_path(),
            std::time::Duration::from_secs(1),
        ) {
            return Some(status);
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "0.8.2-ac-beta.120-koopmeiners";
    const NEW: &str = "0.8.2-ac-beta.137-bonucci";

    #[test]
    fn a_client_that_stays_on_another_build_shows_one_banner_in_its_next_session() {
        assert_eq!(
            with_update_notice(Some("config".into())),
            Some("config".into())
        );
        *UPDATE_NOTICE.lock().expect("notice lock") = Some("update".into());
        assert_eq!(
            with_update_notice(Some("config".into())).as_deref(),
            Some("config; update")
        );
        // Consumed: the next session starts clean.
        assert_eq!(with_update_notice(None), None);
        *UPDATE_NOTICE.lock().expect("notice lock") = Some("update".into());
        assert_eq!(with_update_notice(None).as_deref(), Some("update"));
    }

    #[test]
    fn the_call_site_follows_a_differing_server_build_only_once() {
        let own = std::path::Path::new("/usr/local/bin/herdr-beta");
        let follow = |server: Option<&str>, guard: Option<&str>| {
            follow_server(
                OLD,
                server,
                Some("/usr/local/bin/herdr-beta"),
                guard,
                Some(own),
                |_| true,
            )
        };
        assert_eq!(follow(Some(OLD), None), Follow::Stay);
        assert_eq!(follow(None, None), Follow::Stay);
        assert_eq!(follow(Some(NEW), Some(NEW)), Follow::Stay);
        let Follow::Restart { build, target } = follow(Some(NEW), None) else {
            panic!("an older client restarts onto a newer server");
        };
        assert_eq!(build, ClientBuild::Older);
        #[cfg(unix)]
        assert_eq!(target, Some(PathBuf::from("/usr/local/bin/herdr-beta")));
        #[cfg(not(unix))]
        assert_eq!(target, None);
        // No binary to exec: the call site falls back to the message.
        assert!(matches!(
            follow_server(OLD, Some(NEW), None, None, Some(own), |_| false),
            Follow::Restart { target: None, .. }
        ));
    }

    #[test]
    fn herdr_max_0_10_0_sorts_above_every_herdr_beta_build() {
        for beta in [
            "0.9.5-ac-beta.222-bonucci",
            "0.9.5-ac-beta.1-pirlo",
            "0.9.5-ac-beta.1000-x",
            "0.9.5",
            "0.8.2-ac-beta.137-bonucci",
        ] {
            // A herdr-max client on a herdr-beta server is the newer one, and a
            // herdr-beta client on a herdr-max server the older one.
            assert_eq!(
                compare_builds("0.10.0", Some(beta)),
                ClientBuild::Newer,
                "{beta}"
            );
            assert_eq!(
                compare_builds(beta, Some("0.10.0")),
                ClientBuild::Older,
                "{beta}"
            );
            // Either way the client follows the server, once.
            assert_eq!(
                decide("0.10.0", Some(beta), None),
                ReexecDecision::Reexec(ClientBuild::Newer)
            );
            assert_eq!(
                decide(beta, Some("0.10.0"), None),
                ReexecDecision::Reexec(ClientBuild::Older)
            );
        }
        assert_eq!(compare_builds("0.10.1", Some("0.10.0")), ClientBuild::Newer);
        assert_eq!(compare_builds("0.11.0", Some("0.10.9")), ClientBuild::Newer);
        assert_eq!(compare_builds("0.10.0", Some("0.10.0")), ClientBuild::Same);
        assert_eq!(decide("0.10.0", Some("0.10.0"), None), ReexecDecision::Stay);
    }

    #[test]
    fn the_decision_covers_same_older_newer_and_unknown() {
        assert_eq!(compare_builds(NEW, Some(NEW)), ClientBuild::Same);
        assert_eq!(compare_builds(OLD, Some(NEW)), ClientBuild::Older);
        assert_eq!(compare_builds(NEW, Some(OLD)), ClientBuild::Newer);
        assert_eq!(compare_builds(NEW, None), ClientBuild::Unknown);
        // Not comparable (a stable build): differing is "older".
        assert_eq!(compare_builds("0.8.1", Some(NEW)), ClientBuild::Older);

        // Guard absent: decide on the builds alone.
        assert_eq!(decide(NEW, Some(NEW), None), ReexecDecision::Stay);
        assert_eq!(
            decide(OLD, Some(NEW), None),
            ReexecDecision::Reexec(ClientBuild::Older)
        );
        assert_eq!(
            decide(NEW, Some(OLD), None),
            ReexecDecision::Reexec(ClientBuild::Newer)
        );
        assert_eq!(decide(OLD, None, None), ReexecDecision::Stay);
        // Guard equals the server build: already exec'd for it, never again.
        assert_eq!(decide(OLD, Some(NEW), Some(NEW)), ReexecDecision::Stay);
        // Guard names an older build (left over from the last handoff): a new
        // server build is a new decision, and the client execs again.
        assert_eq!(
            decide(OLD, Some(NEW), Some(OLD)),
            ReexecDecision::Reexec(ClientBuild::Older)
        );
        // No server build to match, so a guard alone blocks nothing, and the
        // unknown server still stays.
        assert_eq!(decide(OLD, None, Some(OLD)), ReexecDecision::Stay);
    }

    #[cfg(unix)]
    #[test]
    fn the_target_is_the_servers_binary_when_it_exists() {
        let server = "/opt/homebrew/Cellar/herdr-beta/0.8.2-ac-beta.137-bonucci/bin/herdr-beta";
        let own = Path::new(
            "/opt/homebrew/Cellar/herdr-beta/0.8.2-ac-beta.120-koopmeiners/bin/herdr-beta",
        );
        assert_eq!(
            target_binary(Some(server), own, |path| path == Path::new(server)),
            Some(PathBuf::from(server))
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_deleted_cellar_path_falls_back_to_the_brew_opt_link() {
        let own = Path::new(
            "/opt/homebrew/Cellar/herdr-beta/0.8.2-ac-beta.120-koopmeiners/bin/herdr-beta",
        );
        let opt = Path::new("/opt/homebrew/opt/herdr-beta/bin/herdr-beta");
        // The server's path is gone and so is our own: the opt link is used.
        assert_eq!(
            target_binary(Some("/gone/herdr-beta"), own, |path| path == opt),
            Some(opt.to_path_buf())
        );
        // The server says nothing: same.
        assert_eq!(
            target_binary(None, own, |path| path == opt),
            Some(opt.to_path_buf())
        );
        // Neither exists: nothing to exec, never a deleted Cellar path.
        assert_eq!(target_binary(Some("/gone/x"), own, |_| false), None);
        // Not a Homebrew install: no opt path to derive.
        assert_eq!(
            target_binary(None, Path::new("/usr/local/bin/herdr"), |_| true),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_herdr_max_formula_and_the_herdr_beta_alias_both_resolve_through_opt() {
        let max_own =
            Path::new("/opt/homebrew/Cellar/herdr-max/0.9.3-ac-beta.223-baresi/bin/herdr-max");
        let max_opt = Path::new("/opt/homebrew/opt/herdr-max/bin/herdr-max");
        assert_eq!(
            target_binary(Some("/gone/herdr-max"), max_own, |path| path == max_opt),
            Some(max_opt.to_path_buf())
        );
        // The transitional `herdr-beta` keg holds only the alias link: a client
        // that runs through it still finds a live opt path, and a server that
        // moved to the `herdr-max` keg is followed by its own (existing) path.
        let alias_own =
            Path::new("/opt/homebrew/Cellar/herdr-beta/0.9.3-ac-beta.223-baresi/bin/herdr-beta");
        let alias_opt = Path::new("/opt/homebrew/opt/herdr-beta/bin/herdr-beta");
        assert_eq!(
            target_binary(None, alias_own, |path| path == alias_opt),
            Some(alias_opt.to_path_buf())
        );
        let server = "/opt/homebrew/Cellar/herdr-max/0.9.3-ac-beta.223-baresi/bin/herdr-max";
        assert_eq!(
            target_binary(Some(server), alias_own, |path| path == Path::new(server)),
            Some(PathBuf::from(server))
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_plan_keeps_argv_and_the_command_adds_only_the_guard_variable() {
        let argv: Vec<OsString> = ["herdr-beta", "--session", "work"]
            .iter()
            .map(OsString::from)
            .collect();
        let plan = plan(
            PathBuf::from("/opt/homebrew/opt/herdr-beta/bin/herdr-beta"),
            &argv,
        );
        assert_eq!(plan.argv0, OsString::from("herdr-beta"));
        assert_eq!(
            plan.args,
            vec![OsString::from("--session"), OsString::from("work")]
        );
        let command = plan.command(NEW);
        assert_eq!(
            command.get_program(),
            "/opt/homebrew/opt/herdr-beta/bin/herdr-beta"
        );
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec!["--session", "work"]
        );
        let envs: Vec<_> = command.get_envs().collect();
        assert_eq!(envs.len(), 1, "only the guard: {envs:?}");
        assert_eq!(envs[0].0, REEXEC_ENV_VAR);
        assert_eq!(envs[0].1, Some(std::ffi::OsStr::new(NEW)));
    }

    #[cfg(unix)]
    #[test]
    fn the_reexec_command_hands_the_running_server_pids_to_the_new_image() {
        // An exec drops the reaper threads, so the new image must be told which servers to reap.
        let child = std::process::Command::new("sh")
            .args(["-c", "sleep 30"])
            .spawn()
            .expect("spawn the stand-in daemon");
        let pid = child.id();
        crate::platform::daemon_reaper::reap_when_it_exits(pid);
        let plan = plan(PathBuf::from("/bin/herdr"), &[OsString::from("herdr")]);
        let command = plan.command(NEW);
        let exported = command
            .get_envs()
            .find(|(key, _)| *key == crate::platform::daemon_reaper::SPAWNED_DAEMONS_ENV_VAR)
            .and_then(|(_, value)| value)
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        assert!(
            exported.split(',').any(|part| part == pid.to_string()),
            "{exported:?} should list {pid}"
        );
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        std::mem::forget(child);
    }
}
