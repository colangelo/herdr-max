//! Git for test setup: self-contained, and self-explaining when it fails.
//!
//! Test repos are built by shelling out to `git`. That has to behave the same
//! on a developer machine, inside a git hook (which exports `GIT_DIR`), and on
//! a CI runner with its own system config, and when a git process dies the
//! panic has to say how. Use [`run_git`] / [`hermetic`] instead of a bare
//! `Command::new("git")` in test setup.

use std::path::Path;
use std::process::{Command, Output};

#[cfg(windows)]
const NULL_DEVICE: &str = "NUL";
#[cfg(not(windows))]
const NULL_DEVICE: &str = "/dev/null";

const IDENTITY_NAME: &str = "Herdr Test";
const IDENTITY_EMAIL: &str = "herdr@example.invalid";

/// Variables that make git act on a repository other than the one named with
/// `-C`. A git hook (for example a pre-push running the test suite) exports
/// them.
const REPO_SELECTION_VARS: [&str; 8] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_PREFIX",
];

/// Layers a repo-agnostic, config-agnostic environment on `command`: no
/// system or global config (so a runner's `core.autocrlf`, signing or
/// `safe.directory` settings cannot change the outcome), a fixed author and
/// committer identity, no inherited repository selection, no credential
/// prompt, and English messages.
pub(crate) fn hermetic(command: &mut Command) -> &mut Command {
    for var in REPO_SELECTION_VARS {
        command.env_remove(var);
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", NULL_DEVICE)
        .env("GIT_AUTHOR_NAME", IDENTITY_NAME)
        .env("GIT_AUTHOR_EMAIL", IDENTITY_EMAIL)
        .env("GIT_COMMITTER_NAME", IDENTITY_NAME)
        .env("GIT_COMMITTER_EMAIL", IDENTITY_EMAIL)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
}

/// Runs `git -C <repo> <args>` and panics with the exit status, stdout and
/// stderr when it does not succeed, so a failure explains itself even when
/// the process died without printing anything.
pub(crate) fn run_git(repo: &Path, args: &[&str]) {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo).args(args);
    let output = hermetic(&mut command).output().unwrap_or_else(|err| {
        panic!(
            "git command could not start: git -C {} {}: {err}",
            repo.display(),
            args.join(" ")
        )
    });
    assert!(
        output.status.success(),
        "{}",
        failure_report(repo, args, &output)
    );
}

fn failure_report(repo: &Path, args: &[&str], output: &Output) -> String {
    format!(
        "git command failed: git -C {} {}\n{}\nstdout:\n{}\nstderr:\n{}",
        repo.display(),
        args.join(" "),
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_path(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("herdr-{name}-{}-{nanos}", std::process::id()))
    }

    fn panic_text(result: std::thread::Result<()>) -> String {
        let payload = result.expect_err("expected a panic");
        match payload.downcast::<String>() {
            Ok(text) => *text,
            Err(payload) => payload
                .downcast_ref::<&str>()
                .map(|text| text.to_string())
                .unwrap_or_default(),
        }
    }

    #[test]
    fn failure_report_names_exit_status_stdout_and_stderr() {
        let repo = unique_temp_path("test-git-report");
        std::fs::create_dir_all(&repo).unwrap();
        run_git(&repo, &["init", "--quiet"]);

        let report = panic_text(std::panic::catch_unwind(|| {
            run_git(&repo, &["definitely-not-a-git-command"]);
        }));

        assert!(report.contains("definitely-not-a-git-command"), "{report}");
        assert!(report.contains("exit"), "no exit status in: {report}");
        assert!(report.contains("stdout:"), "no stdout section in: {report}");
        assert!(report.contains("stderr:"), "no stderr section in: {report}");
        assert!(
            report.contains("is not a git command"),
            "git's own message missing from: {report}"
        );
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn setup_ignores_ambient_git_config_and_repo_selection() {
        let dir = unique_temp_path("test-git-hermetic");
        std::fs::create_dir_all(&dir).unwrap();
        let repo = dir.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        run_git(&repo, &["init", "--quiet"]);

        // An ambient environment that breaks any commit: a global config that
        // demands a signer which does not exist, and a GIT_DIR (a git hook
        // exports one) that points nowhere.
        let poisoned_config = dir.join("poisoned.gitconfig");
        std::fs::write(
            &poisoned_config,
            "[commit]\n\tgpgsign = true\n[gpg]\n\tprogram = herdr-no-such-signer\n",
        )
        .unwrap();
        let poison = |command: &mut Command| {
            command
                .env("GIT_CONFIG_GLOBAL", &poisoned_config)
                .env("GIT_DIR", dir.join("not-a-git-dir"))
                .env("GIT_AUTHOR_NAME", "someone")
                .env("GIT_AUTHOR_EMAIL", "someone@example.invalid")
                .env("GIT_COMMITTER_NAME", "someone")
                .env("GIT_COMMITTER_EMAIL", "someone@example.invalid");
        };
        let commit_args = [
            "-C",
            repo.to_str().unwrap(),
            "commit",
            "--allow-empty",
            "-qm",
            "x",
        ];

        // Control: without `hermetic` the poisoned environment really does
        // break the commit, so the assertion below proves something.
        let mut control = Command::new("git");
        control.args(commit_args);
        poison(&mut control);
        assert!(
            !control.output().unwrap().status.success(),
            "the poisoned environment no longer breaks a commit; the control is stale"
        );

        let mut hermetic_commit = Command::new("git");
        poison(&mut hermetic_commit);
        hermetic(&mut hermetic_commit).args(commit_args);
        let output = hermetic_commit.output().unwrap();
        assert!(
            output.status.success(),
            "hermetic git still saw the ambient environment: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
