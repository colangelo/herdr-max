//! The herdr-core backend behind `PaneIo` (openspec/changes/core-backend).
//! Unix only: core is reached over a Unix socket.

pub(crate) mod client;
#[cfg(test)]
pub(crate) mod fake;
pub(crate) mod frame;
mod pane_io;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Map, Value};
use tracing::warn;

use client::{CoreClient, CoreError};
pub(crate) use pane_io::{Connector, CorePaneIo, CoreReattach, CoreSpawn, StartedCore};

use crate::layout::PaneId;

/// `[experimental] runtime = "core"` (design D1), set at startup and on reload.
static PREFER_CORE: AtomicBool = AtomicBool::new(false);
/// One unavailability warning per server lifetime (D1).
static WARNED_UNAVAILABLE: AtomicBool = AtomicBool::new(false);
/// Set after the first connection: from then on a missing socket is retried,
/// never answered by starting a core.
static EVER_CONNECTED: AtomicBool = AtomicBool::new(false);
/// The server's one connection (D3), replaced when it closes.
static CLIENT: Mutex<Option<CoreClient>> = Mutex::new(None);

pub(crate) fn set_preferred(core: bool) {
    PREFER_CORE.store(core, Ordering::Release);
}

pub(crate) fn preferred() -> bool {
    PREFER_CORE.load(Ordering::Acquire)
}

/// The core session for this herdr session (D2): `herdr` (`herdrdev` for a
/// debug build), and `-<name>` for a named session, so a throwaway or test
/// herdr never shares a core with ac's sessions or touches the default core.
/// No release name can equal a debug one: release names never start with
/// `herdrdev`.
pub(crate) fn core_session_name() -> String {
    session_name_for(
        crate::session::active_name().as_deref(),
        cfg!(debug_assertions),
    )
}

fn session_name_for(herdr_session: Option<&str>, debug_build: bool) -> String {
    let mut name = String::from(if debug_build { "herdrdev" } else { "herdr" });
    if let Some(session) = herdr_session {
        name.push('-');
        name.push_str(session);
    }
    name
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// The server's connector: the live connection, or a new one (starting the
/// core session when its socket is missing).
pub(crate) fn server_connector() -> Connector {
    connector(true)
}

/// Like `server_connector`, but never starts a core: one that is not running
/// holds no terminal to reattach to (D10).
pub(crate) fn existing_core_connector() -> Connector {
    connector(false)
}

fn connector(may_start: bool) -> Connector {
    Arc::new(move || {
        let mut slot = lock(&CLIENT);
        if let Some(client) = slot.as_ref() {
            if !client.is_closed() {
                return Ok(client.clone());
            }
        }
        // Start the core only before the first connection. Later, a missing
        // socket means a core restart in progress: starting one then would race
        // the restart and could leave a second core behind.
        let first = may_start && !EVER_CONNECTED.load(Ordering::Acquire);
        let session = core_session_name();
        let client = client::connect_session(&session, None, first)?;
        // No events subscription: core's status comes only from bytes a
        // program prints, which any printed file can forge, so herdr does not
        // use it (security review M3; design D9). Detection and hooks decide.
        EVER_CONNECTED.store(true, Ordering::Release);
        *slot = Some(client.clone());
        Ok(client)
    })
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
        argv,
        cwd,
        env,
        labels,
        rows,
        cols,
    })
}

/// A live core terminal labelled for this herdr session that no pane shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnclaimedTerminal {
    pub address: String,
    /// The pane it was started for (`herdr.pane_id`), when labelled.
    pub pane_id: Option<String>,
}

/// Live terminals labelled `herdr.session=<ours>` whose address is not in
/// `claimed` (D10 "Orphans"). Never starts a core: with none running there
/// are none, and an unreachable core answers an empty list.
pub(crate) fn unclaimed_terminals(claimed: &[String]) -> Vec<UnclaimedTerminal> {
    // A server that neither prefers core nor attached to a core pane does not
    // use core: it does not go looking in one. (A saved core pane that failed
    // to reattach under `runtime = "pty"` is logged at warn by the restore.)
    if !preferred() && claimed.is_empty() {
        return Vec::new();
    }
    let client = match existing_core_connector()() {
        Ok(client) => client,
        Err(err) => {
            tracing::debug!(err = %err, "no herdr-core to look for unclaimed terminals in");
            return Vec::new();
        }
    };
    let listed = match client.call(
        "terminal.list",
        serde_json::json!({"labels": {"herdr.session": core_session_name()}}),
        std::time::Duration::from_secs(5),
    ) {
        Ok(listed) => listed,
        Err(err) => {
            warn!(err = %err, "could not list herdr-core terminals to find unclaimed ones");
            return Vec::new();
        }
    };
    unclaimed_in(&listed, claimed)
}

fn unclaimed_in(listed: &Value, claimed: &[String]) -> Vec<UnclaimedTerminal> {
    listed["terminals"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|terminal| terminal["state"] == "live")
        .filter_map(|terminal| {
            let address = terminal["address"].as_str()?;
            (!claimed.iter().any(|claimed| claimed == address)).then(|| UnclaimedTerminal {
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
    labels.insert("herdr.session".into(), Value::String(core_session_name()));
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
    core.kill();
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
        return;
    }
    if !pids.is_empty() {
        super::pty::signal_session(pane_id, child_pid, &pids, child_wait_completed);
    }
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
    use super::*;

    #[test]
    fn core_session_names_follow_design_d2() {
        assert_eq!(session_name_for(None, false), "herdr");
        assert_eq!(session_name_for(None, true), "herdrdev");
        assert_eq!(session_name_for(Some("trial"), false), "herdr-trial");
        assert_eq!(session_name_for(Some("trial"), true), "herdrdev-trial");
        // A release session named `dev` must not reach a debug build's core.
        assert_ne!(
            session_name_for(Some("dev"), false),
            session_name_for(None, true)
        );
    }

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

    #[test]
    fn unclaimed_terminals_are_the_live_ones_no_pane_claimed() {
        let listed = serde_json::json!({"terminals": [
            {"address": "t1", "state": "live", "labels": {"herdr.pane_id": "w1-p1"}},
            {"address": "t2", "state": "live", "labels": {"herdr.pane_id": "w1-p2"}},
            {"address": "t3", "state": "finished", "labels": {}},
            {"address": "t4", "state": "live", "labels": {}},
        ]});
        assert_eq!(
            unclaimed_in(&listed, &["t1".to_string()]),
            vec![
                UnclaimedTerminal {
                    address: "t2".into(),
                    pane_id: Some("w1-p2".into())
                },
                UnclaimedTerminal {
                    address: "t4".into(),
                    pane_id: None
                },
            ]
        );
        assert!(unclaimed_in(&serde_json::json!({}), &[]).is_empty());
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
}
