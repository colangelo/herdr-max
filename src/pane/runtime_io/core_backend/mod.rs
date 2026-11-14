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
pub(crate) use pane_io::{Connector, CorePaneIo, CoreSpawn, StartedCore};

use crate::layout::PaneId;

/// `[experimental] runtime = "core"` (design D1), set at startup and on reload.
static PREFER_CORE: AtomicBool = AtomicBool::new(false);
/// One unavailability warning per server lifetime (D1).
static WARNED_UNAVAILABLE: AtomicBool = AtomicBool::new(false);
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
    Arc::new(|| {
        let mut slot = lock(&CLIENT);
        if let Some(client) = slot.as_ref() {
            if !client.is_closed() {
                return Ok(client.clone());
            }
        }
        let client = client::connect_session(&core_session_name(), None)?;
        *slot = Some(client.clone());
        Ok(client)
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
    let mut labels = Map::new();
    labels.insert("herdr.session".into(), Value::String(core_session_name()));
    if let Some(public) = env.get(crate::integration::HERDR_PANE_ID_ENV_VAR) {
        labels.insert("herdr.pane_id".into(), public.clone());
    }
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

/// Closing a core pane (D8): list the program's session, kill the terminal,
/// then run the same local sweep as the PTY backend over that list, unless the
/// session is core's own.
pub(crate) fn terminate_session(
    core: &CorePaneIo,
    pane_id: PaneId,
    child_pid: u32,
    child_wait_completed: Option<&AtomicBool>,
) {
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
    fn the_sweep_guard_refuses_a_session_shared_with_core() {
        let me = std::process::id();
        assert!(sweep_would_reach_core(me, &[me]), "the same process");
        assert!(!sweep_would_reach_core(me, &[]), "no core pids known");
        assert!(!sweep_would_reach_core(0, &[me]), "no program pid");
    }
}
