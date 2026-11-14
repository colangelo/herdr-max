//! The PTY backend behind `PaneIo`: today's `portable_pty` spawn, child wait
//! and I/O actor, unchanged.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use tracing::{error, info, warn};

use super::{PaneIo, PaneIoSink, SpawnSpec};
use crate::layout::PaneId;
use crate::pty::actor::{PtyIoActor, PtyIoActorConfig};

impl PaneIo {
    /// Starts `spec.command` on a new PTY. The order is load-bearing
    /// (runtime-seam roundtable F1, F2): the shell pid is published before the
    /// reader starts, so the first chunk is parsed with it, and the child wait
    /// starts before the reader, as it always has.
    pub(in crate::pane) fn spawn(
        pane_id: PaneId,
        spec: SpawnSpec,
        sink: PaneIoSink,
        child_pid: &AtomicU32,
        child_wait_completed: Arc<AtomicBool>,
    ) -> std::io::Result<Self> {
        let SpawnSpec {
            rows,
            cols,
            command,
            spawn_error_message,
        } = spec;
        let PaneIoSink {
            on_read, on_exit, ..
        } = sink;
        let spawned = crate::pty::backend::spawn_with_portable_pty(rows, cols, command)
            .inspect_err(|err| error!(pane = pane_id.raw(), err = %err, "{spawn_error_message}"))?;

        let mut child = spawned.child;
        if let Some(pid) = child.process_id() {
            child_pid.store(pid, Ordering::Release);
            crate::logging::pane_spawned(pane_id.raw(), pid);
        }
        tokio::task::spawn_blocking(move || {
            let exit_reason = match child.wait() {
                Ok(status) => {
                    let exit_reason = crate::platform::classify_child_exit(&status);
                    let status_text = format!("{status:?}");
                    crate::logging::pane_exited(pane_id.raw(), &status_text);
                    exit_reason
                }
                Err(e) => {
                    crate::logging::pane_exit_failed(pane_id.raw(), &e.to_string());
                    crate::platform::ChildExitReason::WaitFailed
                }
            };
            child_wait_completed.store(true, Ordering::Release);
            on_exit(exit_reason);
        });

        Ok(PaneIo::Actor(PtyIoActor::spawn(PtyIoActorConfig {
            pane_id: pane_id.raw(),
            #[cfg(unix)]
            master_fd: spawned.master_fd,
            #[cfg(windows)]
            master: spawned.master,
            initially_quiesced: false,
            on_read,
            on_reader_exit: None,
        })?))
    }

    /// Takes over a PTY whose master fd came from a live handoff. The reader
    /// starts quiesced, as the handoff protocol expects. There is no child
    /// handle, so the exit is reported as `Handoff` when the reader ends.
    #[cfg(unix)]
    pub(in crate::pane) fn adopt_handoff_fd(
        pane_id: PaneId,
        master_fd: std::os::fd::OwnedFd,
        sink: PaneIoSink,
    ) -> std::io::Result<Self> {
        let PaneIoSink {
            on_read, on_exit, ..
        } = sink;
        Ok(PaneIo::Actor(PtyIoActor::spawn(PtyIoActorConfig {
            pane_id: pane_id.raw(),
            master_fd,
            initially_quiesced: true,
            on_read,
            on_reader_exit: Some(Box::new(move || {
                on_exit(crate::platform::ChildExitReason::Handoff)
            })),
        })?))
    }
}

/// Ends a pane's process session: hangup, then terminate, then kill, each
/// followed by up to 250 ms for every process to go. Unchanged from before the
/// seam; `PaneIo::terminate_session` calls it.
fn process_alive_for_shutdown(
    pid: u32,
    child_pid: u32,
    child_wait_completed: bool,
    process_exists: impl FnOnce(u32) -> bool,
) -> bool {
    if pid == child_pid && child_wait_completed {
        return false;
    }
    process_exists(pid)
}

fn wait_for_processes_to_exit(
    pids: &[u32],
    child_pid: u32,
    child_wait_completed: Option<&AtomicBool>,
    timeout: std::time::Duration,
) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let child_wait_completed =
            child_wait_completed.is_some_and(|flag| flag.load(Ordering::Acquire));
        if pids.iter().all(|pid| {
            !process_alive_for_shutdown(
                *pid,
                child_pid,
                child_wait_completed,
                crate::platform::process_exists,
            )
        }) {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

pub(super) fn shutdown_pane_processes(
    pane_id: PaneId,
    child_pid: u32,
    child_wait_completed: Option<&AtomicBool>,
) {
    if child_pid == 0 {
        return;
    }
    let pids = session_pids(child_pid);
    signal_session(pane_id, child_pid, &pids, child_wait_completed);
}

/// The program's session, the program included. Taken before anything ends the
/// session leader: once it is gone, its session can no longer be looked up.
pub(super) fn session_pids(child_pid: u32) -> Vec<u32> {
    let mut pids = crate::platform::session_processes(child_pid);
    if pids.is_empty() {
        pids.push(child_pid);
    }
    pids.sort_unstable();
    pids.dedup();
    pids
}

/// Hangup, then terminate, then kill `pids`, stopping as soon as all are gone.
pub(super) fn signal_session(
    pane_id: PaneId,
    child_pid: u32,
    pids: &[u32],
    child_wait_completed: Option<&AtomicBool>,
) {
    for (signal, grace) in [
        (
            crate::platform::Signal::Hangup,
            std::time::Duration::from_millis(250),
        ),
        (
            crate::platform::Signal::Terminate,
            std::time::Duration::from_millis(250),
        ),
        (
            crate::platform::Signal::Kill,
            std::time::Duration::from_millis(250),
        ),
    ] {
        crate::platform::signal_processes(pids, signal);
        if wait_for_processes_to_exit(pids, child_pid, child_wait_completed, grace) {
            info!(
                pane = pane_id.raw(),
                pid = child_pid,
                ?signal,
                "pane session terminated"
            );
            return;
        }
    }

    warn!(
        pane = pane_id.raw(),
        pid = child_pid,
        pids = ?pids,
        "pane session still alive after forced shutdown"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_liveness_treats_reaped_direct_child_as_gone() {
        assert!(!process_alive_for_shutdown(42, 42, true, |_| true));
    }

    #[test]
    fn shutdown_liveness_keeps_unreaped_direct_child_alive() {
        assert!(process_alive_for_shutdown(42, 42, false, |_| true));
    }

    #[test]
    fn shutdown_liveness_keeps_other_session_processes_alive() {
        assert!(process_alive_for_shutdown(43, 42, true, |_| true));
    }

    #[test]
    fn shutdown_liveness_treats_missing_process_as_gone() {
        assert!(!process_alive_for_shutdown(43, 42, false, |_| false));
    }
}
