//! The PTY backend behind `PaneIo`: today's `portable_pty` spawn, child wait
//! and I/O actor, unchanged.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use tracing::{error, info, warn};

use super::{PaneIo, PaneIoSink, SpawnSpec};
use crate::layout::PaneId;
use crate::pty::actor::{PtyIoActor, PtyIoActorConfig};

/// How long a handed-off job pane whose terminal closed waits for its
/// keeper's exit report; the keeper reports as soon as its program ends.
#[cfg(unix)]
const ADOPTED_JOB_EXIT_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

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
    /// handle, so the exit is reported when the reader ends: as the status the
    /// pane's keeper job reports (`job_exit`), or else as `Handoff`.
    #[cfg(unix)]
    pub(in crate::pane) fn adopt_handoff_fd(
        pane_id: PaneId,
        master_fd: std::os::fd::OwnedFd,
        sink: PaneIoSink,
        job_exit: Option<crate::platform::AdoptedPaneExit>,
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
                let reported = job_exit.and_then(|wait| wait(ADOPTED_JOB_EXIT_GRACE));
                let exit_reason = match reported {
                    Some(status) => {
                        crate::logging::pane_exited(pane_id.raw(), &format!("{status:?}"));
                        crate::platform::classify_child_exit(&status)
                    }
                    None => crate::platform::ChildExitReason::Handoff,
                };
                on_exit(exit_reason)
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
    let pids = session_pids(child_pid, child_wait_completed);
    signal_session(pane_id, child_pid, &pids, child_wait_completed);
}

/// The program's session, the program included, to end on close. Taken before
/// anything ends the session leader: once it is gone, its session can no
/// longer be looked up.
pub(super) fn session_pids(child_pid: u32, child_wait_completed: Option<&AtomicBool>) -> Vec<u32> {
    let exited = child_wait_completed.is_some_and(|done| done.load(Ordering::Acquire));
    sweep_targets(
        child_pid,
        exited,
        crate::platform::process_exists,
        crate::platform::session_processes,
    )
}

/// Whether `pid` may be signalled at all: never 0 or 1, and never a value that
/// becomes negative as a `pid_t`, which `kill` reads as a process group.
pub(super) fn signallable(pid: u32) -> bool {
    pid > 1 && pid <= i32::MAX as u32
}

/// Which processes a close signals (security review M2). Once the program has
/// exited its pid is free for reuse: a process that has it now is a stranger,
/// and looking its session up would sweep the stranger's session. So an exited
/// program's close signals nothing. While it runs, the pid is ours: its session,
/// or the program alone when the session cannot be listed.
fn sweep_targets(
    child_pid: u32,
    exited: bool,
    pid_exists: impl Fn(u32) -> bool,
    session_of: impl Fn(u32) -> Vec<u32>,
) -> Vec<u32> {
    if !signallable(child_pid) || exited {
        return Vec::new();
    }
    let mut pids: Vec<u32> = session_of(child_pid)
        .into_iter()
        .filter(|pid| signallable(*pid))
        .collect();
    if pids.is_empty() && pid_exists(child_pid) {
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
    // Mutable only where targets are rechecked by session (unix).
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut pids: Vec<u32> = pids
        .iter()
        .copied()
        .filter(|pid| signallable(*pid))
        .collect();
    if pids.is_empty() {
        return;
    }
    // Recheck before each signal that every target is still in the session it
    // was listed in, so a pid freed and reused between stages is left alone.
    #[cfg(unix)]
    let session = crate::platform::process_session(child_pid);
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
        #[cfg(unix)]
        if let Some(session) = session {
            pids.retain(|pid| crate::platform::process_session(*pid) == Some(session));
            if pids.is_empty() {
                return;
            }
        }
        crate::platform::signal_processes(&pids, signal);
        if wait_for_processes_to_exit(&pids, child_pid, child_wait_completed, grace) {
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

    // Security review M2: an exited program's pid may belong to a stranger.
    #[test]
    fn closing_an_exited_program_signals_nothing_even_if_its_pid_is_reused() {
        let stranger_session = |_| vec![4242, 4243];
        assert!(sweep_targets(4242, true, |_| true, stranger_session).is_empty());
        assert!(sweep_targets(4242, true, |_| false, |_| Vec::new()).is_empty());
    }

    #[test]
    fn closing_a_running_program_signals_its_session_or_the_program_alone() {
        assert_eq!(
            sweep_targets(500, false, |_| true, |_| vec![502, 500, 501]),
            vec![500, 501, 502]
        );
        assert_eq!(
            sweep_targets(500, false, |_| true, |_| Vec::new()),
            vec![500]
        );
        assert!(
            sweep_targets(500, false, |_| false, |_| Vec::new()).is_empty(),
            "a program already gone is not guessed at"
        );
    }

    #[test]
    fn pids_that_kill_would_misread_are_never_signalled() {
        assert!(!signallable(0));
        assert!(!signallable(1), "init or launchd");
        assert!(
            !signallable(i32::MAX as u32 + 1),
            "a negative pid_t is a group"
        );
        assert!(signallable(4242));
        assert_eq!(
            sweep_targets(500, false, |_| true, |_| vec![0, 1, u32::MAX, 501]),
            vec![501]
        );
        assert!(sweep_targets(u32::MAX, false, |_| true, |_| vec![7]).is_empty());
    }

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
