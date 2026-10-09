//! Launch-only flags a restored agent must get back (fork issues 123, 127).
//!
//! A session id brings back the conversation, but some choices only live on
//! the command line that started the agent: the settings file a GPT Claude
//! pane routes through, the sandbox a Codex pane was started in. herdr reads
//! the agent process's exact argv once when the agent appears, keeps only the
//! flags on that agent's carry list, and adds them to the resume command.
//!
//! Only listed flags are kept, so prompts, one-shot options and the session id
//! itself never reach `session.json`.

use std::path::{Component, Path, PathBuf};

/// An agent process's command line as the OS reports it, with the directory
/// it runs in (for relative paths).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLaunchArgv {
    pub argv: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// When the process started, in unix ms: transcript records older than
    /// this belong to an earlier run of the session.
    pub started_at_ms: Option<i64>,
}

/// The carried launch flags of the agent that runs in a pane now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLaunchFlags {
    pub agent: String,
    pub flags: Vec<String>,
    /// When the agent process started, in unix ms (see `AgentLaunchArgv`).
    pub started_at_ms: Option<i64>,
}

impl AgentLaunchFlags {
    pub fn from_launch(agent: &str, launch: &AgentLaunchArgv) -> Self {
        Self {
            agent: agent.to_string(),
            flags: carried_launch_flags(agent, &launch.argv, launch.cwd.as_deref()),
            started_at_ms: launch.started_at_ms,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Arity {
    /// A switch: `--search`.
    None,
    /// One value: `--settings <file>`, also written `--settings=<file>`.
    One,
    /// Values up to the next flag: `--add-dir <dir…>`.
    Many,
}

struct CarriedFlag {
    /// Every spelling of the flag; the first is the canonical one.
    names: &'static [&'static str],
    arity: Arity,
    /// Values are paths, made absolute against the agent's directory.
    path: bool,
    /// Flags of one group decide the same thing: when the resume command
    /// already carries one of them, none of the recorded ones is added.
    group: &'static str,
}

const fn flag(
    names: &'static [&'static str],
    arity: Arity,
    path: bool,
    group: &'static str,
) -> CarriedFlag {
    CarriedFlag {
        names,
        arity,
        path,
        group,
    }
}

const ALLOW_BYPASS: &str = "--allow-dangerously-skip-permissions";

const CLAUDE: &[CarriedFlag] = &[
    flag(&["--settings"], Arity::One, true, "settings"),
    flag(&["--add-dir"], Arity::Many, true, "add-dir"),
    flag(&["--mcp-config"], Arity::Many, true, "mcp-config"),
    flag(
        &["--strict-mcp-config"],
        Arity::None,
        false,
        "strict-mcp-config",
    ),
    flag(&["--agent"], Arity::One, false, "agent"),
    flag(&["--plugin-dir"], Arity::One, true, "plugin-dir"),
    flag(&["--setting-sources"], Arity::One, false, "setting-sources"),
    flag(&["--fallback-model"], Arity::One, false, "fallback-model"),
    // The hook reports these as the session changes them; the launch values
    // only fill in when it has not.
    flag(&["--model"], Arity::One, false, "model"),
    flag(&["--effort"], Arity::One, false, "effort"),
    flag(&["--permission-mode"], Arity::One, false, "permission"),
    flag(
        &["--dangerously-skip-permissions"],
        Arity::None,
        false,
        "permission",
    ),
    flag(
        &["--allow-dangerously-skip-permissions"],
        Arity::None,
        false,
        "allow-bypass",
    ),
];

const CODEX: &[CarriedFlag] = &[
    flag(&["-m", "--model"], Arity::One, false, "model"),
    flag(&["-s", "--sandbox"], Arity::One, false, "sandbox"),
    flag(&["-a", "--ask-for-approval"], Arity::One, false, "approval"),
    flag(&["-c", "--config"], Arity::One, false, "config"),
    flag(&["-p", "--profile"], Arity::One, false, "profile"),
    flag(&["-C", "--cd"], Arity::One, true, "cd"),
    flag(&["--add-dir"], Arity::One, true, "add-dir"),
    flag(&["--enable"], Arity::One, false, "enable"),
    flag(&["--disable"], Arity::One, false, "disable"),
    flag(&["--oss"], Arity::None, false, "oss"),
    flag(&["--local-provider"], Arity::One, false, "local-provider"),
    flag(&["--search"], Arity::None, false, "search"),
    flag(&["--strict-config"], Arity::None, false, "strict-config"),
    flag(&["--approve-for-me"], Arity::None, false, "approval"),
    flag(
        &["--dangerously-bypass-approvals-and-sandbox", "--yolo"],
        Arity::None,
        false,
        "bypass",
    ),
    flag(
        &["--dangerously-bypass-hook-trust"],
        Arity::None,
        false,
        "hook-trust",
    ),
    flag(&["--no-alt-screen"], Arity::None, false, "alt-screen"),
    flag(&["--no-daemon"], Arity::None, false, "daemon"),
];

fn carry_list(agent: &str) -> &'static [CarriedFlag] {
    match agent {
        "claude" => CLAUDE,
        "codex" => CODEX,
        _ => &[],
    }
}

/// How a flag is spelled on the command line: `--name`, `--name=value`, and
/// for a short flag `-m`.
fn split_flag(word: &str) -> (&str, Option<&str>) {
    match word.split_once('=') {
        Some((name, value)) if name.starts_with("--") => (name, Some(value)),
        _ => (word, None),
    }
}

fn find_flag<'a>(list: &'a [CarriedFlag], name: &str) -> Option<&'a CarriedFlag> {
    list.iter().find(|flag| flag.names.contains(&name))
}

fn is_flag(word: &str) -> bool {
    word.starts_with('-') && word != "-"
}

/// A value restore can type into a shell and the agent can read back: no
/// control characters or apostrophes (`validate_resume_argv`), and not inline
/// JSON, which a command line split on spaces cannot put back together.
fn carryable_value(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('{')
        && !value.contains('\'')
        && !value.chars().any(char::is_control)
}

fn absolute(value: &str, cwd: Option<&Path>) -> String {
    let path = Path::new(value);
    if path.is_absolute() {
        return value.to_string();
    }
    let Some(cwd) = cwd else {
        return value.to_string();
    };
    let mut joined = PathBuf::new();
    for component in cwd.join(path).components() {
        match component {
            Component::ParentDir => {
                joined.pop();
            }
            Component::CurDir => {}
            other => joined.push(other),
        }
    }
    joined.display().to_string()
}

/// The carry-listed flags of an agent's launch argv (`argv[0]` included), with
/// their values, in launch order. `--name=value` is written as two words and
/// relative paths are made absolute against `cwd`. A flag whose value cannot
/// be carried is left out whole.
pub fn carried_launch_flags(agent: &str, argv: &[String], cwd: Option<&Path>) -> Vec<String> {
    let list = carry_list(agent);
    let mut carried = Vec::new();
    let mut index = 1;
    while index < argv.len() {
        let (name, inline) = split_flag(&argv[index]);
        index += 1;
        let Some(flag) = find_flag(list, name) else {
            continue;
        };
        let values: Vec<&str> = match (flag.arity, inline) {
            (Arity::None, None) => Vec::new(),
            (Arity::None, Some(_)) => continue,
            (_, Some(value)) => vec![value],
            (Arity::One, None) => match argv.get(index) {
                Some(value) if !is_flag(value) => {
                    index += 1;
                    vec![value.as_str()]
                }
                _ => continue,
            },
            (Arity::Many, None) => {
                let start = index;
                while argv.get(index).is_some_and(|value| !is_flag(value)) {
                    index += 1;
                }
                argv[start..index].iter().map(String::as_str).collect()
            }
        };
        if flag.arity != Arity::None && values.is_empty() {
            continue;
        }
        if !values.iter().all(|value| carryable_value(value)) {
            continue;
        }
        carried.push(name.to_string());
        carried.extend(values.into_iter().map(|value| {
            if flag.path {
                absolute(value, cwd)
            } else {
                value.to_string()
            }
        }));
    }
    carried
}

/// The recorded launch flags without a `--settings` file that is not there.
///
/// Claude refuses to start on a settings path it cannot read, so a restore
/// that carried one would fail to resume at all (fork issue 143): the
/// scratchpad a launch pointed at is gone after a reboot. The flag is dropped
/// whole, with a warning, and the session resumes without it. The check is
/// made when the command is built, at snapshot and at restore, not when the
/// flag is first recorded, so a file that is there again is used again.
pub fn without_missing_settings(
    agent: &str,
    launch_flags: &[String],
    exists: impl Fn(&Path) -> bool,
) -> Vec<String> {
    if agent != "claude" {
        return launch_flags.to_vec();
    }
    let mut kept = Vec::with_capacity(launch_flags.len());
    // Inline JSON (`--settings '{"model":"x"}'`) is not a path: keep it.
    let is_gone = |value: &str| {
        let value = value.trim_start();
        !value.is_empty() && !value.starts_with('{') && !exists(Path::new(value))
    };
    let mut index = 0;
    while index < launch_flags.len() {
        let word = &launch_flags[index];
        if let Some(value) = word.strip_prefix("--settings=") {
            if is_gone(value) {
                tracing::warn!(
                    path = %value,
                    "dropping --settings from the resume command: the file is gone"
                );
                index += 1;
                continue;
            }
        } else if word == "--settings" {
            if let Some(value) = launch_flags.get(index + 1).filter(|value| !is_flag(value)) {
                if is_gone(value) {
                    tracing::warn!(
                        path = %value,
                        "dropping --settings from the resume command: the file is gone"
                    );
                    index += 2;
                    continue;
                }
            }
        }
        kept.push(word.clone());
        index += 1;
    }
    kept
}

/// `base` (the resume command) with every recorded launch flag whose group it
/// does not already carry. The base wins: it holds what the session changed
/// since launch.
pub fn compose_resume_argv(agent: &str, base: &[String], launch_flags: &[String]) -> Vec<String> {
    let list = carry_list(agent);
    let group_of = |word: &str| find_flag(list, split_flag(word).0).map(|flag| flag.group);
    let base_groups: Vec<&str> = base.iter().filter_map(|word| group_of(word)).collect();
    let mut argv = base.to_vec();
    let mut keep = false;
    for word in launch_flags {
        if is_flag(word) {
            keep = group_of(word).is_some_and(|group| !base_groups.contains(&group));
            // Started in bypass but resumed in another mode: as the hook does,
            // keep bypass reachable with shift+tab without forcing it.
            if !keep
                && agent == "claude"
                && word == "--dangerously-skip-permissions"
                && !argv.iter().any(|arg| arg == ALLOW_BYPASS)
            {
                argv.push(ALLOW_BYPASS.to_string());
            }
        }
        if keep {
            argv.push(word.clone());
        }
    }
    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rooted path in the host's separator ("/work/other" or "\\work\\other").
    fn rooted(parts: &[&str]) -> String {
        let mut path = std::path::PathBuf::from(std::path::MAIN_SEPARATOR_STR);
        path.extend(parts);
        path.display().to_string()
    }

    fn words(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| part.to_string()).collect()
    }

    /// Fork issue 143: a `--settings` file that is gone must never reach the
    /// resume command; one that is there stays.
    #[test]
    fn a_missing_settings_file_is_dropped_and_a_present_one_kept() {
        let flags = words(&[
            "--model",
            "opus",
            "--settings",
            "/gone/s.json",
            "--agent",
            "x",
        ]);
        assert_eq!(
            without_missing_settings("claude", &flags, |_| false),
            words(&["--model", "opus", "--agent", "x"]),
            "the flag and its value go together"
        );
        assert_eq!(
            without_missing_settings("claude", &flags, |_| true),
            flags,
            "a present file is kept"
        );
        assert_eq!(
            without_missing_settings("codex", &flags, |_| false),
            flags,
            "only Claude's --settings is checked"
        );
    }

    #[test]
    fn inline_json_settings_and_the_equals_form_are_handled() {
        let json = words(&["--settings", " {\"model\":\"x\"}", "--model", "opus"]);
        assert_eq!(
            without_missing_settings("claude", &json, |_| false),
            json,
            "inline JSON is not a path and is kept"
        );
        let equals = words(&["--settings=/gone/s.json", "--model", "opus"]);
        assert_eq!(
            without_missing_settings("claude", &equals, |_| false),
            words(&["--model", "opus"]),
            "a missing path in the = form is dropped"
        );
        assert_eq!(
            without_missing_settings("claude", &equals, |_| true),
            equals,
            "a present one is kept"
        );
        let inline_equals = words(&["--settings={\"a\":1}"]);
        assert_eq!(
            without_missing_settings("claude", &inline_equals, |_| false),
            inline_equals
        );
    }

    #[test]
    fn a_plan_built_over_a_missing_settings_file_still_resumes_without_it() {
        let plan = crate::agent_resume::plan(
            "herdr:claude",
            "claude",
            &crate::agent_resume::AgentSessionRef::id("s1").unwrap(),
        )
        .unwrap();
        let gone =
            std::env::temp_dir().join(format!("herdr-{}-never-written.json", std::process::id()));
        let launch = AgentLaunchFlags {
            agent: "claude".into(),
            flags: words(&["--settings", &gone.display().to_string(), "--model", "opus"]),
            started_at_ms: None,
        };

        let plan = plan.with_launch_flags(Some(&launch));

        assert!(
            !plan.argv.iter().any(|arg| arg == "--settings"),
            "{:?}",
            plan.argv
        );
        assert_eq!(&plan.argv[..3], ["claude", "--resume", "s1"]);
        assert!(
            plan.argv.iter().any(|arg| arg == "opus"),
            "the rest is kept"
        );
    }

    #[test]
    fn a_gpt_claude_launch_keeps_its_settings_file_and_model() {
        let argv = words(&[
            "claude",
            "--settings",
            "/Users/ac/.config/claude/gpt.settings.json",
            "--model",
            "gpt-6-astra",
            "--dangerously-skip-permissions",
            "fix the tests",
        ]);
        assert_eq!(
            carried_launch_flags("claude", &argv, None),
            words(&[
                "--settings",
                "/Users/ac/.config/claude/gpt.settings.json",
                "--model",
                "gpt-6-astra",
                "--dangerously-skip-permissions",
            ])
        );
    }

    #[test]
    fn every_claude_launch_only_flag_is_kept_in_launch_order() {
        let argv = words(&[
            "claude",
            "--settings=/s/gpt.json",
            "--add-dir",
            "/a",
            "/b",
            "--mcp-config",
            "/m.json",
            "--strict-mcp-config",
            "--agent",
            "reviewer",
            "--plugin-dir",
            "/p1",
            "--plugin-dir=/p2",
            "--setting-sources",
            "user,project",
        ]);
        assert_eq!(
            carried_launch_flags("claude", &argv, None),
            words(&[
                "--settings",
                "/s/gpt.json",
                "--add-dir",
                "/a",
                "/b",
                "--mcp-config",
                "/m.json",
                "--strict-mcp-config",
                "--agent",
                "reviewer",
                "--plugin-dir",
                "/p1",
                "--plugin-dir",
                "/p2",
                "--setting-sources",
                "user,project",
            ])
        );
    }

    #[test]
    fn a_restored_claude_keeps_its_effort_for_the_next_restart() {
        // Restore started it as `claude --resume <id> --model m --effort low`;
        // until it takes a turn, those flags are all a later restore has.
        let argv = words(&[
            "claude",
            "--resume",
            "s1",
            "--model",
            "claude-sonnet-5-5",
            "--effort",
            "low",
        ]);
        assert_eq!(
            carried_launch_flags("claude", &argv, None),
            words(&["--model", "claude-sonnet-5-5", "--effort", "low"])
        );
    }

    #[test]
    fn claude_flags_that_cannot_come_back_are_left_out() {
        let argv = words(&[
            "claude",
            "--settings",
            "{\"model\":\"x\"}",
            "--append-system-prompt",
            "be brief",
            "--session-id",
            "11111111-2222-3333-4444-555555555555",
            "--resume",
            "abc",
            "--fork-session",
            "-p",
            "--agent",
        ]);
        assert!(carried_launch_flags("claude", &argv, None).is_empty());
    }

    #[test]
    fn relative_launch_paths_become_absolute() {
        let argv = words(&[
            "claude",
            "--settings",
            "cfg/gpt.json",
            "--add-dir",
            "../other",
        ]);
        assert_eq!(
            carried_launch_flags("claude", &argv, Some(Path::new("/work/repo"))),
            vec![
                "--settings".to_string(),
                rooted(&["work", "repo", "cfg", "gpt.json"]),
                "--add-dir".to_string(),
                rooted(&["work", "other"]),
            ]
        );
    }

    #[test]
    fn a_hand_started_codex_keeps_model_sandbox_and_config() {
        let argv = words(&[
            "codex",
            "-m",
            "gpt-6-astra",
            "-s",
            "read-only",
            "-c",
            "service_tier=default",
            "-c",
            "model_reasoning_effort=medium",
            "--search",
            "--remote",
            "unix:///tmp/s",
            "-i",
            "shot.png",
            "explain this",
        ]);
        assert_eq!(
            carried_launch_flags("codex", &argv, None),
            words(&[
                "-m",
                "gpt-6-astra",
                "-s",
                "read-only",
                "-c",
                "service_tier=default",
                "-c",
                "model_reasoning_effort=medium",
                "--search",
            ])
        );
    }

    #[test]
    fn a_resumed_codex_launch_drops_the_session_it_resumed() {
        let argv = words(&[
            "codex",
            "resume",
            "01a0e9bb-0000-7000-8000-000000000000",
            "-m",
            "gpt-6-astra",
            "--dangerously-bypass-approvals-and-sandbox",
        ]);
        assert_eq!(
            carried_launch_flags("codex", &argv, None),
            words(&[
                "-m",
                "gpt-6-astra",
                "--dangerously-bypass-approvals-and-sandbox"
            ])
        );
    }

    #[test]
    fn an_agent_without_a_carry_list_keeps_nothing() {
        let argv = words(&["pi", "--model", "x"]);
        assert!(carried_launch_flags("pi", &argv, None).is_empty());
    }

    #[test]
    fn composing_adds_only_what_the_resume_command_lacks() {
        let base = words(&[
            "claude",
            "--resume",
            "s1",
            "--model",
            "gpt-6-astra",
            "--effort",
            "medium",
            "--permission-mode",
            "auto",
        ]);
        let launch = words(&[
            "--settings",
            "/u/gpt.json",
            "--model",
            "claude-opus-5-5",
            "--dangerously-skip-permissions",
            "--plugin-dir",
            "/p1",
            "--plugin-dir",
            "/p2",
        ]);
        assert_eq!(
            compose_resume_argv("claude", &base, &launch),
            words(&[
                "claude",
                "--resume",
                "s1",
                "--model",
                "gpt-6-astra",
                "--effort",
                "medium",
                "--permission-mode",
                "auto",
                "--settings",
                "/u/gpt.json",
                "--allow-dangerously-skip-permissions",
                "--plugin-dir",
                "/p1",
                "--plugin-dir",
                "/p2",
            ])
        );
    }

    #[test]
    fn a_bypass_launch_keeps_bypass_reachable_but_not_forced() {
        // As the hook does: started with --dangerously-skip-permissions, now
        // in another mode, the pane can still shift+tab back to bypass.
        let base = words(&["claude", "--resume", "s1", "--permission-mode", "plan"]);
        let launch = words(&["--dangerously-skip-permissions"]);
        assert_eq!(
            compose_resume_argv("claude", &base, &launch),
            words(&[
                "claude",
                "--resume",
                "s1",
                "--permission-mode",
                "plan",
                "--allow-dangerously-skip-permissions"
            ])
        );
        let already = words(&[
            "claude",
            "--resume",
            "s1",
            "--allow-dangerously-skip-permissions",
            "--permission-mode",
            "plan",
        ]);
        assert_eq!(compose_resume_argv("claude", &already, &launch), already);
    }

    #[test]
    fn composing_a_bare_codex_resume_adds_every_launch_flag() {
        let base = words(&["codex", "resume", "t1"]);
        let launch = words(&["-m", "gpt-6-astra", "-c", "a=1", "-c", "b=2"]);
        assert_eq!(
            compose_resume_argv("codex", &base, &launch),
            words(&[
                "codex",
                "resume",
                "t1",
                "-m",
                "gpt-6-astra",
                "-c",
                "a=1",
                "-c",
                "b=2"
            ])
        );
    }
}
