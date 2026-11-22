//! The herdr-core backend behind `PaneIo` (openspec/changes/core-backend).
//! Unix only: core is reached over a Unix socket.

pub(crate) mod client;
#[cfg(test)]
pub(crate) mod fake;
pub(crate) mod frame;
mod pane_io;
mod primary;
mod sessions;
pub(crate) mod spaces_doc;
mod ssh_start;
mod status_mirror;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde_json::{Map, Value};
use tracing::warn;

use client::CoreError;
pub(crate) use pane_io::{
    Connector, CorePaneIo, CoreReattach, CoreSpawn, KillOutcome, ReattachError, StartFailure,
    StartedCore,
};
pub(crate) use sessions::{
    core_session_name, existing_core_connector, existing_session_connector, herdr_session_label,
    known_sessions, note_stopped_by_person, release_secondary, server_connector,
};

use super::UnclaimedCoreTerminal;
use crate::layout::PaneId;

/// `[experimental] runtime = "core"` (design D1), set at startup and on reload.
static PREFER_CORE: AtomicBool = AtomicBool::new(false);
/// One unavailability warning per server lifetime (D1).
static WARNED_UNAVAILABLE: AtomicBool = AtomicBool::new(false);
/// Loss bursts (core-phase3 D3a). Its own lock: the server loop records
/// losses and must never wait behind a connect.
static LOSS_GUARD: Mutex<primary::CoreLossGuard> = Mutex::new(primary::CoreLossGuard::new());
static GAVE_UP: AtomicBool = AtomicBool::new(false);
/// Where the guard's "gave up" goes, for the server's one notice.
static EVENTS: Mutex<Option<tokio::sync::mpsc::Sender<crate::events::AppEvent>>> = Mutex::new(None);

pub(crate) fn set_preferred(core: bool) {
    PREFER_CORE.store(core, Ordering::Release);
}

pub(crate) fn preferred() -> bool {
    PREFER_CORE.load(Ordering::Acquire)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// What a label search found, as `(session, address)`, and the sessions
/// that did not answer: those may hold a match, so an empty search with
/// one is no proof of none (security review M3, Sol H6).
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct LabelledTerminals {
    pub found: Vec<(String, String)>,
    pub unanswered: Vec<String>,
}

/// Live terminals carrying every one of `labels`, in each session that
/// answers (core-phase3 D1b). None is started. A session with no socket
/// has no core and so no terminal; any other failure leaves it unanswered.
pub(crate) fn find_labelled_terminals(
    sessions: &[(String, Connector)],
    labels: &Map<String, Value>,
) -> LabelledTerminals {
    let mut search = LabelledTerminals::default();
    for (session, connector) in sessions {
        let listed = connector().and_then(|client| {
            client.call(
                "terminal.list",
                serde_json::json!({"labels": labels}),
                std::time::Duration::from_secs(5),
            )
        });
        let listed = match listed {
            Ok(listed) => listed,
            Err(CoreError::NoCore(_)) => continue,
            Err(err) => {
                tracing::warn!(%session, err = %err, "could not list herdr-core terminals; the search is incomplete");
                search.unanswered.push(session.clone());
                continue;
            }
        };
        search.found.extend(
            listed["terminals"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|terminal| terminal["state"] == "live")
                .filter(|terminal| {
                    labels
                        .iter()
                        .all(|(key, want)| &terminal["labels"][key] == want)
                })
                .filter_map(|terminal| terminal["address"].as_str())
                .map(|address| (session.clone(), address.to_string())),
        );
    }
    search
}

/// A loss event for the guard (core-phase3 D3a): core panes suspended, or
/// the primary started after it went down. True when this one made the
/// guard give up; the server is then told once, through `set_events`.
pub(crate) fn note_core_loss(now: std::time::Instant) -> bool {
    if !lock(&LOSS_GUARD).record(now) {
        return false;
    }
    GAVE_UP.store(true, Ordering::Release);
    warn!("herdr-core keeps ending panes; no more core starts, panes resume on the PTY");
    tell_server(crate::events::AppEvent::CoreGaveUp);
    true
}

fn tell_server(event: crate::events::AppEvent) {
    if let Some(events) = lock(&EVENTS).as_ref() {
        let _ = events.try_send(event);
    }
}

/// Raises the one warning and notice when the core at `core_pid`, which
/// `session` was just connected to, was started over SSH (core-phase3 D7).
/// The server never restarts that core: other clients share it.
fn check_ssh_start(session: &str, core_pid: u32) {
    if core_pid == 0 || !ssh_start::started_over_ssh(core_pid, &ssh_start::LiveProcesses) {
        return;
    }
    warn!(
        session,
        core_pid, "herdr-core was started over SSH; restart it from a desktop terminal"
    );
    tell_server(crate::events::AppEvent::CoreStartedOverSsh {
        session: session.to_string(),
    });
}

pub(crate) fn gave_up() -> bool {
    GAVE_UP.load(Ordering::Acquire)
}

/// A config reload: the guard starts afresh.
pub(crate) fn reset_loss_guard() {
    *lock(&LOSS_GUARD) = primary::CoreLossGuard::new();
    GAVE_UP.store(false, Ordering::Release);
}

pub(crate) fn set_events(events: tokio::sync::mpsc::Sender<crate::events::AppEvent>) {
    *lock(&EVENTS) = Some(events);
}

/// Whether the primary core answers, through the server's connector, which
/// starts it again by D3's rule.
pub(crate) fn primary_answers() -> bool {
    server_connector()().is_ok()
}

/// Logs the first reason core could not be used; later ones are debug only.
/// True the first time in this server's life, when the caller raises the
/// one notice (D1).
pub(crate) fn note_unavailable(err: &CoreError) -> bool {
    if !WARNED_UNAVAILABLE.swap(true, Ordering::AcqRel) {
        warn!(err = %err, "herdr-core unavailable; new panes use the PTY");
        true
    } else {
        tracing::debug!(err = %err, "herdr-core unavailable; pane uses the PTY");
        false
    }
}

/// The names of the variables a core pane keeps, space-separated (M1).
const KEEP_ENV_VAR: &str = "HERDR_CORE_KEEP_ENV";

/// Starts the program with exactly the variables named in `KEEP_ENV_VAR`:
/// every other variable in the environment core gives it is unset with
/// `env -u`, `KEEP_ENV_VAR` included. Names are read from `env`'s output;
/// a value with a newline can only add a bogus name, which is unset harmlessly.
/// `unset IFS; set -f` first: an `IFS` or glob characters in core's own
/// environment must not change how the names split. A variable whose name
/// is outside `[A-Za-z_][A-Za-z0-9_]*` is not unset: `sed` skips it, and no
/// real credential uses such a name.
const KEEP_ENV_SCRIPT: &str = r#"unset IFS; set -f
keep=" $HERDR_CORE_KEEP_ENV "
set -- -- "$@"
for name in $(/usr/bin/env | /usr/bin/sed -n 's/^\([A-Za-z_][A-Za-z0-9_]*\)=.*/\1/p'); do
  case "$keep" in
    *" $name "*) ;;
    *) set -- -u "$name" "$@" ;;
  esac
done
exec /usr/bin/env "$@""#;

/// Turns a pane's command into a core spawn (D4). `None` when the command
/// cannot be passed to core faithfully (a non-UTF-8 argument or variable); the
/// pane then uses the PTY.
pub(crate) fn core_spawn_from(
    pane_id: PaneId,
    command: &portable_pty::CommandBuilder,
    rows: u16,
    cols: u16,
) -> Option<CoreSpawn> {
    let program: Vec<String> = if command.is_default_prog() {
        vec![command.get_shell()]
    } else {
        command
            .get_argv()
            .iter()
            .map(|arg| arg.to_str().map(str::to_string))
            .collect::<Option<_>>()?
    };
    let mut env = Map::new();
    for (key, value) in command.iter_full_env_as_str() {
        env.insert(key.to_string(), Value::String(value.to_string()));
    }
    // core merges `env` over its own environment, which is whoever started
    // or updated it, and cannot unset (task 1.3; security review M1). The
    // wrapper keeps exactly the keys named in `KEEP_ENV_VAR` and drops every
    // other one core would hand on. Only names travel in it, never values.
    let mut keep: Vec<&str> = env.keys().map(String::as_str).collect();
    keep.sort_unstable();
    let keep = keep.join(" ");
    env.insert(KEEP_ENV_VAR.into(), Value::String(keep));
    let mut argv = vec![
        "/bin/sh".to_string(),
        "-c".to_string(),
        KEEP_ENV_SCRIPT.to_string(),
        "herdr-core-env".to_string(),
    ];
    argv.extend(program);
    let cwd = match command.get_cwd() {
        Some(cwd) => Some(cwd.to_str()?.to_string()),
        None => None,
    };
    let labels = pane_labels(
        env.get(crate::integration::HERDR_PANE_ID_ENV_VAR)
            .and_then(Value::as_str),
    );
    Some(CoreSpawn {
        pane_id,
        session: core_session_name(),
        argv,
        cwd,
        env,
        labels,
        rows,
        cols,
    })
}

/// Live terminals labelled `herdr.session=<label>` that no pane claims, in
/// every core session herdr knows (D10 "Orphans", core-phase3 D5 and D6).
/// Claims are `(session, address)`: addresses are per core, so `t7` can
/// live in two. Never starts a core: one that does not answer has none to
/// list. A secondary that no pane claims and that holds none is released.
pub(crate) fn unclaimed_terminals(claimed: &[(String, String)]) -> Vec<UnclaimedCoreTerminal> {
    // A server that neither prefers core nor attached to a core pane does not
    // use core: it does not go looking in one. (A saved core pane that failed
    // to reattach under `runtime = "pty"` is logged at warn by the restore.)
    if !preferred() && claimed.is_empty() {
        return Vec::new();
    }
    let label = herdr_session_label();
    let primary = core_session_name();
    let mut unclaimed = Vec::new();
    for session in known_sessions(claimed.iter().map(|(session, _)| session.as_str())) {
        let claimed_here: Vec<&str> = claimed
            .iter()
            .filter(|(claim_session, _)| *claim_session == session)
            .map(|(_, address)| address.as_str())
            .collect();
        let found = unclaimed_in_session(&session, &label, &claimed_here);
        if session != primary && claimed_here.is_empty() && found.is_empty() {
            release_secondary(&session);
        }
        unclaimed.extend(found);
    }
    unclaimed
}

fn unclaimed_in_session(
    session: &str,
    label: &str,
    claimed: &[&str],
) -> Vec<UnclaimedCoreTerminal> {
    let client = match existing_session_connector(session)() {
        Ok(client) => client,
        Err(err) => {
            tracing::debug!(%session, err = %err, "no herdr-core to look for unclaimed terminals in");
            return Vec::new();
        }
    };
    let listed = match client.call(
        "terminal.list",
        serde_json::json!({"labels": {"herdr.session": label}}),
        std::time::Duration::from_secs(5),
    ) {
        Ok(listed) => listed,
        Err(err) => {
            warn!(%session, err = %err, "could not list herdr-core terminals to find unclaimed ones");
            return Vec::new();
        }
    };
    unclaimed_in(session, &listed, claimed)
}

fn unclaimed_in(session: &str, listed: &Value, claimed: &[&str]) -> Vec<UnclaimedCoreTerminal> {
    listed["terminals"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|terminal| terminal["state"] == "live")
        .filter_map(|terminal| {
            let address = terminal["address"].as_str()?;
            (!claimed.contains(&address)).then(|| UnclaimedCoreTerminal {
                session: session.to_string(),
                address: address.to_string(),
                pane_id: terminal["labels"]["herdr.pane_id"]
                    .as_str()
                    .map(str::to_string),
            })
        })
        .collect()
}

/// The labels that mark a core terminal as this herdr session's pane (D4,
/// D10): set on `terminal.run`, checked before a reattach.
pub(crate) fn pane_labels(public_pane_id: Option<&str>) -> Map<String, Value> {
    let mut labels = Map::new();
    labels.insert("herdr.session".into(), Value::String(herdr_session_label()));
    if let Some(public) = public_pane_id {
        labels.insert("herdr.pane_id".into(), Value::String(public.to_string()));
    }
    labels
}

/// Closing a core pane (D8): list the program's session, kill the terminal,
/// then run the same local sweep as the PTY backend over that list, unless the
/// session is core's own.
pub(crate) fn terminate_session(
    core: &CorePaneIo,
    pane_id: PaneId,
    child_pid: u32,
    child_wait_completed: Option<&AtomicBool>,
) {
    // The next server takes the program over (`herdr server restart`, D10).
    if core.kept_on_drop() {
        return;
    }
    end_program(core, pane_id, child_pid, child_wait_completed, true);
}

/// The idle move's kill (core-phase3 D9): as a close, but the local sweep
/// runs only once core confirms the terminal ended, and the answer says
/// whether it did. The pane's stream stays attached either way.
pub(crate) fn end_program_confirmed(
    core: &CorePaneIo,
    pane_id: PaneId,
    child_pid: u32,
    child_wait_completed: Option<&AtomicBool>,
) -> Result<(), KillOutcome> {
    let outcome = end_program(core, pane_id, child_pid, child_wait_completed, false);
    if outcome.ended() {
        Ok(())
    } else {
        Err(outcome)
    }
}

fn end_program(
    core: &CorePaneIo,
    pane_id: PaneId,
    child_pid: u32,
    child_wait_completed: Option<&AtomicBool>,
    sweep_unconfirmed: bool,
) -> KillOutcome {
    // List the session before the kill: once core ends the session leader,
    // its session can no longer be looked up, and detached jobs would escape.
    // Without core's pids the guard cannot tell core's session from the
    // program's, so no sweep (security review M2): core's own kill still runs.
    let core_pids = core.core_pids();
    let unknown = core_pids.is_empty();
    let guarded = unknown || sweep_would_reach_core(child_pid, &core_pids);
    let pids = if guarded {
        Vec::new()
    } else {
        super::pty::session_pids(child_pid, child_wait_completed)
    };
    let outcome = core.kill();
    if guarded {
        if unknown {
            warn!(
                pane = pane_id.raw(),
                pid = child_pid,
                "herdr-core did not say which processes are its own; not sweeping the pane's session"
            );
        } else {
            warn!(
                pane = pane_id.raw(),
                pid = child_pid,
                "herdr-core program shares a session with core; not sweeping it"
            );
        }
        return outcome;
    }
    if !pids.is_empty() && (sweep_unconfirmed || outcome.ended()) {
        super::pty::signal_session(pane_id, child_pid, &pids, child_wait_completed);
    }
    outcome
}

/// True when the program's session is the session of a core server or holder
/// (the guard in D8; never true on hc 0.1.21, where each leads its own).
fn sweep_would_reach_core(child_pid: u32, core_pids: &[u32]) -> bool {
    if child_pid == 0 {
        return false;
    }
    let session = unsafe { libc::getsid(child_pid as libc::pid_t) };
    session > 0
        && core_pids
            .iter()
            .any(|pid| *pid == child_pid || unsafe { libc::getsid(*pid as libc::pid_t) } == session)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use client::CoreClient;

    #[test]
    fn a_core_spawn_names_the_panes_keys_and_runs_through_the_keep_wrapper() {
        let mut command = portable_pty::CommandBuilder::new("/bin/sh");
        command.arg("-c");
        command.arg("echo hi");
        command.cwd("/tmp");
        command.env_clear();
        command.env("PATH", "/usr/bin:/bin");
        command.env(crate::integration::HERDR_PANE_ID_ENV_VAR, "w1:p2");
        let spawn = core_spawn_from(PaneId::from_raw(1), &command, 24, 80).expect("spawn");

        assert_eq!(
            spawn.argv,
            [
                "/bin/sh",
                "-c",
                KEEP_ENV_SCRIPT,
                "herdr-core-env",
                "/bin/sh",
                "-c",
                "echo hi"
            ]
        );
        assert_eq!(
            spawn.env.get(KEEP_ENV_VAR),
            Some(&Value::String("HERDR_PANE_ID PATH".into()))
        );
        assert_eq!(spawn.cwd.as_deref(), Some("/tmp"));
        assert_eq!(
            spawn.env.get("PATH"),
            Some(&Value::String("/usr/bin:/bin".into()))
        );
        assert_eq!(
            spawn.labels.get("herdr.pane_id"),
            Some(&Value::String("w1:p2".into()))
        );
        assert!(spawn.labels.contains_key("herdr.session"));
    }

    // Security review M1: whatever core's own environment holds (its starter's
    // tokens), the program sees exactly the keys the pane named, with the
    // pane's values, even keys that look like credentials on purpose.
    #[test]
    fn the_keep_wrapper_drops_every_variable_the_pane_did_not_name() {
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", KEEP_ENV_SCRIPT, "herdr-core-env", "/usr/bin/env"])
            .env_clear()
            .env(KEEP_ENV_VAR, "PATH ANTHROPIC_API_KEY ODD_VALUE")
            .env("PATH", "/usr/bin:/bin")
            .env("ANTHROPIC_API_KEY", "set-by-the-pane")
            .env("ODD_VALUE", "line one\nANTHROPIC_AUTH_TOKEN=forged")
            .env("ANTHROPIC_AUTH_TOKEN", "canary-auth")
            .env("CLAUDE_CODE_OAUTH_TOKEN", "canary-oauth")
            .env("HC_SESSION", "herdr")
            .output()
            .expect("run the wrapper");
        assert!(output.status.success(), "{output:?}");
        let text = String::from_utf8_lossy(&output.stdout);
        let mut names: Vec<&str> = text
            .lines()
            .filter_map(|line| line.split_once('=').map(|(name, _)| name))
            .filter(|name| !name.contains(' '))
            .collect();
        names.sort_unstable();
        names.dedup();
        assert!(!text.contains("canary"), "{text}");
        assert!(!text.contains("HC_SESSION"), "{text}");
        assert!(!text.contains(KEEP_ENV_VAR), "{text}");
        assert!(text.contains("ANTHROPIC_API_KEY=set-by-the-pane"), "{text}");
        assert!(
            text.contains("ODD_VALUE=line one\nANTHROPIC_AUTH_TOKEN=forged"),
            "{text}"
        );
        // What remains is the pane's keys plus what the shell itself sets.
        for name in names {
            assert!(
                [
                    "PATH",
                    "ANTHROPIC_API_KEY",
                    "ODD_VALUE",
                    "ANTHROPIC_AUTH_TOKEN",
                    "PWD",
                    "SHLVL",
                    "_"
                ]
                .contains(&name),
                "unexpected {name} in {text}"
            );
        }
    }

    // Shells that import `IFS` from the environment (dash does) would split
    // the names on it; the wrapper resets it first.
    #[test]
    fn the_keep_wrapper_is_not_misled_by_an_ifs_in_cores_environment() {
        for ifs in ["N", "_", "T"] {
            let output = std::process::Command::new("/bin/sh")
                .args(["-c", KEEP_ENV_SCRIPT, "herdr-core-env", "/usr/bin/env"])
                .env_clear()
                .env(KEEP_ENV_VAR, "PATH")
                .env("PATH", "/usr/bin:/bin")
                .env("IFS", ifs)
                .env("ANTHROPIC_AUTH_TOKEN", "canary-auth")
                .env("CLAUDE_CODE_OAUTH_TOKEN", "canary-oauth")
                .output()
                .expect("run the wrapper");
            assert!(output.status.success(), "{output:?}");
            let text = String::from_utf8_lossy(&output.stdout);
            assert!(!text.contains("canary"), "IFS={ifs}: {text}");
            assert!(text.contains("PATH=/usr/bin:/bin"), "IFS={ifs}: {text}");
        }
    }

    // V9 (D1): with `runtime = "core"` and a core that cannot be used (here
    // too old, then refusing the connection), panes run on the PTY and the
    // server raises exactly one notice.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_unusable_core_falls_back_to_the_pty_with_one_notice() {
        use crate::pane::runtime_characterization_tests::{spawn_sh, Scratch};
        let core = fake::FakeCore::start_with(|config| config.version = "0.1.17".into());
        // Nextest runs each test in its own process, so this stays here.
        std::env::set_var("HC_SOCKET", &core.socket);
        set_preferred(true);
        let scratch = Scratch::new("core-v9");
        let mut notices = Vec::new();
        for _ in 0..2 {
            let (runtime, mut events) = spawn_sh(
                &scratch.0,
                "exec sleep 30",
                &[],
                &crate::pane::PaneLaunchEnv::default(),
                24,
                80,
                0,
            );
            assert!(
                matches!(runtime.io, crate::pane::runtime_io::PaneIo::Actor(_)),
                "the pane runs on the PTY"
            );
            while let Ok(event) = events.try_recv() {
                if let crate::events::AppEvent::CoreUnavailable { reason } = event {
                    notices.push(reason);
                }
            }
            runtime.shutdown();
        }
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert!(notices[0].contains("older than"), "{notices:?}");
    }

    // Security review M3 (Sol H7): when core may have started the program,
    // the pane is not started on the PTY; when the launch is found again,
    // the pane shows it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_launch_core_may_run_never_falls_back_to_the_pty() {
        use crate::pane::runtime_characterization_tests::Scratch;
        let core = fake::FakeCore::start_with(|config| {
            config.lose_run_reply = true;
            config.failing = vec!["terminal.list"];
        });
        std::env::set_var("HC_SOCKET", &core.socket);
        set_preferred(true);
        let scratch = Scratch::new("core-h7");
        let marker = scratch.0.join("pty-ran");
        let (events, _rx) = tokio::sync::mpsc::channel(64);
        let argv = vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            format!("echo ran > {}; exec sleep 30", marker.display()),
        ];
        let started = crate::pane::PaneRuntime::spawn_argv_command(
            PaneId::alloc(),
            24,
            80,
            scratch.0.clone(),
            &argv,
            &crate::pane::PaneLaunchEnv::default(),
            crate::pane::AgentDetection::Disabled,
            crate::pane::RuntimeChoice::Preferred,
            0,
            crate::terminal_theme::TerminalTheme::default(),
            None,
            events,
            std::sync::Arc::new(tokio::sync::Notify::new()),
            std::sync::Arc::new(crate::render_signal::RenderSignal::new()),
        );
        assert!(started.is_err(), "the pane did not start anywhere");
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(!marker.exists(), "no PTY copy ran");
        assert_eq!(
            core.methods()
                .iter()
                .filter(|method| *method == "terminal.run")
                .count(),
            1
        );
    }

    #[test]
    fn unclaimed_terminals_are_the_live_ones_no_pane_claimed() {
        let listed = serde_json::json!({"terminals": [
            {"address": "t1", "state": "live", "labels": {"herdr.pane_id": "w1-p1"}},
            {"address": "t2", "state": "live", "labels": {"herdr.pane_id": "w1-p2"}},
            {"address": "t3", "state": "finished", "labels": {}},
            {"address": "t4", "state": "live", "labels": {}},
        ]});
        assert_eq!(
            unclaimed_in("herdr", &listed, &["t1"]),
            vec![
                UnclaimedCoreTerminal {
                    session: "herdr".into(),
                    address: "t2".into(),
                    pane_id: Some("w1-p2".into())
                },
                UnclaimedCoreTerminal {
                    session: "herdr".into(),
                    address: "t4".into(),
                    pane_id: None
                },
            ]
        );
        assert!(unclaimed_in("herdr", &serde_json::json!({}), &[]).is_empty());
    }

    // The orphan scan against a core: the live terminals labelled for this
    // session that no pane claims, never another client's, and nothing is
    // killed (core-backend D10 "Orphans").
    #[test]
    fn the_orphan_scan_lists_our_unclaimed_terminals_and_kills_nothing() {
        let core = fake::FakeCore::start();
        std::env::set_var("HC_SOCKET", &core.socket);
        set_preferred(true);
        let ours = |address: &str, pane: &str| {
            serde_json::json!({"address": address, "pid": 7, "state": "live",
                "labels": {"herdr.session": herdr_session_label(), "herdr.pane_id": pane}})
        };
        core.add_terminal(ours("t1", "w1-p1"));
        core.add_terminal(ours("t2", "w1-p2"));
        core.add_terminal(
            serde_json::json!({"address": "t3", "state": "live", "labels": {"app": "phone"}}),
        );

        assert_eq!(
            unclaimed_terminals(&[(core_session_name(), "t1".to_string())]),
            vec![UnclaimedCoreTerminal {
                session: core_session_name(),
                address: "t2".into(),
                pane_id: Some("w1-p2".into())
            }]
        );
        assert!(
            !core
                .methods()
                .iter()
                .any(|method| method == "terminal.kill"),
            "{:?}",
            core.methods()
        );
    }

    fn kill_free(core: &fake::FakeCore) -> bool {
        !core
            .methods()
            .iter()
            .any(|method| method == "terminal.kill")
    }

    // V6 (core-phase3 D5, D6): two core sessions, `t7` in both. Each `t7` is
    // claimed only by the pane linked to its own session; an unlabelled
    // terminal in `default` is never listed or killed; orphans come back
    // with their session; a secondary with no claim and no orphan is
    // released.
    #[test]
    fn claims_are_per_core_session_and_orphans_name_their_session() {
        let dir = std::env::temp_dir().join(format!("hcv6-{}", std::process::id()));
        std::env::set_var("XDG_RUNTIME_DIR", &dir);
        let primary = fake::FakeCore::start();
        std::env::set_var("HC_SOCKET", &primary.socket);
        set_preferred(true);
        let ours = |address: &str, pane: &str| {
            serde_json::json!({"address": address, "pid": 7, "state": "live",
                "labels": {"herdr.session": herdr_session_label(), "herdr.pane_id": pane}})
        };
        let herdr = fake::FakeCore::start_in(dir.join("herdr-core"), "herdr.sock", |_| {});
        herdr.add_terminal(ours("t7", "w1-p1"));
        herdr.add_terminal(ours("t8", "w1-p8"));
        let default = fake::FakeCore::start_in(dir.join("herdr-core"), "default.sock", |_| {});
        default.add_terminal(ours("t7", "w1-p2"));
        default.add_terminal(ours("t5", "w1-p5"));
        default.add_terminal(
            serde_json::json!({"address": "t9", "state": "live", "labels": {"app": "phone"}}),
        );
        let empty = fake::FakeCore::start_in(dir.join("herdr-core"), "herdr-x.sock", |_| {});
        existing_session_connector("herdr-x")().expect("herdr-x answers");

        let claim = |session: &str, address: &str| (session.to_string(), address.to_string());
        let found = unclaimed_terminals(&[claim("herdr", "t7"), claim("default", "t5")]);
        let orphan = |session: &str, address: &str, pane: &str| UnclaimedCoreTerminal {
            session: session.into(),
            address: address.into(),
            pane_id: Some(pane.into()),
        };
        assert_eq!(
            found,
            [
                orphan("herdr", "t8", "w1-p8"),
                orphan("default", "t7", "w1-p2")
            ],
            "herdr's t7 does not claim default's"
        );
        assert_eq!(
            unclaimed_terminals(&[
                claim("herdr", "t7"),
                claim("herdr", "t8"),
                claim("default", "t7"),
                claim("default", "t5"),
            ]),
            []
        );
        assert!(kill_free(&herdr) && kill_free(&default) && kill_free(&empty));
        assert_eq!(
            known_sessions(std::iter::empty()),
            [core_session_name(), "default".into(), "herdr".into()],
            "herdr-x had nothing and was released"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn the_sweep_guard_refuses_a_session_shared_with_core() {
        let me = std::process::id();
        assert!(sweep_would_reach_core(me, &[me]), "the same process");
        assert!(!sweep_would_reach_core(me, &[]), "no core pids known");
        assert!(!sweep_would_reach_core(0, &[me]), "no program pid");
    }

    /// A core pane on a fake core, with callbacks that do nothing.
    fn fake_core_pane(core: &fake::FakeCore) -> CorePaneIo {
        let client = CoreClient::connect(&core.socket, None).expect("connect");
        let connector: Connector = Arc::new(move || Ok(client.clone()));
        let spawn = CoreSpawn {
            pane_id: PaneId::from_raw(5),
            session: core_session_name(),
            argv: vec!["sh".into()],
            cwd: None,
            env: Map::new(),
            labels: pane_labels(Some("w1-p5")),
            rows: 24,
            cols: 80,
        };
        let pid = std::sync::atomic::AtomicU32::new(0);
        StartedCore::start(spawn, &pid, connector)
            .expect("start")
            .into_pane(
                Box::new(|_: &[u8]| crate::pty::actor::PtyReadResult {
                    terminal_responses: Vec::new(),
                }),
                Box::new(|_| {}),
                Box::new(|_| {}),
                Arc::new(AtomicBool::new(false)),
            )
            .expect("pane")
    }

    fn kills(core: &fake::FakeCore) -> usize {
        core.methods()
            .iter()
            .filter(|method| *method == "terminal.kill")
            .count()
    }

    #[test]
    fn a_kept_core_pane_is_detached_not_killed_and_a_normal_one_is_killed() {
        // `herdr server restart` keeps the program for the next server (D10).
        let core = fake::FakeCore::start();
        let pane = fake_core_pane(&core);
        pane.set_keep_on_drop(true);
        pane.shutdown();
        terminate_session(&pane, PaneId::from_raw(5), 0, None);
        assert_eq!(kills(&core), 0, "kept: {:?}", core.methods());

        // `server.stop`, a pane close, a respawn, and a handoff the next
        // server took over (`assume_handoff_ownership`) end it (D8).
        pane.set_keep_on_drop(false);
        terminate_session(&pane, PaneId::from_raw(5), 0, None);
        assert_eq!(kills(&core), 1, "{:?}", core.methods());
    }

    // V39 (core-phase3 D9, roundtable item 8): the move's kill counts as
    // confirmed only when core answers `finished` or `not_found`; a `live`
    // answer, an error or no answer leaves the pane to the caller unchanged.
    #[test]
    fn only_a_finished_or_unknown_terminal_confirms_the_kill() {
        let confirmed = |configure: fn(&mut fake::FakeConfig)| {
            let core = fake::FakeCore::start_with(configure);
            let pane = fake_core_pane(&core);
            end_program_confirmed(&pane, PaneId::from_raw(5), 0, None)
                .map_err(|outcome| outcome.to_string())
        };
        assert_eq!(confirmed(|_| {}), Ok(()));
        assert_eq!(
            confirmed(|config| config.kill_leaves_live = true),
            Err("herdr-core says the terminal still runs".into())
        );
        assert!(confirmed(|config| config.failing.push("terminal.kill")).is_err());

        let rpc = |kind: &str| CoreError::Rpc {
            kind: kind.into(),
            message: String::new(),
        };
        assert!(KillOutcome::from_answer(Err(rpc("not_found"))).ended());
        assert!(KillOutcome::from_answer(Err(rpc("finished"))).ended());
        assert!(!KillOutcome::from_answer(Err(CoreError::Timeout("terminal.kill"))).ended());
        assert!(!KillOutcome::from_answer(Err(CoreError::Closed)).ended());
        assert!(!KillOutcome::from_answer(Ok(serde_json::json!({"state": "live"}))).ended());
        assert!(
            !KillOutcome::from_answer(Ok(serde_json::json!({}))).ended(),
            "no state is no proof"
        );
    }
}
