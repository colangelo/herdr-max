//! Which backend a pane's program starts on (openspec/changes/core-phase3
//! D13, D3a, D8). Every path into `PaneIo::spawn_preferred` passes a
//! `RuntimeChoice`, so a pane that skips core says why. A restored pane's
//! fresh program asks like any other (D8); a program adopted by fd in a live
//! handoff never reaches `spawn_preferred`, so it keeps its PTY.

use std::sync::{Mutex, PoisonError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeChoice {
    /// `[experimental] runtime` decides: core when it is `core` and core is
    /// usable, else the PTY.
    Preferred,
    Pty(PtyReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PtyReason {
    /// The pane's stable name matches `[experimental] core_pty_panes` (D13).
    Excluded,
    /// The core loss guard gave up on core (D3a).
    GaveUp,
}

/// `[experimental] core_pty_panes`, set at startup and on every reload.
static CORE_PTY_PANES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Applies `[experimental] core_pty_panes`; the next spawn reads it.
pub(crate) fn set_core_pty_panes(patterns: Vec<String>) {
    *CORE_PTY_PANES
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = patterns;
}

impl RuntimeChoice {
    /// The choice for a program in a pane whose stable name is
    /// `stable_name` (see [`stable_name`]): a configured exclusion first,
    /// then the loss guard.
    pub(crate) fn for_pane(stable_name: Option<&str>) -> Self {
        let excluded = stable_name.is_some_and(|name| {
            excluded_from_core(
                &CORE_PTY_PANES
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner),
                name,
            )
        });
        if excluded {
            Self::Pty(PtyReason::Excluded)
        } else if super::core_gave_up() {
            Self::Pty(PtyReason::GaveUp)
        } else {
            Self::Preferred
        }
    }

    /// A new pane that runs `argv`: its stable name is the `--name` the
    /// command gives its agent.
    pub(crate) fn for_argv(argv: &[String]) -> Self {
        Self::for_pane(crate::agent_resume::argv_launch_name(argv).as_deref())
    }
}

/// The name `core_pty_panes` matches (D13): the `--name` the pane's agent was
/// launched with, else the herdr agent name a person or the API set. A
/// terminal title or a name the agent reported about itself is never one.
pub(crate) fn stable_name<'a>(
    launch: Option<&'a crate::agent_resume::AgentLaunchFlags>,
    agent_name: Option<&'a str>,
) -> Option<&'a str> {
    launch
        .and_then(crate::agent_resume::AgentLaunchFlags::name)
        .or(agent_name)
}

/// D13's only matcher: whether a pattern matches the whole stable name.
pub(crate) fn excluded_from_core(patterns: &[String], stable_name: &str) -> bool {
    patterns
        .iter()
        .any(|pattern| glob_matches(pattern.as_bytes(), stable_name.as_bytes()))
}

/// `*` matches any run of characters, `?` one byte, anything else itself;
/// case-sensitive. Backtracks only to the last `*`, so it is linear in
/// practice.
fn glob_matches(pattern: &[u8], name: &[u8]) -> bool {
    let (mut p, mut n) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        match pattern.get(p) {
            Some(b'*') => {
                star = Some((p, n));
                p += 1;
            }
            Some(&c) if c == b'?' || c == name[n] => {
                p += 1;
                n += 1;
            }
            _ => match star {
                Some((star_p, star_n)) => {
                    p = star_p + 1;
                    n = star_n + 1;
                    star = Some((star_p, star_n + 1));
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&c| c == b'*')
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOSTS: [&str; 2] = ["manager-lab-hlp", "manager-aruba-hlp"];

    fn patterns(list: &[&str]) -> Vec<String> {
        list.iter().map(|pattern| pattern.to_string()).collect()
    }

    // V32 (core-phase3 D13): both computer-use host panes match the
    // explicit list and the `*-hlp` default of ac's configs.
    #[test]
    fn the_computer_use_host_panes_are_excluded() {
        for list in [&HOSTS[..], &["*-hlp"][..]] {
            for host in HOSTS {
                assert!(excluded_from_core(&patterns(list), host), "{list:?} {host}");
            }
        }
        let star = patterns(&["*-hlp"]);
        for other in [
            "manager-lab",
            "manager-lab-hlp-2",
            "hlp",
            "manager-lab-hlpx",
        ] {
            assert!(!excluded_from_core(&star, other), "{other}");
        }
        assert!(!excluded_from_core(&[], "manager-lab-hlp"));
    }

    #[test]
    fn glob_matches_whole_names() {
        let cases = [
            ("*", "", true),
            ("*", "any", true),
            ("a?c", "abc", true),
            ("a?c", "ac", false),
            ("*lab*", "manager-lab-hlp", true),
            ("m*-*-hlp", "manager-aruba-hlp", true),
            ("manager-lab-hlp", "Manager-Lab-Hlp", false),
            ("*-hlp", "-hlp", true),
            ("**-hlp", "x-hlp", true),
            ("", "", true),
            ("", "x", false),
        ];
        for (pattern, name, expected) in cases {
            assert_eq!(
                glob_matches(pattern.as_bytes(), name.as_bytes()),
                expected,
                "{pattern:?} {name:?}"
            );
        }
    }

    #[test]
    fn the_launch_name_wins_over_the_agent_name() {
        let launch = |flags: &[&str]| crate::agent_resume::AgentLaunchFlags {
            agent: "claude".into(),
            flags: flags.iter().map(|word| word.to_string()).collect(),
            started_at_ms: None,
            env: Vec::new(),
        };
        let named = launch(&["--settings", "/s.json", "--name", "manager-lab-hlp"]);
        assert_eq!(
            stable_name(Some(&named), Some("reviewer")),
            Some("manager-lab-hlp")
        );
        let short = launch(&["-n", "manager-aruba-hlp"]);
        assert_eq!(stable_name(Some(&short), None), Some("manager-aruba-hlp"));
        let unnamed = launch(&["--settings", "/s.json"]);
        assert_eq!(
            stable_name(Some(&unnamed), Some("reviewer")),
            Some("reviewer")
        );
        assert_eq!(stable_name(None, None), None);
    }

    // V32: the configured list decides each spawn, and a reload changes the
    // answer for the next one.
    #[test]
    fn the_choice_follows_the_configured_list() {
        set_core_pty_panes(patterns(&["*-hlp"]));
        for host in HOSTS {
            assert_eq!(
                RuntimeChoice::for_pane(Some(host)),
                RuntimeChoice::Pty(PtyReason::Excluded)
            );
        }
        let argv = [
            "/opt/bin/claude",
            "--model",
            "opus",
            "--name",
            "manager-lab-hlp",
        ]
        .map(String::from);
        assert_eq!(
            RuntimeChoice::for_argv(&argv),
            RuntimeChoice::Pty(PtyReason::Excluded)
        );
        assert_eq!(RuntimeChoice::for_pane(None), RuntimeChoice::Preferred);
        set_core_pty_panes(Vec::new());
        assert_eq!(
            RuntimeChoice::for_pane(Some("manager-lab-hlp")),
            RuntimeChoice::Preferred
        );
    }
}
