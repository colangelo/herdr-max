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

use super::address::CoreAddress;
use super::client::{
    stream_channel, CoreClient, CoreError, StreamEvent, StreamEvents, StreamSink, CREDIT_WINDOW,
};
use super::frame::{kind, Frame};
use super::status_mirror::StatusMirror;
use crate::core_machine::panes::MachineLink;
use crate::core_machine::MachineName;
use crate::layout::PaneId;
use crate::pane::runtime_io::{CoreProgramRun, CoreStatusRecord, CoreTerminalLink};
use crate::platform::{
    ChildExitReason, ProgramIdentity, ProgramLiveness, StartTokenLookup, SuspendCause,
};
use crate::pty::actor::{PtyReadResult, ReadCallback};
use crate::view::instance::FarInstance;

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

/// Where a pane's program runs (core-phase5 D5). Every pid- or
/// host-dependent call matches on it: a machine's pid is another host's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Host {
    Local,
    Machine(MachineName),
}

impl Host {
    fn of(address: &CoreAddress) -> Self {
        address.machine.clone().map_or(Self::Local, Self::Machine)
    }
}

/// The `herdr.session` value of a pane herdr runs on a machine:
/// `<label>@<core id>`, which no herdr build matches as its own (D5).
pub(crate) fn machine_owner_label(labels: &mut serde_json::Map<String, Value>, core_id: &str) {
    if let Some(Value::String(label)) = labels.get("herdr.session") {
        let owned = format!("{label}@{core_id}");
        labels.insert("herdr.session".into(), Value::String(owned));
    }
}

/// The record's address when it names a terminal on `on` (this Mac's own
/// core for `None`).
fn address_on<'a>(record: &'a Value, on: Option<&MachineName>) -> Option<&'a str> {
    let raw = record["address"].as_str()?;
    let address = CoreAddress::parse(raw).ok()?;
    (address.machine.as_ref() == on).then_some(raw)
}

/// Whether core lists `machine` as connected now. It only says which wait a
/// pane shows: core leaves the terminals of a machine that is not connected,
/// or not linked yet after a core restart, out of its answers (P5), so a
/// `not_found` is never final for a pane on a machine. A failed
/// `machine.list` is not connected.
fn machine_connected(client: &CoreClient, machine: &MachineName) -> bool {
    client
        .call("machine.list", json!({}), CALL_TIMEOUT)
        .is_ok_and(|listed| {
            listed["machines"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|record| record["name"] == machine.as_str() && record["state"] == "connected")
        })
}

/// What `terminal.run` needs for one pane.
pub(crate) struct CoreSpawn {
    pub pane_id: PaneId,
    /// The core session the terminal runs in, saved in the pane's link.
    pub session: String,
    /// The core machine it runs on; `None` for this Mac (core-phase5 D5).
    pub on: Option<MachineName>,
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
    /// The `herdr.launch` label of the run that started it, saved with the
    /// link: a pane on a machine attaches only to a terminal carrying it
    /// (security review M2).
    pub launch: Option<String>,
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
    /// A terminal this call just started on this Mac is killed if the
    /// attach fails; one on a machine never is (security review M5).
    ours_to_kill: bool,
    /// The run's `herdr.launch` label.
    launch: Option<String>,
    /// The `herdr.launch` label the program's terminal carries, on this
    /// Mac too: what a guarded pane action names (core-phase5 D7, #316).
    run_launch: Option<String>,
    /// For a pane on a machine, the labels its terminal must carry.
    identity: Option<serde_json::Map<String, Value>>,
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
    label_refusal(record, labels)
}

/// The first of `labels` the record does not carry, or `None`.
fn label_refusal(record: &Value, labels: &serde_json::Map<String, Value>) -> Option<String> {
    labels.iter().find_map(|(key, want)| {
        let have = &record["labels"][key];
        (have != want).then(|| format!("has label {key}={have}, not {want}"))
    })
}

/// The labels that show a terminal on a machine is this pane's program
/// (security review M2): the owner value, which names this core, and the
/// run's launch label, which no other run has. A far core whose state was
/// wiped can give the address to another terminal; that one carries
/// neither, and is never attached, taken as ended or killed. `None` when
/// the launch is unknown: nothing can show a terminal is the pane's then.
fn machine_identity(
    labels: &serde_json::Map<String, Value>,
    launch: Option<&str>,
) -> Option<serde_json::Map<String, Value>> {
    let mut identity = serde_json::Map::new();
    identity.insert(LAUNCH_LABEL.into(), Value::String(launch?.to_string()));
    if let Some(owner) = labels.get("herdr.session") {
        identity.insert("herdr.session".into(), owner.clone());
    }
    Some(identity)
}

/// Whether `record` shows the labels of `identity` (a pane on a machine)
/// or no identity is asked for (a pane on this Mac).
fn shows_identity(record: &Value, identity: Option<&serde_json::Map<String, Value>>) -> bool {
    identity.is_none_or(|identity| label_refusal(record, identity).is_none())
}

/// Whether core reports the record's program as ended: only a `finished`
/// record says so (hc 0.1.21: `state` is `live` or `finished`, and a
/// finished one has its `reason`, `exit_code` or `signal`). The only end
/// of a pane on a machine that herdr takes as final (core-phase5 D5).
fn finished(record: &Value) -> bool {
    record["state"] == "finished"
}

/// The exit a finished record reports; one with no known reason is an
/// interrupted program.
fn finished_exit(record: &Value) -> ChildExitReason {
    exit_reason(record).unwrap_or(ChildExitReason::Interrupted)
}

/// What a stream CLOSE says for a pane on `host` (core-phase5 D5). On this
/// Mac its reason is final as core gives it. On a machine a suspend cause
/// goes up as it is (the server makes the pane wait for its machine), and
/// any other end is final only on a finished record; a CLOSE without one
/// is asked about again (`None`).
fn close_exit(shared: &Shared, record: &Value) -> Option<ChildExitReason> {
    let exit = exit_reason(record)?;
    match shared.host {
        Host::Local => Some(exit),
        // A wait, never final: the server asks again with the labels.
        Host::Machine(_) if exit.suspend_cause().is_some() => Some(exit),
        // An end only for the pane's own terminal (security review M2).
        Host::Machine(_) if finished(record) && shared.shows_identity(record) => Some(exit),
        Host::Machine(_) => None,
    }
}

/// A pane on a machine reads its terminal (core-phase5 D5, positive
/// evidence): a live record with the pane's labels attaches, and a finished
/// one with them is the program's end. Nothing else is final: core leaves
/// out the terminals of a machine it has not linked again (P5), and may list
/// them before it has them all, so a `not_found`, a record labelled for
/// another pane or one in another state makes the pane wait and ask again.
/// Why a machine pane's terminal is not attached, as a wait: missing while
/// core has the machine connected, unreachable while it does not.
fn machine_missing(
    client: &CoreClient,
    machine: &MachineName,
    address: &str,
    why: &str,
) -> ReattachError {
    if machine_connected(client, machine) {
        ReattachError::Missing(format!("terminal {address} {why}"))
    } else {
        ReattachError::Unreachable(CoreError::Unavailable(format!(
            "{machine} is not connected; terminal {address} {why}"
        )))
    }
}

fn machine_record(
    client: &CoreClient,
    machine: &MachineName,
    address: &str,
    labels: &serde_json::Map<String, Value>,
) -> Result<Value, ReattachError> {
    let missing = |why: String| machine_missing(client, machine, address, &why);
    let record = match client.call("terminal.get", json!({"terminal": address}), CALL_TIMEOUT) {
        Ok(record) => record,
        Err(CoreError::Rpc { kind, .. }) if kind == "not_found" => {
            return Err(missing("is not listed by herdr-core".into()))
        }
        Err(err) => return Err(ReattachError::Unreachable(err)),
    };
    if let Some(refusal) = label_refusal(&record, labels) {
        return Err(missing(refusal));
    }
    if finished(&record) {
        let reason = record["reason"].as_str().unwrap_or("no reason given");
        return Err(ReattachError::Gone(format!(
            "terminal {address} has finished ({reason})"
        )));
    }
    if record["state"] != "live" {
        let state = record["state"].as_str().unwrap_or("unknown");
        return Err(missing(format!("is {state}")));
    }
    Ok(record)
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
    host: Host,
    /// The machine's link state, for a pane on a machine: input waits for
    /// it (core-phase5 D5).
    machine: Option<Arc<MachineLink>>,
    /// The run's `herdr.launch` label, saved with a machine pane's link.
    launch: Option<String>,
    /// The `herdr.launch` label the program's terminal carries (this
    /// Mac's panes too), `None` when core showed none: a guarded pane
    /// action is checked against it (core-phase5 D7, #316).
    run_launch: Option<String>,
    /// For a pane on a machine, the labels a terminal must show before it
    /// is attached, taken as ended or killed (security review M2).
    identity: Option<serde_json::Map<String, Value>>,
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
    /// Set for a view server's pane (core-phase5 D6): it never becomes the
    /// terminal's active viewer, never ends its program.
    view: Option<ViewIo>,
}

/// How a view's pane talks to its far terminal (core-phase5 D6 Size): the
/// attach is `resumed` at the far size, input goes as AUTOMATIC_INPUT only
/// (never DATA or FOCUS, either of which makes the view the active viewer
/// and resizes the far terminal), and a RESIZE goes only for a far size
/// different from the last one sent, so core records it without applying it
/// and no new size event follows (R7, R8).
struct ViewIo {
    /// Core's attach path does not take AUTOMATIC_INPUT: the pane attached
    /// `observe` and sends no input at all.
    read_only: bool,
    /// The size (rows, cols) last attached at or sent as RESIZE.
    sent: Mutex<(u16, u16)>,
    /// The far terminal the pane first attached to (security review M3):
    /// a reattach takes only that one, never another at its address.
    instance: FarInstance,
    /// Set when the terminal at the address is another one now (a far core
    /// reset gave the address to another program): the stream ends instead
    /// of attaching it, and the view binds the pane afresh, with its input
    /// held until a person's new click or focus.
    replaced: AtomicBool,
}

impl Shared {
    fn client(&self) -> CoreClient {
        lock(&self.client).clone()
    }

    fn shows_identity(&self, record: &Value) -> bool {
        shows_identity(record, self.identity.as_ref())
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
const LAUNCH_LABEL: &str = crate::view::instance::LAUNCH_LABEL;

/// The terminals of `listed` started for `launch` on `on` (this Mac's core
/// for `None`). A record of another host is left out even with the same
/// label: its pid is that host's (core-phase5 F1).
fn launched_on<'a>(listed: &'a Value, launch: &str, on: Option<&MachineName>) -> Vec<&'a Value> {
    listed["terminals"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|terminal| terminal["labels"][LAUNCH_LABEL] == launch)
        .filter(|terminal| address_on(terminal, on).is_some())
        .collect()
}

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

    /// No answer: core may have ended the program, or may still run it.
    pub(crate) fn maybe_ended(&self) -> bool {
        matches!(self, Self::Failed(_))
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
        if spawn.on.is_some() {
            let core_id = client.core_id().ok_or_else(|| {
                StartFailure::NotStarted(CoreError::Unavailable(
                    "herdr-core gave no id, which a pane on a machine is labelled with".into(),
                ))
            })?;
            machine_owner_label(&mut labels, core_id);
        }
        let identity = spawn
            .on
            .as_ref()
            .and_then(|_| machine_identity(&labels, Some(&launch)));
        let mut params = json!({
            "argv": spawn.argv,
            "cwd": spawn.cwd,
            "env": spawn.env,
            "labels": labels,
            "size": {"cols": spawn.cols, "rows": spawn.rows},
            "no_parent": true,
            "no_notify": true,
            "keep": false,
        });
        if let Some(on) = &spawn.on {
            params["on"] = Value::String(on.as_str().to_string());
        }
        let reply = client.call("terminal.run", params, CALL_TIMEOUT);
        let started = match reply {
            // A machine's run answers with its terminal's labels: an address
            // without the launch label is not this run's (security review M2),
            // and is neither attached nor killed.
            Ok(record) if !shows_identity(&record, identity.as_ref()) => {
                return Err(StartFailure::Unknown(CoreError::Rpc {
                    kind: "protocol".into(),
                    message: format!(
                        "terminal.run answered with a terminal not labelled for this launch: {}",
                        record["address"]
                    ),
                }))
            }
            Ok(record) => address_on(&record, spawn.on.as_ref())
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
            launch: spawn.on.as_ref().map(|_| launch.clone()),
            run_launch: Some(launch.clone()),
            identity: identity.clone(),
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
                let ours = launched_on(&listed, &launch, spawn.on.as_ref());
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
        let address = CoreAddress::parse(&target.address)
            .map_err(|err| ReattachError::NotOurs(format!("terminal {}: {err}", target.address)))?;
        let client = connector().map_err(ReattachError::Unreachable)?;
        // A pane on a machine carries the owner label (core-phase5 D5): the
        // far herdr's own pane with our plain labels is not ours (F1).
        let mut labels = target.labels.clone();
        let mut identity = None;
        if let Some(machine) = &address.machine {
            let core_id = client.core_id().ok_or_else(|| {
                ReattachError::Unreachable(CoreError::Unavailable(
                    "herdr-core gave no id, which a pane on a machine is labelled with".into(),
                ))
            })?;
            machine_owner_label(&mut labels, core_id);
            // Without its run's launch label nothing can show a terminal
            // there is this pane's: it waits, and nothing is attached,
            // taken as ended or killed (security review M2).
            let Some(launch) = target.launch.as_deref() else {
                return Err(machine_missing(
                    &client,
                    machine,
                    &target.address,
                    "was saved without its launch label",
                ));
            };
            labels.insert(LAUNCH_LABEL.into(), Value::String(launch.to_string()));
            identity = machine_identity(&labels, Some(launch));
        }
        let record = match &address.machine {
            Some(machine) => machine_record(&client, machine, &target.address, &labels)?,
            None => match client.call(
                "terminal.get",
                json!({"terminal": target.address}),
                CALL_TIMEOUT,
            ) {
                Ok(record) => match reattach_refusal(&record, &labels) {
                    None => record,
                    Some(refusal) => {
                        let message = format!("terminal {} {refusal}", target.address);
                        return Err(if record["state"] == "live" {
                            ReattachError::NotOurs(message)
                        } else {
                            ReattachError::Gone(message)
                        });
                    }
                },
                Err(CoreError::Rpc { kind, message }) if kind == "not_found" => {
                    return Err(ReattachError::Gone(message))
                }
                Err(err) => return Err(ReattachError::Unreachable(err)),
            },
        };
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
                launch: target.launch.clone().filter(|_| identity.is_some()),
                run_launch: record["labels"][LAUNCH_LABEL].as_str().map(str::to_string),
                identity,
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
            launch,
            run_launch,
            identity,
        } = attach;
        let host = match CoreAddress::parse(&address) {
            Ok(parsed) => Host::of(&parsed),
            Err(err) => {
                return Err(StartFailure::Unknown(CoreError::Rpc {
                    kind: "protocol".into(),
                    message: err.to_string(),
                }))
            }
        };
        // A machine's pid is a process of another host: no local fact, sweep
        // or status write may use it (core-phase5 D5, F1).
        let pid = match &host {
            Host::Local => pid,
            Host::Machine(machine) => {
                debug!(pane = pane_id.raw(), %machine, far_pid = pid, "pane on a core machine");
                0
            }
        };
        let machine = match &host {
            Host::Local => None,
            Host::Machine(name) => Some(crate::core_machine::panes::link(name)),
        };
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
            // The stream is to the terminal the answer names: on a machine,
            // it must show the pane's launch, or it is detached unused
            // (security review M2).
            Ok(attach) if !shows_identity(&attach["terminal"], identity.as_ref()) => {
                let channel = attach["channel"].as_u64().unwrap_or(0) as u32;
                let _ = client.send(&Frame::new(channel, kind::CLOSE, Vec::new()));
                client.forget_stream(channel);
                return Err(StartFailure::Unknown(CoreError::Rpc {
                    kind: "protocol".into(),
                    message: format!("terminal {address} is not labelled for this pane's launch"),
                }));
            }
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
                // core does not confirm ended may still run. A program on a
                // machine is never killed for a failed setup: only a
                // person's close ends it (security review M5).
                if matches!(host, Host::Machine(_)) {
                    return Err(StartFailure::Unknown(err));
                }
                if ours_to_kill && !kill_terminal(&client, &address).ended() {
                    return Err(StartFailure::Unknown(err));
                }
                return Err(StartFailure::NotStarted(err));
            }
        };
        // Core attached it (or answered that it finished): the machine is
        // connected now. A failed attach leaves the mark as it was.
        if let Some(link) = &machine {
            link.set_connected(true);
        }
        if pid > 0 {
            child_pid.store(pid, Ordering::Release);
            crate::logging::pane_spawned(pane_id.raw(), pid);
        }
        Ok(Self {
            shared: Arc::new(Shared {
                pane_id,
                session,
                address,
                host,
                machine,
                launch,
                run_launch,
                identity,
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
                view: None,
            }),
            events_rx,
            automatic_input,
        })
    }

    /// Attaches a view server's pane to a far terminal (core-phase5 D6
    /// Size) at the far size `(rows, cols)` read from `terminal.get` just
    /// before (`view_far_size`), `resumed`, so the attach takes no size;
    /// `observe` (read-only) when core's attach path does not take
    /// AUTOMATIC_INPUT. Nothing is started, labelled or killed: the far herdr
    /// owns the terminal.
    pub(crate) fn view(
        pane_id: PaneId,
        session: String,
        address: String,
        (rows, cols): (u16, u16),
        connector: Connector,
    ) -> Result<Self, StartFailure> {
        let parsed = CoreAddress::parse(&address).map_err(|err| {
            StartFailure::NotStarted(CoreError::Rpc {
                kind: "protocol".into(),
                message: err.to_string(),
            })
        })?;
        let Some(machine) = parsed.machine else {
            return Err(StartFailure::NotStarted(CoreError::Unavailable(format!(
                "{address} is not a terminal of a machine"
            ))));
        };
        let client = connector().map_err(StartFailure::NotStarted)?;
        let (events_tx, events_rx) = stream_channel();
        let attach = |observe: bool| {
            let mut params = json!({"terminal": address, "cols": cols, "rows": rows,
                "history": FIRST_ATTACH_HISTORY_LINES, "resumed": true});
            if observe {
                params["observe"] = Value::Bool(true);
            }
            client.attach(params, events_tx.clone(), CALL_TIMEOUT)
        };
        let first = attach(false).map_err(StartFailure::NotStarted)?;
        let mut channel = first["channel"].as_u64().unwrap_or(0) as u32;
        let read_only = !(client.automatic_input() && first["automatic_input"] == true);
        if read_only {
            // Typing as DATA would make the view the active viewer and
            // resize the far terminal (R7): watch only.
            let _ = client.send(&Frame::new(channel, kind::CLOSE, Vec::new()));
            client.forget_stream(channel);
            let observed = attach(true).map_err(StartFailure::NotStarted)?;
            channel = observed["channel"].as_u64().unwrap_or(0) as u32;
            warn!(pane = pane_id.raw(), %address, "herdr-core's path to the machine takes no automatic input; the view is read-only");
        }
        // The terminal this pane is bound to (review M3), as the attach
        // answer describes it, or as core lists it when that has no record.
        let instance = if first["terminal"].is_object() {
            FarInstance::of(&first["terminal"])
        } else {
            client
                .call("terminal.get", json!({"terminal": address}), CALL_TIMEOUT)
                .map(|record| FarInstance::of(&record))
                .unwrap_or_else(|_| FarInstance::unknown())
        };
        let link = crate::core_machine::panes::link(&machine);
        link.set_connected(true);
        debug!(pane = pane_id.raw(), %address, rows, cols, read_only, ?instance, "view attached");
        Ok(Self {
            shared: Arc::new(Shared {
                pane_id,
                session,
                address,
                host: Host::Machine(machine),
                machine: Some(link),
                // The far herdr labelled the terminal; the view checks none.
                launch: None,
                run_launch: None,
                identity: None,
                program: None,
                policy: ReattachPolicy::default(),
                connector,
                client: Mutex::new(client),
                channel: AtomicU32::new(channel),
                sink: events_tx,
                size: Mutex::new((rows, cols)),
                closing: AtomicBool::new(false),
                // The far herdr owns the program: nothing here ends it.
                keep_on_drop: AtomicBool::new(true),
                pid: 0,
                terminal: Mutex::new(None),
                mirror_slot: Mutex::new(None),
                view: Some(ViewIo {
                    read_only,
                    sent: Mutex::new((rows, cols)),
                    instance,
                    replaced: AtomicBool::new(false),
                }),
            }),
            events_rx,
            automatic_input: !read_only,
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
            abandon_setup(&shared);
            return Err(std::io::Error::other(err.to_string()));
        }
        Ok(CorePaneIo {
            shared,
            writer: Mutex::new(Some(writer)),
        })
    }
}

/// The pane's threads did not start: no pane shows its terminal. On this
/// Mac it is killed, as before; on a machine it is only detached, and its
/// program runs on there, since only a person's close ends one (security
/// review M5).
fn abandon_setup(shared: &Shared) {
    let client = shared.client();
    let channel = shared.channel.load(Ordering::Acquire);
    if shared.host == Host::Local {
        let _ = kill_terminal(&client, &shared.address);
    } else if channel != 0 {
        let _ = client.send(&Frame::new(channel, kind::CLOSE, Vec::new()));
        client.forget_stream(channel);
    }
}

impl CorePaneIo {
    fn queue(&self, cmd: WriteCmd) -> Result<(), mpsc::error::TrySendError<WriteCmd>> {
        match lock(&self.writer).as_ref() {
            Some(writer) => writer.try_send(cmd),
            None => Err(mpsc::error::TrySendError::Closed(cmd)),
        }
    }

    /// The machine this pane runs on, `None` on this Mac.
    pub(crate) fn machine(&self) -> Option<&MachineName> {
        match &self.shared.host {
            Host::Local => None,
            Host::Machine(machine) => Some(machine),
        }
    }

    /// Input to a pane whose machine is away is not sent: it would land on
    /// a screen nobody sees. It posts a wake instead (core-phase5 D5).
    fn machine_away(&self) -> bool {
        match &self.shared.machine {
            Some(link) if !link.connected() => {
                link.wake();
                true
            }
            _ => false,
        }
    }

    /// A view's pane: the far size its grid keeps (rows, cols), whatever
    /// the view's own pane rect is (core-phase5 D6).
    pub(crate) fn view_size(&self) -> Option<(u16, u16)> {
        self.shared.view.as_ref().map(|_| *lock(&self.shared.size))
    }

    /// A view's pane that sends no input (core's path takes no automatic
    /// input).
    pub(crate) fn view_read_only(&self) -> bool {
        self.shared.view.as_ref().is_some_and(|view| view.read_only)
    }

    /// A view's pane whose far address now holds another terminal than the
    /// one it attached to (review M3): its stream ended for that.
    pub(crate) fn view_replaced(&self) -> bool {
        self.shared
            .view
            .as_ref()
            .is_some_and(|view| view.replaced.load(Ordering::Acquire))
    }

    /// The far program a view's pane is bound to (security review M3),
    /// `None` for any other pane.
    pub(crate) fn view_instance(&self) -> Option<FarInstance> {
        self.shared.view.as_ref().map(|view| view.instance.clone())
    }

    /// Whether this is a view server's pane.
    pub(crate) fn is_view(&self) -> bool {
        self.shared.view.is_some()
    }

    pub(crate) fn try_send_bytes(
        &self,
        bytes: Bytes,
    ) -> Result<(), mpsc::error::TrySendError<Bytes>> {
        if self.view_read_only() {
            return Err(mpsc::error::TrySendError::Closed(bytes));
        }
        if self.machine_away() {
            return Err(mpsc::error::TrySendError::Closed(bytes));
        }
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
        if self.machine_away() {
            return Err(std::io::Error::other(format!(
                "{} is not connected; the input was not sent",
                self.machine().map_or("the machine", MachineName::as_str)
            )));
        }
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
        // A view's grid follows the far size, clamped to the pane minimum
        // here; that grid size is never the view's RESIZE (`follow_far_size`
        // sends the far size itself).
        if self.shared.view.is_some() {
            return;
        }
        *lock(&self.shared.size) = (rows, cols);
        let _ = self.queue(WriteCmd::Resize {
            rows,
            cols,
            cell: (cell_width_px, cell_height_px),
            replies,
        });
    }

    /// A view's pane takes the far terminal's new size (core-phase5 D6
    /// Size, R8): it is the size the grid keeps, and the view echoes it,
    /// exactly as core gave it, as RESIZE when it differs from the last one
    /// sent. False, and nothing changes, for any other pane or for a size
    /// over the view's grid budget (security review M4).
    pub(crate) fn follow_far_size(&self, rows: u16, cols: u16) -> bool {
        if self.shared.view.is_none() {
            return false;
        }
        if !crate::view::far_size_fits(rows, cols) {
            warn!(pane = self.shared.pane_id.raw(), terminal = %self.shared.address, rows, cols, "a far size over the view's budget; the pane keeps its last size");
            return false;
        }
        *lock(&self.shared.size) = (rows, cols);
        let _ = self.queue(WriteCmd::Resize {
            rows,
            cols,
            cell: (0, 0),
            replies: Vec::new(),
        });
        true
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
            launch: self.shared.launch.clone(),
            machine_cwd: None,
        }
    }

    /// The program's run as core showed it at attach: its terminal's
    /// `herdr.launch` label and its pid (core-phase5 D7, #316).
    pub(crate) fn run(&self) -> CoreProgramRun {
        CoreProgramRun {
            launch: self.shared.run_launch.clone(),
            pid: self.shared.pid,
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
    /// `terminate_session`) and says what core observed. On a machine, only
    /// a terminal core shows live with the pane's launch label is killed: a
    /// far core whose state was wiped can have given the address to another
    /// (security review M2). Core has no kill bound to a label (hc 0.1.21),
    /// so the check and the kill are two calls.
    pub(crate) fn kill(&self) -> KillOutcome {
        // A view's close is a detach: the far herdr owns the program (D6).
        if self.is_view() {
            return KillOutcome::Live;
        }
        let client = self.shared.client();
        if self.shared.identity.is_some() {
            let record = match client.call(
                "terminal.get",
                json!({"terminal": self.shared.address}),
                CALL_TIMEOUT,
            ) {
                Ok(record) => record,
                Err(err) => return KillOutcome::Failed(err),
            };
            if !self.shared.shows_identity(&record) {
                return KillOutcome::Failed(CoreError::Unavailable(format!(
                    "terminal {} is not shown to carry this pane's launch label; not killed",
                    self.shared.address
                )));
            }
            if finished(&record) {
                return KillOutcome::Finished;
            }
        }
        kill_terminal(&client, &self.shared.address)
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

/// A far terminal's size (rows, cols) for a view's pane, from
/// `terminal.get`; refused unless it is live.
pub(crate) fn view_far_size(connector: &Connector, address: &str) -> Result<(u16, u16), CoreError> {
    let record = connector()?.call("terminal.get", json!({"terminal": address}), CALL_TIMEOUT)?;
    if record["state"] != "live" {
        return Err(CoreError::Rpc {
            kind: "finished".into(),
            message: format!("{address} is not live"),
        });
    }
    let far = |key: &str| {
        record[key]
            .as_u64()
            .and_then(|value| u16::try_from(value).ok())
            .filter(|value| *value > 0)
    };
    let (rows, cols) = (far("rows").unwrap_or(24), far("cols").unwrap_or(80));
    // Checked before any grid is built at it (security review M4).
    if !crate::view::far_size_fits(rows, cols) {
        return Err(CoreError::Rpc {
            kind: "too_large".into(),
            message: format!(
                "{address} is {cols}x{rows}, larger than a view shows ({}x{} and {} cells at most)",
                crate::view::MAX_FAR_COLS,
                crate::view::MAX_FAR_ROWS,
                crate::view::MAX_FAR_CELLS
            ),
        });
    }
    Ok((rows, cols))
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
            if let Some(view) = &shared.view {
                return view_writer(&shared, view, rx);
            }
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

/// A view's writer (core-phase5 D6 Size): keys, pastes and mouse reports
/// go as AUTOMATIC_INPUT, which core delivers without making the view the
/// active viewer; a read-only view sends none. A view sends no terminal
/// reply at all (DA, DSR, `?u`, OSC 10/11, XTVERSION, mode 2031/2048, a
/// resize's report): the far herdr answers its own programs, and a reply
/// from here would be input no person typed, a second answer, or one to a
/// query replayed from history. A resize carries the far size (the grid
/// keeps it) and goes as RESIZE only when it differs from the last one sent,
/// so core records it and emits no event; no FACTS frame, no FOCUS frame,
/// no DATA ever.
fn view_writer(shared: &Shared, view: &ViewIo, mut rx: mpsc::Receiver<WriteCmd>) {
    // Input typed for a far terminal goes to no other one (review M3): once
    // the address holds another program, nothing is sent.
    let replaced = || view.replaced.load(Ordering::Acquire);
    let typed = |bytes: &Bytes| {
        if !view.read_only && !replaced() {
            let _ = shared.send(kind::AUTOMATIC_INPUT, bytes.to_vec());
        }
    };
    while let Some(cmd) = rx.blocking_recv() {
        match cmd {
            WriteCmd::User(bytes) => typed(&bytes),
            WriteCmd::Reply(_) => {}
            WriteCmd::Submit {
                text,
                enter,
                delay,
                reply,
            } => {
                let result = if view.read_only {
                    Err(std::io::Error::other(
                        "this view is read-only: herdr-core's path takes no automatic input",
                    ))
                } else if replaced() {
                    Err(std::io::Error::other(
                        "the far terminal at this address is another program now",
                    ))
                } else {
                    shared
                        .send(kind::AUTOMATIC_INPUT, text.to_vec())
                        .and_then(|()| {
                            std::thread::sleep(delay);
                            shared.send(kind::AUTOMATIC_INPUT, enter.to_vec())
                        })
                        .map_err(std::io::Error::other)
                };
                let _ = reply.send(result);
            }
            WriteCmd::Resize {
                rows,
                cols,
                cell: _,
                replies: _,
            } => {
                let changed = {
                    let mut sent = lock(&view.sent);
                    std::mem::replace(&mut *sent, (rows, cols)) != (rows, cols)
                };
                if changed {
                    let channel = shared.channel.load(Ordering::Acquire);
                    let _ = shared.client().send(&Frame::resize(channel, cols, rows));
                }
            }
        }
    }
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
            // A suspend cause for a pane on a machine goes up as it is: the
            // server makes that pane wait for its machine, never starts it
            // again here (core-phase5 D5). A stop of this Mac's core says
            // nothing about the program there.
            let mut finish = |reason: ChildExitReason, ended_here: bool| {
                // Only this Mac's own terminals say its core was stopped: a
                // machine's `server_stop` is the far core's (security
                // review M3).
                if reason == ChildExitReason::Suspended(SuspendCause::CoreStopped)
                    && shared.host == Host::Local
                {
                    super::note_stopped_by_person(&shared.session);
                }
                // A machine pane whose program core reports finished has
                // nothing to kill, and a kill by its address could reach a
                // terminal that took it over. After a suspend cause its
                // program may run on: the server keeps it for the wait it
                // starts, so a person's close racing this still ends it (D5).
                if ended_here && matches!(shared.host, Host::Machine(_)) {
                    shared.keep_on_drop.store(true, Ordering::Release);
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
                        // A view answers no query (see `view_writer`): its
                        // replies are not even queued, so a history replayed
                        // at attach cannot fill the queue ahead of a key.
                        if shared.view.is_none() {
                            for reply in terminal_responses {
                                let _ = writer.try_send(WriteCmd::Reply(reply));
                            }
                        }
                        let channel = shared.channel.load(Ordering::Acquire);
                        if let Some(bytes) = credit.consumed(channel, consumed) {
                            let _ = shared.client().send(&Frame::credit(channel, bytes));
                        }
                    }
                    StreamEvent::Close(reason) => {
                        // A view's pane ends only when its far program has
                        // (D6); anything else is a stream to attach again.
                        let exit = if shared.view.is_some() {
                            finished(&reason).then(|| finished_exit(&reason))
                        } else {
                            close_exit(&shared, &reason)
                        };
                        if let Some(exit) = exit {
                            finish(exit, finished(&reason));
                            return;
                        }
                        if shared.closing.load(Ordering::Acquire) {
                            return;
                        }
                        if let Some(exit) = reattach(&shared, &mut on_unreachable) {
                            finish(exit, true);
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
                            finish(exit, true);
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
    let Some(write) = mirror.take_due(Instant::now()) else {
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
    match terminal.write(&write.encode()) {
        Ok(()) => mirror.written(write),
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
            mirror.busy(write, Instant::now());
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
    /// A pane on a machine (core-phase5 D5): core, with the machine
    /// connected, does not list its terminal, or lists it for another pane
    /// or in no known state. Never final: the pane waits and asks again.
    Missing(String),
    /// Core did not answer.
    Unreachable(CoreError),
}

impl std::fmt::Display for ReattachError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Gone(message) | Self::NotOurs(message) | Self::Missing(message) => {
                f.write_str(message)
            }
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
    if let Host::Machine(machine) = &shared.host {
        return machine_reattach_check(shared, &client, machine)
            .unwrap_or_else(|| attach_again(shared, &client));
    }
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
    attach_again(shared, &client)
}

/// For a pane on a machine (core-phase5 D5, positive evidence): `None` when
/// core has its terminal live, so the stream attaches again; `Gone` only for
/// a finished record, whose suspend cause goes up as it is (the server makes
/// the pane wait, then finds the record finished); anything else waits.
fn machine_reattach_check(
    shared: &Shared,
    client: &CoreClient,
    machine: &MachineName,
) -> Option<ReattachOutcome> {
    match client.call(
        "terminal.get",
        json!({"terminal": shared.address}),
        CALL_TIMEOUT,
    ) {
        // A view takes only the far terminal it first attached to: another
        // at the address ends this stream, and the view binds the pane
        // afresh with its input held (security review M3).
        Ok(record) if record["state"] == "live" && view_replaced(shared, &record) => Some(
            ReattachOutcome::Gone(ChildExitReason::Suspended(SuspendCause::TerminalGone)),
        ),
        // Only the pane's own terminal, shown by its launch label, is
        // attached or ended; any other at the address is waited out
        // (security review M2, third read 1).
        Ok(record) if !shared.shows_identity(&record) => {
            debug!(
                pane = shared.pane_id.raw(),
                %machine,
                terminal = %shared.address,
                "the terminal at the pane's address is not labelled for its launch; waiting"
            );
            Some(ReattachOutcome::Unreachable)
        }
        Ok(record) if record["state"] == "live" => None,
        Ok(record) if finished(&record) => Some(ReattachOutcome::Gone(finished_exit(&record))),
        answer => {
            debug!(
                pane = shared.pane_id.raw(),
                %machine,
                terminal = %shared.address,
                answer = ?answer.err(),
                "herdr-core does not have the pane's terminal live on its machine; waiting for it"
            );
            Some(ReattachOutcome::Unreachable)
        }
    }
}

/// For a view's pane: whether `record` is another terminal than the one it
/// first attached to; if so, marks the pane replaced (review M3).
fn view_replaced(shared: &Shared, record: &Value) -> bool {
    let Some(view) = &shared.view else {
        return false;
    };
    if view.instance.matches(record) {
        return false;
    }
    warn!(
        pane = shared.pane_id.raw(),
        terminal = %shared.address,
        "the far terminal at this address is another program now; the view binds it afresh"
    );
    view.replaced.store(true, Ordering::Release);
    true
}

/// Attaches the pane's stream again, after core said its terminal is live.
fn attach_again(shared: &Shared, client: &CoreClient) -> ReattachOutcome {
    let (rows, cols) = *lock(&shared.size);
    let mut params =
        json!({"terminal": shared.address, "cols": cols, "rows": rows, "resumed": true});
    if shared.view.as_ref().is_some_and(|view| view.read_only) {
        params["observe"] = Value::Bool(true);
    }
    match client.attach(params, shared.sink.clone(), CALL_TIMEOUT) {
        Ok(attach) if !shared.shows_identity(&attach["terminal"]) => {
            let channel = attach["channel"].as_u64().unwrap_or(0) as u32;
            let _ = client.send(&Frame::new(channel, kind::CLOSE, Vec::new()));
            client.forget_stream(channel);
            debug!(pane = shared.pane_id.raw(), terminal = %shared.address, "the attached terminal is not labelled for the pane's launch; detached, retrying");
            ReattachOutcome::Unreachable
        }
        Ok(attach)
            if attach["terminal"].is_object() && view_replaced(shared, &attach["terminal"]) =>
        {
            let channel = attach["channel"].as_u64().unwrap_or(0) as u32;
            let _ = client.send(&Frame::new(channel, kind::CLOSE, Vec::new()));
            client.forget_stream(channel);
            ReattachOutcome::Gone(ChildExitReason::Suspended(SuspendCause::TerminalGone))
        }
        Ok(attach) => {
            let channel = attach["channel"].as_u64().unwrap_or(0) as u32;
            shared.channel.store(channel, Ordering::Release);
            if let Some(view) = &shared.view {
                *lock(&view.sent) = (rows, cols);
            }
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
            launch: None,
            rows: 24,
            cols: 80,
        }
    }

    /// A saved link to `address` on a machine, for the run `launch`.
    fn machine_target(address: &str, pane: &str, launch: &str) -> CoreReattach {
        CoreReattach {
            launch: Some(launch.into()),
            ..target(address, pane)
        }
    }

    /// The labels the fake core's pane `pane` on a machine carries, for the
    /// run `launch`: the owner value, the pane and the launch.
    fn owned(pane: &str, launch: &str) -> serde_json::Map<String, Value> {
        let mut labels = labels(pane);
        machine_owner_label(&mut labels, "fake");
        labels.insert(LAUNCH_LABEL.into(), json!(launch));
        labels
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
            on: None,
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
            on: None,
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
            on: None,
            argv: vec!["sh".into()],
            cwd: None,
            env: serde_json::Map::new(),
            labels: labels(pane),
            rows: 24,
            cols: 80,
        }
    }

    // V26 (core-phase5 F1): a terminal of another host with our launch
    // label is not this launch, and its pid is never taken for the pane's.
    #[test]
    fn the_launch_recovery_keeps_the_launchs_own_host() {
        let listed = json!({"terminals": [
            {"address": "loop/t1", "pid": 4321, "state": "live", "labels": {LAUNCH_LABEL: "abc"}},
            {"address": "t1", "pid": 11, "state": "live", "labels": {LAUNCH_LABEL: "abc"}},
            {"address": "t2", "pid": 12, "state": "live", "labels": {LAUNCH_LABEL: "other"}},
        ]});
        let here = launched_on(&listed, "abc", None);
        assert_eq!(here.len(), 1);
        assert_eq!(here[0]["address"], "t1");
        let machine = MachineName::parse("loop").expect("name");
        let there = launched_on(&listed, "abc", Some(&machine));
        assert_eq!(there.len(), 1);
        assert_eq!(there[0]["address"], "loop/t1");
        let only_far = json!({"terminals": [
            {"address": "loop/t1", "pid": 4321, "state": "live", "labels": {LAUNCH_LABEL: "abc"}},
        ]});
        assert!(launched_on(&only_far, "abc", None).is_empty());
    }

    // V26, V10's other side: the far herdr's own pane carries our plain
    // labels, not the owner value, so it is never attached or killed; and
    // it is not final for our pane either, which waits (N2).
    #[test]
    fn a_reattach_refuses_a_machines_terminal_without_the_owner_label() {
        let core = FakeCore::start();
        core.add_terminal(json!({"address": "loop/t1", "pid": 777, "state": "live",
                                  "labels": labels("w1-p2")}));
        core.set_machine("loop", "connected");
        let pid = AtomicU32::new(0);
        let refused = StartedCore::reattach(
            machine_target("loop/t1", "w1-p2", "L1"),
            &pid,
            connector_to(&core),
        );
        assert!(
            matches!(refused, Err(ReattachError::Missing(_))),
            "{:?}",
            refused.err()
        );
        assert_eq!(pid.load(Ordering::Acquire), 0, "no far pid");
        assert_eq!(
            core.methods(),
            ["terminal.get", "machine.list"],
            "never attached, never killed"
        );
    }

    fn machine_spawn(pane: &str) -> CoreSpawn {
        CoreSpawn {
            on: Some(MachineName::parse("loop").expect("name")),
            ..spawn_for(pane)
        }
    }

    fn ignore_pane(started: StartedCore) -> CorePaneIo {
        started
            .into_pane(
                Box::new(|_: &[u8]| PtyReadResult {
                    terminal_responses: Vec::new(),
                }),
                Box::new(|_| {}),
                Box::new(|_| {}),
                Arc::new(AtomicBool::new(false)),
            )
            .expect("pane")
    }

    // V9 (D5): a pane on a machine runs there under the owner label, and
    // no pid fact of this Mac is taken from it.
    #[test]
    fn a_machine_pane_runs_on_the_machine_under_the_owner_label_with_no_pid() {
        let core = FakeCore::start();
        let pid = AtomicU32::new(0);
        let pane = ignore_pane(
            StartedCore::start(machine_spawn("w1-p6"), &pid, connector_to(&core)).expect("start"),
        );
        let run = core
            .requests()
            .into_iter()
            .find(|request| request["method"] == "terminal.run")
            .expect("run");
        assert_eq!(run["params"]["on"], "loop");
        let session = labels("w1-p6")["herdr.session"].clone();
        assert_eq!(
            run["params"]["labels"]["herdr.session"],
            format!("{}@fake", session.as_str().expect("label"))
        );
        assert_eq!(run["params"]["labels"]["herdr.pane_id"], "w1-p6");
        assert_eq!(
            pid.load(Ordering::Acquire),
            0,
            "the far pid is not published"
        );
        assert_eq!(pane.machine().map(MachineName::as_str), Some("loop"));
        assert_eq!(pane.link().address, "loop/t1");
        assert_eq!(pane.link().program, None, "no local program identity");
        pane.shutdown();
    }

    // V12 (D5): close of a pane on a machine is `terminal.kill` only: no
    // `server.status` for a sweep guard, no local signal.
    #[test]
    fn closing_a_machine_pane_is_a_kill_only() {
        let core = FakeCore::start();
        let pid = AtomicU32::new(0);
        let pane = ignore_pane(
            StartedCore::start(machine_spawn("w1-p6"), &pid, connector_to(&core)).expect("start"),
        );
        pane.shutdown();
        let before = core.methods().len();
        super::super::terminate_session(&pane, PaneId::from_raw(6), 0, None);
        assert_eq!(
            core.methods()[before..],
            ["terminal.get", "terminal.kill"],
            "the launch label is checked, then the terminal killed"
        );
        let kill = core.requests().pop().expect("kill");
        assert_eq!(kill["params"]["terminal"], "loop/t1");
    }

    // D5: a pane on a machine attaches again by its address when its
    // terminal carries the owner label.
    #[test]
    fn a_machine_pane_reattaches_by_address_with_the_owner_label() {
        let core = FakeCore::start();
        core.add_terminal(json!({"address": "loop/t3", "pid": 777, "state": "live",
                                  "labels": owned("w1-p2", "L1")}));
        let pid = AtomicU32::new(0);
        let pane = ignore_pane(
            StartedCore::reattach(
                machine_target("loop/t3", "w1-p2", "L1"),
                &pid,
                connector_to(&core),
            )
            .expect("attached"),
        );
        assert_eq!(pane.link().launch.as_deref(), Some("L1"), "saved again");
        assert_eq!(pid.load(Ordering::Acquire), 0, "no far pid");
        assert_eq!(pane.machine().map(MachineName::as_str), Some("loop"));
        assert_eq!(core.methods(), ["terminal.get", "terminal.attach"]);
        pane.shutdown();
    }

    // N2 (D5, P5, positive evidence): core leaves out the terminals of a
    // machine it has not linked (yet), and may list the machine connected
    // before it has them, so `not_found` is never final: the pane waits,
    // its terminal missing while the machine is connected. Only a finished
    // record with the pane's labels ends it.
    #[test]
    fn only_a_finished_record_ends_a_machine_pane_at_reattach() {
        let core = FakeCore::start();
        core.forget_terminal("loop/t3");
        let pid = AtomicU32::new(0);
        let answer = |core: &FakeCore| {
            StartedCore::reattach(
                machine_target("loop/t3", "w1-p2", "L1"),
                &pid,
                connector_to(core),
            )
            .err()
            .map(|err| format!("{err:?}"))
            .unwrap_or_default()
        };
        for state in [None, Some("connecting"), Some("unreachable")] {
            if let Some(state) = state {
                core.set_machine("loop", state);
            }
            assert!(answer(&core).starts_with("Unreachable"), "{state:?}");
        }
        core.set_machine("loop", "connected");
        assert!(
            answer(&core).starts_with("Missing"),
            "connected, not listed"
        );
        core.add_terminal(json!({"address": "loop/t3", "state": "finished",
                                  "reason": "exited", "labels": labels("w1-p2")}));
        assert!(
            answer(&core).starts_with("Missing"),
            "a finished record of another pane is not ours"
        );
        core.add_terminal(json!({"address": "loop/t3", "state": "finished",
                                  "reason": "exited", "labels": owned("w1-p2", "L0")}));
        assert!(
            answer(&core).starts_with("Missing"),
            "a finished record of another launch at the address is not ours"
        );
        core.add_terminal(json!({"address": "loop/t3", "state": "finished",
                                  "reason": "exited", "labels": owned("w1-p2", "L1")}));
        assert!(answer(&core).starts_with("Gone"), "finished: its end");
        assert!(
            !core.methods().iter().any(|method| {
                method == "terminal.attach" || method == "terminal.run" || method == "terminal.kill"
            }),
            "{:?}",
            core.methods()
        );
    }

    /// A pane on machine `on`, started on `core`, with its exits.
    fn machine_pane_on(
        core: &FakeCore,
        on: &str,
    ) -> (CorePaneIo, std_mpsc::Receiver<ChildExitReason>) {
        let (exit_tx, exit_rx) = std_mpsc::channel();
        let mut spawn = machine_spawn("w1-p6");
        spawn.on = Some(MachineName::parse(on).expect("name"));
        let pid = AtomicU32::new(0);
        let pane = StartedCore::start_with(
            spawn,
            &pid,
            connector_to(core),
            short_policy(|_| crate::platform::ProcessStart::table(None)),
        )
        .expect("start")
        .into_pane(
            Box::new(|_: &[u8]| PtyReadResult {
                terminal_responses: Vec::new(),
            }),
            Box::new(move |reason| {
                let _ = exit_tx.send(reason);
            }),
            Box::new(|_| {}),
            Arc::new(AtomicBool::new(false)),
        )
        .expect("pane");
        (pane, exit_rx)
    }

    // C6 (D5): a stop of this Mac's core ends the pane's stream with a
    // suspend cause; for a pane on a machine it goes up as it is, so the
    // server makes the pane wait for its machine instead of ending it.
    #[test]
    fn a_suspend_cause_on_a_machine_pane_stream_goes_up_as_it_is() {
        let core = FakeCore::start();
        let (pane, exits) = machine_pane_on(&core, "loop");
        core.close_stream(1, br#"{"reason": "server_stop"}"#);
        assert_eq!(
            exits.recv_timeout(Duration::from_secs(5)).ok(),
            Some(ChildExitReason::Suspended(SuspendCause::CoreStopped))
        );
        pane.shutdown();
    }

    // N2 (D5, P5): a live pane on a machine whose stream dropped and whose
    // terminal core does not list keeps retrying, with the machine away or
    // connected; it ends only once core reports its record finished, and
    // then nothing is sent to kill it.
    #[test]
    fn a_dropped_machine_pane_ends_only_on_a_finished_record() {
        let core = FakeCore::start();
        let (pane, exits) = machine_pane_on(&core, "loop");
        core.forget_terminal("loop/t1");
        core.set_machine("loop", "connecting");
        drop_stream(&core);
        assert_eq!(exits.recv_timeout(NO_EXIT).ok(), None, "waits for loop");
        core.set_machine("loop", "connected");
        assert_eq!(
            exits.recv_timeout(NO_EXIT).ok(),
            None,
            "connected and not listed: still not final"
        );
        let launch = pane.link().launch.expect("a machine pane saves its launch");
        core.add_terminal(json!({"address": "loop/t1", "state": "finished",
                                  "reason": "exited", "exit_code": 0,
                                  "labels": owned("w1-p6", &launch)}));
        assert_eq!(
            exits.recv_timeout(Duration::from_secs(5)).ok(),
            Some(ChildExitReason::Exited),
            "finished: it ends, and is never started again"
        );
        pane.shutdown();
        super::super::terminate_session(&pane, PaneId::from_raw(6), 0, None);
        assert!(
            !core
                .methods()
                .iter()
                .any(|method| method == "terminal.kill"),
            "{:?}",
            core.methods()
        );
    }

    // N2 (D5): a machine pane's program that exits closes its stream with
    // the finished record: the pane ends, and its runtime's drop sends no
    // `terminal.kill`, which only a person's close sends.
    #[test]
    fn a_finished_machine_pane_is_never_killed_after_its_end() {
        let core = FakeCore::start();
        let (pane, exits) = machine_pane_on(&core, "loop");
        let launch = pane.link().launch.expect("launch");
        let record = json!({"address": "loop/t1", "state": "finished", "reason": "exited",
                            "exit_code": 0, "labels": owned("w1-p6", &launch)});
        core.close_stream(1, record.to_string().as_bytes());
        assert_eq!(
            exits.recv_timeout(Duration::from_secs(5)).ok(),
            Some(ChildExitReason::Exited)
        );
        assert!(pane.kept_on_drop());
        pane.shutdown();
        super::super::terminate_session(&pane, PaneId::from_raw(6), 0, None);
        assert!(
            !core
                .methods()
                .iter()
                .any(|method| method == "terminal.kill"),
            "{:?}",
            core.methods()
        );
    }

    // N2 (D5): a CLOSE that names an exit without a finished record is not
    // final for a machine pane: it asks core, which has the terminal live,
    // and attaches again.
    #[test]
    fn a_machine_pane_close_without_a_finished_record_asks_again() {
        let core = FakeCore::start();
        let (pane, exits) = machine_pane_on(&core, "loop");
        core.close_stream(1, br#"{"reason": "exited"}"#);
        assert_eq!(exits.recv_timeout(NO_EXIT).ok(), None, "not ended");
        let attaches = core
            .methods()
            .iter()
            .filter(|method| *method == "terminal.attach")
            .count();
        assert_eq!(attaches, 2, "attached again: {:?}", core.methods());
        pane.shutdown();
    }

    // Security review M2 (Sol C-03): a far core whose state was wiped can
    // give a saved address to another terminal, even one with the pane's
    // owner and pane labels. Without the run's launch label it is never
    // attached; a link saved without a launch attaches to nothing.
    #[test]
    fn a_restored_machine_pane_attaches_only_to_its_launch() {
        let core = FakeCore::start();
        core.set_machine("loop", "connected");
        core.add_terminal(json!({"address": "loop/t3", "pid": 777, "state": "live",
                                  "labels": owned("w1-p2", "other-run")}));
        let pid = AtomicU32::new(0);
        for (case, target) in [
            (
                "another run at the address",
                machine_target("loop/t3", "w1-p2", "L1"),
            ),
            ("no launch saved", target("loop/t3", "w1-p2")),
        ] {
            let refused = StartedCore::reattach(target, &pid, connector_to(&core));
            assert!(
                matches!(refused, Err(ReattachError::Missing(_))),
                "{case}: {:?}",
                refused.err()
            );
        }
        assert!(
            !core
                .methods()
                .iter()
                .any(|method| { method == "terminal.attach" || method == "terminal.kill" }),
            "{:?}",
            core.methods()
        );
        core.add_terminal(json!({"address": "loop/t3", "pid": 777, "state": "live",
                                  "labels": owned("w1-p2", "L1")}));
        let pane = ignore_pane(
            StartedCore::reattach(
                machine_target("loop/t3", "w1-p2", "L1"),
                &pid,
                connector_to(&core),
            )
            .expect("its own run attaches"),
        );
        pane.shutdown();
    }

    // Security review M2 and the third read's finding 1: a live machine
    // pane whose address core gave to another run (here with the pane's
    // own owner and pane labels) does not attach to it again, does not take
    // its finished record for its end, and a person's close does not kill
    // it. Its own run's finished record still ends it.
    #[test]
    fn a_live_machine_pane_never_takes_another_run_at_its_address() {
        let core = FakeCore::start();
        core.set_machine("loop", "connected");
        let (pane, exits) = machine_pane_on(&core, "loop");
        let launch = pane.link().launch.expect("launch");
        let attaches = |core: &FakeCore| {
            core.methods()
                .iter()
                .filter(|method| *method == "terminal.attach")
                .count()
        };
        core.add_terminal(json!({"address": "loop/t1", "state": "live",
                                  "labels": owned("w1-p6", "other-run")}));
        drop_stream(&core);
        assert_eq!(exits.recv_timeout(NO_EXIT).ok(), None, "waits");
        assert_eq!(attaches(&core), 1, "not attached again");
        let other_end = json!({"address": "loop/t1", "state": "finished",
                               "reason": "exited", "exit_code": 0,
                               "labels": owned("w1-p6", "other-run")});
        core.add_terminal(other_end);
        assert_eq!(exits.recv_timeout(NO_EXIT).ok(), None, "not its end");
        core.add_terminal(json!({"address": "loop/t1", "state": "live",
                                  "labels": owned("w1-p6", "other-run")}));
        pane.shutdown();
        super::super::terminate_session(&pane, PaneId::from_raw(6), 0, None);
        assert!(
            !core
                .methods()
                .iter()
                .any(|method| method == "terminal.kill"),
            "{:?}",
            core.methods()
        );
        assert!(!pane.kill().ended(), "a kill says the program may run on");

        let (pane, exits) = machine_pane_on(&core, "loop");
        let launch_two = pane.link().launch.expect("launch");
        assert_ne!(launch, launch_two, "every run has its own launch");
        core.add_terminal(json!({"address": "loop/t1", "state": "finished",
                                  "reason": "exited", "exit_code": 0,
                                  "labels": owned("w1-p6", &launch_two)}));
        drop_stream(&core);
        assert_eq!(
            exits.recv_timeout(Duration::from_secs(5)).ok(),
            Some(ChildExitReason::Exited)
        );
        pane.shutdown();
    }

    // Security review M2: a stream CLOSE whose finished record does not
    // show the pane's launch is not its end: the pane asks core again.
    #[test]
    fn a_machine_close_without_the_launch_label_is_not_an_end() {
        let core = FakeCore::start();
        let (pane, exits) = machine_pane_on(&core, "loop");
        core.close_stream(
            1,
            br#"{"address": "loop/t1", "state": "finished", "reason": "exited", "exit_code": 0}"#,
        );
        assert_eq!(exits.recv_timeout(NO_EXIT).ok(), None, "not ended");
        pane.shutdown();
    }

    // Security review M3 (Sol C-04): a machine stream's `server_stop` is
    // the far core's stop, not this Mac's: the primary is not marked as
    // stopped by a person. This Mac's own terminal still marks it.
    #[test]
    fn a_machine_streams_server_stop_is_not_this_macs_core_stop() {
        let core = FakeCore::start();
        let (exit_tx, exits) = std_mpsc::channel();
        let spawn = CoreSpawn {
            session: super::super::core_session_name(),
            ..machine_spawn("w1-p6")
        };
        let pid = AtomicU32::new(0);
        let pane = StartedCore::start(spawn, &pid, connector_to(&core))
            .expect("start")
            .into_pane(
                Box::new(|_: &[u8]| PtyReadResult {
                    terminal_responses: Vec::new(),
                }),
                Box::new(move |reason| {
                    let _ = exit_tx.send(reason);
                }),
                Box::new(|_| {}),
                Arc::new(AtomicBool::new(false)),
            )
            .expect("pane");
        core.close_stream(1, br#"{"reason": "server_stop"}"#);
        assert_eq!(
            exits.recv_timeout(Duration::from_secs(5)).ok(),
            Some(ChildExitReason::Suspended(SuspendCause::CoreStopped))
        );
        assert!(!super::super::sessions::primary_stopped_by_person());
        pane.shutdown();
    }

    // Third read, finding 6: after a suspend cause the server keeps the
    // program for the wait it starts; the stream end itself keeps nothing,
    // so a person's close that races it still ends the program there.
    #[test]
    fn a_close_racing_a_suspend_cause_still_ends_the_machine_program() {
        let core = FakeCore::start();
        let (pane, exits) = machine_pane_on(&core, "loop");
        core.close_stream(1, br#"{"reason": "server_stop"}"#);
        assert!(exits.recv_timeout(Duration::from_secs(5)).is_ok());
        assert!(!pane.kept_on_drop());
        pane.shutdown();
        super::super::terminate_session(&pane, PaneId::from_raw(6), 0, None);
        assert_eq!(
            core.methods()
                .iter()
                .filter(|method| *method == "terminal.kill")
                .count(),
            1,
            "{:?}",
            core.methods()
        );
    }

    // Security review M5 (Sol C-07): a setup failure of a machine pane
    // kills nothing: a failed attach, or threads that did not start, leave
    // its program running there, detached.
    #[test]
    fn a_machine_pane_setup_failure_kills_nothing() {
        let failing = FakeCore::start_with(|config| config.failing = vec!["terminal.attach"]);
        let pid = AtomicU32::new(0);
        assert!(matches!(
            StartedCore::start(machine_spawn("w1-p6"), &pid, connector_to(&failing)),
            Err(StartFailure::Unknown(_))
        ));
        assert!(
            !failing
                .methods()
                .iter()
                .any(|method| method == "terminal.kill"),
            "{:?}",
            failing.methods()
        );

        let core = FakeCore::start();
        let started =
            StartedCore::start(machine_spawn("w1-p6"), &pid, connector_to(&core)).expect("start");
        abandon_setup(&started.shared);
        assert!(
            !core
                .methods()
                .iter()
                .any(|method| method == "terminal.kill"),
            "{:?}",
            core.methods()
        );
        let local =
            StartedCore::start(spawn_for("w1-p6"), &pid, connector_to(&core)).expect("start here");
        abandon_setup(&local.shared);
        assert_eq!(
            core.methods().last().map(String::as_str),
            Some("terminal.kill"),
            "a program on this Mac is killed as before"
        );
    }

    // C10 (D5): the machine is marked connected only once core attached the
    // pane; a machine's first link starts away, and a failed attach leaves
    // it so.
    #[test]
    fn a_failed_machine_attach_leaves_the_machine_marked_away() {
        let core = FakeCore::start_with(|config| config.failing = vec!["terminal.attach"]);
        let machine = MachineName::parse("attach-fails").expect("name");
        let link = crate::core_machine::panes::link(&machine);
        let mut spawn = machine_spawn("w1-p6");
        spawn.on = Some(machine);
        let pid = AtomicU32::new(0);
        assert!(StartedCore::start(spawn, &pid, connector_to(&core)).is_err());
        assert!(!link.connected());
    }

    // C.3 (D5): input to a pane whose machine is away is not sent.
    #[test]
    fn input_to_a_pane_whose_machine_is_away_is_not_sent() {
        let core = FakeCore::start();
        let pid = AtomicU32::new(0);
        let mut spawn = machine_spawn("w1-p6");
        spawn.on = Some(MachineName::parse("away-test").expect("name"));
        let pane =
            ignore_pane(StartedCore::start(spawn, &pid, connector_to(&core)).expect("start"));
        crate::core_machine::panes::link(&MachineName::parse("away-test").expect("name"))
            .set_connected(false);
        assert!(matches!(
            pane.try_send_bytes(Bytes::from_static(b"x")),
            Err(mpsc::error::TrySendError::Closed(_))
        ));
        assert!(pane
            .queue_user_input_submission(
                Bytes::from_static(b"y"),
                Bytes::from_static(b"\r"),
                Duration::ZERO
            )
            .is_err());
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            core.received()
                .iter()
                .all(|frame| frame.channel == 0 || frame.kind != kind::DATA),
            "{:?}",
            core.received()
        );
        pane.shutdown();
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

    fn view_pane(core: &FakeCore, machine: &str) -> CorePaneIo {
        let address = format!("{machine}/t1");
        core.add_terminal(
            json!({"address": address, "state": "live", "cols": 100, "rows": 30,
            "labels": {"herdr.session": "herdr", "herdr.pane_id": "w1:p1"}}),
        );
        let size = view_far_size(&connector_to(core), &address).expect("far size");
        assert_eq!(size, (30, 100), "rows and cols from terminal.get");
        ignore_pane(
            StartedCore::view(
                PaneId::from_raw(41),
                "herdr".into(),
                address,
                size,
                connector_to(core),
            )
            .expect("view"),
        )
    }

    fn stream_frames(core: &FakeCore) -> Vec<(u8, Vec<u8>)> {
        core.received()
            .into_iter()
            .filter(|frame| frame.channel != 0)
            .map(|frame| (frame.kind, frame.payload))
            .collect()
    }

    // V14's unit part (core-phase5 D6 Size, R7, R8): a view attaches
    // `resumed` at the far size, types only AUTOMATIC_INPUT, sends a RESIZE
    // only for a far size different from the last one it sent, and never
    // DATA, FOCUS or FACTS; its close is a detach, never `terminal.kill`.
    #[test]
    fn a_view_pane_never_takes_the_far_terminals_size() {
        let core = FakeCore::start();
        let pane = view_pane(&core, "view-size");
        let attach = core
            .requests()
            .into_iter()
            .find(|request| request["method"] == "terminal.attach")
            .expect("attached");
        assert_eq!(attach["params"]["resumed"], true);
        assert_eq!(
            (
                attach["params"]["cols"].as_u64(),
                attach["params"]["rows"].as_u64()
            ),
            (Some(100), Some(30))
        );
        assert!(attach["params"]["observe"].is_null());
        assert_eq!(pane.view_size(), Some((30, 100)));
        assert!(!pane.view_read_only());
        assert_eq!(pane.machine().map(MachineName::as_str), Some("view-size"));

        pane.try_send_bytes(Bytes::from_static(b"k"))
            .expect("typed");
        pane.queue_user_input_submission(
            Bytes::from_static(b"line"),
            Bytes::from_static(b"\r"),
            Duration::ZERO,
        )
        .expect("submitted")
        .recv_timeout(Duration::from_secs(5))
        .expect("answer")
        .expect("sent");
        // A view sends no terminal reply, nor a resize's report (H1).
        pane.write_terminal_response(|| Some(Bytes::from_static(b"\x1b[0n")));
        // The grid's own resizes (its rect, its 2x4 minimum) never reach the
        // wire (item 6 of PR D's second read); only the far size does, and a
        // repeat of the attach size sends nothing.
        pane.resize(
            25,
            90,
            8,
            16,
            vec![Bytes::from_static(b"\x1b[48;25;90;400;720t")],
        );
        pane.resize(2, 4, 8, 16, Vec::new());
        assert!(pane.follow_far_size(30, 100));
        assert!(pane.follow_far_size(25, 90));
        assert!(pane.follow_far_size(25, 90));
        std::thread::sleep(Duration::from_millis(200));
        let frames = stream_frames(&core);
        let kinds: Vec<u8> = frames.iter().map(|(kind, _)| *kind).collect();
        assert_eq!(
            kinds,
            [
                kind::AUTOMATIC_INPUT,
                kind::AUTOMATIC_INPUT,
                kind::AUTOMATIC_INPUT,
                kind::RESIZE
            ],
            "{frames:?}"
        );
        let resize = &frames[3].1;
        assert_eq!(
            (
                u16::from_le_bytes([resize[0], resize[1]]),
                u16::from_le_bytes([resize[2], resize[3]])
            ),
            (90, 25),
            "one RESIZE, of the changed far size"
        );
        assert_eq!(pane.view_size(), Some((25, 90)));

        assert!(!pane.kill().ended(), "a view's close kills nothing");
        pane.shutdown();
        std::thread::sleep(Duration::from_millis(100));
        assert!(
            !core
                .methods()
                .iter()
                .any(|method| method == "terminal.kill"),
            "{:?}",
            core.methods()
        );
        assert!(
            pane.kept_on_drop(),
            "dropping a view's pane leaves the far program"
        );
    }

    // M3 (security review): a view's pane is bound to the far terminal it
    // first attached to, by its launch label and pid. After the stream
    // drops, the same terminal is attached again and typing goes on; once
    // the address holds another program (a far core reset reused it), the
    // stream ends instead, the pane says it was replaced, and no input
    // reaches that program.
    #[test]
    fn a_view_reattaches_only_the_far_terminal_it_attached_to() {
        let core = FakeCore::start();
        let address = "view-bound/t1".to_string();
        core.add_terminal(
            json!({"address": address, "state": "live", "cols": 80, "rows": 24,
            "pid": 100, "labels": {"herdr.launch": "launch-1"}}),
        );
        let (exit_tx, exits) = std_mpsc::channel();
        let pane = StartedCore::view(
            PaneId::from_raw(42),
            "herdr".into(),
            address.clone(),
            (24, 80),
            reconnecting(&core),
        )
        .expect("view")
        .into_pane(
            Box::new(|_: &[u8]| PtyReadResult {
                terminal_responses: Vec::new(),
            }),
            Box::new(move |reason| {
                let _ = exit_tx.send(reason);
            }),
            Box::new(|_| {}),
            Arc::new(AtomicBool::new(false)),
        )
        .expect("pane");
        let attaches = |core: &FakeCore| {
            core.methods()
                .iter()
                .filter(|method| *method == "terminal.attach")
                .count()
        };
        // The same terminal: attached again, not replaced.
        drop_stream(&core);
        let deadline = Instant::now() + Duration::from_secs(5);
        while attaches(&core) < 2 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(attaches(&core), 2, "{:?}", core.methods());
        assert_eq!(exits.recv_timeout(NO_EXIT).ok(), None);
        assert!(!pane.view_replaced());

        // Another program at the address: the stream ends, nothing attaches.
        core.add_terminal(
            json!({"address": address, "state": "live", "cols": 80, "rows": 24,
            "pid": 200, "labels": {"herdr.launch": "launch-2"}}),
        );
        core.close_stream(2, b"");
        assert_eq!(
            exits.recv_timeout(Duration::from_secs(5)).ok(),
            Some(ChildExitReason::Suspended(SuspendCause::TerminalGone))
        );
        assert!(pane.view_replaced());
        assert_eq!(attaches(&core), 2, "the other program is never attached");
        let typed_before = stream_frames(&core)
            .iter()
            .filter(|(kind, _)| *kind == kind::AUTOMATIC_INPUT)
            .count();
        let _ = pane.try_send_bytes(Bytes::from_static(b"stale"));
        std::thread::sleep(Duration::from_millis(100));
        let typed = stream_frames(&core)
            .iter()
            .filter(|(kind, _)| *kind == kind::AUTOMATIC_INPUT)
            .count();
        assert_eq!(typed, typed_before, "no input after the end");
        pane.shutdown();
    }

    // M4 (security review): a far size over the view's grid budget is
    // refused before a grid is built at it (attach) or resized to it (a size
    // event); the pane keeps its last size and sends no RESIZE. A size under
    // the grid's 2x4 minimum is echoed as it is, not as the minimum.
    #[test]
    fn a_far_size_over_the_budget_never_reaches_a_grid_or_the_wire() {
        let core = FakeCore::start();
        for (rows, cols) in [(65_535, 65_535), (1001, 80), (24, 1001), (600, 600)] {
            let address = format!("view-huge/t{rows}x{cols}");
            core.add_terminal(
                json!({"address": address, "state": "live", "cols": cols, "rows": rows}),
            );
            let refused = view_far_size(&connector_to(&core), &address);
            assert!(
                matches!(&refused, Err(CoreError::Rpc { kind, .. }) if kind == "too_large"),
                "{rows}x{cols}: {refused:?}"
            );
        }
        core.add_terminal(
            json!({"address": "view-huge/t1000x300", "state": "live", "cols": 1000, "rows": 300}),
        );
        assert_eq!(
            view_far_size(&connector_to(&core), "view-huge/t1000x300").expect("in budget"),
            (300, 1000)
        );

        let pane = view_pane(&core, "view-budget");
        assert!(!pane.follow_far_size(65_535, 65_535));
        assert!(!pane.follow_far_size(0, 80));
        assert_eq!(pane.view_size(), Some((30, 100)), "the last good size");
        assert!(pane.follow_far_size(1, 1));
        std::thread::sleep(Duration::from_millis(200));
        let resizes: Vec<(u16, u16)> = stream_frames(&core)
            .into_iter()
            .filter(|(kind, _)| *kind == kind::RESIZE)
            .map(|(_, payload)| {
                (
                    u16::from_le_bytes([payload[0], payload[1]]),
                    u16::from_le_bytes([payload[2], payload[3]]),
                )
            })
            .collect();
        assert_eq!(resizes, [(1, 1)], "only the far size, exactly");
        pane.shutdown();
    }

    // H1 (core-phase5 D6): far output a view replays or streams can hold
    // queries (DA, DSR, `?u`, OSC 10/11); the pane's terminal answers them,
    // and the view sends none of those answers to the far program.
    #[test]
    fn a_view_pane_sends_no_reply_to_far_queries() {
        let core = FakeCore::start_with(|config| {
            config.first_paint = b"history\x1b[c\x1b[6n".to_vec();
        });
        let address = "view-replies/t1".to_string();
        core.add_terminal(json!({"address": address, "state": "live", "cols": 80, "rows": 24}));
        let (reads_tx, reads_rx) = std_mpsc::channel();
        let pane = StartedCore::view(
            PaneId::from_raw(42),
            "herdr".into(),
            address,
            (24, 80),
            connector_to(&core),
        )
        .expect("view")
        .into_pane(
            Box::new(move |bytes: &[u8]| {
                let _ = reads_tx.send(bytes.to_vec());
                PtyReadResult {
                    terminal_responses: vec![
                        Bytes::from_static(b"\x1b[?62;22c"),
                        Bytes::from_static(b"\x1b[1;1R"),
                    ],
                }
            }),
            Box::new(|_| {}),
            Box::new(|_| {}),
            Arc::new(AtomicBool::new(false)),
        )
        .expect("pane");
        reads_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the replayed history was read");
        pane.try_send_bytes(Bytes::from_static(b"k"))
            .expect("typed");
        std::thread::sleep(Duration::from_millis(200));
        let typed: Vec<Vec<u8>> = stream_frames(&core)
            .into_iter()
            .filter(|(kind, _)| *kind == kind::AUTOMATIC_INPUT || *kind == kind::DATA)
            .map(|(_, payload)| payload)
            .collect();
        assert_eq!(typed, [b"k".to_vec()], "only the person's key");
        pane.shutdown();
    }

    // D6 Size: a path that takes no AUTOMATIC_INPUT makes the view read-only
    // (observe), never a DATA typist.
    #[test]
    fn a_view_without_automatic_input_watches_only() {
        let core = FakeCore::start_with(|config| {
            config.features = vec!["framed", "attach", "events", "status"];
        });
        let pane = view_pane(&core, "view-observe");
        let attaches: Vec<Value> = core
            .requests()
            .into_iter()
            .filter(|request| request["method"] == "terminal.attach")
            .collect();
        assert_eq!(attaches.len(), 2);
        assert_eq!(attaches[1]["params"]["observe"], true);
        assert!(pane.view_read_only());
        assert!(pane.try_send_bytes(Bytes::from_static(b"k")).is_err());
        assert!(pane
            .queue_user_input_submission(
                Bytes::from_static(b"x"),
                Bytes::from_static(b"\r"),
                Duration::ZERO
            )
            .expect("queued")
            .recv_timeout(Duration::from_secs(5))
            .expect("answer")
            .is_err());
        std::thread::sleep(Duration::from_millis(100));
        assert!(
            stream_frames(&core)
                .iter()
                .all(|(kind, _)| *kind == super::kind::CLOSE),
            "only the first attach's detach: {:?}",
            stream_frames(&core)
        );
        pane.shutdown();
    }
}
