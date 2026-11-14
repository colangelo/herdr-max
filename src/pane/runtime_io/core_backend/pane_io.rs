//! One pane on the herdr-core backend (openspec/changes/core-backend, D3-D8).
//!
//! A pane owns two threads, as a PTY pane owns its actor thread:
//! - the parse thread feeds the attach stream to the pane's `on_read`, writes
//!   the replies it returns, then returns CREDIT for the bytes it consumed, so a
//!   slow parser slows its own stream only (D3);
//! - the writer thread sends input, replies and resizes in order.
//!
//! Reconnecting after the connection drops goes through `Connector`, which the
//! caller provides, so tests can point it at a fake core.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tracing::{debug, warn};

use super::client::{CoreClient, CoreError, StreamEvent, StreamSink};
use super::frame::{kind, Frame};
use crate::layout::PaneId;
use crate::pane::runtime_io::StatusCallback;
use crate::platform::ChildExitReason;
use crate::pty::actor::{PtyReadResult, ReadCallback};

const CALL_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_QUEUE: usize = 1024;
/// How long a pane keeps trying to reach its terminal again after the
/// connection drops (a core update takes well under a second, Q6).
const REATTACH_LIMIT: Duration = Duration::from_secs(60);
/// Scrollback lines asked for on a pane's first attach: more than any pane
/// keeps, so all of it arrives.
const FIRST_ATTACH_HISTORY_LINES: u32 = 1_000_000;

/// Returns a live client, reconnecting when the last one closed.
pub(crate) type Connector = Arc<dyn Fn() -> Result<CoreClient, CoreError> + Send + Sync>;

/// What `terminal.run` needs for one pane.
pub(crate) struct CoreSpawn {
    pub pane_id: PaneId,
    pub argv: Vec<String>,
    pub cwd: Option<String>,
    pub env: serde_json::Map<String, Value>,
    pub labels: serde_json::Map<String, Value>,
    pub rows: u16,
    pub cols: u16,
}

enum WriteCmd {
    User(Bytes),
    Reply(Bytes),
    Submit {
        text: Bytes,
        enter: Bytes,
        delay: Duration,
        reply: std_mpsc::Sender<std::io::Result<()>>,
    },
    Resize {
        rows: u16,
        cols: u16,
        cell: (u32, u32),
        replies: Vec<Bytes>,
    },
}

struct Shared {
    pane_id: PaneId,
    address: String,
    connector: Connector,
    client: Mutex<CoreClient>,
    channel: AtomicU32,
    /// The size last asked for, for a reattach.
    size: Mutex<(u16, u16)>,
    /// Set when the pane shuts down: stream ends are then not reattached.
    closing: AtomicBool,
}

impl Shared {
    fn client(&self) -> CoreClient {
        lock(&self.client).clone()
    }

    fn send(&self, frame_kind: u8, payload: Vec<u8>) -> Result<(), CoreError> {
        let channel = self.channel.load(Ordering::Acquire);
        self.client()
            .send(&Frame::new(channel, frame_kind, payload))
    }
}

pub(crate) struct CorePaneIo {
    shared: Arc<Shared>,
    writer: mpsc::Sender<WriteCmd>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Maps a stream CLOSE reason or a finished terminal record to an exit (D7).
/// `None`: not an exit (our own detach, or a record still live).
pub(crate) fn exit_reason(record: &Value) -> Option<ChildExitReason> {
    let reason = record.get("reason")?.as_str()?;
    let signal = record.get("signal").is_some_and(|signal| !signal.is_null());
    Some(match reason {
        "exited" | "killed" if signal => ChildExitReason::Interrupted,
        "exited" | "killed" => ChildExitReason::Exited,
        "server_stop" | "host_shutdown" | "holder_lost" | "stopped_by_restart" | "signaled" => {
            ChildExitReason::Interrupted
        }
        _ => ChildExitReason::Interrupted,
    })
}

/// A core terminal that is running and attached, before the pane hands over
/// its callbacks. Splitting the spawn this way keeps the callbacks with the
/// caller until core has said yes, so a refusal can fall back to the PTY.
pub(crate) struct StartedCore {
    shared: Arc<Shared>,
    events_rx: std_mpsc::Receiver<StreamEvent>,
    events_tx: StreamSink,
    automatic_input: bool,
}

impl StartedCore {
    /// Runs the program as a core terminal and attaches to it (D4). The pid is
    /// published once the stream exists and before the parser starts, so the
    /// first chunk always sees it.
    pub(crate) fn start(
        spawn: CoreSpawn,
        child_pid: &AtomicU32,
        connector: Connector,
    ) -> Result<Self, CoreError> {
        let client = connector()?;
        let record = client.call(
            "terminal.run",
            json!({
                "argv": spawn.argv,
                "cwd": spawn.cwd,
                "env": spawn.env,
                "labels": spawn.labels,
                "size": {"cols": spawn.cols, "rows": spawn.rows},
                "no_parent": true,
                "no_notify": true,
                "keep": false,
            }),
            CALL_TIMEOUT,
        )?;
        let address = record["address"]
            .as_str()
            .ok_or_else(|| CoreError::Rpc {
                kind: "protocol".into(),
                message: format!("terminal.run returned no address: {record}"),
            })?
            .to_string();
        let pid = record["pid"].as_u64().unwrap_or(0) as u32;

        let (events_tx, events_rx) = std_mpsc::channel();
        let (channel, automatic_input) = match client.attach(
            // Output printed before this attach has scrolled past the first
            // paint's screen: ask for the history too, or the pane loses it.
            // A reattach does not, so lines are not shown twice (D5).
            json!({"terminal": address, "cols": spawn.cols, "rows": spawn.rows,
                   "history": FIRST_ATTACH_HISTORY_LINES}),
            events_tx.clone(),
            CALL_TIMEOUT,
        ) {
            Ok(attach) => (
                attach["channel"].as_u64().unwrap_or(0) as u32,
                client.automatic_input() && attach["automatic_input"] == true,
            ),
            // A program that ended before we attached (a quick command): the
            // pane still opens, shows its last screen and reports the exit, as
            // a PTY pane does. Channel 0 marks "no stream".
            Err(CoreError::Rpc { kind, .. }) if kind == "finished" => {
                queue_finished(&client, &address, &events_tx);
                (0, false)
            }
            Err(err) => {
                // Do not leave a program running that no pane shows.
                let _ = client.call("terminal.kill", json!({"terminal": address}), CALL_TIMEOUT);
                return Err(err);
            }
        };
        if pid > 0 {
            child_pid.store(pid, Ordering::Release);
            crate::logging::pane_spawned(spawn.pane_id.raw(), pid);
        }
        Ok(Self {
            shared: Arc::new(Shared {
                pane_id: spawn.pane_id,
                address,
                connector,
                client: Mutex::new(client),
                channel: AtomicU32::new(channel),
                size: Mutex::new((spawn.rows, spawn.cols)),
                closing: AtomicBool::new(false),
            }),
            events_rx,
            events_tx,
            automatic_input,
        })
    }

    /// Starts the pane's parser and writer threads.
    pub(crate) fn into_pane(
        self,
        on_read: ReadCallback,
        on_exit: Box<dyn FnOnce(ChildExitReason) + Send>,
        on_status: Option<StatusCallback>,
        child_wait_completed: Arc<AtomicBool>,
    ) -> std::io::Result<CorePaneIo> {
        if let Some(on_status) = on_status {
            super::register_status(&self.shared.address, on_status);
        }
        let Self {
            shared,
            events_rx,
            events_tx,
            automatic_input,
        } = self;
        let (writer, writer_rx) = mpsc::channel(WRITE_QUEUE);
        let started =
            spawn_writer(Arc::clone(&shared), writer_rx, automatic_input).and_then(|()| {
                spawn_parser(
                    Arc::clone(&shared),
                    events_rx,
                    events_tx,
                    writer.clone(),
                    on_read,
                    on_exit,
                    child_wait_completed,
                )
            });
        if let Err(err) = started {
            let _ = shared.client().call(
                "terminal.kill",
                json!({"terminal": shared.address}),
                CALL_TIMEOUT,
            );
            return Err(std::io::Error::other(err.to_string()));
        }
        Ok(CorePaneIo { shared, writer })
    }
}

impl CorePaneIo {
    pub(crate) fn try_send_bytes(
        &self,
        bytes: Bytes,
    ) -> Result<(), mpsc::error::TrySendError<Bytes>> {
        self.writer
            .try_send(WriteCmd::User(bytes))
            .map_err(|err| match err {
                mpsc::error::TrySendError::Full(WriteCmd::User(bytes)) => {
                    mpsc::error::TrySendError::Full(bytes)
                }
                mpsc::error::TrySendError::Closed(WriteCmd::User(bytes)) => {
                    mpsc::error::TrySendError::Closed(bytes)
                }
                // Only User commands are sent here.
                mpsc::error::TrySendError::Full(_) | mpsc::error::TrySendError::Closed(_) => {
                    mpsc::error::TrySendError::Closed(Bytes::new())
                }
            })
    }

    pub(crate) fn write_terminal_response(&self, response: impl FnOnce() -> Option<Bytes>) {
        if let Some(bytes) = response() {
            let _ = self.writer.try_send(WriteCmd::Reply(bytes));
        }
    }

    pub(crate) fn queue_user_input_submission(
        &self,
        text: Bytes,
        enter: Bytes,
        delay: Duration,
    ) -> std::io::Result<std_mpsc::Receiver<std::io::Result<()>>> {
        let (reply, reply_rx) = std_mpsc::channel();
        self.writer
            .try_send(WriteCmd::Submit {
                text,
                enter,
                delay,
                reply,
            })
            .map_err(|_| std::io::Error::other("herdr-core pane input queue is full or closed"))?;
        Ok(reply_rx)
    }

    pub(crate) fn resize(
        &self,
        rows: u16,
        cols: u16,
        cell_width_px: u32,
        cell_height_px: u32,
        replies: Vec<Bytes>,
    ) {
        *lock(&self.shared.size) = (rows, cols);
        let _ = self.writer.try_send(WriteCmd::Resize {
            rows,
            cols,
            cell: (cell_width_px, cell_height_px),
            replies,
        });
    }

    /// Stops this pane's streams without ending the program (a shutdown or a
    /// handoff detach). The program keeps running in core.
    pub(crate) fn shutdown(&self) {
        if self.shared.closing.swap(true, Ordering::AcqRel) {
            return;
        }
        super::forget_status(&self.shared.address);
        let _ = self.shared.send(kind::CLOSE, Vec::new());
        self.shared
            .client()
            .forget_stream(self.shared.channel.load(Ordering::Acquire));
    }

    /// Ends the program in core (D8's first half; the local sweep follows in
    /// `terminate_session`).
    pub(crate) fn kill(&self) {
        let _ = self.shared.client().call(
            "terminal.kill",
            json!({"terminal": self.shared.address}),
            CALL_TIMEOUT,
        );
    }

    /// The core server's and its holders' pids, for the sweep guard (D8).
    pub(crate) fn core_pids(&self) -> Vec<u32> {
        let Ok(status) = self
            .shared
            .client()
            .call("server.status", json!({}), CALL_TIMEOUT)
        else {
            return Vec::new();
        };
        let mut pids: Vec<u32> = status["holders"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|holder| holder["pid"].as_u64())
            .map(|pid| pid as u32)
            .collect();
        if let Some(pid) = status["pid"].as_u64() {
            pids.push(pid as u32);
        }
        pids
    }
}

/// Queues a finished terminal's last screen and its exit for the parser.
fn queue_finished(client: &CoreClient, address: &str, events: &StreamSink) {
    let screen = client
        .call("terminal.read", json!({"terminal": address}), CALL_TIMEOUT)
        .ok()
        .and_then(|read| read["text"].as_str().map(str::to_string))
        .unwrap_or_default();
    if !screen.is_empty() {
        let _ = events.send(StreamEvent::Data(screen.replace('\n', "\r\n").into_bytes()));
    }
    let record = client
        .call("terminal.get", json!({"terminal": address}), CALL_TIMEOUT)
        .unwrap_or_else(|_| json!({"reason": "exited"}));
    let _ = events.send(StreamEvent::Close(record));
}

fn spawn_writer(
    shared: Arc<Shared>,
    mut rx: mpsc::Receiver<WriteCmd>,
    automatic_input: bool,
) -> Result<(), CoreError> {
    let name = format!("herdr-core-w-{}", shared.pane_id.raw());
    std::thread::Builder::new()
        .name(name)
        .spawn(move || {
            let reply_kind = if automatic_input {
                kind::AUTOMATIC_INPUT
            } else {
                kind::DATA
            };
            while let Some(cmd) = rx.blocking_recv() {
                match cmd {
                    WriteCmd::User(bytes) => {
                        let _ = shared.send(kind::DATA, bytes.to_vec());
                    }
                    WriteCmd::Reply(bytes) => {
                        let _ = shared.send(reply_kind, bytes.to_vec());
                    }
                    WriteCmd::Submit {
                        text,
                        enter,
                        delay,
                        reply,
                    } => {
                        let result = shared
                            .send(kind::DATA, text.to_vec())
                            .and_then(|()| {
                                std::thread::sleep(delay);
                                shared.send(kind::DATA, enter.to_vec())
                            })
                            .map_err(std::io::Error::other);
                        let _ = reply.send(result);
                    }
                    WriteCmd::Resize {
                        rows,
                        cols,
                        cell,
                        replies,
                    } => {
                        let channel = shared.channel.load(Ordering::Acquire);
                        let client = shared.client();
                        let _ = client.send(&Frame::resize(channel, cols, rows));
                        if cell.0 > 0 && cell.1 > 0 {
                            let facts = json!({"cell": [cell.0, cell.1]}).to_string();
                            let _ = client.send(&Frame::new(channel, kind::FACTS, facts));
                        }
                        for reply in replies {
                            let _ = shared.send(reply_kind, reply.to_vec());
                        }
                    }
                }
            }
        })
        .map(|_| ())
        .map_err(|err| CoreError::Unavailable(format!("writer thread: {err}")))
}

fn spawn_parser(
    shared: Arc<Shared>,
    events: std_mpsc::Receiver<StreamEvent>,
    sink: StreamSink,
    writer: mpsc::Sender<WriteCmd>,
    mut on_read: ReadCallback,
    on_exit: Box<dyn FnOnce(ChildExitReason) + Send>,
    child_wait_completed: Arc<AtomicBool>,
) -> Result<(), CoreError> {
    let name = format!("herdr-core-r-{}", shared.pane_id.raw());
    std::thread::Builder::new()
        .name(name)
        .spawn(move || {
            let mut on_exit = Some(on_exit);
            let mut finish = |reason: ChildExitReason| {
                super::forget_status(&shared.address);
                child_wait_completed.store(true, Ordering::Release);
                if let Some(on_exit) = on_exit.take() {
                    on_exit(reason);
                }
            };
            loop {
                let event = match events.recv() {
                    Ok(event) => event,
                    Err(_) => return,
                };
                match event {
                    StreamEvent::Data(bytes) => {
                        let consumed = bytes.len() as u32;
                        let PtyReadResult { terminal_responses } = on_read(&bytes);
                        for reply in terminal_responses {
                            let _ = writer.try_send(WriteCmd::Reply(reply));
                        }
                        let channel = shared.channel.load(Ordering::Acquire);
                        if channel != 0 {
                            let _ = shared.client().send(&Frame::credit(channel, consumed));
                        }
                    }
                    StreamEvent::Close(reason) => {
                        if let Some(exit) = exit_reason(&reason) {
                            finish(exit);
                            return;
                        }
                        if shared.closing.load(Ordering::Acquire) {
                            return;
                        }
                        if let Some(exit) = reattach(&shared, &sink) {
                            finish(exit);
                            return;
                        }
                    }
                    StreamEvent::ConnectionLost => {
                        if shared.closing.load(Ordering::Acquire) {
                            return;
                        }
                        if let Some(exit) = reattach(&shared, &sink) {
                            finish(exit);
                            return;
                        }
                    }
                }
            }
        })
        .map(|_| ())
        .map_err(|err| CoreError::Unavailable(format!("parser thread: {err}")))
}

/// Reaches the terminal again after its stream ended without an exit (D5).
/// `Some(reason)`: the program is gone, report it. `None`: reattached.
fn reattach(shared: &Shared, sink: &StreamSink) -> Option<ChildExitReason> {
    let deadline = Instant::now() + REATTACH_LIMIT;
    let mut backoff = Duration::from_millis(50);
    loop {
        if shared.closing.load(Ordering::Acquire) {
            return None;
        }
        match try_reattach(shared, sink) {
            Ok(None) => return None,
            Ok(Some(exit)) => return Some(exit),
            Err(err) => {
                debug!(pane = shared.pane_id.raw(), err = %err, "herdr-core reattach failed; retrying");
            }
        }
        if Instant::now() >= deadline {
            warn!(
                pane = shared.pane_id.raw(),
                terminal = %shared.address,
                "herdr-core terminal unreachable for 60 s; the pane ends"
            );
            return Some(ChildExitReason::WaitFailed);
        }
        std::thread::sleep(backoff);
        backoff = (backoff * 2).min(Duration::from_secs(2));
    }
}

fn try_reattach(shared: &Shared, sink: &StreamSink) -> Result<Option<ChildExitReason>, CoreError> {
    let client = {
        let current = shared.client();
        if current.is_closed() {
            let fresh = (shared.connector)()?;
            *lock(&shared.client) = fresh.clone();
            fresh
        } else {
            current
        }
    };
    let record = match client.call(
        "terminal.get",
        json!({"terminal": shared.address}),
        CALL_TIMEOUT,
    ) {
        Ok(record) => record,
        Err(CoreError::Rpc { kind, .. }) if kind == "not_found" => {
            return Ok(Some(ChildExitReason::WaitFailed));
        }
        Err(err) => return Err(err),
    };
    if record["state"] != "live" {
        return Ok(Some(
            exit_reason(&record).unwrap_or(ChildExitReason::WaitFailed),
        ));
    }
    let (rows, cols) = *lock(&shared.size);
    let attach = client.attach(
        json!({"terminal": shared.address, "cols": cols, "rows": rows, "resumed": true}),
        sink.clone(),
        CALL_TIMEOUT,
    )?;
    let channel = attach["channel"].as_u64().unwrap_or(0) as u32;
    shared.channel.store(channel, Ordering::Release);
    debug!(pane = shared.pane_id.raw(), terminal = %shared.address, "herdr-core stream reattached");
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_reasons_map_as_design_d7() {
        let case = |v: Value| exit_reason(&v);
        assert_eq!(
            case(json!({"reason": "exited", "exit_code": 3, "signal": null})),
            Some(ChildExitReason::Exited)
        );
        assert_eq!(
            case(json!({"reason": "exited", "exit_code": null, "signal": 9})),
            Some(ChildExitReason::Interrupted)
        );
        assert_eq!(
            case(json!({"reason": "killed", "exit_code": null, "signal": null})),
            Some(ChildExitReason::Exited)
        );
        for reason in [
            "server_stop",
            "host_shutdown",
            "holder_lost",
            "stopped_by_restart",
        ] {
            assert_eq!(
                case(json!({"reason": reason})),
                Some(ChildExitReason::Interrupted),
                "{reason}"
            );
        }
        assert_eq!(case(json!({})), None, "an empty CLOSE is our own detach");
        assert_eq!(case(Value::Null), None);
    }
}
