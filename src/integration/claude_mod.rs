//! The `herdr-attention` Claude Code mod (fork issue 157): install and uninstall.
//!
//! The mod ships inside the binary (`integrations/claude-mod/`), is written to a
//! herdr-owned folder that doubles as a one-plugin local marketplace, and is
//! registered with Claude through `claude plugin marketplace add` and
//! `claude plugin install`. Both are idempotent, and Claude loads the plugin in
//! place from that folder, so rewriting the files is also the upgrade; a running
//! Claude picks the change up at its next start or on `/reload-plugins`.
//! Mods need Claude Code 2.1.287 or later: below that, or with no `claude` on
//! `PATH`, the mod is skipped and the settings hooks work as before.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::version::extract_version_triple;
use super::INSTALL_WARNING_PREFIX;

const PLUGIN_NAME: &str = "herdr-attention";
const MARKETPLACE_NAME: &str = "herdr-local";
/// The first Claude Code with mods.
const MIN_CLAUDE_VERSION: &str = "2.1.287";

/// Every file of the plugin, relative to the plugin folder.
const PLUGIN_FILES: &[(&str, &str)] = &[
    (
        ".claude-plugin/plugin.json",
        include_str!("../../integrations/claude-mod/.claude-plugin/plugin.json"),
    ),
    (
        "hooks/hooks.json",
        include_str!("../../integrations/claude-mod/hooks/hooks.json"),
    ),
    (
        "hooks/register.js",
        include_str!("../../integrations/claude-mod/hooks/register.js"),
    ),
    (
        "hooks/herdr-hint.sh",
        include_str!("../../integrations/claude-mod/hooks/herdr-hint.sh"),
    ),
];

const MARKETPLACE_JSON: &str = r#"{
  "name": "herdr-local",
  "owner": { "name": "herdr" },
  "plugins": [
    {
      "name": "herdr-attention",
      "source": "./herdr-attention",
      "description": "Tells herdr when Claude Code waits on a question or a permission prompt"
    }
  ]
}
"#;

/// What the mod install did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ClaudeModOutcome {
    /// Written to this folder and registered with Claude.
    Installed(PathBuf),
    /// Left out, with the reason; not an error.
    Skipped(String),
}

/// The part of the `claude` CLI the install needs, so tests can stand in.
pub(crate) trait ClaudeCli {
    /// The output of `claude --version`, or `None` when it cannot run.
    fn version(&self) -> Option<String>;
    /// Run `claude <args>`: whether it succeeded, and its output.
    fn run(&self, args: &[&str]) -> io::Result<(bool, String)>;
}

struct SystemClaude;

impl ClaudeCli for SystemClaude {
    fn version(&self) -> Option<String> {
        let output = crate::noninteractive_process::command("claude")
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|output| output.status.success())?;
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn run(&self, args: &[&str]) -> io::Result<(bool, String)> {
        let output = crate::noninteractive_process::command("claude")
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()?;
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        Ok((output.status.success(), text))
    }
}

/// The folder that holds the plugin and its marketplace manifest.
pub(crate) fn mod_root() -> PathBuf {
    crate::config::config_dir().join("claude-mod")
}

pub(crate) fn install_claude_mod() -> io::Result<ClaudeModOutcome> {
    // Tests of the integration actions must never run the real `claude` or
    // write the real herdr config folder; the logic is tested with a fake CLI.
    if cfg!(test) {
        return Ok(ClaudeModOutcome::Skipped(format!(
            "{INSTALL_WARNING_PREFIX} the claude mod is not installed under test"
        )));
    }
    install_with(&SystemClaude, &mod_root())
}

pub(crate) fn uninstall_claude_mod() -> io::Result<Vec<String>> {
    if cfg!(test) {
        return Ok(Vec::new());
    }
    uninstall_with(&SystemClaude, &mod_root())
}

fn install_with(cli: &dyn ClaudeCli, root: &Path) -> io::Result<ClaudeModOutcome> {
    if cfg!(windows) {
        return Ok(ClaudeModOutcome::Skipped(format!(
            "{INSTALL_WARNING_PREFIX} the claude mod needs a POSIX shell and python3; not installed on windows"
        )));
    }
    let Some(version_text) = cli.version() else {
        return Ok(ClaudeModOutcome::Skipped(format!(
            "{INSTALL_WARNING_PREFIX} claude is not on PATH, so the claude mod was not installed; run the install again once it is"
        )));
    };
    let floor = extract_version_triple(MIN_CLAUDE_VERSION).unwrap_or((2, 1, 287));
    match extract_version_triple(&version_text) {
        Some(found) if found >= floor => {}
        Some(found) => {
            return Ok(ClaudeModOutcome::Skipped(format!(
                "{INSTALL_WARNING_PREFIX} claude {}.{}.{} is older than {MIN_CLAUDE_VERSION}, which mods need; the claude mod was not installed",
                found.0, found.1, found.2
            )));
        }
        None => {
            return Ok(ClaudeModOutcome::Skipped(format!(
                "{INSTALL_WARNING_PREFIX} could not read the claude version from `claude --version`; the claude mod was not installed"
            )));
        }
    }

    write_if_changed(
        &root.join(".claude-plugin/marketplace.json"),
        MARKETPLACE_JSON,
    )?;
    let plugin_dir = root.join(PLUGIN_NAME);
    for (relative, content) in PLUGIN_FILES {
        write_if_changed(&plugin_dir.join(relative), content)?;
    }

    let root_arg = root.to_string_lossy().into_owned();
    claude_step(
        cli,
        &["plugin", "marketplace", "add", &root_arg],
        "add the herdr marketplace",
    )?;
    claude_step(
        cli,
        &[
            "plugin",
            "install",
            &format!("{PLUGIN_NAME}@{MARKETPLACE_NAME}"),
            "--scope",
            "user",
        ],
        "install the herdr-attention plugin",
    )?;
    Ok(ClaudeModOutcome::Installed(root.to_path_buf()))
}

fn uninstall_with(cli: &dyn ClaudeCli, root: &Path) -> io::Result<Vec<String>> {
    let mut messages = Vec::new();
    if cli.version().is_some() {
        // A plugin or marketplace that is already gone is not a failure.
        let (_, _) = cli.run(&[
            "plugin",
            "uninstall",
            &format!("{PLUGIN_NAME}@{MARKETPLACE_NAME}"),
            "--scope",
            "user",
        ])?;
        let (_, _) = cli.run(&["plugin", "marketplace", "remove", MARKETPLACE_NAME])?;
        messages.push("removed the herdr-attention plugin from claude".to_string());
    } else {
        messages.push(format!(
            "{INSTALL_WARNING_PREFIX} claude is not on PATH; remove the herdr-attention plugin from claude by hand if it was installed"
        ));
    }
    if super::file_ops::remove_dir_all_if_exists(root)? {
        messages.push(format!("removed the claude mod at {}", root.display()));
    } else {
        messages.push(format!("no claude mod found at {}", root.display()));
    }
    Ok(messages)
}

fn claude_step(cli: &dyn ClaudeCli, args: &[&str], what: &str) -> io::Result<()> {
    let (ok, output) = cli.run(args)?;
    if ok {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "claude could not {what}: {}",
        output.trim()
    )))
}

fn write_if_changed(path: &Path, content: &str) -> io::Result<()> {
    if fs::read_to_string(path).is_ok_and(|existing| existing == content) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    struct FakeClaude {
        version: Option<String>,
        calls: RefCell<Vec<Vec<String>>>,
        fail_on: Option<&'static str>,
    }

    impl FakeClaude {
        fn new(version: Option<&str>) -> Self {
            Self {
                version: version.map(str::to_string),
                calls: RefCell::new(Vec::new()),
                fail_on: None,
            }
        }
        fn calls(&self) -> Vec<String> {
            self.calls
                .borrow()
                .iter()
                .map(|call| call.join(" "))
                .collect()
        }
    }

    impl ClaudeCli for FakeClaude {
        fn version(&self) -> Option<String> {
            self.version.clone()
        }
        fn run(&self, args: &[&str]) -> io::Result<(bool, String)> {
            self.calls
                .borrow_mut()
                .push(args.iter().map(|arg| arg.to_string()).collect());
            let failing = self
                .fail_on
                .is_some_and(|needle| args.iter().any(|arg| arg.contains(needle)));
            Ok((
                !failing,
                if failing {
                    "nope".into()
                } else {
                    String::new()
                },
            ))
        }
    }

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "herdr-claude-mod-test-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[cfg(unix)]
    #[test]
    fn a_new_enough_claude_gets_the_files_the_marketplace_and_the_plugin() {
        let root = temp_root("install");
        let cli = FakeClaude::new(Some("2.1.288 (Claude Code)\n"));
        let outcome = install_with(&cli, &root).unwrap();
        assert_eq!(outcome, ClaudeModOutcome::Installed(root.clone()));

        for (relative, content) in PLUGIN_FILES {
            assert_eq!(
                &fs::read_to_string(root.join(PLUGIN_NAME).join(relative)).unwrap(),
                content,
                "{relative}"
            );
        }
        let marketplace = fs::read_to_string(root.join(".claude-plugin/marketplace.json")).unwrap();
        assert!(
            marketplace.contains("\"herdr-local\"")
                && marketplace.contains("\"./herdr-attention\"")
        );
        assert_eq!(
            cli.calls(),
            vec![
                format!("plugin marketplace add {}", root.display()),
                "plugin install herdr-attention@herdr-local --scope user".to_string(),
            ]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn installing_again_rewrites_nothing_and_asks_claude_again() {
        let root = temp_root("again");
        let cli = FakeClaude::new(Some("2.1.287"));
        install_with(&cli, &root).unwrap();
        let register = root.join(PLUGIN_NAME).join("hooks/register.js");
        let first = fs::metadata(&register).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        install_with(&cli, &root).unwrap();
        assert_eq!(fs::metadata(&register).unwrap().modified().unwrap(), first);
        assert_eq!(cli.calls().len(), 4, "both claude commands run each time");

        // An edit (a herdr upgrade changes the mod) is rewritten.
        fs::write(&register, "// old").unwrap();
        install_with(&cli, &root).unwrap();
        assert_eq!(
            fs::read_to_string(&register).unwrap(),
            include_str!("../../integrations/claude-mod/hooks/register.js")
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_old_or_missing_claude_skips_the_mod_without_writing_anything() {
        for (version, expect) in [
            (None, "not on PATH"),
            (Some("2.1.286 (Claude Code)"), "older than 2.1.287"),
            (Some("1.9.0"), "older than 2.1.287"),
            (Some("no version here"), "could not read the claude version"),
        ] {
            let root = temp_root("skip");
            let cli = FakeClaude::new(version);
            match install_with(&cli, &root).unwrap() {
                ClaudeModOutcome::Skipped(reason) if cfg!(windows) => {
                    assert!(reason.contains("windows"))
                }
                ClaudeModOutcome::Skipped(reason) => {
                    assert!(reason.starts_with(INSTALL_WARNING_PREFIX), "{reason}");
                    assert!(reason.contains(expect), "{reason}");
                }
                other => panic!("expected a skip, got {other:?}"),
            }
            assert!(!root.exists(), "nothing written");
            assert!(cli.calls().is_empty());
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_claude_command_that_fails_is_an_error_with_its_output() {
        let root = temp_root("fail");
        let mut cli = FakeClaude::new(Some("2.1.288"));
        cli.fail_on = Some("install");
        let err = install_with(&cli, &root).unwrap_err().to_string();
        assert!(
            err.contains("could not install the herdr-attention plugin"),
            "{err}"
        );
        assert!(err.contains("nope"), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn uninstall_removes_the_plugin_the_marketplace_and_the_folder() {
        let root = temp_root("uninstall");
        let cli = FakeClaude::new(Some("2.1.288"));
        install_with(&cli, &root).unwrap();
        cli.calls.borrow_mut().clear();

        let messages = uninstall_with(&cli, &root).unwrap();
        assert!(!root.exists());
        assert_eq!(
            cli.calls(),
            vec![
                "plugin uninstall herdr-attention@herdr-local --scope user",
                "plugin marketplace remove herdr-local",
            ]
        );
        assert!(messages
            .iter()
            .any(|m| m.contains("removed the claude mod")));

        // Already gone: still fine.
        let messages = uninstall_with(&cli, &root).unwrap();
        assert!(messages.iter().any(|m| m.contains("no claude mod found")));
    }

    #[test]
    fn uninstall_without_claude_still_removes_the_folder_and_says_what_is_left() {
        let root = temp_root("nocli");
        fs::create_dir_all(&root).unwrap();
        let cli = FakeClaude::new(None);
        let messages = uninstall_with(&cli, &root).unwrap();
        assert!(!root.exists());
        assert!(cli.calls().is_empty());
        assert!(messages.iter().any(|m| m.contains("by hand")));
    }
}
