//! One framed connection to a herdr-core session (openspec/changes/core-backend, D2 and D3).
//!
//! The writer is a mutex over the socket; one reader thread routes channel 0
//! (JSON-RPC replies and notifications) and each attach stream's frames to the
//! pane that owns the stream.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tracing::{debug, info, warn};

use super::frame::{kind, Frame, FrameDecoder};

/// The oldest core this client speaks to (phase 0 ran 0.1.21).
pub(crate) const MIN_CORE_VERSION: (u64, u64, u64) = (0, 1, 21);
const PROTOCOL_GENERATION: u64 = 1;
/// How long one frame's write may block before the connection is given up.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// The longest `hello` reply line this client reads.
const HELLO_LIMIT: u64 = 64 * 1024;
/// Core's credit window per stream (hc 0.1.21): a core that honours credit
/// never has more than this in flight on one stream.
pub(crate) const CREDIT_WINDOW: u32 = 256 * 1024;
/// What one stream may hold queued for its parser. More means core ignored
/// credit, a protocol error for that stream.
const STREAM_QUEUE_LIMIT: usize = 2 * CREDIT_WINDOW as usize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CoreError {
    /// No core to talk to, or one this client refuses (reason for the notice).
    Unavailable(String),
    /// Core answered with an error.
    Rpc {
        kind: String,
        message: String,
    },
    Timeout(&'static str),
    /// The connection is gone.
    Closed,
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoreError::Unavailable(reason) => write!(f, "herdr-core unavailable: {reason}"),
            CoreError::Rpc { kind, message } => write!(f, "herdr-core {kind}: {message}"),
            CoreError::Timeout(what) => write!(f, "herdr-core did not answer {what} in time"),
            CoreError::Closed => write!(f, "herdr-core connection closed"),
        }
    }
}

impl std::error::Error for CoreError {}

/// What a stream's owner receives, in order.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StreamEvent {
    Data(Vec<u8>),
    /// Core closed the stream; the JSON reason (empty for a detach answer).
    Close(Value),
    /// The connection ended under the stream.
    ConnectionLost,
    /// Not from core: the pane has a status record to mirror
    /// (openspec/changes/core-status-mirror, D3).
    Wake,
}

/// Where the reader queues a stream's events, counting the queued bytes so a
/// core that ignores credit cannot grow the queue without bound.
#[derive(Clone)]
pub(crate) struct StreamSink {
    tx: mpsc::Sender<StreamEvent>,
    queued: Arc<AtomicUsize>,
}

/// The receiving end of a [`StreamSink`].
pub(crate) struct StreamEvents {
    rx: mpsc::Receiver<StreamEvent>,
    queued: Arc<AtomicUsize>,
}

pub(crate) fn stream_channel() -> (StreamSink, StreamEvents) {
    let (tx, rx) = mpsc::channel();
    let queued = Arc::new(AtomicUsize::new(0));
    (
        StreamSink {
            tx,
            queued: Arc::clone(&queued),
        },
        StreamEvents { rx, queued },
    )
}

impl StreamSink {
    pub(crate) fn send(&self, event: StreamEvent) -> Result<(), mpsc::SendError<StreamEvent>> {
        let bytes = match &event {
            StreamEvent::Data(bytes) => bytes.len(),
            _ => 0,
        };
        self.queued.fetch_add(bytes, Ordering::AcqRel);
        self.tx.send(event).inspect_err(|_| {
            self.queued.fetch_sub(bytes, Ordering::AcqRel);
        })
    }

    fn queued(&self) -> usize {
        self.queued.load(Ordering::Acquire)
    }
}

impl StreamEvents {
    pub(crate) fn recv(&self) -> Result<StreamEvent, mpsc::RecvError> {
        self.rx.recv().inspect(|event| self.taken(event))
    }

    pub(crate) fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<StreamEvent, mpsc::RecvTimeoutError> {
        self.rx
            .recv_timeout(timeout)
            .inspect(|event| self.taken(event))
    }

    fn taken(&self, event: &StreamEvent) {
        if let StreamEvent::Data(bytes) = event {
            self.queued.fetch_sub(bytes.len(), Ordering::AcqRel);
        }
    }
}

type Reply = Result<Value, CoreError>;
type EventHandler = Box<dyn Fn(&Value) + Send + Sync>;

struct Pending {
    reply: mpsc::Sender<Reply>,
    /// For `terminal.attach`: registered by the reader as soon as the reply
    /// arrives, before any frame of the stream can be routed.
    stream: Option<StreamSink>,
}

struct Inner {
    writer: Mutex<UnixStream>,
    pending: Mutex<HashMap<u64, Pending>>,
    streams: Mutex<HashMap<u32, StreamSink>>,
    next_id: AtomicU64,
    closed: AtomicBool,
    automatic_input: bool,
    on_event: Option<EventHandler>,
}

#[derive(Clone)]
pub(crate) struct CoreClient {
    inner: Arc<Inner>,
    _handle: Arc<Handle>,
}

/// Shuts the connection when the last client handle goes. The reader thread
/// keeps `Inner`, so without this a dropped client (one refused for its
/// version, say) would leave its thread and connection behind for good.
struct Handle(UnixStream);

impl Drop for Handle {
    fn drop(&mut self) {
        let _ = self.0.shutdown(std::net::Shutdown::Both);
    }
}

impl CoreClient {
    /// Connects to the core at `socket`, says hello and switches to framed
    /// mode. Refuses a socket another user could have placed or serves, and a
    /// core of another protocol generation.
    pub(crate) fn connect(
        socket: &Path,
        on_event: Option<EventHandler>,
    ) -> Result<Self, CoreError> {
        let euid = unsafe { libc::geteuid() };
        check_socket_folder(socket, euid)?;
        let mut stream = UnixStream::connect(socket).map_err(|err| {
            CoreError::Unavailable(format!("cannot connect to {}: {err}", socket.display()))
        })?;
        match crate::platform::unix_stream_peer_uid(&stream) {
            Some(uid) if uid == euid => {}
            Some(uid) => {
                return Err(CoreError::Unavailable(format!(
                    "{} is served by uid {uid}, not {euid}",
                    socket.display()
                )))
            }
            None => {
                return Err(CoreError::Unavailable(format!(
                    "cannot tell which user serves {}",
                    socket.display()
                )))
            }
        }
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .and_then(|()| stream.set_write_timeout(Some(WRITE_TIMEOUT)))
            .map_err(|err| CoreError::Unavailable(err.to_string()))?;
        let hello =
            json!({"jsonrpc": "2.0", "id": 0, "method": "hello", "params": {"framed": true}});
        let mut line =
            serde_json::to_vec(&hello).map_err(|err| CoreError::Unavailable(err.to_string()))?;
        line.push(b'\n');
        stream
            .write_all(&line)
            .map_err(|err| CoreError::Unavailable(format!("hello not sent: {err}")))?;
        let mut reader = BufReader::new(
            stream
                .try_clone()
                .map_err(|err| CoreError::Unavailable(err.to_string()))?,
        );
        let mut reply = String::new();
        let read = (&mut reader)
            .take(HELLO_LIMIT)
            .read_line(&mut reply)
            .map_err(|err| CoreError::Unavailable(format!("no hello reply: {err}")))?;
        if read as u64 == HELLO_LIMIT && !reply.ends_with('\n') {
            return Err(CoreError::Unavailable(format!(
                "hello reply longer than {} KiB",
                HELLO_LIMIT / 1024
            )));
        }
        let reply: Value = serde_json::from_str(reply.trim())
            .map_err(|err| CoreError::Unavailable(format!("bad hello reply: {err}")))?;
        let result = reply
            .get("result")
            .ok_or_else(|| CoreError::Unavailable(format!("hello refused: {}", reply["error"])))?;
        let generation = result["generation"].as_u64().unwrap_or(0);
        if generation != PROTOCOL_GENERATION {
            return Err(CoreError::Unavailable(format!(
                "protocol generation {generation}, this herdr speaks {PROTOCOL_GENERATION}"
            )));
        }
        let automatic_input = result["features"]
            .as_array()
            .is_some_and(|features| features.iter().any(|f| f == "automatic_input"));
        stream
            .set_read_timeout(None)
            .map_err(|err| CoreError::Unavailable(err.to_string()))?;
        let handle = Arc::new(Handle(
            stream
                .try_clone()
                .map_err(|err| CoreError::Unavailable(err.to_string()))?,
        ));

        let inner = Arc::new(Inner {
            writer: Mutex::new(stream),
            pending: Mutex::new(HashMap::new()),
            streams: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            closed: AtomicBool::new(false),
            automatic_input,
            on_event,
        });
        let reader_inner = Arc::clone(&inner);
        // Bytes already buffered past the hello line belong to the framed stream.
        let leftover = reader.buffer().to_vec();
        let socket = reader.into_inner();
        std::thread::Builder::new()
            .name("herdr-core-reader".into())
            .spawn(move || read_loop(reader_inner, socket, leftover))
            .map_err(|err| CoreError::Unavailable(format!("reader thread: {err}")))?;
        Ok(Self {
            inner,
            _handle: handle,
        })
    }

    /// Whether core negotiated AUTOMATIC_INPUT frames (terminal replies that
    /// are not a person typing).
    pub(crate) fn automatic_input(&self) -> bool {
        self.inner.automatic_input
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Acquire)
    }

    /// One JSON-RPC call on channel 0.
    pub(crate) fn call(&self, method: &str, params: Value, timeout: Duration) -> Reply {
        self.call_inner(method, params, timeout, None)
    }

    /// `terminal.attach`: returns the reply and routes the stream's frames to
    /// `sink` from the first one on.
    pub(crate) fn attach(&self, params: Value, sink: StreamSink, timeout: Duration) -> Reply {
        self.call_inner("terminal.attach", params, timeout, Some(sink))
    }

    fn call_inner(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        stream: Option<StreamSink>,
    ) -> Reply {
        if self.is_closed() {
            return Err(CoreError::Closed);
        }
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (reply_tx, reply_rx) = mpsc::channel();
        lock(&self.inner.pending).insert(
            id,
            Pending {
                reply: reply_tx,
                stream,
            },
        );
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let payload = serde_json::to_vec(&request).map_err(|err| CoreError::Rpc {
            kind: "encode".into(),
            message: err.to_string(),
        })?;
        if let Err(err) = self.send(&Frame::new(0, kind::DATA, payload)) {
            lock(&self.inner.pending).remove(&id);
            return Err(err);
        }
        match reply_rx.recv_timeout(timeout) {
            Ok(reply) => reply,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                lock(&self.inner.pending).remove(&id);
                Err(CoreError::Timeout("a call"))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(CoreError::Closed),
        }
    }

    /// Writes one frame. Frames are written whole under the writer lock, so
    /// streams interleave only at frame boundaries. A failed or stalled write
    /// (core not reading for `WRITE_TIMEOUT`) may have sent part of a frame,
    /// so it shuts the connection: every stream then sees ConnectionLost.
    pub(crate) fn send(&self, frame: &Frame) -> Result<(), CoreError> {
        let mut bytes = Vec::new();
        frame
            .encode_into(&mut bytes)
            .map_err(|err| CoreError::Rpc {
                kind: "encode".into(),
                message: err.to_string(),
            })?;
        let mut writer = lock(&self.inner.writer);
        writer.write_all(&bytes).map_err(|err| {
            warn!(err = %err, "herdr-core write failed; closing the connection");
            let _ = writer.shutdown(std::net::Shutdown::Both);
            CoreError::Closed
        })
    }

    /// Stops routing a stream (after a detach or the stream's end).
    pub(crate) fn forget_stream(&self, channel: u32) {
        lock(&self.inner.streams).remove(&channel);
    }
}

/// Refuses a socket whose folder another user owns or can write to: they
/// could have put their own socket there.
fn check_socket_folder(socket: &Path, euid: u32) -> Result<(), CoreError> {
    use std::os::unix::fs::MetadataExt as _;

    let folder = match socket.parent() {
        Some(folder) if !folder.as_os_str().is_empty() => folder,
        _ => Path::new("."),
    };
    let metadata = std::fs::metadata(folder).map_err(|err| {
        CoreError::Unavailable(format!("cannot inspect {}: {err}", folder.display()))
    })?;
    match socket_folder_refusal(metadata.uid(), metadata.mode(), euid) {
        Some(refusal) => Err(CoreError::Unavailable(format!(
            "the folder of {} {refusal}",
            socket.display()
        ))),
        None => Ok(()),
    }
}

/// Why a socket folder with this owner and mode is not trusted, or `None`.
fn socket_folder_refusal(owner: u32, mode: u32, euid: u32) -> Option<String> {
    if owner != euid {
        return Some(format!("is owned by uid {owner}, not {euid}"));
    }
    if mode & 0o022 != 0 {
        return Some(format!(
            "is writable by group or others (mode {:o})",
            mode & 0o7777
        ));
    }
    None
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn read_loop(inner: Arc<Inner>, mut socket: UnixStream, leftover: Vec<u8>) {
    let mut decoder = FrameDecoder::default();
    decoder.push(&leftover);
    let mut buf = vec![0u8; 64 * 1024];
    'read: loop {
        loop {
            match decoder.next_frame() {
                Ok(Some(frame)) => route(&inner, frame),
                Ok(None) => break,
                Err(err) => {
                    warn!(err = %err, "herdr-core sent a bad frame; closing the connection");
                    break 'read;
                }
            }
        }
        match socket.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => decoder.push(&buf[..n]),
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => {
                debug!(err = %err, "herdr-core read failed");
                break;
            }
        }
    }
    inner.closed.store(true, Ordering::Release);
    for (_, pending) in lock(&inner.pending).drain() {
        let _ = pending.reply.send(Err(CoreError::Closed));
    }
    for (_, sink) in lock(&inner.streams).drain() {
        let _ = sink.send(StreamEvent::ConnectionLost);
    }
}

fn route(inner: &Inner, frame: Frame) {
    if frame.channel == 0 {
        if frame.kind == kind::DATA {
            route_rpc(inner, &frame.payload);
        }
        return;
    }
    let event = match frame.kind {
        kind::DATA => StreamEvent::Data(frame.payload),
        kind::CLOSE => {
            let reason = serde_json::from_slice(&frame.payload).unwrap_or(Value::Null);
            StreamEvent::Close(reason)
        }
        // RESIZE is only sent to observers that asked for it; this client
        // attaches as a viewer. Other kinds are ignored, as core ignores ours.
        _ => return,
    };
    let closing = matches!(event, StreamEvent::Close(_));
    let mut streams = lock(&inner.streams);
    let Some(sink) = streams.get(&frame.channel) else {
        return;
    };
    // Never wait on the parser here: this thread serves every stream.
    if let StreamEvent::Data(bytes) = &event {
        if sink.queued().saturating_add(bytes.len()) > STREAM_QUEUE_LIMIT {
            warn!(
                channel = frame.channel,
                queued = sink.queued(),
                "herdr-core sent past the stream's credit; dropping the stream"
            );
            if let Some(sink) = streams.remove(&frame.channel) {
                let _ = sink.send(StreamEvent::ConnectionLost);
            }
            return;
        }
    }
    if sink.send(event).is_err() || closing {
        streams.remove(&frame.channel);
    }
}

fn route_rpc(inner: &Inner, payload: &[u8]) {
    let Ok(message) = serde_json::from_slice::<Value>(payload) else {
        warn!("herdr-core sent a message that is not JSON");
        return;
    };
    if let Some(id) = message.get("id").and_then(Value::as_u64) {
        let Some(pending) = lock(&inner.pending).remove(&id) else {
            return;
        };
        let reply = match (message.get("result"), message.get("error")) {
            (Some(result), _) => Ok(result.clone()),
            (None, Some(error)) => Err(CoreError::Rpc {
                kind: error["data"]["kind"]
                    .as_str()
                    .unwrap_or("error")
                    .to_string(),
                message: error["message"].as_str().unwrap_or_default().to_string(),
            }),
            (None, None) => Err(CoreError::Rpc {
                kind: "protocol".into(),
                message: "reply with neither result nor error".into(),
            }),
        };
        if let (Ok(result), Some(sink)) = (&reply, pending.stream) {
            if let Some(channel) = result.get("channel").and_then(Value::as_u64) {
                lock(&inner.streams).insert(channel as u32, sink);
            }
        }
        let _ = pending.reply.send(reply);
        return;
    }
    if let Some(on_event) = &inner.on_event {
        on_event(&message);
    }
}

/// Variables a core must not inherit from our server (design D2): the core
/// keeps the environment it starts with and hands it to every program.
const CORE_START_ENV_REMOVE: &[&str] = &[
    "HERDR_PANE_ID",
    "HERDR_TAB_ID",
    "HERDR_WORKSPACE_ID",
    "HC_TERMINAL_ID",
    "HC_TERMINAL_SOCKET",
    "HC_SCOPE",
];
const CORE_START_ENV_REMOVE_PREFIXES: &[&str] = &["CLAUDE_CODE_", "ANTHROPIC_"];

/// The `hc` binary: found on `PATH` once, then used by that absolute path,
/// so a later `PATH` cannot swap it. Its path and version are logged then.
fn hc_binary() -> Result<&'static Path, CoreError> {
    static HC: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    if let Some(path) = HC.get() {
        return Ok(path);
    }
    let path = find_on_path("hc", std::env::var_os("PATH").as_deref())
        .ok_or_else(|| CoreError::Unavailable("no hc on PATH".into()))?;
    let version = binary_version(&path).unwrap_or_else(|| "unknown".into());
    info!(path = %path.display(), version, "herdr-core is started with this hc");
    Ok(HC.get_or_init(|| path))
}

/// The first executable file `name` in an absolute `PATH` folder.
fn find_on_path(name: &str, path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt as _;

    std::env::split_paths(path?)
        .filter(|folder| folder.is_absolute())
        .map(|folder| folder.join(name))
        .find(|candidate| {
            std::fs::metadata(candidate)
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
}

/// What `<binary> --version` prints, waiting at most two seconds.
fn binary_version(binary: &Path) -> Option<String> {
    let mut child = std::process::Command::new(binary)
        .arg("--version")
        .env_clear()
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while child.try_wait().ok()?.is_none() {
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    Some(out.trim().to_string())
}

/// The `hc server start` command for `session`, with the environment D2 asks
/// for.
pub(crate) fn core_start_command(
    hc: &Path,
    session: &str,
    inherited: impl IntoIterator<Item = (String, String)>,
) -> std::process::Command {
    let mut command = std::process::Command::new(hc);
    command.args(["server", "start"]);
    command.env_clear();
    for (key, value) in inherited {
        let stripped = CORE_START_ENV_REMOVE.contains(&key.as_str())
            || CORE_START_ENV_REMOVE_PREFIXES
                .iter()
                .any(|prefix| key.starts_with(prefix))
            || key == "CLAUDECODE";
        if !stripped {
            command.env(key, value);
        }
    }
    command.env("HC_SESSION", session);
    command
}

/// Connects to the core session `session` and refuses a core older than
/// [`MIN_CORE_VERSION`]. With `start_if_missing`, a missing socket starts the
/// core; without it (a reconnect while a core restarts, when its socket is
/// briefly gone), it is an error the caller retries.
pub(crate) fn connect_session(
    session: &str,
    on_event: Option<EventHandler>,
    start_if_missing: bool,
) -> Result<CoreClient, CoreError> {
    let socket = socket_path(session);
    let started = !socket.exists();
    if started {
        if !start_if_missing {
            return Err(CoreError::Unavailable(format!(
                "no herdr-core socket at {}",
                socket.display()
            )));
        }
        let status = core_start_command(hc_binary()?, session, std::env::vars())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|err| CoreError::Unavailable(format!("cannot run hc: {err}")))?;
        if !status.success() {
            return Err(CoreError::Unavailable(format!(
                "hc server start exited with {status}"
            )));
        }
    }
    let client = CoreClient::connect(&socket, on_event)?;
    let status = check_version(&client)?;
    // core-backend D2: a core we start must outlive us. On macOS `hc server
    // start` goes through launchd; a start that fell back to our own child
    // would end with this server.
    let core_pid = status["pid"].as_u64().unwrap_or(0) as u32;
    if started && is_our_child(core_pid) {
        tracing::warn!(
            core_pid,
            session,
            "herdr-core runs in this server's session; it may end when this server does, and core panes with it"
        );
    }
    Ok(client)
}

/// True when `pid` is this process's child: a core that fell back to a
/// direct child would be (D2); a launchd-started one has parent 1. (Comparing
/// sessions would not do: a herdr launchd started is in launchd's session.)
fn is_our_child(pid: u32) -> bool {
    pid != 0 && crate::platform::parent_pid(pid) == Some(std::process::id())
}

fn check_version(client: &CoreClient) -> Result<Value, CoreError> {
    let status = client.call("server.status", json!({}), Duration::from_secs(5))?;
    let text = status["version"].as_str().unwrap_or_default();
    match parse_version(text) {
        Some(version) if version >= MIN_CORE_VERSION => Ok(status),
        _ => Err(CoreError::Unavailable(format!(
            "herdr-core {text} is older than {}.{}.{}",
            MIN_CORE_VERSION.0, MIN_CORE_VERSION.1, MIN_CORE_VERSION.2
        ))),
    }
}

/// The client socket for a core session, by core's endpoint rules
/// (`docs/security.md` § State and endpoints).
pub(crate) fn socket_path(session: &str) -> PathBuf {
    let hc_socket = std::env::var_os("HC_SOCKET").map(PathBuf::from);
    if let Some(path) = &hc_socket {
        static LOGGED: std::sync::Once = std::sync::Once::new();
        LOGGED.call_once(|| {
            info!(
                socket = %path.display(),
                session,
                "HC_SOCKET is set: herdr-core is reached there, not at the session's own socket"
            );
        });
    }
    socket_path_from(
        session,
        hc_socket,
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        std::env::var_os("XDG_STATE_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
        Path::new(&format!("/run/user/{}", unsafe { libc::getuid() })).is_dir(),
    )
}

fn socket_path_from(
    session: &str,
    hc_socket: Option<PathBuf>,
    runtime_dir: Option<PathBuf>,
    state_home: Option<PathBuf>,
    home: Option<PathBuf>,
    run_user_exists: bool,
) -> PathBuf {
    let name = format!("{session}.sock");
    if let Some(path) = hc_socket {
        return path;
    }
    if let Some(dir) = runtime_dir {
        return dir.join("herdr-core").join(name);
    }
    if run_user_exists {
        let uid = unsafe { libc::getuid() };
        return PathBuf::from(format!("/run/user/{uid}/herdr-core")).join(name);
    }
    let state = state_home.unwrap_or_else(|| home.unwrap_or_default().join(".local/state"));
    state.join("herdr-core").join(session).join(name)
}

/// Parses `0.1.21` (or `0.1.21-rc.1`) as a comparable triple.
pub(crate) fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let core = text
        .trim()
        .trim_start_matches("hc ")
        .split(['-', '+'])
        .next()?;
    let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

#[cfg(test)]
mod tests {
    use super::super::fake::FakeCore;
    use super::*;

    #[test]
    fn socket_path_follows_core_endpoint_rules() {
        let home = Some(PathBuf::from("/home/u"));
        assert_eq!(
            socket_path_from(
                "herdr",
                Some("/x.sock".into()),
                None,
                None,
                home.clone(),
                false
            ),
            PathBuf::from("/x.sock")
        );
        assert_eq!(
            socket_path_from(
                "herdr",
                None,
                Some("/run/rt".into()),
                None,
                home.clone(),
                true
            ),
            PathBuf::from("/run/rt/herdr-core/herdr.sock")
        );
        assert_eq!(
            socket_path_from("herdr", None, None, None, home.clone(), false),
            PathBuf::from("/home/u/.local/state/herdr-core/herdr/herdr.sock")
        );
        assert_eq!(
            socket_path_from("herdr", None, None, Some("/st".into()), home, false),
            PathBuf::from("/st/herdr-core/herdr/herdr.sock")
        );
    }

    #[test]
    fn only_a_folder_we_own_and_alone_can_write_holds_a_trusted_socket() {
        assert_eq!(socket_folder_refusal(501, 0o40700, 501), None);
        assert_eq!(socket_folder_refusal(501, 0o40755, 501), None);
        assert!(socket_folder_refusal(502, 0o40700, 501).is_some_and(|r| r.contains("uid 502")));
        assert!(socket_folder_refusal(0, 0o41777, 501).is_some(), "/tmp");
        for mode in [0o40770, 0o40720, 0o40702, 0o40777] {
            assert!(
                socket_folder_refusal(501, mode, 501).is_some_and(|r| r.contains("writable")),
                "{mode:o}"
            );
        }
    }

    #[test]
    fn a_core_socket_in_a_folder_others_can_write_is_refused_before_hello() {
        use std::os::unix::fs::PermissionsExt as _;
        let core = FakeCore::start();
        let folder = core.socket.parent().expect("socket folder");
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o770)).expect("chmod");
        let err = CoreClient::connect(&core.socket, None)
            .err()
            .expect("refused");
        assert!(
            matches!(&err, CoreError::Unavailable(reason) if reason.contains("writable")),
            "{err}"
        );
        assert!(core.requests().is_empty(), "no hello was sent");
    }

    #[test]
    fn parses_core_versions() {
        assert_eq!(parse_version("0.1.21"), Some((0, 1, 21)));
        assert_eq!(parse_version("hc 0.1.21"), Some((0, 1, 21)));
        assert_eq!(parse_version("0.2.0-rc.1"), Some((0, 2, 0)));
        assert_eq!(parse_version("nope"), None);
        assert!(parse_version("0.1.17").unwrap() < MIN_CORE_VERSION);
    }

    #[test]
    fn connects_says_hello_and_calls() {
        let core = FakeCore::start();
        let client = CoreClient::connect(&core.socket, None).expect("connect");
        assert!(client.automatic_input());
        let status = client
            .call("server.status", json!({}), Duration::from_secs(5))
            .expect("status");
        assert_eq!(status["version"], "0.1.21");
        let requests = core.requests();
        assert_eq!(
            requests.last().map(|r| r["method"].clone()),
            Some(json!("server.status"))
        );
        assert!(core
            .received()
            .iter()
            .all(|frame| frame.channel == 0 && frame.kind == kind::DATA));
    }

    #[test]
    fn refuses_another_protocol_generation() {
        let core = FakeCore::start_with(|config| config.generation = 2);
        let err = CoreClient::connect(&core.socket, None)
            .err()
            .expect("refused");
        assert!(matches!(err, CoreError::Unavailable(reason) if reason.contains("generation 2")));
    }

    #[test]
    fn refuses_a_core_older_than_the_minimum() {
        let core = FakeCore::start_with(|config| config.version = "0.1.17".into());
        let client = CoreClient::connect(&core.socket, None).expect("connect");
        let err = check_version(&client).expect_err("too old");
        assert!(matches!(err, CoreError::Unavailable(reason) if reason.contains("0.1.17")));
    }

    #[test]
    fn the_core_starts_without_our_panes_identity() {
        let inherited = [
            ("PATH", "/usr/bin"),
            ("HOME", "/home/u"),
            ("HERDR_PANE_ID", "w1:p1"),
            ("CLAUDECODE", "1"),
            ("CLAUDE_CODE_CHILD_SESSION", "1"),
            ("ANTHROPIC_BASE_URL", "http://proxy"),
            ("HC_TERMINAL_ID", "t7"),
        ]
        .map(|(k, v)| (k.to_string(), v.to_string()));
        let command = core_start_command(Path::new("/opt/hc/bin/hc"), "herdr", inherited);
        let env: Vec<(String, Option<String>)> = command
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        let keys: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"PATH") && keys.contains(&"HOME"), "{keys:?}");
        for stripped in [
            "HERDR_PANE_ID",
            "CLAUDECODE",
            "CLAUDE_CODE_CHILD_SESSION",
            "ANTHROPIC_BASE_URL",
            "HC_TERMINAL_ID",
        ] {
            assert!(!keys.contains(&stripped), "{stripped} leaked: {keys:?}");
        }
        assert!(env.contains(&("HC_SESSION".into(), Some("herdr".into()))));
        assert_eq!(command.get_program(), "/opt/hc/bin/hc");
        assert_eq!(command.get_args().collect::<Vec<_>>(), ["server", "start"]);
    }

    #[test]
    fn hc_is_the_first_executable_in_an_absolute_path_folder() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = std::env::temp_dir().join(format!("hcpath-{}", std::process::id()));
        let (plain, exec, later) = (root.join("a"), root.join("b"), root.join("c"));
        for (folder, mode) in [(&plain, 0o644), (&exec, 0o755), (&later, 0o755)] {
            std::fs::create_dir_all(folder).expect("folder");
            let hc = folder.join("hc");
            std::fs::write(&hc, "#!/bin/sh\n").expect("hc");
            std::fs::set_permissions(&hc, std::fs::Permissions::from_mode(mode)).expect("mode");
        }
        let path = std::env::join_paths([
            "relative",
            plain.to_str().unwrap(),
            exec.to_str().unwrap(),
            later.to_str().unwrap(),
        ])
        .expect("PATH");
        assert_eq!(find_on_path("hc", Some(&path)), Some(exec.join("hc")));
        assert_eq!(find_on_path("hc", None), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_socket_is_unavailable() {
        let err = CoreClient::connect(Path::new("/nonexistent/herdr-core-test.sock"), None)
            .err()
            .expect("refused");
        assert!(matches!(err, CoreError::Unavailable(_)));
    }

    #[test]
    fn a_core_that_is_our_child_is_told_apart_from_a_detached_one() {
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .spawn()
            .expect("spawn sleep");
        assert!(is_our_child(child.id()), "a direct child");
        let _ = child.kill();
        let _ = child.wait();
        assert!(
            !is_our_child(std::process::id()),
            "this process is not its own child"
        );
        assert!(!is_our_child(1), "launchd or init");
        assert!(!is_our_child(0), "no pid");
    }

    #[test]
    fn dropping_the_last_client_closes_the_connection() {
        let core = FakeCore::start();
        let client = CoreClient::connect(&core.socket, None).expect("connect");
        let other = client.clone();
        drop(client);
        assert!(!other.is_closed(), "a clone keeps the connection");
        drop(other);
        // Behaviour, not timing: the core sees the hang-up (a leaked
        // connection would keep it open for good). Generous under load.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !core.client_hung_up() {
            assert!(
                std::time::Instant::now() < deadline,
                "the dropped client's connection stayed open"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn core_errors_come_back_with_their_kind() {
        let core = FakeCore::start();
        let client = CoreClient::connect(&core.socket, None).expect("connect");
        let err = client
            .call(
                "terminal.get",
                json!({"terminal": "t404"}),
                Duration::from_secs(5),
            )
            .expect_err("not found");
        assert_eq!(
            err,
            CoreError::Rpc {
                kind: "not_found".into(),
                message: "no terminal t404".into()
            }
        );
    }

    #[test]
    fn stream_frames_sent_right_after_the_attach_reply_are_not_lost() {
        let core = FakeCore::start_with(|config| config.first_paint = b"first-paint".to_vec());
        let client = CoreClient::connect(&core.socket, None).expect("connect");
        let (sink, events) = stream_channel();
        let reply = client
            .attach(json!({"terminal": "t1"}), sink, Duration::from_secs(5))
            .expect("attach");
        assert_eq!(reply["channel"], 1);
        assert_eq!(
            events.recv_timeout(Duration::from_secs(5)).unwrap(),
            StreamEvent::Data(b"first-paint".to_vec())
        );
    }

    #[test]
    fn a_hello_reply_longer_than_the_limit_is_refused() {
        let mut padded = vec![b' '; HELLO_LIMIT as usize];
        padded.extend_from_slice(br#"{"jsonrpc":"2.0","id":0,"result":{"generation":1}}"#);
        padded.push(b'\n');
        let core = FakeCore::start_with(|config| config.hello_line = Some(padded));
        let err = CoreClient::connect(&core.socket, None)
            .err()
            .expect("refused");
        assert!(
            matches!(&err, CoreError::Unavailable(reason) if reason.contains("hello reply longer")),
            "{err}"
        );
    }

    #[test]
    fn a_stream_sent_past_its_credit_is_dropped_and_the_connection_stays() {
        let core = FakeCore::start();
        let client = CoreClient::connect(&core.socket, None).expect("connect");
        let (sink, events) = stream_channel();
        let reply = client
            .attach(json!({"terminal": "t1"}), sink, Duration::from_secs(5))
            .expect("attach");
        let channel = reply["channel"].as_u64().expect("channel") as u32;
        // Nothing reads the stream and nothing credits it: a core that
        // honours credit would stop at CREDIT_WINDOW.
        let chunk = vec![b'x'; 16 * 1024];
        for _ in 0..(3 * CREDIT_WINDOW as usize / chunk.len()) {
            core.send(Frame::new(channel, kind::DATA, chunk.clone()));
        }
        // The reader handles frames in order, so once this reply is back every
        // data frame above has been queued or refused; only then drain, or a
        // fast drain keeps the queue under the bound.
        client
            .call("server.status", json!({}), Duration::from_secs(5))
            .expect("the reader serves calls");
        let mut queued = 0;
        let lost = loop {
            match events.recv_timeout(Duration::from_secs(5)) {
                Ok(StreamEvent::Data(bytes)) => queued += bytes.len(),
                other => break other,
            }
        };
        assert_eq!(lost, Ok(StreamEvent::ConnectionLost));
        assert!(queued <= STREAM_QUEUE_LIMIT, "{queued} bytes queued");
        assert!(!client.is_closed(), "other streams keep the connection");
        client
            .call("server.status", json!({}), Duration::from_secs(5))
            .expect("the reader still serves calls");
    }

    #[test]
    fn a_core_that_stops_reading_is_disconnected_after_the_write_deadline() {
        let core = FakeCore::start_with(|config| config.reads_after_hello = false);
        let client = CoreClient::connect(&core.socket, None).expect("connect");
        let frame = Frame::new(1, kind::DATA, vec![0u8; 1024 * 1024]);
        let started = std::time::Instant::now();
        let err = loop {
            if let Err(err) = client.send(&frame) {
                break err;
            }
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "writes never stalled"
            );
        };
        assert_eq!(err, CoreError::Closed);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !client.is_closed() {
            assert!(
                std::time::Instant::now() < deadline,
                "the stalled connection stayed open"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn notifications_reach_the_event_handler() {
        let core = FakeCore::start();
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let client = CoreClient::connect(
            &core.socket,
            Some(Box::new(move |message: &Value| {
                let _ = lock(&tx).send(message.clone());
            })),
        )
        .expect("connect");
        core.notify(json!({"jsonrpc": "2.0", "method": "event", "params": {"seq": 1, "type": "status", "terminal": "t1", "data": {"state": "working"}}}));
        let event = rx.recv_timeout(Duration::from_secs(5)).expect("event");
        assert_eq!(event["params"]["data"]["state"], "working");
        drop(client);
    }

    #[test]
    fn a_dropped_connection_ends_pending_calls_and_streams() {
        let core = FakeCore::start();
        let client = CoreClient::connect(&core.socket, None).expect("connect");
        let (sink, events) = stream_channel();
        client
            .attach(json!({"terminal": "t1"}), sink, Duration::from_secs(5))
            .expect("attach");
        core.hang_up();
        assert_eq!(
            events.recv_timeout(Duration::from_secs(5)).unwrap(),
            StreamEvent::ConnectionLost
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !client.is_closed() {
            assert!(
                std::time::Instant::now() < deadline,
                "client never saw the hang-up"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            client.call("server.status", json!({}), Duration::from_secs(1)),
            Err(CoreError::Closed)
        );
    }

    /// Live check against a real core: `HERDR_TEST_HC_SOCKET=<socket>` (a
    /// scratch or pilot session, never the user's default core).
    #[test]
    #[ignore = "needs a running herdr-core; set HERDR_TEST_HC_SOCKET"]
    fn live_client_runs_attaches_and_reads_a_real_core() {
        let Some(socket) = std::env::var_os("HERDR_TEST_HC_SOCKET") else {
            return;
        };
        let client = CoreClient::connect(Path::new(&socket), None).expect("connect");
        let status = client
            .call("server.status", json!({}), Duration::from_secs(5))
            .expect("status");
        let version = parse_version(status["version"].as_str().unwrap_or_default());
        assert!(version.is_some_and(|v| v >= MIN_CORE_VERSION), "{status}");
        let terminal = client
            .call(
                "terminal.run",
                json!({"argv": ["/bin/sh", "-c", "printf live-ok; exec sleep 5"],
                       "no_parent": true, "no_notify": true, "rm": true,
                       "labels": {"herdr.test": "client"}}),
                Duration::from_secs(5),
            )
            .expect("run");
        assert!(
            terminal["pid"].as_u64().is_some_and(|pid| pid > 0),
            "{terminal}"
        );
        let (sink, events) = stream_channel();
        let reply = client
            .attach(
                json!({"terminal": terminal["address"], "cols": 80, "rows": 24}),
                sink,
                Duration::from_secs(5),
            )
            .expect("attach");
        let channel = reply["channel"].as_u64().expect("channel") as u32;
        let mut seen = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !String::from_utf8_lossy(&seen).contains("live-ok") {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match events.recv_timeout(left) {
                Ok(StreamEvent::Data(bytes)) => {
                    client
                        .send(&Frame::credit(channel, bytes.len() as u32))
                        .expect("credit");
                    seen.extend(bytes);
                }
                other => panic!("no output before {other:?}"),
            }
        }
        let _ = client.call(
            "terminal.kill",
            json!({"terminal": terminal["address"]}),
            Duration::from_secs(5),
        );
    }
}
