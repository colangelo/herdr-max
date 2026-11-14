//! The pane backend seam (openspec/changes/runtime-seam): the one place a
//! pane reaches the terminal program it shows. Today its only real backend is
//! the PTY actor; the herdr-core backend joins it in phase 2.

use bytes::Bytes;
use tokio::sync::mpsc;
#[cfg(test)]
use tokio::sync::watch;

use crate::pty::actor::{PtyIoActorHandle, ReadCallback};

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
    #[cfg(test)]
    TestChannel {
        sender: mpsc::Sender<Bytes>,
        resize_tx: watch::Sender<(u16, u16, u32, u32)>,
    },
}

impl PaneIo {
    pub(super) fn shutdown(&self) {
        match self {
            PaneIo::Actor(actor) => actor.shutdown(),
            #[cfg(test)]
            PaneIo::TestChannel { .. } => {}
        }
    }

    #[cfg(unix)]
    pub(super) fn duplicate_handoff_fd(&self) -> std::io::Result<std::os::fd::RawFd> {
        match self {
            PaneIo::Actor(actor) => actor.duplicate_for_handoff(),
            #[cfg(test)]
            PaneIo::TestChannel { .. } => {
                Err(std::io::Error::other("test runtime has no PTY master fd"))
            }
        }
    }

    #[cfg(unix)]
    pub(super) fn foreground_process_group_id(&self) -> Option<u32> {
        match self {
            PaneIo::Actor(actor) => actor.foreground_process_group_id(),
            #[cfg(test)]
            PaneIo::TestChannel { .. } => None,
        }
    }

    #[cfg(unix)]
    pub(super) fn begin_handoff(&self, timeout: std::time::Duration) -> std::io::Result<()> {
        match self {
            PaneIo::Actor(actor) => actor.begin_handoff(timeout),
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
            #[cfg(test)]
            PaneIo::TestChannel { .. } => Ok(()),
        }
    }

    #[cfg(unix)]
    pub(super) fn release_after_commit(&self) -> std::io::Result<()> {
        match self {
            PaneIo::Actor(actor) => actor.release_after_commit(),
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
            #[cfg(test)]
            PaneIo::TestChannel { sender, .. } => sender.try_send(bytes),
        }
    }

    pub(super) fn write_terminal_response(&self, response: impl FnOnce() -> Option<Bytes>) {
        match self {
            PaneIo::Actor(actor) => actor.write_terminal_response(response),
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
            #[cfg(test)]
            PaneIo::TestChannel { .. } => Ok(()),
        }
    }
}
