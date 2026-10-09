//! Codex panes on the shared Codex app-server daemon.
//!
//! agent-bell's Codex receiver reaches only threads that live on the shared
//! daemon, and addresses them by thread name
//! (https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/85). Layer A adds
//! `--remote unix://<socket>` and `-C <pane cwd>` to Codex launches. Layer B
//! names the pane's daemon thread after the pane's agent name over the same
//! socket. Both are off by default, and B needs A.

use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

// Thread naming talks to the daemon's unix socket, so the naming helpers
// below exist on unix only.

/// How long a naming job keeps looking for its thread.
#[cfg(unix)]
const NAME_JOB_DEADLINE: Duration = Duration::from_secs(20);
#[cfg(unix)]
const NAME_JOB_POLL: Duration = Duration::from_millis(250);
/// A thread counts as the new pane's if it was created no earlier than this
/// before the launch.
#[cfg(unix)]
const CREATED_SLACK_MS: i64 = 1_000;
/// How long after a hand-launched Codex process starts its thread can appear.
/// Codex creates it at startup; the bound keeps a later launch's thread out.
#[cfg(unix)]
const HAND_LAUNCH_WINDOW_MS: i64 = 30_000;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CodexAppServer {
    /// The daemon socket when layer A is on.
    socket: Option<PathBuf>,
    name_threads: bool,
}

impl CodexAppServer {
    pub(crate) fn from_config(config: &crate::config::CodexAgentConfig) -> Self {
        // The daemon is reached over a unix socket; elsewhere the switches do nothing.
        if !cfg!(unix) || !config.app_server {
            return Self::default();
        }
        Self {
            socket: Some(expand_home(&config.app_server_socket)),
            name_threads: config.name_threads,
        }
    }

    /// The daemon socket when layer A is on.
    pub(crate) fn socket(&self) -> Option<&Path> {
        self.socket.as_deref()
    }

    /// The daemon socket when threads should be named (layers A and B on).
    pub(crate) fn naming_socket(&self) -> Option<&Path> {
        self.socket.as_deref().filter(|_| self.name_threads)
    }

    /// Arguments to append to a Codex launch. Arguments the caller already
    /// passed (`--remote`, `-C`/`--cd`) win.
    pub(crate) fn launch_args(&self, cwd: &Path, existing: &[String]) -> Vec<String> {
        let Some(socket) = &self.socket else {
            return Vec::new();
        };
        if existing.iter().any(|arg| arg == "--no-daemon") {
            return Vec::new();
        }
        let mut args = Vec::new();
        if !existing
            .iter()
            .any(|arg| arg == "--remote" || arg.starts_with("--remote="))
        {
            args.push("--remote".to_string());
            args.push(format!("unix://{}", socket.display()));
        }
        if !existing
            .iter()
            .any(|arg| arg == "-C" || arg == "--cd" || arg.starts_with("--cd="))
        {
            args.push("-C".to_string());
            args.push(cwd.display().to_string());
        }
        args
    }
}

fn expand_home(raw: &str) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match (raw.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ if raw == "~" => {
            std::env::var_os("HOME").map_or_else(|| PathBuf::from(raw), PathBuf::from)
        }
        _ => PathBuf::from(raw),
    }
}

pub(crate) fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

/// The creation time a UUIDv7 carries in its first 48 bits, in unix ms.
/// Codex thread ids are UUIDv7. Unlike `createdAt`, which the daemon refreshes
/// on threads that have no turns yet, it never changes.
fn uuid_v7_millis(id: &str) -> Option<i64> {
    let hex: String = id.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 || hex.as_bytes().get(12) != Some(&b'7') {
        return None;
    }
    i64::from_str_radix(hex.get(..12)?, 16).ok()
}

/// What to name, and how to find the thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NameJob {
    /// The thread id is known: a resumed pane (its saved session id) or a
    /// session the Codex hook reported.
    Known {
        thread_id: String,
        name: String,
        only_if_unnamed: bool,
    },
    /// A new pane: find the one unnamed thread created for it.
    Discover {
        cwd: PathBuf,
        /// Unix ms.
        launched_at: i64,
        name: String,
    },
    /// The agent was renamed.
    Rename {
        thread_id: Option<String>,
        cwd: PathBuf,
        /// The name herdr gave the thread before, if it named it.
        old_name: Option<String>,
        new_name: String,
        /// When the pane's Codex process started, in unix ms: the anchor for a
        /// thread herdr neither launched nor learned the id of.
        process_started_at: Option<i64>,
    },
}

/// The fields of `thread/read` that naming needs.
#[cfg(unix)]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ThreadSummary {
    pub id: String,
    pub name: Option<String>,
    pub cwd: Option<String>,
    pub ephemeral: bool,
    pub parent_thread_id: Option<String>,
    pub created_at: Option<i64>,
}

#[cfg(unix)]
impl ThreadSummary {
    fn from_read(value: &serde_json::Value) -> Option<Self> {
        let thread = value.get("thread")?;
        let text = |key: &str| {
            thread
                .get(key)
                .and_then(serde_json::Value::as_str)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
        };
        Some(Self {
            id: text("id")?,
            name: text("name"),
            cwd: text("cwd"),
            ephemeral: thread
                .get("ephemeral")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            parent_thread_id: text("parentThreadId"),
            created_at: thread.get("createdAt").and_then(serde_json::Value::as_i64),
        })
    }

    /// A thread a pane can own: top level and able to take a name.
    fn nameable(&self) -> bool {
        !self.ephemeral && self.parent_thread_id.is_none()
    }

    /// When the thread was created, in unix ms: from its UUIDv7 id, else
    /// from `createdAt`.
    fn created_ms(&self) -> Option<i64> {
        uuid_v7_millis(&self.id).or_else(|| self.created_at.map(|secs| secs.saturating_mul(1000)))
    }

    fn in_cwd(&self, cwd: &Path) -> bool {
        self.cwd
            .as_deref()
            .is_some_and(|thread_cwd| same_dir(Path::new(thread_cwd), cwd))
    }
}

#[cfg(unix)]
fn same_dir(left: &Path, right: &Path) -> bool {
    let canonical =
        |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    left == right || canonical(left) == canonical(right)
}

#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Pick {
    None,
    One(String),
    Ambiguous(usize),
}

/// The thread a newly launched pane created: the only unnamed, nameable one
/// in its cwd created since the launch. Several means a guess, so none.
#[cfg(unix)]
pub(crate) fn pick_new_thread(threads: &[ThreadSummary], cwd: &Path, launched_at_ms: i64) -> Pick {
    pick_unnamed_thread(threads, cwd, |created| {
        created >= launched_at_ms - CREATED_SLACK_MS
    })
}

/// The thread a Codex process herdr did not launch created: the only unnamed,
/// nameable one in its cwd created within `HAND_LAUNCH_WINDOW_MS` of the
/// process starting. Several means a guess, so none.
#[cfg(unix)]
pub(crate) fn pick_process_thread(
    threads: &[ThreadSummary],
    cwd: &Path,
    started_at_ms: i64,
) -> Pick {
    pick_unnamed_thread(threads, cwd, |created| {
        created >= started_at_ms - CREATED_SLACK_MS
            && created <= started_at_ms + HAND_LAUNCH_WINDOW_MS
    })
}

/// The thread id a `codex … resume <id> …` command line reopens.
pub(crate) fn resumed_thread_id(argv: &[String]) -> Option<String> {
    argv.windows(2)
        .find(|pair| pair[0] == "resume" && uuid_v7_millis(&pair[1]).is_some())
        .map(|pair| pair[1].clone())
}

#[cfg(unix)]
fn pick_unnamed_thread(
    threads: &[ThreadSummary],
    cwd: &Path,
    created_in_window: impl Fn(i64) -> bool,
) -> Pick {
    let candidates: Vec<_> = threads
        .iter()
        .filter(|thread| {
            thread.nameable()
                && thread.name.is_none()
                && thread.created_ms().is_some_and(&created_in_window)
                && thread.in_cwd(cwd)
        })
        .collect();
    match candidates.as_slice() {
        [] => Pick::None,
        [only] => Pick::One(only.id.clone()),
        many => Pick::Ambiguous(many.len()),
    }
}

/// Where to report the thread a naming job resolved for a pane.
pub(crate) struct ThreadReply {
    pub events: tokio::sync::mpsc::Sender<crate::events::AppEvent>,
    pub pane_id: crate::layout::PaneId,
}

/// Run a naming job on its own thread, so the app loop never waits on the
/// daemon. Failures are logged; they never affect the pane. With `reply`, the
/// thread the job resolved is reported back as the pane's Codex thread.
pub(crate) fn spawn_name_job(socket: PathBuf, job: NameJob, reply: Option<ThreadReply>) {
    let spawned = std::thread::Builder::new()
        .name("codex-thread-name".into())
        .spawn(move || {
            #[cfg(unix)]
            {
                let mut resolved = None;
                let result = client::run_name_job_resolving(&socket, &job, &mut resolved);
                if let (Some(reply), Some(thread_id)) = (reply, resolved) {
                    let event = crate::events::AppEvent::CodexThreadResolved {
                        pane_id: reply.pane_id,
                        thread_id,
                    };
                    if let Err(err) = reply.events.try_send(event) {
                        tracing::warn!(err = %err, "failed to report the resolved codex thread");
                    }
                }
                match result {
                    Ok(outcome) => {
                        tracing::info!(?job, %outcome, "codex thread naming finished");
                    }
                    Err(err) => {
                        tracing::warn!(?job, err = %err, "codex thread naming failed");
                    }
                }
            }
            #[cfg(not(unix))]
            {
                let _ = (&socket, &job);
                drop(reply.map(|reply| (reply.events, reply.pane_id)));
            }
        });
    if let Err(err) = spawned {
        tracing::warn!(err = %err, "failed to spawn codex thread naming");
    }
}

#[cfg(unix)]
mod client {
    use super::{
        pick_new_thread, pick_process_thread, NameJob, Pick, ThreadSummary, NAME_JOB_DEADLINE,
        NAME_JOB_POLL,
    };
    use base64::Engine as _;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::time::{Duration, Instant};

    const IO_TIMEOUT: Duration = Duration::from_secs(5);
    const MAX_HANDSHAKE_BYTES: usize = 16 * 1024;
    const MAX_MESSAGE_BYTES: u64 = 64 * 1024 * 1024;

    const OP_CONTINUATION: u8 = 0x0;
    const OP_TEXT: u8 = 0x1;
    const OP_CLOSE: u8 = 0x8;
    const OP_PING: u8 = 0x9;
    const OP_PONG: u8 = 0xA;

    #[derive(Debug)]
    pub(super) enum RpcError {
        Io(std::io::Error),
        Protocol(String),
        /// The daemon answered the call with an error.
        Call(String),
    }

    impl std::fmt::Display for RpcError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Io(err) => write!(f, "{err}"),
                Self::Protocol(message) => write!(f, "protocol error: {message}"),
                Self::Call(message) => write!(f, "{message}"),
            }
        }
    }

    impl From<std::io::Error> for RpcError {
        fn from(err: std::io::Error) -> Self {
            Self::Io(err)
        }
    }

    /// A blocking JSON-RPC client for the daemon's WebSocket endpoint
    /// (`ws://localhost/rpc`) on its unix control socket. Text frames only;
    /// notifications and server requests are read past.
    pub(super) struct Rpc {
        stream: UnixStream,
        buffer: Vec<u8>,
        next_id: u64,
        mask_seed: u64,
    }

    impl Rpc {
        pub(super) fn connect(socket: &Path) -> Result<Self, RpcError> {
            let stream = UnixStream::connect(socket)?;
            stream.set_read_timeout(Some(IO_TIMEOUT))?;
            stream.set_write_timeout(Some(IO_TIMEOUT))?;
            let mask_seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0x9e37_79b9_7f4a_7c15, |elapsed| {
                    elapsed.as_nanos() as u64 ^ u64::from(std::process::id())
                });
            let mut rpc = Self {
                stream,
                buffer: Vec::new(),
                next_id: 0,
                mask_seed,
            };
            rpc.handshake()?;
            rpc.call(
                "initialize",
                serde_json::json!({
                    "clientInfo": {"name": "herdr", "version": crate::build_info::version(), "title": null},
                    "capabilities": null,
                }),
            )?;
            rpc.send_text(&serde_json::json!({"method": "initialized"}).to_string())?;
            Ok(rpc)
        }

        fn next_random(&mut self) -> u64 {
            // xorshift: masking keys only have to vary, not be secret, on a
            // local socket.
            let mut x = self.mask_seed;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.mask_seed = x;
            x
        }

        fn handshake(&mut self) -> Result<(), RpcError> {
            let mut key = [0u8; 16];
            key[..8].copy_from_slice(&self.next_random().to_le_bytes());
            key[8..].copy_from_slice(&self.next_random().to_le_bytes());
            let key = base64::engine::general_purpose::STANDARD.encode(key);
            let request = format!(
                "GET /rpc HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
            );
            self.stream.write_all(request.as_bytes())?;
            loop {
                if let Some(end) = find_subslice(&self.buffer, b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&self.buffer[..end]).to_string();
                    self.buffer.drain(..end + 4);
                    let status = head.lines().next().unwrap_or_default();
                    if status.split_whitespace().nth(1) != Some("101") {
                        return Err(RpcError::Protocol(format!("upgrade refused: {status}")));
                    }
                    return Ok(());
                }
                if self.buffer.len() > MAX_HANDSHAKE_BYTES {
                    return Err(RpcError::Protocol("oversized upgrade response".into()));
                }
                self.fill()?;
            }
        }

        fn fill(&mut self) -> Result<(), RpcError> {
            let mut chunk = [0u8; 8192];
            let read = self.stream.read(&mut chunk)?;
            if read == 0 {
                return Err(RpcError::Protocol("daemon closed the connection".into()));
            }
            self.buffer.extend_from_slice(&chunk[..read]);
            Ok(())
        }

        fn take(&mut self, count: usize) -> Result<Vec<u8>, RpcError> {
            while self.buffer.len() < count {
                self.fill()?;
            }
            Ok(self.buffer.drain(..count).collect())
        }

        fn send_frame(&mut self, opcode: u8, payload: &[u8]) -> Result<(), RpcError> {
            let mut frame = Vec::with_capacity(payload.len() + 14);
            frame.push(0x80 | opcode);
            match payload.len() {
                len if len < 126 => frame.push(0x80 | len as u8),
                len if len <= usize::from(u16::MAX) => {
                    frame.push(0x80 | 126);
                    frame.extend_from_slice(&(len as u16).to_be_bytes());
                }
                len => {
                    frame.push(0x80 | 127);
                    frame.extend_from_slice(&(len as u64).to_be_bytes());
                }
            }
            let mask = (self.next_random() as u32).to_be_bytes();
            frame.extend_from_slice(&mask);
            frame.extend(
                payload
                    .iter()
                    .enumerate()
                    .map(|(index, byte)| byte ^ mask[index % 4]),
            );
            self.stream.write_all(&frame)?;
            Ok(())
        }

        fn send_text(&mut self, text: &str) -> Result<(), RpcError> {
            self.send_frame(OP_TEXT, text.as_bytes())
        }

        fn read_frame(&mut self) -> Result<(bool, u8, Vec<u8>), RpcError> {
            let head = self.take(2)?;
            let fin = head[0] & 0x80 != 0;
            let opcode = head[0] & 0x0F;
            let masked = head[1] & 0x80 != 0;
            let len = match head[1] & 0x7F {
                126 => {
                    let bytes = self.take(2)?;
                    u64::from(u16::from_be_bytes([bytes[0], bytes[1]]))
                }
                127 => {
                    let bytes = self.take(8)?;
                    u64::from_be_bytes(
                        bytes
                            .try_into()
                            .map_err(|_| RpcError::Protocol("short extended length".into()))?,
                    )
                }
                short => u64::from(short),
            };
            if len > MAX_MESSAGE_BYTES {
                return Err(RpcError::Protocol(format!("frame of {len} bytes")));
            }
            let mask = if masked { Some(self.take(4)?) } else { None };
            let mut payload = self.take(len as usize)?;
            if let Some(mask) = mask {
                for (index, byte) in payload.iter_mut().enumerate() {
                    *byte ^= mask[index % 4];
                }
            }
            Ok((fin, opcode, payload))
        }

        fn read_message(&mut self) -> Result<String, RpcError> {
            let mut message = Vec::new();
            loop {
                let (fin, opcode, payload) = self.read_frame()?;
                match opcode {
                    OP_TEXT | OP_CONTINUATION => {
                        message.extend_from_slice(&payload);
                        if fin {
                            return String::from_utf8(message)
                                .map_err(|_| RpcError::Protocol("non-UTF-8 text".into()));
                        }
                    }
                    OP_PING => self.send_frame(OP_PONG, &payload)?,
                    OP_PONG => {}
                    OP_CLOSE => {
                        return Err(RpcError::Protocol("daemon closed the connection".into()))
                    }
                    other => return Err(RpcError::Protocol(format!("unexpected opcode {other}"))),
                }
            }
        }

        pub(super) fn call(
            &mut self,
            method: &str,
            params: serde_json::Value,
        ) -> Result<serde_json::Value, RpcError> {
            self.next_id += 1;
            let id = self.next_id;
            self.send_text(
                &serde_json::json!({"id": id, "method": method, "params": params}).to_string(),
            )?;
            loop {
                let message = self.read_message()?;
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&message) else {
                    continue;
                };
                if value.get("method").is_some() || value.get("id") != Some(&serde_json::json!(id))
                {
                    continue;
                }
                if let Some(error) = value.get("error") {
                    let message = error
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .map_or_else(|| error.to_string(), str::to_string);
                    return Err(RpcError::Call(format!("{method}: {message}")));
                }
                return Ok(value
                    .get("result")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null));
            }
        }

        pub(super) fn read_thread(&mut self, id: &str) -> Result<ThreadSummary, RpcError> {
            let result = self.call("thread/read", serde_json::json!({"threadId": id}))?;
            ThreadSummary::from_read(&result)
                .ok_or_else(|| RpcError::Protocol(format!("thread/read {id}: no thread")))
        }

        pub(super) fn loaded_threads(&mut self) -> Result<Vec<ThreadSummary>, RpcError> {
            let result = self.call("thread/loaded/list", serde_json::json!({}))?;
            let ids: Vec<String> = result
                .get("data")
                .and_then(serde_json::Value::as_array)
                .map(|ids| {
                    ids.iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let mut threads = Vec::with_capacity(ids.len());
            for id in ids {
                // A thread can unload between the list and the read.
                if let Ok(thread) = self.read_thread(&id) {
                    threads.push(thread);
                }
            }
            Ok(threads)
        }

        pub(super) fn set_name(&mut self, id: &str, name: &str) -> Result<(), RpcError> {
            self.call(
                "thread/name/set",
                serde_json::json!({"threadId": id, "name": name}),
            )
            .map(|_| ())
        }
    }

    fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }

    /// Name a thread unless it cannot take a name. A thread that carries the
    /// name afterwards is the pane's, and goes into `resolved`.
    fn name_thread(
        rpc: &mut Rpc,
        thread: &ThreadSummary,
        name: &str,
        resolved: &mut Option<String>,
    ) -> Result<String, RpcError> {
        if !thread.nameable() {
            return Ok(format!("skipped {}: ephemeral or a sub-agent", thread.id));
        }
        if thread.name.as_deref() == Some(name) {
            *resolved = Some(thread.id.clone());
            return Ok(format!("{} already named {name}", thread.id));
        }
        match rpc.set_name(&thread.id, name) {
            Ok(()) => {
                *resolved = Some(thread.id.clone());
                Ok(format!("named {} {name}", thread.id))
            }
            // Ephemeral threads refuse metadata updates; not an error for us.
            Err(RpcError::Call(message)) if message.contains("metadata updates") => {
                Ok(format!("skipped {}: {message}", thread.id))
            }
            Err(err) => Err(err),
        }
    }

    #[cfg(test)]
    pub(super) fn run_name_job(socket: &Path, job: &NameJob) -> Result<String, RpcError> {
        run_name_job_resolving(socket, job, &mut None)
    }

    /// Run `job`; the thread it named, or found already named, goes into
    /// `resolved`.
    pub(super) fn run_name_job_resolving(
        socket: &Path,
        job: &NameJob,
        resolved: &mut Option<String>,
    ) -> Result<String, RpcError> {
        let deadline = Instant::now() + NAME_JOB_DEADLINE;
        let mut rpc = Rpc::connect(socket)?;
        loop {
            let done = match job {
                NameJob::Known {
                    thread_id,
                    name,
                    only_if_unnamed,
                } => match rpc.read_thread(thread_id) {
                    Ok(thread) if *only_if_unnamed && thread.name.is_some() => {
                        Some(Ok(format!("{thread_id} already named")))
                    }
                    Ok(thread) => Some(name_thread(&mut rpc, &thread, name, resolved)),
                    // Not loaded yet: a resume attaches after the shell starts it.
                    Err(RpcError::Call(_)) => None,
                    Err(err) => Some(Err(err)),
                },
                NameJob::Discover {
                    cwd,
                    launched_at,
                    name,
                } => {
                    let threads = rpc.loaded_threads()?;
                    match pick_new_thread(&threads, cwd, *launched_at) {
                        Pick::One(id) => {
                            let thread = threads
                                .into_iter()
                                .find(|thread| thread.id == id)
                                .unwrap_or_default();
                            Some(name_thread(&mut rpc, &thread, name, resolved))
                        }
                        Pick::Ambiguous(count) => Some(Ok(format!(
                            "{count} new threads in {}; naming none until the session is reported",
                            cwd.display()
                        ))),
                        Pick::None => None,
                    }
                }
                NameJob::Rename {
                    thread_id: Some(id),
                    new_name,
                    ..
                } => Some(match rpc.read_thread(id) {
                    Ok(thread) => name_thread(&mut rpc, &thread, new_name, resolved),
                    Err(_) => Ok(format!("thread {id} is not on the daemon")),
                }),
                NameJob::Rename {
                    thread_id: None,
                    cwd,
                    old_name,
                    new_name,
                    process_started_at,
                } => {
                    let threads = rpc.loaded_threads()?;
                    let by_old_name = old_name.as_deref().and_then(|old_name| {
                        threads.iter().find(|thread| {
                            thread.name.as_deref() == Some(old_name) && thread.in_cwd(cwd)
                        })
                    });
                    Some(match (by_old_name, process_started_at) {
                        (Some(thread), _) => {
                            name_thread(&mut rpc, &thread.clone(), new_name, resolved)
                        }
                        (None, Some(started_at)) => {
                            match pick_process_thread(&threads, cwd, *started_at) {
                                Pick::One(id) => {
                                    let thread = threads
                                        .into_iter()
                                        .find(|thread| thread.id == id)
                                        .unwrap_or_default();
                                    name_thread(&mut rpc, &thread, new_name, resolved)
                                }
                                Pick::Ambiguous(count) => Ok(format!(
                                    "{count} unnamed threads in {} started with the pane's codex; naming none",
                                    cwd.display()
                                )),
                                Pick::None => Ok(format!(
                                    "no unnamed thread in {} started with the pane's codex",
                                    cwd.display()
                                )),
                            }
                        }
                        (None, None) => Ok(format!(
                            "no thread named {} in {}",
                            old_name.as_deref().unwrap_or("(none)"),
                            cwd.display()
                        )),
                    })
                }
            };
            if let Some(outcome) = done {
                return outcome;
            }
            if Instant::now() >= deadline {
                return Ok("thread did not appear before the deadline".into());
            }
            std::thread::sleep(NAME_JOB_POLL);
        }
    }

    #[cfg(test)]
    pub(super) mod fake_daemon {
        //! A WebSocket JSON-RPC server on a unix socket that answers from a
        //! closure, for tests.
        use super::find_subslice;
        use std::io::{Read, Write};
        use std::os::unix::net::UnixListener;
        use std::path::{Path, PathBuf};

        pub(crate) fn serve(
            dir: &Path,
            answer: impl Fn(&str, &serde_json::Value) -> serde_json::Value + Send + 'static,
        ) -> (
            PathBuf,
            std::sync::mpsc::Receiver<(String, serde_json::Value)>,
        ) {
            let path = dir.join("daemon.sock");
            let listener = UnixListener::bind(&path).expect("bind fake daemon");
            let (calls_tx, calls_rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { return };
                    let mut buffer = Vec::new();
                    let mut chunk = [0u8; 4096];
                    while find_subslice(&buffer, b"\r\n\r\n").is_none() {
                        let read = stream.read(&mut chunk).unwrap_or(0);
                        if read == 0 {
                            break;
                        }
                        buffer.extend_from_slice(&chunk[..read]);
                    }
                    let end = find_subslice(&buffer, b"\r\n\r\n").map_or(buffer.len(), |e| e + 4);
                    buffer.drain(..end);
                    let _ = stream.write_all(
                        b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n",
                    );
                    // Unsolicited notification first: the client must read past it.
                    write_text(&mut stream, r#"{"method":"thread/started","params":{}}"#);
                    while let Some(text) = read_text(&mut stream, &mut buffer) {
                        let Ok(request) = serde_json::from_str::<serde_json::Value>(&text) else {
                            continue;
                        };
                        let Some(id) = request.get("id").cloned() else {
                            continue;
                        };
                        let method = request["method"].as_str().unwrap_or_default().to_string();
                        let params = request["params"].clone();
                        let _ = calls_tx.send((method.clone(), params.clone()));
                        let mut response = answer(&method, &params);
                        response["id"] = id;
                        write_text(&mut stream, &response.to_string());
                    }
                }
            });
            (path, calls_rx)
        }

        fn write_text(stream: &mut std::os::unix::net::UnixStream, text: &str) {
            let payload = text.as_bytes();
            let mut frame = vec![0x81];
            if payload.len() < 126 {
                frame.push(payload.len() as u8);
            } else {
                frame.push(126);
                frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
            }
            frame.extend_from_slice(payload);
            let _ = stream.write_all(&frame);
        }

        fn read_text(
            stream: &mut std::os::unix::net::UnixStream,
            buffer: &mut Vec<u8>,
        ) -> Option<String> {
            let mut need = |buffer: &mut Vec<u8>, count: usize| -> Option<Vec<u8>> {
                let mut chunk = [0u8; 4096];
                while buffer.len() < count {
                    let read = stream.read(&mut chunk).ok()?;
                    if read == 0 {
                        return None;
                    }
                    buffer.extend_from_slice(&chunk[..read]);
                }
                Some(buffer.drain(..count).collect())
            };
            let head = need(buffer, 2)?;
            assert_eq!(head[1] & 0x80, 0x80, "client frames must be masked");
            let len = match head[1] & 0x7F {
                126 => {
                    let bytes = need(buffer, 2)?;
                    usize::from(u16::from_be_bytes([bytes[0], bytes[1]]))
                }
                127 => {
                    let bytes = need(buffer, 8)?;
                    u64::from_be_bytes(bytes.try_into().ok()?) as usize
                }
                short => usize::from(short),
            };
            let mask = need(buffer, 4)?;
            let payload: Vec<u8> = need(buffer, len)?
                .iter()
                .enumerate()
                .map(|(index, byte)| byte ^ mask[index % 4])
                .collect();
            String::from_utf8(payload).ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(app_server: bool, name_threads: bool) -> CodexAppServer {
        CodexAppServer::from_config(&crate::config::CodexAgentConfig {
            app_server,
            app_server_socket: "/run/codex.sock".into(),
            name_threads,
        })
    }

    #[test]
    fn launch_args_are_empty_when_off() {
        let off = settings(false, true);
        assert!(off.launch_args(Path::new("/repo"), &[]).is_empty());
        assert!(off.naming_socket().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn launch_args_add_the_daemon_and_the_pane_cwd() {
        assert_eq!(
            settings(true, false).launch_args(Path::new("/repo"), &[]),
            ["--remote", "unix:///run/codex.sock", "-C", "/repo"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_codex_started_off_the_daemon_is_restored_off_it() {
        // A hand-started `codex --no-daemon` keeps that choice when restore
        // gives it back its launch flags (fork issue 127).
        assert!(settings(true, false)
            .launch_args(Path::new("/repo"), &["--no-daemon".into()])
            .is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn launch_args_leave_caller_arguments_alone() {
        let on = settings(true, false);
        assert_eq!(
            on.launch_args(Path::new("/repo"), &["--remote".into(), "ws://x".into()]),
            ["-C", "/repo"]
        );
        assert_eq!(
            on.launch_args(Path::new("/repo"), &["--cd=/elsewhere".into()]),
            ["--remote", "unix:///run/codex.sock"]
        );
        assert!(on
            .launch_args(
                Path::new("/repo"),
                &["--remote=unix:///x".into(), "-C".into(), "/y".into()]
            )
            .is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn naming_needs_both_layers() {
        assert!(settings(true, false).naming_socket().is_none());
        assert_eq!(
            settings(true, true).naming_socket(),
            Some(Path::new("/run/codex.sock"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn home_is_expanded_in_the_socket_path() {
        let home = std::env::var("HOME").expect("HOME");
        assert_eq!(
            expand_home("~/.codex/x.sock"),
            Path::new(&home).join(".codex/x.sock")
        );
        assert_eq!(expand_home("/abs.sock"), Path::new("/abs.sock"));
    }

    /// A non-UUID id, so creation time comes from `createdAt` (seconds).
    #[cfg(unix)]
    fn thread(id: &str, cwd: &str, created_at: i64) -> ThreadSummary {
        ThreadSummary {
            id: id.into(),
            cwd: Some(cwd.into()),
            created_at: Some(created_at),
            ..ThreadSummary::default()
        }
    }

    #[cfg(unix)]
    #[test]
    fn pick_new_thread_takes_the_single_candidate() {
        let threads = [
            thread("old", "/repo", 50),
            thread("elsewhere", "/other", 100),
            ThreadSummary {
                name: Some("taken".into()),
                ..thread("named", "/repo", 100)
            },
            ThreadSummary {
                ephemeral: true,
                ..thread("ephemeral", "/repo", 100)
            },
            ThreadSummary {
                parent_thread_id: Some("parent".into()),
                ..thread("subagent", "/repo", 100)
            },
            thread("mine", "/repo", 100),
        ];
        assert_eq!(
            pick_new_thread(&threads, Path::new("/repo"), 100_000),
            Pick::One("mine".into())
        );
    }

    #[cfg(unix)]
    #[test]
    fn pick_process_thread_keeps_to_the_window_after_the_process_started() {
        let threads = [
            thread("before", "/repo", 90),
            thread("mine", "/repo", 101),
            thread("later", "/repo", 131),
        ];
        assert_eq!(
            pick_process_thread(&threads, Path::new("/repo"), 100_000),
            Pick::One("mine".into())
        );
    }

    #[cfg(unix)]
    #[test]
    fn pick_process_thread_refuses_to_guess() {
        let threads = [thread("a", "/repo", 101), thread("b", "/repo", 110)];
        assert_eq!(
            pick_process_thread(&threads, Path::new("/repo"), 100_000),
            Pick::Ambiguous(2)
        );
    }

    #[test]
    fn resumed_thread_id_is_read_from_the_command_line() {
        let argv = |parts: &[&str]| {
            parts
                .iter()
                .map(|part| part.to_string())
                .collect::<Vec<_>>()
        };
        let id = "01a0e4f8-16fa-78f1-b2a4-fe126b57b4b2";
        assert_eq!(
            resumed_thread_id(&argv(&["codex", "--remote", "unix:///s", "resume", id])).as_deref(),
            Some(id)
        );
        assert_eq!(
            resumed_thread_id(&argv(&["codex", "resume", id, "-C", "/repo"])).as_deref(),
            Some(id)
        );
        assert_eq!(resumed_thread_id(&argv(&["codex", "-C", "/repo"])), None);
        assert_eq!(
            resumed_thread_id(&argv(&["codex", "resume", "--last"])),
            None,
            "only an id names the thread"
        );
    }

    #[cfg(unix)]
    #[test]
    fn pick_new_thread_refuses_to_guess() {
        let threads = [thread("a", "/repo", 100), thread("b", "/repo", 101)];
        assert_eq!(
            pick_new_thread(&threads, Path::new("/repo"), 100_000),
            Pick::Ambiguous(2)
        );
        assert_eq!(
            pick_new_thread(&[], Path::new("/repo"), 100_000),
            Pick::None
        );
    }

    #[cfg(unix)]
    #[test]
    fn uuid_v7_time_wins_over_a_refreshed_created_at() {
        // Measured on the daemon: an old thread with no turns had its
        // createdAt refreshed to the moment a new pane launched, and was
        // named in the new pane's place. Its UUIDv7 still says when it was
        // really created.
        let stale = ThreadSummary {
            id: "01a0e489-2067-77b0-bf80-101afa6d8900".into(),
            cwd: Some("/repo".into()),
            created_at: Some(1_790_541_454),
            ..ThreadSummary::default()
        };
        let mine = ThreadSummary {
            id: "01a0e496-6048-7a50-8648-6957cba43399".into(),
            cwd: Some("/repo".into()),
            created_at: Some(1_790_541_455),
            ..ThreadSummary::default()
        };
        assert_eq!(uuid_v7_millis(&stale.id), Some(1_790_540_587_111));
        assert_eq!(uuid_v7_millis("not-a-uuid"), None);
        assert_eq!(
            pick_new_thread(
                std::slice::from_ref(&stale),
                Path::new("/repo"),
                1_790_541_454_000
            ),
            Pick::None,
            "a stale thread is not the new pane's even with a fresh createdAt"
        );
        assert_eq!(
            pick_new_thread(&[stale, mine], Path::new("/repo"), 1_790_541_454_000),
            Pick::One("01a0e496-6048-7a50-8648-6957cba43399".into())
        );
    }

    #[cfg(unix)]
    #[test]
    fn thread_summary_reads_the_daemon_shape() {
        let read = serde_json::json!({"thread": {
            "id": "t1", "name": null, "cwd": "/repo", "ephemeral": false,
            "parentThreadId": null, "createdAt": 1790540622, "turns": []
        }});
        assert_eq!(
            ThreadSummary::from_read(&read),
            Some(ThreadSummary {
                id: "t1".into(),
                name: None,
                cwd: Some("/repo".into()),
                ephemeral: false,
                parent_thread_id: None,
                created_at: Some(1790540622),
            })
        );
    }

    #[cfg(unix)]
    fn temp_dir(tag: &str) -> PathBuf {
        // Short on purpose: macOS caps unix socket paths at 104 bytes, and
        // the default temp directory is already most of that.
        let dir = PathBuf::from("/tmp").join(format!("hcx-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[cfg(unix)]
    fn loaded_daemon(
        dir: &Path,
        threads: Vec<serde_json::Value>,
    ) -> (
        PathBuf,
        std::sync::mpsc::Receiver<(String, serde_json::Value)>,
    ) {
        client::fake_daemon::serve(dir, move |method, params| {
            match method {
            "initialize" => serde_json::json!({"result": {}}),
            "thread/loaded/list" => serde_json::json!({"result": {
                "data": threads.iter().map(|t| t["id"].clone()).collect::<Vec<_>>(),
                "nextCursor": null
            }}),
            "thread/read" => threads
                .iter()
                .find(|t| t["id"] == params["threadId"])
                .map_or_else(
                    || serde_json::json!({"error": {"code": -32600, "message": "thread not loaded"}}),
                    |t| serde_json::json!({"result": {"thread": t}}),
                ),
            "thread/name/set" => {
                let ephemeral = threads
                    .iter()
                    .find(|t| t["id"] == params["threadId"])
                    .is_some_and(|t| t["ephemeral"] == true);
                if ephemeral {
                    serde_json::json!({"error": {"code": -32600, "message": "thread does not support metadata updates"}})
                } else {
                    serde_json::json!({"result": {}})
                }
            }
            _ => serde_json::json!({"error": {"code": -32601, "message": "unknown method"}}),
        }
        })
    }

    #[cfg(unix)]
    fn name_calls(
        calls: &std::sync::mpsc::Receiver<(String, serde_json::Value)>,
    ) -> Vec<serde_json::Value> {
        calls
            .try_iter()
            .filter(|(method, _)| method == "thread/name/set")
            .map(|(_, params)| params)
            .collect()
    }

    #[cfg(unix)]
    #[test]
    fn discover_names_the_new_thread_over_the_websocket() {
        let dir = temp_dir("discover");
        let (socket, calls) = loaded_daemon(
            &dir,
            vec![
                serde_json::json!({"id": "old", "cwd": "/repo", "ephemeral": false, "createdAt": 10}),
                serde_json::json!({"id": "new", "cwd": "/repo", "ephemeral": false, "createdAt": 100}),
            ],
        );
        let outcome = client::run_name_job(
            &socket,
            &NameJob::Discover {
                cwd: "/repo".into(),
                launched_at: 100_000,
                name: "worker".into(),
            },
        )
        .expect("naming job");
        assert!(outcome.starts_with("named new"), "{outcome}");
        assert_eq!(
            name_calls(&calls),
            [serde_json::json!({"threadId": "new", "name": "worker"})]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn known_ephemeral_thread_is_skipped_without_error() {
        let dir = temp_dir("ephemeral");
        let (socket, calls) = loaded_daemon(
            &dir,
            vec![
                serde_json::json!({"id": "eph", "cwd": "/repo", "ephemeral": true, "createdAt": 1}),
            ],
        );
        let outcome = client::run_name_job(
            &socket,
            &NameJob::Known {
                thread_id: "eph".into(),
                name: "worker".into(),
                only_if_unnamed: false,
            },
        )
        .expect("an ephemeral thread is not an error");
        assert!(outcome.starts_with("skipped eph"), "{outcome}");
        assert!(name_calls(&calls).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn rename_finds_the_thread_by_its_old_name() {
        let dir = temp_dir("rename");
        let (socket, calls) = loaded_daemon(
            &dir,
            vec![
                serde_json::json!({"id": "t1", "name": "old", "cwd": "/repo", "ephemeral": false, "createdAt": 1}),
                serde_json::json!({"id": "t2", "name": "old", "cwd": "/other", "ephemeral": false, "createdAt": 1}),
            ],
        );
        client::run_name_job(
            &socket,
            &NameJob::Rename {
                thread_id: None,
                cwd: "/repo".into(),
                old_name: Some("old".into()),
                new_name: "new".into(),
                process_started_at: None,
            },
        )
        .expect("rename job");
        assert_eq!(
            name_calls(&calls),
            [serde_json::json!({"threadId": "t1", "name": "new"})]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn rename_of_a_hand_launched_pane_finds_the_thread_its_process_created() {
        let dir = temp_dir("hand-launch");
        let (socket, calls) = loaded_daemon(
            &dir,
            vec![
                // Before the process started: someone else's.
                serde_json::json!({"id": "before", "cwd": "/repo", "ephemeral": false, "createdAt": 90}),
                serde_json::json!({"id": "mine", "cwd": "/repo", "ephemeral": false, "createdAt": 102}),
                // Long after: a later launch's.
                serde_json::json!({"id": "later", "cwd": "/repo", "ephemeral": false, "createdAt": 200}),
            ],
        );
        let outcome = client::run_name_job(
            &socket,
            &NameJob::Rename {
                thread_id: None,
                cwd: "/repo".into(),
                old_name: None,
                new_name: "worker".into(),
                process_started_at: Some(100_000),
            },
        )
        .expect("rename job");
        assert!(outcome.starts_with("named mine"), "{outcome}");
        assert_eq!(
            name_calls(&calls),
            [serde_json::json!({"threadId": "mine", "name": "worker"})]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_naming_job_reports_the_thread_it_named_and_nothing_else() {
        let dir = temp_dir("resolved");
        let (socket, _calls) = loaded_daemon(
            &dir,
            vec![
                serde_json::json!({"id": "mine", "cwd": "/repo", "ephemeral": false, "createdAt": 102}),
            ],
        );
        let mut resolved = None;
        client::run_name_job_resolving(
            &socket,
            &NameJob::Rename {
                thread_id: None,
                cwd: "/repo".into(),
                old_name: None,
                new_name: "worker".into(),
                process_started_at: Some(100_000),
            },
            &mut resolved,
        )
        .expect("rename job");
        assert_eq!(resolved.as_deref(), Some("mine"));

        let mut unresolved = None;
        client::run_name_job_resolving(
            &socket,
            &NameJob::Rename {
                thread_id: None,
                cwd: "/elsewhere".into(),
                old_name: None,
                new_name: "worker".into(),
                process_started_at: Some(100_000),
            },
            &mut unresolved,
        )
        .expect("rename job");
        assert_eq!(unresolved, None, "no thread found, none reported");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn rename_without_a_known_thread_or_process_names_nothing() {
        let dir = temp_dir("rename-none");
        let (socket, calls) = loaded_daemon(
            &dir,
            vec![
                serde_json::json!({"id": "t1", "cwd": "/repo", "ephemeral": false, "createdAt": 1}),
            ],
        );
        let outcome = client::run_name_job(
            &socket,
            &NameJob::Rename {
                thread_id: None,
                cwd: "/repo".into(),
                old_name: None,
                new_name: "worker".into(),
                process_started_at: None,
            },
        )
        .expect("rename job");
        assert!(outcome.starts_with("no thread named"), "{outcome}");
        assert!(name_calls(&calls).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn known_named_thread_is_left_alone_when_only_unnamed_is_asked() {
        let dir = temp_dir("only-unnamed");
        let (socket, calls) = loaded_daemon(
            &dir,
            vec![
                serde_json::json!({"id": "t1", "name": "someone", "cwd": "/repo", "ephemeral": false, "createdAt": 1}),
            ],
        );
        client::run_name_job(
            &socket,
            &NameJob::Known {
                thread_id: "t1".into(),
                name: "worker".into(),
                only_if_unnamed: true,
            },
        )
        .expect("naming job");
        assert!(name_calls(&calls).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
