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
mod core_status;
mod pty;

pub(crate) use core_status::{CoreStatusApp, CoreStatusRecord, CoreStatusWord};

/// What to start in a pane. `command` stays a `portable_pty::CommandBuilder`
/// for now (runtime-seam design D5).
pub(in crate::pane) struct SpawnSpec {
    pub rows: u16,
    pub cols: u16,
    pub command: portable_pty::CommandBuilder,
    /// Logged when the program cannot be started.
    pub spawn_error_message: &'static str,
}

/// A pane's program in herdr-core, saved with the pane so the next server
/// attaches to it instead of starting it again (core-backend D10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CoreTerminalLink {
    /// The core session (`herdr`, `herdr-dev`, `herdr-<name>`; D2).
    pub session: String,
    /// The terminal's address in that session (`t7`).
    pub address: String,
}

/// Live herdr-core terminals labelled for this session that no pane claimed
/// (D10 "Orphans"): `(address, pane id label)`. Empty without a core.
pub(crate) fn unclaimed_core_terminals(claimed: &[String]) -> Vec<(String, Option<String>)> {
    #[cfg(unix)]
    {
        core_backend::unclaimed_terminals(claimed)
            .into_iter()
            .map(|terminal| (terminal.address, terminal.pane_id))
            .collect()
    }
    #[cfg(not(unix))]
    {
        let _ = claimed;
        Vec::new()
    }
}

/// The herdr-core session this herdr session's panes run in (D2).
pub(crate) fn core_session_name() -> String {
    #[cfg(unix)]
    {
        core_backend::core_session_name()
    }
    #[cfg(not(unix))]
    {
        String::new()
    }
}

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
    /// when `runtime = "core"` and core is usable, else a PTY (D1). The first
    /// time core cannot be used, `on_unavailable` gets the reason, for the
    /// server's one notice.
    pub(in crate::pane) fn spawn_preferred(
        pane_id: crate::layout::PaneId,
        spec: SpawnSpec,
        sink: PaneIoSink,
        child_pid: &std::sync::atomic::AtomicU32,
        child_wait_completed: std::sync::Arc<std::sync::atomic::AtomicBool>,
        on_unavailable: impl FnOnce(String),
    ) -> std::io::Result<Self> {
        #[cfg(unix)]
        if core_backend::preferred() {
            let started =
                core_backend::core_spawn_from(pane_id, &spec.command, spec.rows, spec.cols)
                    .ok_or_else(|| {
                        core_backend::client::CoreError::Unavailable(
                            "the command has a non-UTF-8 argument, folder or variable".into(),
                        )
                    })
                    .and_then(|spawn| {
                        core_backend::StartedCore::start(
                            spawn,
                            child_pid,
                            core_backend::server_connector(),
                        )
                    });
            match started {
                Ok(started) => {
                    let PaneIoSink { on_read, on_exit } = sink;
                    return started
                        .into_pane(on_read, on_exit, child_wait_completed)
                        .map(PaneIo::Core);
                }
                Err(err) => {
                    if core_backend::note_unavailable(&err) {
                        on_unavailable(err.to_string());
                    }
                }
            }
        }
        #[cfg(not(unix))]
        let _ = on_unavailable;
        Self::spawn(pane_id, spec, sink, child_pid, child_wait_completed)
    }

    /// Attaches to a program an earlier server left in herdr-core (D10
    /// "Arriving"). Fails, without touching the terminal, when it is in
    /// another core session, gone, finished or labelled for another pane; the
    /// caller then starts the pane fresh.
    #[cfg(unix)]
    // The reattach threads the pane's geometry, sink and pid slots, as spawn does.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::pane) fn reattach_core(
        pane_id: crate::layout::PaneId,
        link: &CoreTerminalLink,
        public_pane_id: Option<&str>,
        rows: u16,
        cols: u16,
        sink: PaneIoSink,
        child_pid: &std::sync::atomic::AtomicU32,
        child_wait_completed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> std::io::Result<Self> {
        let session = core_backend::core_session_name();
        if link.session != session {
            return Err(std::io::Error::other(format!(
                "terminal {} is in core session {}, not {session}",
                link.address, link.session
            )));
        }
        let target = core_backend::CoreReattach {
            pane_id,
            address: link.address.clone(),
            labels: core_backend::pane_labels(public_pane_id),
            rows,
            cols,
        };
        let started = core_backend::StartedCore::reattach(
            target,
            child_pid,
            core_backend::existing_core_connector(),
        )
        .map_err(|err| std::io::Error::other(err.to_string()))?;
        let PaneIoSink { on_read, on_exit } = sink;
        started
            .into_pane(on_read, on_exit, child_wait_completed)
            .map(PaneIo::Core)
    }

    /// The core terminal this pane shows, if it runs in herdr-core.
    pub(super) fn core_terminal(&self) -> Option<CoreTerminalLink> {
        match self {
            #[cfg(unix)]
            PaneIo::Core(core) => Some(CoreTerminalLink {
                session: core_backend::core_session_name(),
                address: core.address().to_string(),
            }),
            _ => None,
        }
    }

    /// Whether dropping a core pane leaves its program running, for the next
    /// server to attach to (D10). A PTY pane is unaffected: its program cannot
    /// outlive this server.
    pub(super) fn set_keep_core_terminal(&self, keep: bool) {
        #[cfg(unix)]
        if let PaneIo::Core(core) = self {
            core.set_keep_on_drop(keep);
        }
        #[cfg(not(unix))]
        let _ = keep;
    }

    /// Mirrors herdr's agent status into the pane's core terminal
    /// (openspec/changes/core-status-mirror). Other backends ignore it.
    pub(super) fn mirror_core_status(&self, record: CoreStatusRecord) {
        #[cfg(unix)]
        if let PaneIo::Core(core) = self {
            core.mirror_status(record);
        }
        #[cfg(not(unix))]
        let _ = record;
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
