//! The herdr-core backend behind `PaneIo` (openspec/changes/core-backend).
//! Unix only: core is reached over a Unix socket.

pub(crate) mod client;
#[cfg(test)]
pub(crate) mod fake;
pub(crate) mod frame;
mod pane_io;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Map, Value};
use tracing::warn;

use client::{CoreClient, CoreError};
pub(crate) use pane_io::{Connector, CorePaneIo, CoreReattach, CoreSpawn, StartedCore};

use crate::layout::PaneId;
use crate::pane::runtime_io::{ProgramState, ProgramStatus, StatusCallback};

/// `[experimental] runtime = "core"` (design D1), set at startup and on reload.
static PREFER_CORE: AtomicBool = AtomicBool::new(false);
/// One unavailability warning per server lifetime (D1).
static WARNED_UNAVAILABLE: AtomicBool = AtomicBool::new(false);
/// Set after the first connection: from then on a missing socket is retried,
/// never answered by starting a core.
static EVER_CONNECTED: AtomicBool = AtomicBool::new(false);
/// Each core pane's status callback, by terminal address (D9).
static STATUS: Mutex<Option<HashMap<String, Arc<StatusCallback>>>> = Mutex::new(None);
/// The server's one connection (D3), replaced when it closes.
static CLIENT: Mutex<Option<CoreClient>> = Mutex::new(None);

pub(crate) fn set_preferred(core: bool) {
    PREFER_CORE.store(core, Ordering::Release);
}

pub(crate) fn preferred() -> bool {
    PREFER_CORE.load(Ordering::Acquire)
}

/// The core session for this herdr session (D2): `herdr`, `herdr-dev` for a
/// debug build, and `-<name>` for a named session, so a throwaway or test herdr
/// never shares a core with ac's sessions or touches the default core.
pub(crate) fn core_session_name() -> String {
    session_name_for(
        crate::session::active_name().as_deref(),
        cfg!(debug_assertions),
    )
}

fn session_name_for(herdr_session: Option<&str>, debug_build: bool) -> String {
    let mut name = String::from("herdr");
    if debug_build {
        name.push_str("-dev");
    }
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
        // Status for our terminals only (D9). Failing to subscribe costs status
        // reports, not the pane: detection still runs.
        if let Err(err) = client.subscribe_events(
            serde_json::json!({"labels": {"herdr.session": session}}),
            Box::new(route_event),
            std::time::Duration::from_secs(5),
        ) {
            warn!(err = %err, "herdr-core events not subscribed; status comes from detection only");
        }
        EVER_CONNECTED.store(true, Ordering::Release);
        *slot = Some(client.clone());
        Ok(client)
    })
}

pub(crate) fn register_status(address: &str, callback: StatusCallback) {
    lock(&STATUS)
        .get_or_insert_with(HashMap::new)
        .insert(address.to_string(), Arc::new(callback));
}

pub(crate) fn forget_status(address: &str) {
    if let Some(map) = lock(&STATUS).as_mut() {
        map.remove(address);
    }
}

/// Routes a core notification: a `status` event for one of our terminals goes
/// to its pane (D9). Everything else is ignored here.
fn route_event(message: &Value) {
    if message["method"] != "event" || message["params"]["type"] != "status" {
        return;
    }
    let Some(address) = message["params"]["terminal"].as_str() else {
        return;
    };
    let Some(status) = parse_status(&message["params"]["data"]["status"]) else {
        return;
    };
    // Call outside the lock: a callback may wait on the app's event queue.
    let callback = lock(&STATUS)
        .as_ref()
        .and_then(|map| map.get(address).cloned());
    if let Some(callback) = callback {
        callback(status);
    }
}

/// A core status record as a `ProgramStatus`; `None` when cleared or unknown.
fn parse_status(status: &Value) -> Option<ProgramStatus> {
    let state = match status["state"].as_str()? {
        "idle" => ProgramState::Idle,
        "working" => ProgramState::Working,
        "done" => ProgramState::Done,
        "blocked" => ProgramState::Blocked,
        "error" => ProgramState::Error,
        _ => return None,
    };
    let text = |key: &str| status[key].as_str().map(str::to_string);
    Some(ProgramStatus {
        app: text("app"),
        state,
        kind: text("kind"),
        message: text("msg"),
    })
}

/// Logs the first reason core could not be used; later ones are debug only.
pub(crate) fn note_unavailable(err: &CoreError) {
    if !WARNED_UNAVAILABLE.swap(true, Ordering::AcqRel) {
        warn!(err = %err, "herdr-core unavailable; new panes use the PTY");
    } else {
        tracing::debug!(err = %err, "herdr-core unavailable; pane uses the PTY");
    }
}

/// Turns a pane's command into a core spawn (D4). `None` when the command
/// cannot be passed to core faithfully (a non-UTF-8 argument or variable); the
/// pane then uses the PTY.
pub(crate) fn core_spawn_from(
    pane_id: PaneId,
    command: &portable_pty::CommandBuilder,
    rows: u16,
    cols: u16,
    stripped_keys: impl Iterator<Item = &'static str>,
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
    // core merges `env` into its own environment and cannot unset (task 1.3):
    // unset every key herdr strips that this pane's environment lacks.
    let mut argv = vec!["/usr/bin/env".to_string()];
    for key in stripped_keys {
        if !env.contains_key(key) {
            argv.push("-u".into());
            argv.push(key.into());
        }
    }
    argv.push("--".into());
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
    let guarded = sweep_would_reach_core(child_pid, &core.core_pids());
    let pids = if guarded || child_pid == 0 {
        Vec::new()
    } else {
        super::pty::session_pids(child_pid)
    };
    core.kill();
    if guarded {
        warn!(
            pane = pane_id.raw(),
            pid = child_pid,
            "herdr-core program shares a session with core; not sweeping it"
        );
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
    fn status_records_parse_and_cleared_ones_are_ignored() {
        let status = parse_status(&serde_json::json!({
            "app": "claude-code", "state": "blocked", "kind": "permission", "msg": "Run tests?"
        }))
        .expect("status");
        assert_eq!(
            status,
            ProgramStatus {
                app: Some("claude-code".into()),
                state: ProgramState::Blocked,
                kind: Some("permission".into()),
                message: Some("Run tests?".into()),
            }
        );
        assert_eq!(parse_status(&Value::Null), None, "a cleared status");
        assert_eq!(parse_status(&serde_json::json!({"state": "dancing"})), None);
    }

    #[test]
    fn status_events_reach_the_registered_pane_only() {
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = Mutex::new(tx);
        register_status(
            "t-status-test",
            Box::new(move |status| {
                let _ = lock(&tx).send(status.state);
            }),
        );
        let event = |terminal: &str, state: &str| {
            serde_json::json!({"jsonrpc": "2.0", "method": "event", "params": {
                "seq": 1, "type": "status", "terminal": terminal,
                "data": {"status": {"app": "codex", "state": state}, "records": {}, "reported": true}
            }})
        };
        route_event(&event("t-other", "working"));
        route_event(&event("t-status-test", "working"));
        assert_eq!(rx.try_recv(), Ok(ProgramState::Working));
        assert!(
            rx.try_recv().is_err(),
            "the other terminal's event went nowhere"
        );
        forget_status("t-status-test");
        route_event(&event("t-status-test", "idle"));
        assert!(rx.try_recv().is_err(), "a forgotten pane gets nothing");
    }

    #[test]
    fn core_session_names_follow_design_d2() {
        assert_eq!(session_name_for(None, false), "herdr");
        assert_eq!(session_name_for(None, true), "herdr-dev");
        assert_eq!(session_name_for(Some("trial"), false), "herdr-trial");
        assert_eq!(session_name_for(Some("trial"), true), "herdr-dev-trial");
    }

    #[test]
    fn a_core_spawn_unsets_stripped_keys_the_pane_lacks_and_keeps_its_own() {
        let mut command = portable_pty::CommandBuilder::new("/bin/sh");
        command.arg("-c");
        command.arg("echo hi");
        command.cwd("/tmp");
        command.env_clear();
        command.env("PATH", "/usr/bin:/bin");
        command.env(crate::integration::HERDR_PANE_ID_ENV_VAR, "w1:p2");
        let stripped = [
            "TMUX",
            "CLAUDECODE",
            crate::integration::HERDR_PANE_ID_ENV_VAR,
        ];
        let spawn = core_spawn_from(PaneId::from_raw(1), &command, 24, 80, stripped.into_iter())
            .expect("spawn");

        assert_eq!(
            spawn.argv,
            [
                "/usr/bin/env",
                "-u",
                "TMUX",
                "-u",
                "CLAUDECODE",
                "--",
                "/bin/sh",
                "-c",
                "echo hi"
            ]
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
                None,
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
