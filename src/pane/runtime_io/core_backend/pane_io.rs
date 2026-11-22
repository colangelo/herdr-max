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
use super::status_mirror::StatusMirror;
use crate::layout::PaneId;
use crate::pane::runtime_io::{CoreStatusRecord, CoreTerminalLink};
use crate::platform::{
    ChildExitReason, ProgramIdentity, ProgramLiveness, StartTokenLookup, SuspendCause,
};
use crate::pty::actor::{PtyReadResult, ReadCallback};

const CALL_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_QUEUE: usize = 1024;
/// How long a pane retries quickly after the connection drops before it says
/// core is not answering (a core update takes well under a second, Q6). It
/// keeps retrying after that, every `SLOW_RETRY` (core-phase3 D1a).
const REATTACH_LIMIT: Duration = Duration::from_secs(60);
const SLOW_RETRY: Duration = Duration::from_secs(10);
/// Scrollback lines asked for on a pane's first attach: more than any pane
/// keeps, so all of it arrives.
const FIRST_ATTACH_HISTORY_LINES: u32 = 1_000_000;

/// Returns a live client, reconnecting when the last one closed.
pub(crate) type Connector = Arc<dyn Fn() -> Result<CoreClient, CoreError> + Send + Sync>;

/// Told when core stops answering a pane (the unix ms it started) and when
/// it answers again (`None`), core-phase3 D1a.
pub(crate) type UnreachableCallback = Box<dyn FnMut(Option<u64>) + Send>;

/// How a pane reaches its terminal again after its stream ends, and how it
/// tells whether the program still runs (core-phase3 D1a). Tests shorten the
/// times and pass a fake process table.
#[derive(Clone, Copy)]
pub(crate) struct ReattachPolicy {
    pub limit: Duration,
    pub slow_retry: Duration,
    pub start_token: StartTokenLookup,
}

impl Default for ReattachPolicy {
    fn default() -> Self {
        Self {
            limit: REATTACH_LIMIT,
            slow_retry: SLOW_RETRY,
            start_token: crate::platform::process_start,
        }
    }
}

/// What `terminal.run` needs for one pane.
pub(crate) struct CoreSpawn {
    pub pane_id: PaneId,
    /// The core session the terminal runs in, saved in the pane's link.
    pub session: String,
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
    pub session: String,
    pub address: String,
    /// Labels the terminal must carry to be this pane's program.
    pub labels: serde_json::Map<String, Value>,
    pub rows: u16,
    pub cols: u16,
}

struct Attach {
    pane_id: PaneId,
    session: String,
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
    session: String,
    address: String,
    /// The program as it was at attach, for telling a gone program from one
    /// core lost track of (core-phase3 D1a).
    program: Option<ProgramIdentity>,
    policy: ReattachPolicy,
    connector: Connector,
    client: Mutex<CoreClient>,
    channel: AtomicU32,
    /// Where the client routes this pane's stream; shutdown also uses it to
    /// end the parser without waiting for core's CLOSE.
    sink: StreamSink,
    /// The size last asked for, for a reattach.
    size: Mutex<(u16, u16)>,
    /// Set when the pane shuts down: stream ends are then not reattached.
    closing: AtomicBool,
    /// Set when the next server takes the program over (`herdr server
    /// restart`, D10): dropping the pane then detaches without a kill.
    keep_on_drop: AtomicBool,
    /// The program's pid from core, 0 when unknown.
    pid: u32,
    /// The program's own terminal, pinned at attach (or at the first record
    /// when it had none yet): where mirrored status goes, and only while
    /// that same process owns it (core-status-mirror D4, security M1).
    terminal: Mutex<Option<crate::platform::ProcessTerminal>>,
    /// The latest status record to mirror, until the parser takes it: one
    /// slot, so a burst of changes never queues (core-status-mirror D3).
    mirror_slot: Mutex<Option<CoreStatusRecord>>,
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
    /// Taken on shutdown, so the writer thread ends with the parser.
    writer: Mutex<Option<mpsc::Sender<WriteCmd>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Maps a stream CLOSE reason or a finished terminal record to an exit
/// (core-phase3 D1). The program's own end, or a person's kill, ends the
/// pane; a runtime that ended the program suspends it. `None`: not an exit
/// (our own detach, or a record still live).
pub(crate) fn exit_reason(record: &Value) -> Option<ChildExitReason> {
    let reason = record.get("reason")?.as_str()?;
    let signal = record.get("signal").is_some_and(|signal| !signal.is_null());
    Some(match reason {
        "exited" | "killed" if signal => ChildExitReason::Interrupted,
        "exited" | "killed" => ChildExitReason::Exited,
        "holder_lost" => ChildExitReason::Suspended(SuspendCause::HolderLost),
        "host_shutdown" => ChildExitReason::Suspended(SuspendCause::HostShutdown),
        "server_stop" => ChildExitReason::Suspended(SuspendCause::CoreStopped),
        "stopped_by_restart" => ChildExitReason::Suspended(SuspendCause::StoppedByRestart),
        _ => ChildExitReason::Interrupted,
    })
}

/// The label that names one `terminal.run`, so a launch whose answer was
/// lost can be found again (security review M3, Sol H7).
const LAUNCH_LABEL: &str = "herdr.launch";

/// Why a core start failed, and whether core may run the program anyway
/// (security review M3, Sol H7).
#[derive(Debug)]
pub(crate) enum StartFailure {
    /// Core never ran the program, or ended it and said so: the pane may
    /// start it on the PTY.
    NotStarted(CoreError),
    /// Core may run the program: starting it anywhere else could make a
    /// second copy.
    Unknown(CoreError),
}

impl StartFailure {
    fn into_error(self) -> CoreError {
        match self {
            Self::NotStarted(err) | Self::Unknown(err) => err,
        }
    }
}

/// What core said to `terminal.kill`, which returns the terminal as it
/// observed it (core-phase3 D9, roundtable item 8).
#[derive(Debug)]
pub(crate) enum KillOutcome {
    /// The terminal's record says `finished`.
    Finished,
    /// Core no longer knows the terminal.
    NotFound,
    /// Core answered, and the terminal still runs.
    Live,
    /// No answer, or an error: core may still run the program.
    Failed(CoreError),
}

impl KillOutcome {
    pub(super) fn from_answer(answer: Result<Value, CoreError>) -> Self {
        match answer {
            Ok(record) if record["state"] == "finished" => Self::Finished,
            Ok(_) => Self::Live,
            Err(CoreError::Rpc { kind, .. }) if kind == "finished" => Self::Finished,
            Err(CoreError::Rpc { kind, .. }) if kind == "not_found" => Self::NotFound,
            Err(err) => Self::Failed(err),
        }
    }

    /// Core confirms the program ended: only `finished` or `not_found`.
    pub(crate) fn ended(&self) -> bool {
        matches!(self, Self::Finished | Self::NotFound)
    }
}

impl std::fmt::Display for KillOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Finished => write!(f, "herdr-core ended the terminal"),
            Self::NotFound => write!(f, "herdr-core no longer knows the terminal"),
            Self::Live => write!(f, "herdr-core says the terminal still runs"),
            Self::Failed(err) => write!(f, "{err}"),
        }
    }
}

fn kill_terminal(client: &CoreClient, address: &str) -> KillOutcome {
    KillOutcome::from_answer(client.call(
        "terminal.kill",
        json!({"terminal": address}),
        CALL_TIMEOUT,
    ))
}

/// A core terminal that is running and attached, before the pane hands over
/// its callbacks. Splitting the spawn this way keeps the callbacks with the
/// caller until core has said yes, so a refusal can fall back to the PTY.
pub(crate) struct StartedCore {
    shared: Arc<Shared>,
    events_rx: StreamEvents,
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
    ) -> Result<Self, StartFailure> {
        Self::start_with(spawn, child_pid, connector, ReattachPolicy::default())
    }

    /// A start falls back to the PTY only when core certainly runs nothing
    /// for it: core was not reached, refused the run, or confirmed it ended
    /// the program. A run whose answer was lost is found again by its launch
    /// label and attached (security review M3, Sol H7).
    pub(crate) fn start_with(
        spawn: CoreSpawn,
        child_pid: &AtomicU32,
        connector: Connector,
        policy: ReattachPolicy,
    ) -> Result<Self, StartFailure> {
        let client = connector().map_err(StartFailure::NotStarted)?;
        let launch = crate::platform::random_token().map_err(|err| {
            StartFailure::NotStarted(CoreError::Unavailable(format!("no launch id: {err}")))
        })?;
        let mut labels = spawn.labels.clone();
        labels.insert(LAUNCH_LABEL.into(), Value::String(launch.clone()));
        let reply = client.call(
            "terminal.run",
            json!({
                "argv": spawn.argv,
                "cwd": spawn.cwd,
                "env": spawn.env,
                "labels": labels,
                "size": {"cols": spawn.cols, "rows": spawn.rows},
                "no_parent": true,
                "no_notify": true,
                "keep": false,
            }),
            CALL_TIMEOUT,
        );
        let started = match reply {
            Ok(record) => record["address"]
                .as_str()
                .map(|address| (address.to_string(), core_pid(&record["pid"])))
                .ok_or_else(|| CoreError::Rpc {
                    kind: "protocol".into(),
                    message: format!("terminal.run returned no address: {record}"),
                }),
            Err(err @ CoreError::Rpc { .. }) => return Err(StartFailure::NotStarted(err)),
            Err(err) => Err(err),
        };
        let attach = |address: String, pid: u32| Attach {
            pane_id: spawn.pane_id,
            session: spawn.session.clone(),
            address,
            pid,
            rows: spawn.rows,
            cols: spawn.cols,
            ours_to_kill: true,
        };
        match started {
            Ok((address, pid)) => {
                Self::attach(client, attach(address, pid), child_pid, connector, policy)
            }
            Err(err) => {
                warn!(
                    pane = spawn.pane_id.raw(),
                    err = %err,
                    "terminal.run gave no answer; looking for the launch in herdr-core"
                );
                let found = connector().and_then(|client| {
                    let listed = client.call(
                        "terminal.list",
                        json!({"labels": {LAUNCH_LABEL: launch}}),
                        CALL_TIMEOUT,
                    )?;
                    Ok((client, listed))
                });
                let (client, listed) = found.map_err(|_| StartFailure::Unknown(err.clone()))?;
                let ours: Vec<&Value> = listed["terminals"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|terminal| terminal["labels"][LAUNCH_LABEL] == launch.as_str())
                    .collect();
                match ours.as_slice() {
                    [one] => {
                        let address = one["address"]
                            .as_str()
                            .ok_or_else(|| StartFailure::Unknown(err.clone()))?
                            .to_string();
                        let pid = core_pid(&one["pid"]);
                        Self::attach(client, attach(address, pid), child_pid, connector, policy)
                    }
                    // None listed proves nothing: a slow run may land after
                    // the list.
                    _ => Err(StartFailure::Unknown(err)),
                }
            }
        }
    }

    /// Attaches to a terminal an earlier server left running (D10 "Arriving").
    /// It must still be live and carry `labels`, or it is not this pane's
    /// program. A refusal never kills the terminal, since it may be someone
    /// else's. The error says what core answered, so the caller can tell a
    /// gone program from an unreachable core (core-phase3 D1b).
    pub(crate) fn reattach(
        target: CoreReattach,
        child_pid: &AtomicU32,
        connector: Connector,
    ) -> Result<Self, ReattachError> {
        let client = connector().map_err(ReattachError::Unreachable)?;
        let record = match client.call(
            "terminal.get",
            json!({"terminal": target.address}),
            CALL_TIMEOUT,
        ) {
            Ok(record) => record,
            Err(CoreError::Rpc { kind, message }) if kind == "not_found" => {
                return Err(ReattachError::Gone(message));
            }
            Err(err) => return Err(ReattachError::Unreachable(err)),
        };
        if let Some(refusal) = reattach_refusal(&record, &target.labels) {
            let message = format!("terminal {} {refusal}", target.address);
            return Err(if record["state"] == "live" {
                ReattachError::NotOurs(message)
            } else {
                ReattachError::Gone(message)
            });
        }
        let pid = core_pid(&record["pid"]);
        Self::attach(
            client,
            Attach {
                pane_id: target.pane_id,
                session: target.session,
                address: target.address,
                pid,
                rows: target.rows,
                cols: target.cols,
                ours_to_kill: false,
            },
            child_pid,
            connector,
            ReattachPolicy::default(),
        )
        .map_err(|failure| ReattachError::Unreachable(failure.into_error()))
    }

    fn attach(
        client: CoreClient,
        attach: Attach,
        child_pid: &AtomicU32,
        connector: Connector,
        policy: ReattachPolicy,
    ) -> Result<Self, StartFailure> {
        let Attach {
            pane_id,
            session,
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
                // Do not leave a program running that no pane shows; one
                // core does not confirm ended may still run.
                if ours_to_kill && !kill_terminal(&client, &address).ended() {
                    return Err(StartFailure::Unknown(err));
                }
                return Err(StartFailure::NotStarted(err));
            }
        };
        if pid > 0 {
            child_pid.store(pid, Ordering::Release);
            crate::logging::pane_spawned(pane_id.raw(), pid);
        }
        Ok(Self {
            shared: Arc::new(Shared {
                pane_id,
                session,
                address,
                program: ProgramIdentity::of(pid, policy.start_token),
                policy,
                connector,
                client: Mutex::new(client),
                channel: AtomicU32::new(channel),
                sink: events_tx,
                size: Mutex::new((rows, cols)),
                closing: AtomicBool::new(false),
                keep_on_drop: AtomicBool::new(false),
                pid,
                terminal: Mutex::new(
                    (pid > 0)
                        .then(|| crate::platform::ProcessTerminal::of(pid))
                        .flatten(),
                ),
                mirror_slot: Mutex::new(None),
            }),
            events_rx,
            automatic_input,
        })
    }

    /// Starts the pane's parser and writer threads.
    pub(crate) fn into_pane(
        self,
        on_read: ReadCallback,
        on_exit: Box<dyn FnOnce(ChildExitReason) + Send>,
        on_unreachable: UnreachableCallback,
        child_wait_completed: Arc<AtomicBool>,
    ) -> std::io::Result<CorePaneIo> {
        let Self {
            shared,
            events_rx,
            automatic_input,
        } = self;
        let (writer, writer_rx) = mpsc::channel(WRITE_QUEUE);
        let started =
            spawn_writer(Arc::clone(&shared), writer_rx, automatic_input).and_then(|()| {
                spawn_parser(
                    Arc::clone(&shared),
                    events_rx,
                    writer.clone(),
                    on_read,
                    on_exit,
                    on_unreachable,
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
        Ok(CorePaneIo {
            shared,
            writer: Mutex::new(Some(writer)),
        })
    }
}

impl CorePaneIo {
    fn queue(&self, cmd: WriteCmd) -> Result<(), mpsc::error::TrySendError<WriteCmd>> {
        match lock(&self.writer).as_ref() {
            Some(writer) => writer.try_send(cmd),
            None => Err(mpsc::error::TrySendError::Closed(cmd)),
        }
    }

    pub(crate) fn try_send_bytes(
        &self,
        bytes: Bytes,
    ) -> Result<(), mpsc::error::TrySendError<Bytes>> {
        self.queue(WriteCmd::User(bytes)).map_err(|err| match err {
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
            let _ = self.queue(WriteCmd::Reply(bytes));
        }
    }

    pub(crate) fn queue_user_input_submission(
        &self,
        text: Bytes,
        enter: Bytes,
        delay: Duration,
    ) -> std::io::Result<std_mpsc::Receiver<std::io::Result<()>>> {
        let (reply, reply_rx) = std_mpsc::channel();
        self.queue(WriteCmd::Submit {
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
        let _ = self.queue(WriteCmd::Resize {
            rows,
            cols,
            cell: (cell_width_px, cell_height_px),
            replies,
        });
    }

    /// Stops this pane's streams and threads without ending the program (a
    /// shutdown or a handoff detach). The program keeps running in core.
    /// The parser and writer end here, not on core's CLOSE: once the stream
    /// is forgotten, that CLOSE is never routed to the pane.
    pub(crate) fn shutdown(&self) {
        if self.shared.closing.swap(true, Ordering::AcqRel) {
            return;
        }
        let _ = self.shared.send(kind::CLOSE, Vec::new());
        self.shared
            .client()
            .forget_stream(self.shared.channel.load(Ordering::Acquire));
        let _ = self.shared.sink.send(StreamEvent::Close(Value::Null));
        lock(&self.writer).take();
    }

    /// The terminal in core, saved with the pane (D10, core-phase3 D1b).
    pub(crate) fn link(&self) -> CoreTerminalLink {
        CoreTerminalLink {
            agent: None,
            session: self.shared.session.clone(),
            address: self.shared.address.clone(),
            program: self.shared.program,
        }
    }

    /// Whether dropping the pane leaves the program running (D10).
    pub(crate) fn set_keep_on_drop(&self, keep: bool) {
        self.shared.keep_on_drop.store(keep, Ordering::Release);
    }

    pub(crate) fn kept_on_drop(&self) -> bool {
        self.shared.keep_on_drop.load(Ordering::Acquire)
    }

    /// Mirrors `record` into core through the program's terminal once the
    /// pane's output allows it (core-status-mirror D3). Replaces a record not
    /// yet written; wakes the parser only when the slot was empty.
    pub(crate) fn mirror_status(&self, record: CoreStatusRecord) {
        let was_empty = lock(&self.shared.mirror_slot).replace(record).is_none();
        if was_empty {
            let _ = self.shared.sink.send(StreamEvent::Wake);
        }
    }

    /// Ends the program in core (D8's first half; the local sweep follows in
    /// `terminate_session`) and says what core observed.
    pub(crate) fn kill(&self) -> KillOutcome {
        kill_terminal(&self.shared.client(), &self.shared.address)
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
    writer: mpsc::Sender<WriteCmd>,
    mut on_read: ReadCallback,
    on_exit: Box<dyn FnOnce(ChildExitReason) + Send>,
    mut on_unreachable: UnreachableCallback,
    child_wait_completed: Arc<AtomicBool>,
) -> Result<(), CoreError> {
    let name = format!("herdr-core-r-{}", shared.pane_id.raw());
    std::thread::Builder::new()
        .name(name)
        .spawn(move || {
            let mut on_exit = Some(on_exit);
            let mut finish = |reason: ChildExitReason| {
                if reason == ChildExitReason::Suspended(SuspendCause::CoreStopped) {
                    super::note_stopped_by_person(&shared.session);
                }
                child_wait_completed.store(true, Ordering::Release);
                if let Some(on_exit) = on_exit.take() {
                    on_exit(reason);
                }
            };
            let mut credit = CreditBatch::default();
            let mut mirror = StatusMirror::default();
            loop {
                let event = match mirror.wake_in(Instant::now()) {
                    None => match events.recv() {
                        Ok(event) => event,
                        Err(_) => return,
                    },
                    Some(wait) => match events.recv_timeout(wait) {
                        Ok(event) => event,
                        Err(std_mpsc::RecvTimeoutError::Timeout) => {
                            write_mirrored_status(&shared, &mut mirror);
                            continue;
                        }
                        Err(std_mpsc::RecvTimeoutError::Disconnected) => return,
                    },
                };
                match event {
                    StreamEvent::Wake => {
                        if let Some(record) = lock(&shared.mirror_slot).take() {
                            mirror.offer(record);
                        }
                        write_mirrored_status(&shared, &mut mirror);
                    }
                    StreamEvent::Data(bytes) => {
                        mirror.observe(&bytes, Instant::now());
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
                        if let Some(exit) = reattach(&shared, &mut on_unreachable) {
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
                        if let Some(exit) = reattach(&shared, &mut on_unreachable) {
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

/// Writes the waiting status record when the gate is open (core-status-mirror
/// D3, D4). A full terminal buffer keeps it for a retry; any other failure
/// drops it, since the next status change sends a fresh one.
fn write_mirrored_status(shared: &Shared, mirror: &mut StatusMirror) {
    let Some(record) = mirror.take_due(Instant::now()) else {
        return;
    };
    let terminal = {
        let mut pinned = lock(&shared.terminal);
        if pinned.is_none() && shared.pid > 0 {
            *pinned = crate::platform::ProcessTerminal::of(shared.pid);
        }
        *pinned
    };
    let Some(terminal) = terminal else {
        debug!(
            pane = shared.pane_id.raw(),
            terminal = %shared.address,
            "no program terminal to mirror agent status into"
        );
        return;
    };
    match terminal.write(&record.encode()) {
        Ok(()) => mirror.written(record),
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
            mirror.busy(record, Instant::now());
        }
        Err(err) => debug!(
            pane = shared.pane_id.raw(),
            terminal = %shared.address,
            err = %err,
            "could not mirror agent status into herdr-core"
        ),
    }
}

/// Why a saved terminal was not attached (core-phase3 D1b).
#[derive(Debug)]
pub(crate) enum ReattachError {
    /// Core answered: it no longer has the terminal, or the program finished.
    Gone(String),
    /// The address is live but labelled for another pane.
    NotOurs(String),
    /// Core did not answer.
    Unreachable(CoreError),
}

impl std::fmt::Display for ReattachError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Gone(message) | Self::NotOurs(message) => f.write_str(message),
            Self::Unreachable(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for ReattachError {}

/// What one try to reach the terminal again found (core-phase3 D1a).
#[derive(Debug, PartialEq, Eq)]
enum ReattachOutcome {
    Reattached,
    /// Core answered and the program is gone: the pane reports this exit.
    Gone(ChildExitReason),
    /// Core did not answer, or answered while the program may still run.
    Unreachable,
}

fn unix_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or(0)
}

/// Reaches the terminal again after its stream ended without an exit (D5).
/// `Some(reason)`: the program is gone, report it. `None`: reattached, or the
/// pane is closing. An unreachable core never ends the pane: after the
/// policy's limit the pane is told, and retries go on (core-phase3 D1a).
fn reattach(shared: &Shared, on_unreachable: &mut UnreachableCallback) -> Option<ChildExitReason> {
    let started = Instant::now();
    let mut backoff = Duration::from_millis(50);
    let mut reported = false;
    loop {
        if shared.closing.load(Ordering::Acquire) {
            return None;
        }
        match try_reattach(shared) {
            ReattachOutcome::Reattached => {
                if reported {
                    on_unreachable(None);
                }
                return None;
            }
            ReattachOutcome::Gone(exit) => return Some(exit),
            ReattachOutcome::Unreachable => {}
        }
        let slow = started.elapsed() >= shared.policy.limit;
        if slow && !reported {
            warn!(
                pane = shared.pane_id.raw(),
                session = %shared.session,
                terminal = %shared.address,
                "herdr-core is not answering; the program keeps running and the pane keeps retrying"
            );
            on_unreachable(Some(unix_ms_now()));
            reported = true;
        }
        let pause = if slow {
            shared.policy.slow_retry
        } else {
            backoff
        };
        sleep_unless_closing(shared, pause);
        backoff = (backoff * 2).min(Duration::from_secs(2));
    }
}

fn sleep_unless_closing(shared: &Shared, pause: Duration) {
    let until = Instant::now() + pause;
    while !shared.closing.load(Ordering::Acquire) {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        std::thread::sleep(left.min(Duration::from_millis(100)));
    }
}

fn try_reattach(shared: &Shared) -> ReattachOutcome {
    let client = {
        let current = shared.client();
        if current.is_closed() {
            match (shared.connector)() {
                Ok(fresh) => {
                    *lock(&shared.client) = fresh.clone();
                    fresh
                }
                Err(err) => {
                    debug!(pane = shared.pane_id.raw(), err = %err, "herdr-core reconnect failed; retrying");
                    return ReattachOutcome::Unreachable;
                }
            }
        } else {
            current
        }
    };
    let answered = match client.call(
        "terminal.get",
        json!({"terminal": shared.address}),
        CALL_TIMEOUT,
    ) {
        Ok(record) if record["state"] == "live" => None,
        Ok(record) => Some(
            exit_reason(&record).unwrap_or(ChildExitReason::Suspended(SuspendCause::TerminalGone)),
        ),
        Err(CoreError::Rpc { kind, .. }) if kind == "not_found" => {
            Some(ChildExitReason::Suspended(SuspendCause::TerminalGone))
        }
        Err(err) => {
            debug!(pane = shared.pane_id.raw(), err = %err, "herdr-core terminal.get failed; retrying");
            return ReattachOutcome::Unreachable;
        }
    };
    if let Some(exit) = answered {
        return match ProgramIdentity::liveness(shared.program, shared.policy.start_token) {
            ProgramLiveness::Gone => ReattachOutcome::Gone(exit),
            ProgramLiveness::Runs | ProgramLiveness::Unknown => {
                debug!(
                    pane = shared.pane_id.raw(),
                    terminal = %shared.address,
                    pid = shared.program.map_or(0, |program| program.pid),
                    "herdr-core says the terminal is gone, but its program may still run; retrying"
                );
                ReattachOutcome::Unreachable
            }
        };
    }
    let (rows, cols) = *lock(&shared.size);
    match client.attach(
        json!({"terminal": shared.address, "cols": cols, "rows": rows, "resumed": true}),
        shared.sink.clone(),
        CALL_TIMEOUT,
    ) {
        Ok(attach) => {
            let channel = attach["channel"].as_u64().unwrap_or(0) as u32;
            shared.channel.store(channel, Ordering::Release);
            debug!(pane = shared.pane_id.raw(), terminal = %shared.address, "herdr-core stream reattached");
            ReattachOutcome::Reattached
        }
        Err(err) => {
            debug!(pane = shared.pane_id.raw(), err = %err, "herdr-core reattach failed; retrying");
            ReattachOutcome::Unreachable
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // V1 (core-phase3 D1): the program's own end and a person's kill end the
    // pane; a runtime that ended the program suspends it, by cause.
    #[test]
    fn exit_reasons_map_as_core_phase3_d1() {
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
            case(json!({"reason": "killed", "exit_code": null, "signal": 15})),
            Some(ChildExitReason::Interrupted)
        );
        assert_eq!(
            case(json!({"reason": "killed", "exit_code": null, "signal": null})),
            Some(ChildExitReason::Exited),
            "a close, `hc kill` or a phone close ends the pane"
        );
        for (reason, cause) in [
            ("holder_lost", SuspendCause::HolderLost),
            ("host_shutdown", SuspendCause::HostShutdown),
            ("server_stop", SuspendCause::CoreStopped),
            ("stopped_by_restart", SuspendCause::StoppedByRestart),
        ] {
            assert_eq!(
                case(json!({"reason": reason})),
                Some(ChildExitReason::Suspended(cause)),
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
            session: "herdr-test".into(),
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
    fn a_shut_down_pane_ends_its_threads_without_waiting_for_cores_close() {
        // The fake core never answers a detach with CLOSE.
        let core = FakeCore::start();
        let client = CoreClient::connect(&core.socket, None).expect("connect");
        let (pane_alive, pane_gone) = std_mpsc::channel::<()>();
        let connector: Connector = Arc::new(move || {
            let _held = &pane_alive;
            Ok(client.clone())
        });
        let (parser_alive, parser_gone) = std_mpsc::channel::<()>();
        let on_read: ReadCallback = Box::new(move |_: &[u8]| {
            let _held = &parser_alive;
            PtyReadResult {
                terminal_responses: Vec::new(),
            }
        });
        let spawn = CoreSpawn {
            pane_id: PaneId::from_raw(4),
            session: "herdr-test".into(),
            argv: vec!["sh".into()],
            cwd: None,
            env: serde_json::Map::new(),
            labels: labels("w1-p4"),
            rows: 24,
            cols: 80,
        };
        let pid = AtomicU32::new(0);
        let pane = StartedCore::start(spawn, &pid, connector)
            .expect("start")
            .into_pane(
                on_read,
                Box::new(|_| {}),
                Box::new(|_| {}),
                Arc::new(AtomicBool::new(false)),
            )
            .expect("pane");

        pane.shutdown();
        assert_eq!(
            parser_gone.recv_timeout(Duration::from_secs(5)),
            Err(std_mpsc::RecvTimeoutError::Disconnected),
            "the parser ended"
        );
        assert!(
            matches!(
                pane.try_send_bytes(Bytes::from_static(b"x")),
                Err(mpsc::error::TrySendError::Closed(_))
            ),
            "input after shutdown is refused"
        );
        drop(pane);
        assert_eq!(
            pane_gone.recv_timeout(Duration::from_secs(5)),
            Err(std_mpsc::RecvTimeoutError::Disconnected),
            "no thread still holds the pane"
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

    /// A started pane on `core` whose exits and unreachable reports arrive
    /// on the returned channels.
    fn pane_on(
        connector: Connector,
        policy: ReattachPolicy,
    ) -> (
        CorePaneIo,
        std_mpsc::Receiver<ChildExitReason>,
        std_mpsc::Receiver<Option<u64>>,
    ) {
        let (exit_tx, exit_rx) = std_mpsc::channel();
        let (unreachable_tx, unreachable_rx) = std_mpsc::channel();
        let spawn = CoreSpawn {
            pane_id: PaneId::from_raw(6),
            session: "herdr-test".into(),
            argv: vec!["sh".into()],
            cwd: None,
            env: serde_json::Map::new(),
            labels: labels("w1-p6"),
            rows: 24,
            cols: 80,
        };
        let pid = AtomicU32::new(0);
        let pane = StartedCore::start_with(spawn, &pid, connector, policy)
            .expect("start")
            .into_pane(
                Box::new(|_: &[u8]| PtyReadResult {
                    terminal_responses: Vec::new(),
                }),
                Box::new(move |reason| {
                    let _ = exit_tx.send(reason);
                }),
                Box::new(move |since| {
                    let _ = unreachable_tx.send(since);
                }),
                Arc::new(AtomicBool::new(false)),
            )
            .expect("pane");
        (pane, exit_rx, unreachable_rx)
    }

    fn spawn_for(pane: &str) -> CoreSpawn {
        CoreSpawn {
            pane_id: PaneId::from_raw(6),
            session: "herdr-test".into(),
            argv: vec!["sh".into()],
            cwd: None,
            env: serde_json::Map::new(),
            labels: labels(pane),
            rows: 24,
            cols: 80,
        }
    }

    /// A connector that connects anew each time, as the server's does after
    /// a connection drops.
    fn reconnecting(core: &FakeCore) -> Connector {
        let socket = core.socket.clone();
        Arc::new(move || CoreClient::connect(&socket, None))
    }

    fn runs(core: &FakeCore) -> usize {
        core.methods()
            .iter()
            .filter(|method| *method == "terminal.run")
            .count()
    }

    // Security review M3 (Sol H7): a run whose answer was lost is found by
    // its launch label and attached; it is never run again.
    #[test]
    fn a_lost_run_answer_is_found_by_its_launch_label() {
        let core = FakeCore::start_with(|config| config.lose_run_reply = true);
        let pid = AtomicU32::new(0);
        let started = StartedCore::start(spawn_for("w1-p6"), &pid, reconnecting(&core))
            .expect("the launch reconciled");
        assert_eq!(started.shared.address, "t1");
        assert_eq!(pid.load(Ordering::Acquire), 4242);
        assert_eq!(runs(&core), 1, "{:?}", core.methods());
        let run = &core.requests()[0]["params"]["labels"];
        let list = core
            .requests()
            .into_iter()
            .find(|request| request["method"] == "terminal.list")
            .expect("a list by launch");
        assert_eq!(list["params"]["labels"][LAUNCH_LABEL], run[LAUNCH_LABEL]);
        assert_eq!(run[LAUNCH_LABEL].as_str().map(str::len), Some(32));
    }

    // Security review M3 (Sol H7): only a definite failure lets the pane use
    // the PTY.
    #[test]
    fn only_a_definite_failure_allows_the_pty() {
        let not_started = |failure: Result<StartedCore, StartFailure>| {
            matches!(failure, Err(StartFailure::NotStarted(_)))
        };
        let unknown = |failure: Result<StartedCore, StartFailure>| {
            matches!(failure, Err(StartFailure::Unknown(_)))
        };
        let pid = AtomicU32::new(0);
        let start =
            |core: &FakeCore| StartedCore::start(spawn_for("w1-p6"), &pid, reconnecting(core));

        let refused = FakeCore::start_with(|config| config.failing = vec!["terminal.run"]);
        assert!(not_started(start(&refused)), "core refused the run");

        let no_core: Connector = Arc::new(|| Err(CoreError::Unavailable("no core".into())));
        assert!(not_started(StartedCore::start(
            spawn_for("w1-p6"),
            &pid,
            no_core
        )));

        let killed = FakeCore::start_with(|config| config.failing = vec!["terminal.attach"]);
        assert!(not_started(start(&killed)), "core confirmed the kill");

        let lost = FakeCore::start_with(|config| {
            config.lose_run_reply = true;
            config.failing = vec!["terminal.list"];
        });
        assert!(unknown(start(&lost)), "the run's outcome is unknown");

        let unkilled = FakeCore::start_with(|config| {
            config.failing = vec!["terminal.attach", "terminal.kill"];
        });
        assert!(unknown(start(&unkilled)), "the kill was not confirmed");
    }

    fn short_policy(start_token: StartTokenLookup) -> ReattachPolicy {
        ReattachPolicy {
            limit: Duration::from_millis(200),
            slow_retry: Duration::from_millis(50),
            start_token,
        }
    }

    /// Ends the pane's stream the way a dropped stream does, with the
    /// connection still up, so the pane asks core about its terminal again.
    fn drop_stream(core: &FakeCore) {
        core.close_stream(1, b"");
    }

    const NO_EXIT: Duration = Duration::from_millis(600);

    // V3 (D1a): a core that never answers again does not end the pane; after
    // the limit the pane is told, and it keeps retrying.
    #[test]
    fn an_unreachable_core_never_ends_the_pane_and_the_loop_keeps_retrying() {
        static CALLS: AtomicU32 = AtomicU32::new(0);
        let core = FakeCore::start();
        let client = CoreClient::connect(&core.socket, None).expect("connect");
        let connector: Connector = Arc::new(move || {
            if CALLS.fetch_add(1, Ordering::AcqRel) == 0 {
                Ok(client.clone())
            } else {
                Err(CoreError::Unavailable("socket refused".into()))
            }
        });
        let (pane, exits, unreachable) = pane_on(
            connector,
            short_policy(|pid| crate::platform::ProcessStart::table((pid == 4242).then_some(7))),
        );

        core.hang_up();
        let since = unreachable
            .recv_timeout(Duration::from_secs(5))
            .expect("the pane is told core is not answering");
        assert!(since.is_some_and(|since| since > 0), "{since:?}");
        let calls = CALLS.load(Ordering::Acquire);
        assert_eq!(exits.recv_timeout(NO_EXIT).ok(), None, "never an exit");
        assert!(
            CALLS.load(Ordering::Acquire) > calls,
            "the loop still retries after the limit"
        );
        pane.shutdown();
    }

    static TOKEN_OF_4242: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(7);

    fn token_of_4242(pid: u32) -> crate::platform::ProcessStart {
        let token = TOKEN_OF_4242.load(Ordering::Acquire);
        crate::platform::ProcessStart::table((pid == 4242 && token > 0).then_some(token))
    }

    // V3 (D1a): core says `not_found`, the outcome depends on the program
    // recorded at attach: running, reused (another start token) or gone.
    #[test]
    fn not_found_is_gone_only_when_the_recorded_program_is() {
        let core = FakeCore::start();
        let (pane, exits, unreachable) = pane_on(connector_to(&core), short_policy(token_of_4242));
        assert_eq!(
            pane.link().program,
            Some(ProgramIdentity {
                pid: 4242,
                started: 7
            })
        );

        core.forget_terminal("t1");
        drop_stream(&core);
        assert!(
            unreachable
                .recv_timeout(Duration::from_secs(5))
                .ok()
                .flatten()
                .is_some(),
            "running: not answering as far as the pane can tell"
        );
        assert_eq!(exits.recv_timeout(NO_EXIT).ok(), None, "running: no exit");

        // The pid now belongs to another process.
        TOKEN_OF_4242.store(8, Ordering::Release);
        assert_eq!(
            exits.recv_timeout(Duration::from_secs(5)).ok(),
            Some(ChildExitReason::Suspended(SuspendCause::TerminalGone)),
            "reused pid: gone"
        );
    }

    #[test]
    fn not_found_with_the_program_gone_suspends_the_pane() {
        static ALIVE: AtomicBool = AtomicBool::new(true);
        fn lookup(pid: u32) -> crate::platform::ProcessStart {
            crate::platform::ProcessStart::table(
                (pid == 4242 && ALIVE.load(Ordering::Acquire)).then_some(7),
            )
        }
        let core = FakeCore::start();
        let (_pane, exits, _unreachable) = pane_on(connector_to(&core), short_policy(lookup));
        ALIVE.store(false, Ordering::Release);
        core.forget_terminal("t1");
        drop_stream(&core);
        assert_eq!(
            exits.recv_timeout(Duration::from_secs(5)).ok(),
            Some(ChildExitReason::Suspended(SuspendCause::TerminalGone))
        );
    }

    // V3 (D1a): core gave no pid, so the program may run: never gone.
    #[test]
    fn not_found_with_no_recorded_pid_is_unreachable() {
        let core = FakeCore::start_with(|config| config.pid = 0);
        let (pane, exits, unreachable) = pane_on(
            connector_to(&core),
            short_policy(|_| crate::platform::ProcessStart::Started(1)),
        );
        assert_eq!(pane.link().program, None);
        core.forget_terminal("t1");
        drop_stream(&core);
        assert!(unreachable
            .recv_timeout(Duration::from_secs(5))
            .ok()
            .flatten()
            .is_some());
        assert_eq!(exits.recv_timeout(NO_EXIT).ok(), None);
        pane.shutdown();
    }

    // V34 (D1a): with a real process standing in for the program, `not_found`
    // keeps the pane while it runs, and suspends it once it is gone.
    #[test]
    fn not_found_while_a_real_program_runs_waits_for_it_to_end() {
        let mut sleep = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let pid = sleep.id();
        let core = FakeCore::start_with(|config| config.pid = pid);
        let (pane, exits, unreachable) = pane_on(
            connector_to(&core),
            short_policy(crate::platform::process_start),
        );
        assert_eq!(pane.link().program.map(|program| program.pid), Some(pid));
        core.forget_terminal("t1");
        drop_stream(&core);
        assert!(unreachable
            .recv_timeout(Duration::from_secs(5))
            .ok()
            .flatten()
            .is_some());
        assert_eq!(
            exits.recv_timeout(NO_EXIT).ok(),
            None,
            "no suspension while it runs"
        );
        assert_eq!(
            core.methods()
                .iter()
                .filter(|method| *method == "terminal.run")
                .count(),
            1,
            "no second program"
        );

        sleep.kill().expect("kill sleep");
        sleep.wait().expect("reap sleep");
        assert_eq!(
            exits.recv_timeout(Duration::from_secs(5)).ok(),
            Some(ChildExitReason::Suspended(SuspendCause::TerminalGone))
        );
    }
}
