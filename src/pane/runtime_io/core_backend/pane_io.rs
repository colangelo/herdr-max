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

use super::client::{
    stream_channel, CoreClient, CoreError, StreamEvent, StreamEvents, StreamSink, CREDIT_WINDOW,
};
use super::frame::{kind, Frame};
use crate::layout::PaneId;
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

/// A terminal an earlier server left running, to attach to again (D10).
pub(crate) struct CoreReattach {
    pub pane_id: PaneId,
    pub address: String,
    /// Labels the terminal must carry to be this pane's program.
    pub labels: serde_json::Map<String, Value>,
    pub rows: u16,
    pub cols: u16,
}

struct Attach {
    pane_id: PaneId,
    address: String,
    pid: u32,
    rows: u16,
    cols: u16,
    /// A terminal this call just started is killed if the attach fails.
    ours_to_kill: bool,
}

/// A pid from core, or 0 when it is missing or out of range (a pid_t is
/// signed; security review M2).
fn core_pid(value: &Value) -> u32 {
    value
        .as_u64()
        .filter(|pid| (1..=i32::MAX as u64).contains(pid))
        .unwrap_or(0) as u32
}

/// Why a `terminal.get` record is not the pane's program, or `None` if it is.
pub(crate) fn reattach_refusal(
    record: &Value,
    labels: &serde_json::Map<String, Value>,
) -> Option<String> {
    let state = record["state"].as_str().unwrap_or("unknown");
    if state != "live" {
        return Some(format!("is {state}, not live"));
    }
    labels.iter().find_map(|(key, want)| {
        let have = &record["labels"][key];
        (have != want).then(|| format!("has label {key}={have}, not {want}"))
    })
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
    /// Set when the next server takes the program over (`herdr server
    /// restart`, D10): dropping the pane then detaches without a kill.
    keep_on_drop: AtomicBool,
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
    events_rx: StreamEvents,
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
        let pid = core_pid(&record["pid"]);
        Self::attach(
            client,
            Attach {
                pane_id: spawn.pane_id,
                address,
                pid,
                rows: spawn.rows,
                cols: spawn.cols,
                ours_to_kill: true,
            },
            child_pid,
            connector,
        )
    }

    /// Attaches to a terminal an earlier server left running (D10 "Arriving").
    /// It must still be live and carry `labels`, or it is not this pane's
    /// program: the caller then starts the pane fresh. A refusal never kills
    /// the terminal, since it may be someone else's.
    pub(crate) fn reattach(
        target: CoreReattach,
        child_pid: &AtomicU32,
        connector: Connector,
    ) -> Result<Self, CoreError> {
        let client = connector()?;
        let record = client.call(
            "terminal.get",
            json!({"terminal": target.address}),
            CALL_TIMEOUT,
        )?;
        if let Some(refusal) = reattach_refusal(&record, &target.labels) {
            return Err(CoreError::Rpc {
                kind: "not_ours".into(),
                message: format!("terminal {} {refusal}", target.address),
            });
        }
        let pid = core_pid(&record["pid"]);
        Self::attach(
            client,
            Attach {
                pane_id: target.pane_id,
                address: target.address,
                pid,
                rows: target.rows,
                cols: target.cols,
                ours_to_kill: false,
            },
            child_pid,
            connector,
        )
    }

    fn attach(
        client: CoreClient,
        attach: Attach,
        child_pid: &AtomicU32,
        connector: Connector,
    ) -> Result<Self, CoreError> {
        let Attach {
            pane_id,
            address,
            pid,
            rows,
            cols,
            ours_to_kill,
        } = attach;
        let (events_tx, events_rx) = stream_channel();
        let (channel, automatic_input) = match client.attach(
            // Output printed before this attach has scrolled past the first
            // paint's screen: ask for the history too, or the pane loses it.
            // A reattach inside one pane's life does not, so lines are not
            // shown twice (D5); a new server's pane starts empty, so it does.
            json!({"terminal": address, "cols": cols, "rows": rows,
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
                if ours_to_kill {
                    let _ =
                        client.call("terminal.kill", json!({"terminal": address}), CALL_TIMEOUT);
                }
                return Err(err);
            }
        };
        if pid > 0 {
            child_pid.store(pid, Ordering::Release);
            crate::logging::pane_spawned(pane_id.raw(), pid);
        }
        Ok(Self {
            shared: Arc::new(Shared {
                pane_id,
                address,
                connector,
                client: Mutex::new(client),
                channel: AtomicU32::new(channel),
                size: Mutex::new((rows, cols)),
                closing: AtomicBool::new(false),
                keep_on_drop: AtomicBool::new(false),
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
        child_wait_completed: Arc<AtomicBool>,
    ) -> std::io::Result<CorePaneIo> {
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
        let _ = self.shared.send(kind::CLOSE, Vec::new());
        self.shared
            .client()
            .forget_stream(self.shared.channel.load(Ordering::Acquire));
    }

    /// The terminal's address in core, saved with the pane (D10).
    pub(crate) fn address(&self) -> &str {
        &self.shared.address
    }

    /// Whether dropping the pane leaves the program running (D10).
    pub(crate) fn set_keep_on_drop(&self, keep: bool) {
        self.shared.keep_on_drop.store(keep, Ordering::Release);
    }

    pub(crate) fn kept_on_drop(&self) -> bool {
        self.shared.keep_on_drop.load(Ordering::Acquire)
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
            .map(|holder| core_pid(&holder["pid"]))
            .filter(|pid| *pid != 0)
            .collect();
        let server = core_pid(&status["pid"]);
        if server != 0 {
            pids.push(server);
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

/// Bytes a pane consumed but has not credited back yet. Credit goes back in
/// batches of `CREDIT_BATCH`, not per chunk: each CREDIT frame wakes core, and
/// a chunk is often one line (V12). Batching cannot stall a stream: core
/// stops only when `CREDIT_WINDOW` bytes are outstanding, and fewer than
/// `CREDIT_BATCH` of those are consumed-but-owed, so the rest is still to be
/// read, and reading it sends the credit.
#[derive(Default)]
struct CreditBatch {
    channel: u32,
    owed: u32,
}

const CREDIT_BATCH: u32 = CREDIT_WINDOW / 4;

impl CreditBatch {
    /// Records `bytes` consumed on `channel`; returns the credit to send now.
    /// Channel 0 is "no stream". A new channel (a reattach) starts with a
    /// fresh window, so what the old one was owed is dropped.
    fn consumed(&mut self, channel: u32, bytes: u32) -> Option<u32> {
        if channel != self.channel {
            self.channel = channel;
            self.owed = 0;
        }
        if channel == 0 {
            return None;
        }
        self.owed = self.owed.saturating_add(bytes);
        (self.owed >= CREDIT_BATCH).then(|| std::mem::take(&mut self.owed))
    }
}

fn spawn_parser(
    shared: Arc<Shared>,
    events: StreamEvents,
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
                child_wait_completed.store(true, Ordering::Release);
                if let Some(on_exit) = on_exit.take() {
                    on_exit(reason);
                }
            };
            let mut credit = CreditBatch::default();
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
                        if let Some(bytes) = credit.consumed(channel, consumed) {
                            let _ = shared.client().send(&Frame::credit(channel, bytes));
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
                        // The connection is still up when the client dropped
                        // only this stream (core sent past its credit):
                        // detach it in core too before attaching anew.
                        if !shared.client().is_closed() {
                            let _ = shared.send(kind::CLOSE, Vec::new());
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

    #[test]
    fn credit_goes_back_in_batches_and_a_new_channel_starts_fresh() {
        let mut credit = CreditBatch::default();
        assert_eq!(credit.consumed(0, CREDIT_BATCH), None, "no stream");
        assert_eq!(credit.consumed(1, 100), None);
        assert_eq!(credit.consumed(1, CREDIT_BATCH - 101), None);
        assert_eq!(credit.consumed(1, 1), Some(CREDIT_BATCH));
        assert_eq!(credit.consumed(1, 10), None);
        // A reattach: the new stream's window is fresh; the old debt is gone.
        assert_eq!(credit.consumed(2, CREDIT_BATCH - 1), None);
        assert_eq!(credit.consumed(2, 2), Some(CREDIT_BATCH + 1));
        // One chunk larger than a batch is credited whole, at once.
        assert_eq!(credit.consumed(2, 3 * CREDIT_BATCH), Some(3 * CREDIT_BATCH));
    }

    use super::super::fake::FakeCore;

    /// One connection per fake core: every connector call reuses it.
    fn connector_to(core: &FakeCore) -> Connector {
        let client = CoreClient::connect(&core.socket, None).expect("connect");
        Arc::new(move || Ok(client.clone()))
    }

    fn labels(pane: &str) -> serde_json::Map<String, Value> {
        let mut labels = serde_json::Map::new();
        labels.insert("herdr.session".into(), json!("herdr-test"));
        labels.insert("herdr.pane_id".into(), json!(pane));
        labels
    }

    fn target(address: &str, pane: &str) -> CoreReattach {
        CoreReattach {
            pane_id: PaneId::from_raw(3),
            address: address.into(),
            labels: labels(pane),
            rows: 24,
            cols: 80,
        }
    }

    #[test]
    fn a_reattach_attaches_to_its_own_live_terminal_and_types_nothing() {
        let core = FakeCore::start_with(|config| config.first_paint = b"the screen".to_vec());
        core.add_terminal(json!({"address": "t7", "pid": 777, "state": "live",
                                  "labels": labels("w1-p2")}));
        let pid = AtomicU32::new(0);
        let started = StartedCore::reattach(target("t7", "w1-p2"), &pid, connector_to(&core))
            .expect("reattach");

        assert_eq!(
            pid.load(Ordering::Acquire),
            777,
            "the running program's pid"
        );
        assert_eq!(core.methods(), ["terminal.get", "terminal.attach"]);
        let attach = &core.requests()[1]["params"];
        assert_eq!(attach["terminal"], "t7");
        assert!(
            attach["history"].as_u64().is_some(),
            "a new server's pane starts empty, so it asks for the scrollback: {attach}"
        );
        assert_eq!(
            started
                .events_rx
                .recv_timeout(Duration::from_secs(5))
                .unwrap(),
            StreamEvent::Data(b"the screen".to_vec()),
            "the first paint repaints the pane"
        );
        // Roundtable R6: nothing goes into the running program. Every frame
        // the client sent is a channel-0 request; none is stream input.
        assert!(
            core.received().iter().all(|frame| frame.channel == 0),
            "{:?}",
            core.received()
        );
    }

    #[test]
    fn a_reattach_refuses_a_terminal_that_is_not_the_panes_and_leaves_it_alone() {
        let cases = [
            (
                "another pane's program",
                json!({"address": "t7", "pid": 7, "state": "live", "labels": labels("w1-p9")}),
                "t7",
            ),
            (
                "a finished program",
                json!({"address": "t7", "pid": 7, "state": "finished", "labels": labels("w1-p2")}),
                "t7",
            ),
            (
                "another session's program",
                json!({"address": "t7", "pid": 7, "state": "live",
                       "labels": {"herdr.session": "herdr", "herdr.pane_id": "w1-p2"}}),
                "t7",
            ),
            ("a terminal core no longer has", json!({}), "t404"),
        ];
        for (case, record, address) in cases {
            let core = FakeCore::start();
            if record.get("address").is_some() {
                core.add_terminal(record);
            }
            let pid = AtomicU32::new(0);
            let result = StartedCore::reattach(target(address, "w1-p2"), &pid, connector_to(&core));
            assert!(result.is_err(), "{case}: refused");
            assert_eq!(pid.load(Ordering::Acquire), 0, "{case}: no pid published");
            assert_eq!(
                core.methods(),
                ["terminal.get"],
                "{case}: never attached, never killed"
            );
        }
    }
}
