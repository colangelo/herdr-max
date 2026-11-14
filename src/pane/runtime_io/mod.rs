//! The pane backend seam (openspec/changes/runtime-seam): the one place a
//! pane reaches the terminal program it shows. Today its only real backend is
//! the PTY actor; the herdr-core backend joins it in phase 2.

use bytes::Bytes;
use tokio::sync::mpsc;
#[cfg(test)]
use tokio::sync::watch;

use crate::pty::actor::PtyIoActorHandle;
/// What the pane's read callback returns to the backend: replies to write back.
pub(in crate::pane) use crate::pty::actor::PtyReadResult;
/// The read callback a backend calls for every chunk.
pub(in crate::pane) use crate::pty::actor::ReadCallback;

#[cfg(unix)]
mod core_backend;
mod pty;

/// What to start in a pane. `command` stays a `portable_pty::CommandBuilder`
/// for now (runtime-seam design D5).
pub(in crate::pane) struct SpawnSpec {
    pub rows: u16,
    pub cols: u16,
    pub command: portable_pty::CommandBuilder,
    /// Logged when the program cannot be started.
    pub spawn_error_message: &'static str,
}

/// Where a backend delivers a pane's output and its exit.
pub(in crate::pane) struct PaneIoSink {
    /// Feeds the pane's terminal parser; called for every chunk read.
    pub on_read: ReadCallback,
    /// Called once, after the wait is marked complete.
    pub on_exit: Box<dyn FnOnce(crate::platform::ChildExitReason) + Send>,
}

pub(super) enum PaneIo {
    Actor(PtyIoActorHandle),
    /// The program runs in a herdr-core terminal (openspec/changes/core-backend).
    #[cfg(unix)]
    Core(core_backend::CorePaneIo),
    #[cfg(test)]
    TestChannel {
        sender: mpsc::Sender<Bytes>,
        resize_tx: watch::Sender<(u16, u16, u32, u32)>,
    },
}

/// Which backend new panes prefer (`[experimental] runtime`, core-backend D1).
/// On a platform without the core backend, `core` warns once and the PTY is used.
pub(crate) fn set_preferred_runtime(core: bool) {
    #[cfg(unix)]
    core_backend::set_preferred(core);
    #[cfg(not(unix))]
    if core {
        tracing::warn!("[experimental] runtime = \"core\" is Unix only; panes use the PTY");
    }
}

impl PaneIo {
    /// Starts `spec.command` on the preferred backend: a herdr-core terminal
    /// when `runtime = "core"` and core is usable, else a PTY (D1).
    pub(in crate::pane) fn spawn_preferred(
        pane_id: crate::layout::PaneId,
        spec: SpawnSpec,
        sink: PaneIoSink,
        child_pid: &std::sync::atomic::AtomicU32,
        child_wait_completed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> std::io::Result<Self> {
        #[cfg(unix)]
        if core_backend::preferred() {
            let started = core_backend::core_spawn_from(
                pane_id,
                &spec.command,
                spec.rows,
                spec.cols,
                super::pane_stripped_env_keys(),
            )
            .ok_or_else(|| {
                core_backend::client::CoreError::Unavailable(
                    "the command has a non-UTF-8 argument, folder or variable".into(),
                )
            })
            .and_then(|spawn| {
                core_backend::StartedCore::start(spawn, child_pid, core_backend::server_connector())
            });
            match started {
                Ok(started) => {
                    let PaneIoSink { on_read, on_exit } = sink;
                    return started
                        .into_pane(on_read, on_exit, child_wait_completed)
                        .map(PaneIo::Core);
                }
                Err(err) => core_backend::note_unavailable(&err),
            }
        }
        Self::spawn(pane_id, spec, sink, child_pid, child_wait_completed)
    }

    pub(super) fn shutdown(&self) {
        match self {
            PaneIo::Actor(actor) => actor.shutdown(),
            #[cfg(unix)]
            PaneIo::Core(core) => core.shutdown(),
            #[cfg(test)]
            PaneIo::TestChannel { .. } => {}
        }
    }

    #[cfg(unix)]
    pub(super) fn duplicate_handoff_fd(&self) -> std::io::Result<std::os::fd::RawFd> {
        match self {
            PaneIo::Actor(actor) => actor.duplicate_for_handoff(),
            #[cfg(unix)]
            PaneIo::Core(_) => Err(no_master_fd()),
            #[cfg(test)]
            PaneIo::TestChannel { .. } => Err(no_master_fd()),
        }
    }

    #[cfg(unix)]
    pub(super) fn foreground_process_group_id(&self) -> Option<u32> {
        match self {
            PaneIo::Actor(actor) => actor.foreground_process_group_id(),
            #[cfg(unix)]
            PaneIo::Core(_) => None,
            #[cfg(test)]
            PaneIo::TestChannel { .. } => None,
        }
    }

    #[cfg(unix)]
    pub(super) fn begin_handoff(&self, timeout: std::time::Duration) -> std::io::Result<()> {
        match self {
            PaneIo::Actor(actor) => actor.begin_handoff(timeout),
            #[cfg(unix)]
            PaneIo::Core(_) => Ok(()),
            #[cfg(test)]
            PaneIo::TestChannel { .. } => Ok(()),
        }
    }

    #[cfg(unix)]
    pub(super) fn set_handoff_paused(&self, paused: bool) -> std::io::Result<()> {
        match self {
            PaneIo::Actor(actor) => {
                if paused {
                    actor.begin_handoff(std::time::Duration::from_secs(1))
                } else {
                    actor.rollback_handoff()
                }
            }
            #[cfg(unix)]
            PaneIo::Core(_) => Ok(()),
            #[cfg(test)]
            PaneIo::TestChannel { .. } => Ok(()),
        }
    }

    #[cfg(unix)]
    pub(super) fn release_after_commit(&self) -> std::io::Result<()> {
        match self {
            PaneIo::Actor(actor) => actor.release_after_commit(),
            #[cfg(unix)]
            PaneIo::Core(_) => Ok(()),
            #[cfg(test)]
            PaneIo::TestChannel { .. } => Ok(()),
        }
    }

    pub(super) fn resize(
        &self,
        rows: u16,
        cols: u16,
        cell_width_px: u32,
        cell_height_px: u32,
        terminal_responses: Vec<Bytes>,
    ) {
        match self {
            PaneIo::Actor(actor) => {
                actor.resize(
                    rows,
                    cols,
                    cell_width_px,
                    cell_height_px,
                    terminal_responses,
                );
            }
            #[cfg(unix)]
            PaneIo::Core(core) => core.resize(
                rows,
                cols,
                cell_width_px,
                cell_height_px,
                terminal_responses,
            ),
            #[cfg(test)]
            PaneIo::TestChannel { resize_tx, .. } => {
                let _ = resize_tx.send((rows, cols, cell_width_px, cell_height_px));
            }
        }
    }

    #[cfg(unix)]
    pub(super) fn nudge_child_redraw_after_handoff(
        &self,
        rows: u16,
        cols: u16,
        cell_width_px: u32,
        cell_height_px: u32,
    ) {
        match self {
            PaneIo::Actor(actor) => {
                actor.nudge_child_redraw_after_handoff(rows, cols, cell_width_px, cell_height_px);
            }
            #[cfg(unix)]
            PaneIo::Core(_) => {}
            #[cfg(test)]
            PaneIo::TestChannel { .. } => {}
        }
    }

    pub(super) fn try_send_bytes(
        &self,
        bytes: Bytes,
    ) -> Result<(), mpsc::error::TrySendError<Bytes>> {
        match self {
            PaneIo::Actor(actor) => actor.try_write_user_input(bytes),
            #[cfg(unix)]
            PaneIo::Core(core) => core.try_send_bytes(bytes),
            #[cfg(test)]
            PaneIo::TestChannel { sender, .. } => sender.try_send(bytes),
        }
    }

    pub(super) fn write_terminal_response(&self, response: impl FnOnce() -> Option<Bytes>) {
        match self {
            PaneIo::Actor(actor) => actor.write_terminal_response(response),
            #[cfg(unix)]
            PaneIo::Core(core) => core.write_terminal_response(response),
            #[cfg(test)]
            PaneIo::TestChannel { sender, .. } => {
                if let Some(bytes) = response() {
                    let _ = sender.try_send(bytes);
                }
            }
        }
    }

    pub(super) fn queue_user_input_submission(
        &self,
        text: Bytes,
        enter: Bytes,
        delay: std::time::Duration,
        deadline: Option<std::time::Instant>,
    ) -> std::io::Result<std::sync::mpsc::Receiver<std::io::Result<()>>> {
        match self {
            PaneIo::Actor(actor) => {
                #[cfg(windows)]
                return actor.queue_user_input_submission(text, enter, delay, deadline);
                #[cfg(unix)]
                {
                    let _ = deadline;
                    actor.queue_user_input_submission(text, enter, delay)
                }
            }
            #[cfg(unix)]
            PaneIo::Core(core) => {
                let _ = deadline;
                core.queue_user_input_submission(text, enter, delay)
            }
            #[cfg(test)]
            PaneIo::TestChannel { sender, .. } => {
                let _ = deadline;
                let sender = sender.clone();
                let (reply_tx, reply_rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let result = sender
                        .try_send(text)
                        .map_err(std::io::Error::other)
                        .and_then(|()| {
                            std::thread::sleep(delay);
                            sender.try_send(enter).map_err(std::io::Error::other)
                        });
                    let _ = reply_tx.send(result);
                });
                Ok(reply_rx)
            }
        }
    }

    /// Ends the pane's program and everything in its session. Callers stop
    /// I/O first (`shutdown`); a pane preserved for a handoff never calls it.
    pub(super) fn terminate_session(
        &self,
        pane_id: crate::layout::PaneId,
        child_pid: u32,
        child_wait_completed: Option<&std::sync::atomic::AtomicBool>,
    ) {
        match self {
            PaneIo::Actor(_) => {
                pty::shutdown_pane_processes(pane_id, child_pid, child_wait_completed)
            }
            #[cfg(unix)]
            PaneIo::Core(core) => {
                core_backend::terminate_session(core, pane_id, child_pid, child_wait_completed)
            }
            #[cfg(test)]
            PaneIo::TestChannel { .. } => {
                pty::shutdown_pane_processes(pane_id, child_pid, child_wait_completed)
            }
        }
    }

    /// Preflight a complete API command without changing input queue semantics.
    pub(super) fn validate_input_submission(&self, bytes: &[u8]) -> std::io::Result<()> {
        match self {
            #[cfg(unix)]
            PaneIo::Actor(actor) => actor.validate_input_submission(bytes),
            #[cfg(windows)]
            PaneIo::Actor(_) => {
                let _ = bytes;
                Ok(())
            }
            #[cfg(unix)]
            PaneIo::Core(_) => Ok(()),
            #[cfg(test)]
            PaneIo::TestChannel { .. } => Ok(()),
        }
    }
}

/// The answer a backend without a PTY master fd gives to a handoff by fd
/// (runtime-seam design D4). Phase 3 matches on this kind to hand such a pane
/// over another way.
#[cfg(unix)]
fn no_master_fd() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "this pane backend has no PTY master fd",
    )
}

#[cfg(all(unix, test))]
mod tests {
    use super::*;

    #[test]
    fn a_backend_without_a_master_fd_refuses_a_handoff_by_fd_as_unsupported() {
        let (sender, _receiver) = mpsc::channel(1);
        let (resize_tx, _resize_rx) = watch::channel((24, 80, 0, 0));
        let io = PaneIo::TestChannel { sender, resize_tx };

        let err = io.duplicate_handoff_fd().expect_err("no fd to hand off");

        assert_eq!(err.kind(), std::io::ErrorKind::Unsupported);
    }
}

/// A pane runtime whose backend is an existing PTY actor, for tests that drive
/// the actor themselves.
#[cfg(all(test, unix))]
impl super::PaneRuntime {
    pub(crate) fn test_with_actor(actor: PtyIoActorHandle) -> Self {
        let (mut runtime, _) = Self::test_with_channel(80, 24);
        runtime.io = PaneIo::Actor(actor);
        runtime
    }
}

/// A close-on-exec duplicate of a test PTY's master fd, as a live handoff
/// delivers one to the replacement server.
#[cfg(all(test, unix))]
pub(in crate::pane) fn test_dup_master_fd(pair: &portable_pty::PtyPair) -> std::os::fd::RawFd {
    let master_fd = pair
        .master
        .as_raw_fd()
        .expect("pty master fd is available in tests");
    crate::pty::fd::duplicate_cloexec_fd(master_fd).expect("dup master fd")
}
