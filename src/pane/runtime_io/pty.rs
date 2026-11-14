//! The PTY backend behind `PaneIo`: today's `portable_pty` spawn, child wait
//! and I/O actor, unchanged.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use tracing::error;

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
        let PaneIoSink { on_read, on_exit } = sink;
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
}
