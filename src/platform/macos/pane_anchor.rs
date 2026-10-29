//! The pane anchor: a long-lived process that starts pane programs for the
//! server (fork issue 246).
//!
//! macOS keeps a privacy (TCC) answer only while the requesting process's
//! responsible process lives. A pane spawned by the server is attributed to the
//! server, which a live handoff replaces, so every upgrade made each open pane
//! ask again. The anchor is started with responsibility disclaimed (as the
//! handoff server is, issue 240) and outlives handoffs, so the panes it starts
//! keep one live responsible process across upgrades.
//!
//! The server keeps the PTY: it sends the anchor the command and the slave
//! descriptor, the anchor starts the program on it with the PTY library's own
//! setup, and reports the pid and, later, the exit status.

use std::collections::{HashMap, VecDeque};
use std::io::{self, BufRead, BufReader, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use portable_pty::{Child, ChildKiller, CommandBuilder, ExitStatus};
use serde::{Deserialize, Serialize};

const PROTOCOL: u32 = 1;
pub(crate) const SUBCOMMAND: &str = "__pane-anchor";
/// Appended to the API socket's name, so each session has its own anchor.
const SOCKET_SUFFIX: &str = ".pane-anchor";
/// The anchor ends after this long with no child and no server.
const IDLE_EXIT: Duration = Duration::from_secs(10);
/// After a failed start, panes are spawned directly for this long before
/// the anchor is tried again.
const RETRY_AFTER: Duration = Duration::from_secs(60);
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_LINE: usize = 4 * 1024 * 1024;
/// `sun_path` holds 104 bytes on macOS, the terminating NUL included.
const MAX_SOCKET_PATH: usize = 103;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Request {
    Hello {
        version: u32,
    },
    Spawn {
        id: u64,
        #[serde(with = "command_codec")]
        command: CommandBuilder,
    },
}

/// The command's environment is keyed by `OsString`, which JSON cannot key a
/// map by, so the command travels as base64 of its bincode encoding.
mod command_codec {
    use base64::Engine;
    use portable_pty::CommandBuilder;
    use serde::{de, ser, Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(
        command: &CommandBuilder,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let bytes = bincode::serde::encode_to_vec(command, bincode::config::standard())
            .map_err(ser::Error::custom)?;
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<CommandBuilder, D::Error> {
        let text = String::deserialize(deserializer)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(text)
            .map_err(de::Error::custom)?;
        bincode::serde::decode_from_slice(&bytes, bincode::config::standard())
            .map(|(command, _)| command)
            .map_err(de::Error::custom)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Reply {
    Hello {
        version: u32,
    },
    Spawned {
        id: u64,
        pid: u32,
    },
    Failed {
        id: u64,
        error: String,
    },
    Exited {
        pid: u32,
        code: u32,
        signal: Option<String>,
    },
    /// The anchor could not read the request (a server of another build): the
    /// server spawns the pane itself.
    Refused {
        id: u64,
        error: String,
    },
}

fn encode<T: Serialize>(message: &T) -> io::Result<Vec<u8>> {
    let mut line = serde_json::to_vec(message).map_err(io::Error::other)?;
    line.push(b'\n');
    Ok(line)
}

fn write_reply(writer: &Mutex<UnixStream>, reply: &Reply) -> io::Result<()> {
    let line = encode(reply)?;
    let mut stream = writer
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    stream.write_all(&line)
}

/// Sends `bytes` with `fd` attached to the first segment.
fn send_with_fd(stream: &UnixStream, bytes: &[u8], fd: RawFd) -> io::Result<()> {
    let mut iov = libc::iovec {
        iov_base: bytes.as_ptr() as *mut libc::c_void,
        iov_len: bytes.len(),
    };
    let space = unsafe { libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) } as usize;
    let mut control = vec![0u8; space];
    let mut header: libc::msghdr = unsafe { std::mem::zeroed() };
    header.msg_iov = &mut iov;
    header.msg_iovlen = 1;
    header.msg_control = control.as_mut_ptr().cast();
    header.msg_controllen = space as _;
    // SAFETY: the control buffer holds exactly one descriptor-sized message.
    unsafe {
        let cmsg = libc::CMSG_FIRSTHDR(&header);
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<RawFd>() as u32) as _;
        std::ptr::write_unaligned(libc::CMSG_DATA(cmsg).cast::<RawFd>(), fd);
    }
    let sent = loop {
        let sent = unsafe { libc::sendmsg(stream.as_raw_fd(), &header, 0) };
        if sent >= 0 {
            break sent as usize;
        }
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::Interrupted {
            return Err(err);
        }
    };
    let mut rest = &bytes[sent..];
    let mut stream = stream;
    while !rest.is_empty() {
        let written = stream.write(rest)?;
        if written == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        rest = &rest[written..];
    }
    Ok(())
}

/// Reads into `buf`, queuing any received descriptors.
fn recv_with_fds(
    stream: &UnixStream,
    buf: &mut [u8],
    fds: &mut VecDeque<OwnedFd>,
) -> io::Result<usize> {
    let mut iov = libc::iovec {
        iov_base: buf.as_mut_ptr().cast(),
        iov_len: buf.len(),
    };
    let space = unsafe { libc::CMSG_SPACE((8 * std::mem::size_of::<RawFd>()) as u32) } as usize;
    let mut control = vec![0u8; space];
    let mut header: libc::msghdr = unsafe { std::mem::zeroed() };
    header.msg_iov = &mut iov;
    header.msg_iovlen = 1;
    header.msg_control = control.as_mut_ptr().cast();
    header.msg_controllen = space as _;
    let received = loop {
        let received = unsafe { libc::recvmsg(stream.as_raw_fd(), &mut header, 0) };
        if received >= 0 {
            break received as usize;
        }
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::Interrupted {
            return Err(err);
        }
    };
    // SAFETY: walks the control messages recvmsg filled in.
    unsafe {
        let mut cmsg = libc::CMSG_FIRSTHDR(&header);
        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                let data = libc::CMSG_DATA(cmsg);
                let len = (*cmsg).cmsg_len as usize - (data as usize - cmsg as usize);
                for index in 0..len / std::mem::size_of::<RawFd>() {
                    let fd = std::ptr::read_unaligned(data.cast::<RawFd>().add(index));
                    fds.push_back(OwnedFd::from_raw_fd(fd));
                }
            }
            cmsg = libc::CMSG_NXTHDR(&header, cmsg);
        }
    }
    Ok(received)
}

/// The anchor socket of the session this server serves.
fn socket_path() -> PathBuf {
    let api = crate::session::active_api_socket_path();
    let name = api
        .file_name()
        .map(|name| format!("{}{SOCKET_SUFFIX}", name.to_string_lossy()))
        .unwrap_or_else(|| format!("herdr.sock{SOCKET_SUFFIX}"));
    api.with_file_name(name)
}

// ----- the anchor process -----

struct AnchorState {
    children: AtomicUsize,
    connections: AtomicUsize,
}

/// `herdr __pane-anchor --socket <path>`.
pub(crate) fn run(args: &[String]) -> io::Result<()> {
    let socket = match args {
        [flag, path] if flag == "--socket" => PathBuf::from(path),
        _ => {
            return Err(io::Error::other(format!(
                "usage: herdr {SUBCOMMAND} --socket <path>"
            )))
        }
    };
    // SAFETY: changes this process's own dispositions before any thread starts.
    unsafe {
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
        libc::signal(libc::SIGINT, libc::SIG_IGN);
    }
    let _ = std::fs::remove_file(&socket);
    // SAFETY: no other thread runs yet, so the umask change is not observed.
    let umask = unsafe { libc::umask(0o077) };
    let bound = UnixListener::bind(&socket);
    unsafe { libc::umask(umask) };
    let listener = bound?;
    let socket_inode = std::fs::metadata(&socket).map(|meta| meta.ino()).ok();
    std::fs::set_permissions(
        &socket,
        <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
    )?;
    let pid_marker = std::env::current_exe().ok().and_then(|exe| {
        exe.parent()
            .map(|dir| dir.join("pids").join(std::process::id().to_string()))
    });
    if let Some(marker) = &pid_marker {
        if let Some(dir) = marker.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(marker, b"");
    }

    let state = Arc::new(AnchorState {
        children: AtomicUsize::new(0),
        connections: AtomicUsize::new(0),
    });
    {
        let state = state.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                state.connections.fetch_add(1, Ordering::AcqRel);
                let state = state.clone();
                std::thread::spawn(move || {
                    serve_connection(&stream, &state);
                    state.connections.fetch_sub(1, Ordering::AcqRel);
                });
            }
        });
    }

    let mut idle_since: Option<Instant> = None;
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let idle = state.children.load(Ordering::Acquire) == 0
            && state.connections.load(Ordering::Acquire) == 0;
        if !idle {
            idle_since = None;
            continue;
        }
        if idle_since.get_or_insert_with(Instant::now).elapsed() >= IDLE_EXIT {
            break;
        }
    }
    // A newer anchor may have taken the path; leave its socket alone.
    if std::fs::metadata(&socket).map(|meta| meta.ino()).ok() == socket_inode {
        let _ = std::fs::remove_file(&socket);
    }
    if let Some(marker) = pid_marker {
        let _ = std::fs::remove_file(marker);
    }
    Ok(())
}

fn serve_connection(stream: &UnixStream, state: &Arc<AnchorState>) {
    serve_requests(stream, state);
    // Waiters of earlier children hold clones of the stream; shutting it down
    // makes the server see the end now instead of a silent connection.
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

fn serve_requests(stream: &UnixStream, state: &Arc<AnchorState>) {
    let Ok(writer) = stream.try_clone() else {
        return;
    };
    let writer = Arc::new(Mutex::new(writer));
    let mut fds = VecDeque::new();
    let mut pending = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut greeted = false;
    loop {
        let read = match recv_with_fds(stream, &mut buf, &mut fds) {
            Ok(0) | Err(_) => return,
            Ok(read) => read,
        };
        pending.extend_from_slice(&buf[..read]);
        while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            let request = match serde_json::from_slice::<Request>(&line) {
                Ok(request) => request,
                Err(err) => {
                    let Some(id) = refusable_spawn(&line) else {
                        return;
                    };
                    fds.pop_front();
                    let _ = write_reply(
                        &writer,
                        &Reply::Refused {
                            id,
                            error: err.to_string(),
                        },
                    );
                    continue;
                }
            };
            match request {
                Request::Hello { version } => {
                    if write_reply(&writer, &Reply::Hello { version: PROTOCOL }).is_err()
                        || version != PROTOCOL
                    {
                        return;
                    }
                    greeted = true;
                }
                Request::Spawn { id, command } if greeted => {
                    let Some(slave_fd) = fds.pop_front() else {
                        let _ = write_reply(
                            &writer,
                            &Reply::Failed {
                                id,
                                error: "no pty descriptor arrived".into(),
                            },
                        );
                        continue;
                    };
                    spawn_for(id, command, slave_fd, &writer, state);
                }
                Request::Spawn { .. } => return,
            }
        }
        if pending.len() > MAX_LINE {
            return;
        }
    }
}

/// The id of a spawn request this anchor cannot decode.
fn refusable_spawn(line: &[u8]) -> Option<u64> {
    let value = serde_json::from_slice::<serde_json::Value>(line).ok()?;
    (value.get("type")?.as_str()? == "spawn")
        .then(|| value.get("id")?.as_u64())
        .flatten()
}

fn spawn_for(
    id: u64,
    command: CommandBuilder,
    slave_fd: OwnedFd,
    writer: &Arc<Mutex<UnixStream>>,
    state: &Arc<AnchorState>,
) {
    let slave = portable_pty::unix::slave_from_fd(slave_fd);
    let spawned = slave.spawn_command(command);
    // The anchor must not keep the slave open, or the server never sees EOF.
    drop(slave);
    let mut child = match spawned {
        Ok(child) => child,
        Err(err) => {
            let _ = write_reply(
                writer,
                &Reply::Failed {
                    id,
                    error: err.to_string(),
                },
            );
            return;
        }
    };
    let Some(pid) = child.process_id() else {
        let _ = child.kill();
        let _ = write_reply(
            writer,
            &Reply::Failed {
                id,
                error: "spawned child has no pid".into(),
            },
        );
        return;
    };
    state.children.fetch_add(1, Ordering::AcqRel);
    // Spawned is written before the waiter exists, so it always precedes Exited.
    let _ = write_reply(writer, &Reply::Spawned { id, pid });
    let writer = writer.clone();
    let state = state.clone();
    std::thread::spawn(move || {
        let status = child
            .wait()
            .unwrap_or_else(|_| ExitStatus::with_signal("unknown"));
        state.children.fetch_sub(1, Ordering::AcqRel);
        let _ = write_reply(
            &writer,
            &Reply::Exited {
                pid,
                code: status.exit_code(),
                signal: status.signal().map(str::to_owned),
            },
        );
    });
}

// ----- the server side -----

enum SpawnOutcome {
    Spawned(u32, mpsc::Receiver<ExitStatus>),
    Failed(String),
    Refused(String),
}

struct Client {
    writer: Mutex<UnixStream>,
    pending: Mutex<HashMap<u64, mpsc::Sender<SpawnOutcome>>>,
    exits: Mutex<HashMap<u32, mpsc::Sender<ExitStatus>>>,
    next_id: AtomicU64,
    broken: AtomicBool,
}

impl Client {
    fn connect(path: &Path) -> io::Result<Arc<Self>> {
        let mut stream = UnixStream::connect(path)?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.write_all(&encode(&Request::Hello { version: PROTOCOL })?)?;
        let mut reader = BufReader::new(stream.try_clone()?);
        let mut line = String::new();
        reader.read_line(&mut line)?;
        match serde_json::from_str::<Reply>(&line) {
            Ok(Reply::Hello { version }) if version == PROTOCOL => {}
            Ok(Reply::Hello { version }) => {
                return Err(io::Error::other(format!(
                    "the pane anchor speaks protocol {version}, this server {PROTOCOL}"
                )))
            }
            _ => return Err(io::Error::other("the pane anchor did not greet")),
        }
        stream.set_read_timeout(None)?;
        let client = Arc::new(Self {
            writer: Mutex::new(stream),
            pending: Mutex::new(HashMap::new()),
            exits: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            broken: AtomicBool::new(false),
        });
        let reader_client = client.clone();
        std::thread::Builder::new()
            .name("pane-anchor-reader".into())
            .spawn(move || reader_client.read_replies(reader))?;
        Ok(client)
    }

    fn read_replies(&self, reader: BufReader<UnixStream>) {
        for line in reader.lines() {
            let Ok(line) = line else { break };
            let Ok(reply) = serde_json::from_str::<Reply>(&line) else {
                continue;
            };
            match reply {
                Reply::Spawned { id, pid } => {
                    let (exit_tx, exit_rx) = mpsc::channel();
                    lock(&self.exits).insert(pid, exit_tx);
                    if let Some(tx) = lock(&self.pending).remove(&id) {
                        let _ = tx.send(SpawnOutcome::Spawned(pid, exit_rx));
                    }
                }
                Reply::Failed { id, error } => {
                    if let Some(tx) = lock(&self.pending).remove(&id) {
                        let _ = tx.send(SpawnOutcome::Failed(error));
                    }
                }
                Reply::Refused { id, error } => {
                    if let Some(tx) = lock(&self.pending).remove(&id) {
                        let _ = tx.send(SpawnOutcome::Refused(error));
                    }
                }
                Reply::Exited { pid, code, signal } => {
                    if let Some(tx) = lock(&self.exits).remove(&pid) {
                        let status = match signal {
                            Some(signal) => ExitStatus::with_signal(&signal),
                            None => ExitStatus::with_exit_code(code),
                        };
                        let _ = tx.send(status);
                    }
                }
                Reply::Hello { .. } => {}
            }
        }
        self.broken.store(true, Ordering::Release);
        lock(&self.pending).clear();
        lock(&self.exits).clear();
    }

    /// Sends one spawn; `Ok(None)` when the request never left, so the caller
    /// may spawn the pane itself.
    fn spawn(
        &self,
        command: &CommandBuilder,
        slave_fd: RawFd,
    ) -> io::Result<Option<AnchoredChild>> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let line = encode(&Request::Spawn {
            id,
            command: command.clone(),
        })?;
        let (tx, rx) = mpsc::channel();
        lock(&self.pending).insert(id, tx);
        let sent = send_with_fd(&lock(&self.writer), &line, slave_fd);
        if let Err(err) = sent {
            lock(&self.pending).remove(&id);
            self.broken.store(true, Ordering::Release);
            tracing::warn!(%err, "the pane anchor did not take a spawn; spawning the pane directly");
            return Ok(None);
        }
        match rx.recv_timeout(REPLY_TIMEOUT) {
            Ok(SpawnOutcome::Spawned(pid, exit)) => Ok(Some(AnchoredChild {
                pid,
                exit: Mutex::new(exit),
                done: None,
            })),
            Ok(SpawnOutcome::Failed(error)) => Err(io::Error::other(error)),
            Ok(SpawnOutcome::Refused(error)) => {
                self.broken.store(true, Ordering::Release);
                tracing::warn!(%error, "the pane anchor cannot read this server's requests; spawning panes directly");
                Ok(None)
            }
            Err(_) => {
                lock(&self.pending).remove(&id);
                self.broken.store(true, Ordering::Release);
                Err(io::Error::other("the pane anchor did not answer a spawn"))
            }
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A pane program started by the anchor.
#[derive(Debug)]
pub(crate) struct AnchoredChild {
    pid: u32,
    exit: Mutex<mpsc::Receiver<ExitStatus>>,
    done: Option<ExitStatus>,
}

impl AnchoredChild {
    /// The anchor went away while the program runs: it now belongs to launchd,
    /// which reaps it, so its end is seen but not its status.
    fn wait_without_anchor(&self) -> ExitStatus {
        while process_exists(self.pid) {
            std::thread::sleep(Duration::from_millis(250));
        }
        ExitStatus::with_exit_code(0)
    }
}

fn process_exists(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    let alive = unsafe { libc::kill(pid, 0) } == 0;
    alive || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

impl Child for AnchoredChild {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if self.done.is_none() {
            match lock(&self.exit).try_recv() {
                Ok(status) => self.done = Some(status),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    if !process_exists(self.pid) {
                        self.done = Some(ExitStatus::with_exit_code(0));
                    }
                }
            }
        }
        Ok(self.done.clone())
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        if let Some(status) = &self.done {
            return Ok(status.clone());
        }
        let received = lock(&self.exit).recv();
        let status = received.unwrap_or_else(|_| self.wait_without_anchor());
        self.done = Some(status.clone());
        Ok(status)
    }

    fn process_id(&self) -> Option<u32> {
        Some(self.pid)
    }
}

impl ChildKiller for AnchoredChild {
    fn kill(&mut self) -> io::Result<()> {
        signal(self.pid, libc::SIGHUP)?;
        // As the PTY library's own child: a grace period, then SIGKILL.
        for attempt in 0..5 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(50));
            }
            if let Ok(Some(_)) = self.try_wait() {
                return Ok(());
            }
        }
        signal(self.pid, libc::SIGKILL)
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(PidKiller { pid: self.pid })
    }
}

#[derive(Debug)]
struct PidKiller {
    pid: u32,
}

impl ChildKiller for PidKiller {
    fn kill(&mut self) -> io::Result<()> {
        signal(self.pid, libc::SIGHUP)
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(PidKiller { pid: self.pid })
    }
}

fn signal(pid: u32, signal: libc::c_int) -> io::Result<()> {
    let pid = libc::pid_t::try_from(pid).map_err(io::Error::other)?;
    if unsafe { libc::kill(pid, signal) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

// ----- connecting, and starting the anchor -----

#[derive(Default)]
struct Connection {
    client: Option<Arc<Client>>,
    retry_at: Option<Instant>,
}

fn connection() -> &'static Mutex<Connection> {
    static CONNECTION: OnceLock<Mutex<Connection>> = OnceLock::new();
    CONNECTION.get_or_init(|| Mutex::new(Connection::default()))
}

fn disabled() -> bool {
    // A test binary would run its tests if started as the anchor.
    cfg!(test) || std::env::var_os("HERDR_PANE_ANCHOR").is_some_and(|value| value == "off")
}

/// The live anchor client, connecting to or starting the anchor when needed.
fn client() -> Option<Arc<Client>> {
    if disabled() {
        return None;
    }
    let mut connection = lock(connection());
    if let Some(client) = &connection.client {
        if !client.broken.load(Ordering::Acquire) {
            return Some(client.clone());
        }
        connection.client = None;
    }
    if connection.retry_at.is_some_and(|at| Instant::now() < at) {
        return None;
    }
    match connect_or_start() {
        Ok(client) => {
            connection.client = Some(client.clone());
            connection.retry_at = None;
            Some(client)
        }
        Err(err) => {
            tracing::warn!(%err, "no pane anchor; panes are spawned by the server");
            connection.retry_at = Some(Instant::now() + RETRY_AFTER);
            None
        }
    }
}

fn connect_or_start() -> io::Result<Arc<Client>> {
    let path = socket_path();
    if path.as_os_str().len() > MAX_SOCKET_PATH {
        return Err(io::Error::other(format!(
            "socket path too long: {}",
            path.display()
        )));
    }
    match Client::connect(&path) {
        Ok(client) => return Ok(client),
        // Only when nothing listens: an anchor that answers wrongly keeps its
        // socket and its panes, and this server spawns panes itself.
        Err(err)
            if !matches!(
                err.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Err(err)
        }
        Err(_) => {}
    }
    let exe = anchor_executable()?;
    let mut command = std::process::Command::new(&exe);
    command.arg(SUBCOMMAND).arg("--socket").arg(&path);
    let pid = super::spawn_disclaimed_daemon(&command)?;
    crate::platform::daemon_reaper::reap_when_it_exits(pid);
    tracing::info!(pid, exe = %exe.display(), "started the pane anchor");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match Client::connect(&path) {
            Ok(client) => return Ok(client),
            Err(err) if Instant::now() >= deadline => return Err(err),
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

/// A link to this executable that outlives the package manager removing it.
///
/// Homebrew deletes the old version's files on upgrade, while the anchor keeps
/// running for months; a deleted executable may fail tccd's code-signing check.
/// The directory is named after the executable's inode, so another build never
/// reuses it, and the file keeps the executable's name, which the prompt shows.
fn anchor_executable() -> io::Result<PathBuf> {
    link_executable(
        &std::env::current_exe()?,
        &crate::config::config_dir().join("pane-anchor"),
    )
}

/// Links `exe` under `root`. `exe` may be a symlink, as Homebrew's
/// `/opt/homebrew/bin/herdr-beta` is: macOS `link(2)` would link the symlink
/// itself, whose relative target dangles in another directory, so the real
/// file is linked, under the name the process was started with.
fn link_executable(exe: &Path, root: &Path) -> io::Result<PathBuf> {
    let name = exe
        .file_name()
        .ok_or_else(|| io::Error::other("the executable has no file name"))?
        .to_owned();
    let exe = std::fs::canonicalize(exe)?;
    let inode = std::fs::metadata(&exe)?.ino();
    let dir = root.join(format!("{}-{inode}", env!("CARGO_PKG_VERSION")));
    remove_unused_links(root, &dir);
    std::fs::create_dir_all(&dir)?;
    let link = dir.join(name);
    let size = std::fs::metadata(&exe)?.len();
    // A copy (no hard link across volumes) has another inode but the same size;
    // replacing it would pull the file from under a running anchor.
    if std::fs::metadata(&link).is_ok_and(|meta| meta.ino() == inode || meta.len() == size) {
        return Ok(link);
    }
    let staged = dir.join(".staged");
    let _ = std::fs::remove_file(&staged);
    if std::fs::hard_link(&exe, &staged).is_err() {
        std::fs::copy(&exe, &staged)?;
    }
    std::fs::rename(&staged, &link)?;
    Ok(link)
}

/// Removes link directories no running anchor uses, never `keep`.
fn remove_unused_links(root: &Path, keep: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if dir == keep || !dir.is_dir() {
            continue;
        }
        let in_use = std::fs::read_dir(dir.join("pids")).is_ok_and(|pids| {
            pids.flatten().any(|pid| {
                pid.file_name()
                    .to_str()
                    .and_then(|pid| pid.parse::<u32>().ok())
                    .is_some_and(process_exists)
            })
        });
        if !in_use {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

/// Starts `command` on the PTY whose slave is `tty` through the anchor.
/// `None` means the caller spawns the pane itself.
pub(crate) fn spawn(
    command: &CommandBuilder,
    tty: &Path,
) -> Option<io::Result<Box<dyn Child + Send + Sync>>> {
    let client = client()?;
    let slave = match open_slave(tty) {
        Ok(slave) => slave,
        Err(err) => {
            tracing::warn!(%err, tty = %tty.display(), "could not open the pane's tty for the anchor");
            return None;
        }
    };
    match client.spawn(command, slave.as_raw_fd()) {
        Ok(Some(child)) => Some(Ok(Box::new(child))),
        Ok(None) => None,
        Err(err) => Some(Err(err)),
    }
}

fn open_slave(tty: &Path) -> io::Result<OwnedFd> {
    let path = std::ffi::CString::new(std::os::unix::ffi::OsStrExt::as_bytes(tty.as_os_str()))
        .map_err(io::Error::other)?;
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn requests_and_replies_round_trip() {
        let mut command = CommandBuilder::new("/bin/echo");
        command.arg("hi");
        let line = encode(&Request::Spawn {
            id: 7,
            command: command.clone(),
        })
        .unwrap();
        match serde_json::from_slice::<Request>(&line).unwrap() {
            Request::Spawn {
                id,
                command: decoded,
            } => {
                assert_eq!(id, 7);
                assert_eq!(decoded, command);
            }
            other => panic!("unexpected {other:?}"),
        }
        let line = encode(&Reply::Exited {
            pid: 9,
            code: 3,
            signal: None,
        })
        .unwrap();
        assert!(matches!(
            serde_json::from_slice::<Reply>(&line).unwrap(),
            Reply::Exited {
                pid: 9,
                code: 3,
                signal: None
            }
        ));
    }

    fn serve_one(listener: UnixListener) -> Arc<AnchorState> {
        let state = Arc::new(AnchorState {
            children: AtomicUsize::new(0),
            connections: AtomicUsize::new(0),
        });
        let served = state.clone();
        std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                serve_connection(&stream, &served);
            }
        });
        state
    }

    fn temp_socket(name: &str) -> PathBuf {
        let path = PathBuf::from(format!(
            "/tmp/herdr-anchor-{name}-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn an_anchor_spawns_on_the_passed_pty_and_reports_the_exit() {
        let path = temp_socket("spawn");
        let state = serve_one(UnixListener::bind(&path).unwrap());
        let client = Client::connect(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        let pair = portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize::default())
            .unwrap();
        let tty = pair.master.tty_name().unwrap();
        let mut reader = pair.master.try_clone_reader().unwrap();
        let slave = open_slave(&tty).unwrap();
        drop(pair.slave);
        let mut command = CommandBuilder::new("/bin/sh");
        command.args(["-c", "echo anchored; exit 3"]);
        let mut child = client.spawn(&command, slave.as_raw_fd()).unwrap().unwrap();
        drop(slave);

        let mut output = Vec::new();
        let mut buf = [0u8; 256];
        let deadline = Instant::now() + Duration::from_secs(10);
        while !String::from_utf8_lossy(&output).contains("anchored") {
            assert!(Instant::now() < deadline, "no output: {output:?}");
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(read) => output.extend_from_slice(&buf[..read]),
            }
        }
        assert!(String::from_utf8_lossy(&output).contains("anchored"));
        let status = child.wait().unwrap();
        assert_eq!(status.exit_code(), 3);
        assert!(status.signal().is_none());
        assert_eq!(state.children.load(Ordering::Acquire), 0);
    }

    /// The command travels bincode-encoded, which is positional: a change to the
    /// vendored `CommandBuilder` changes it, and a long-lived anchor of the old
    /// build would misread new servers. Bump `PROTOCOL` when this fails.
    #[test]
    fn the_command_encoding_is_pinned_to_the_protocol() {
        let mut command = CommandBuilder::new("/bin/sh");
        command.env_clear();
        command.arg("-c");
        command.env("A", "b");
        command.cwd("/tmp");
        let line = String::from_utf8(encode(&Request::Spawn { id: 1, command }).unwrap()).unwrap();
        assert_eq!(PROTOCOL, 1);
        assert_eq!(
            line.trim_end(),
            PINNED_SPAWN_V1,
            "the spawn encoding changed: bump PROTOCOL"
        );
    }

    const PINNED_SPAWN_V1: &str =
        r#"{"type":"spawn","id":1,"command":"AgAHL2Jpbi9zaAACLWMBAAFBAAABQQABYgEABC90bXAAAQ=="}"#;

    #[test]
    fn an_undecodable_spawn_is_refused_with_its_id() {
        let line = br#"{"type":"spawn","id":42,"command":"not base64 of a command"}"#;
        assert_eq!(refusable_spawn(line), Some(42));
        assert_eq!(refusable_spawn(br#"{"type":"hello","version":9}"#), None);
    }

    #[test]
    fn a_symlinked_executable_is_linked_as_the_real_file() {
        let root = std::env::temp_dir().join(format!("herdr-anchor-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bin = root.join("Cellar/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("herdr-real"), b"#!/bin/sh\n").unwrap();
        std::os::unix::fs::symlink("Cellar/bin/herdr-real", root.join("herdr-beta")).unwrap();

        let link = link_executable(&root.join("herdr-beta"), &root.join("pane-anchor")).unwrap();
        let meta = std::fs::symlink_metadata(&link).unwrap();
        let real = std::fs::metadata(bin.join("herdr-real")).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(link.file_name().unwrap(), "herdr-beta");
        assert!(meta.file_type().is_file(), "linked a symlink: {link:?}");
        assert_eq!(meta.ino(), real.ino());
    }

    #[test]
    fn a_different_protocol_is_refused() {
        let path = temp_socket("version");
        let listener = UnixListener::bind(&path).unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut line = String::new();
                let _ = BufReader::new(stream.try_clone().unwrap()).read_line(&mut line);
                let _ = stream.write_all(
                    &encode(&Reply::Hello {
                        version: PROTOCOL + 1,
                    })
                    .unwrap(),
                );
            }
        });
        let err = Client::connect(&path).err().expect("refused");
        let _ = std::fs::remove_file(&path);
        assert!(err.to_string().contains("protocol"), "{err}");
    }

    #[test]
    fn a_lost_anchor_still_sees_its_child_end() {
        let (tx, rx) = mpsc::channel::<ExitStatus>();
        drop(tx);
        let mut sleeper = std::process::Command::new("/bin/sleep")
            .arg("0.3")
            .spawn()
            .unwrap();
        let mut child = AnchoredChild {
            pid: sleeper.id(),
            exit: Mutex::new(rx),
            done: None,
        };
        let reaper = std::thread::spawn(move || sleeper.wait());
        let status = child.wait().unwrap();
        assert_eq!(status.exit_code(), 0);
        let _ = reaper.join();
    }
}
