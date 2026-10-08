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

/// How the client's build relates to the server's. The `-beta.N` build number
/// orders two builds of one base version; anything else that differs is
/// treated as older, since the server is the one that was just updated.
pub(crate) fn compare_builds(client: &str, server: Option<&str>) -> ClientBuild {
    let Some(server) = server else {
        return ClientBuild::Unknown;
    };
    if client == server {
        return ClientBuild::Same;
    }
    match (build_number(client), build_number(server)) {
        (Some(client), Some(server)) if client > server => ClientBuild::Newer,
        _ => ClientBuild::Older,
    }
}

/// The `N` of `...-beta.N...`, when the version has one.
fn build_number(version: &str) -> Option<u64> {
    let rest = version.split("-beta.").nth(1)?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
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
        {
            use std::os::unix::process::CommandExt as _;
            command.arg0(&self.argv0);
        }
        command
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "0.8.2-ac-beta.120-koopmeiners";
    const NEW: &str = "0.8.2-ac-beta.137-bonucci";

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
}
