//! Pane keepers: each pane program runs under a keeper that is its own live
//! launchd job (fork issue 288, openspec/changes/pane-launchd-jobs).
//!
//! An app launched from a pane is tied by LaunchServices to the pane's
//! resource coalition. Once that coalition's launchd job process has exited,
//! the app's leftover entry stays stoppable, and stopping it ends the whole
//! coalition: on 2026-11-16 one Dock stop ended 26 agent sessions that shared
//! the server's coalition. A keeper stays its job's process for the pane's
//! life, so no such entry appears, and a stop ends at most that pane.
//!
//! The server keeps the PTY, as with the pane anchor: it submits the keeper
//! job, the keeper connects to the server's pane-job socket and presents its
//! label and token, receives the command and the slave descriptor, starts the
//! program responsible for itself, and reports the pid and, later, the exit.
//! Across a live handoff the importer serves the same socket path, and a
//! keeper whose server went away reports the exit there.

use std::collections::{HashMap, VecDeque};
use std::io::{self, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use portable_pty::{Child, ChildKiller, CommandBuilder, ExitStatus};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use super::launchd_spawn::PANE_LABEL_PREFIX;
use super::pane_anchor::{encode, lock, open_slave, recv_with_fds, send_with_fds, MAX_LINE};
use crate::platform::{PaneJobLink, ProcessIdentity};
// Exit statuses that are not the program's own: the pane is suspended
// (design D3), not closed.
use crate::platform::{PANE_JOB_STOPPED as KEEPER_STOPPED, PANE_KEEPER_LOST as KEEPER_LOST};

const PROTOCOL: u32 = 1;
pub(crate) const SUBCOMMAND: &str = "__pane-job";
/// Appended to the API socket's name, so each session has its own socket.
const SOCKET_SUFFIX: &str = ".pane-jobs";
/// Carries the keeper's token; the environment, unlike the arguments, is not
/// visible to other users.
const TOKEN_ENV: &str = "HERDR_PANE_JOB_TOKEN";
/// How long a submitted keeper may take to connect. Panes restored before the
/// server serves wait like the anchor's; later spawns hold the app loop.
const START_TIMEOUT: Duration = Duration::from_secs(10);
const START_TIMEOUT_SERVING: Duration = Duration::from_secs(3);
/// After a keeper that never connected, panes skip the job backend this long.
const RETRY_AFTER: Duration = Duration::from_secs(60);
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
const ACK_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a keeper whose server went away keeps trying to report the exit.
const REPORT_WINDOW: Duration = Duration::from_secs(10);
const STALE_SWEEP_EVERY: Duration = Duration::from_secs(60);
/// How often a pane whose keeper is gone checks that its program still runs.
const LOST_KEEPER_POLL: Duration = Duration::from_millis(250);

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum FromKeeper {
    Hello {
        version: u32,
        label: String,
        token: String,
    },
    Spawned {
        pid: u32,
    },
    Failed {
        error: String,
    },
    Exited(ExitReport),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ExitReport {
    code: u32,
    signal: Option<String>,
    /// The keeper was told to stop (its job was removed, or the user logged
    /// out) and hung up on the program.
    stopped: bool,
}

impl ExitReport {
    /// A stopped job wins over the program's own status: the keeper hung up
    /// on the program, so its `Hangup` is the session's doing, not its own.
    fn status(&self) -> ExitStatus {
        match (&self.signal, self.stopped) {
            (_, true) => ExitStatus::with_signal(KEEPER_STOPPED),
            (Some(signal), false) => ExitStatus::with_signal(signal),
            (None, false) => ExitStatus::with_exit_code(self.code),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ToKeeper {
    Spawn {
        #[serde(with = "super::pane_anchor::command_codec")]
        command: CommandBuilder,
    },
    /// The server took the pane (after `Spawned`) or the exit report.
    Ack,
    /// No pane of this server waits for the keeper.
    Unknown,
}

/// Reads one newline-terminated message, keeping what follows it in
/// `pending`; with `fds`, also queues descriptors that arrive. `Ok(None)` at
/// the end of the stream.
fn read_message<T: DeserializeOwned>(
    stream: &UnixStream,
    pending: &mut Vec<u8>,
    mut fds: Option<&mut VecDeque<OwnedFd>>,
) -> io::Result<Option<T>> {
    let mut buf = [0u8; 16 * 1024];
    loop {
        if let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            return serde_json::from_slice(&line)
                .map(Some)
                .map_err(io::Error::other);
        }
        if pending.len() > MAX_LINE {
            return Err(io::Error::other("message too long"));
        }
        let read = match fds.as_deref_mut() {
            Some(fds) => recv_with_fds(stream, &mut buf, fds)?,
            None => io::Read::read(&mut &*stream, &mut buf)?,
        };
        if read == 0 {
            return Ok(None);
        }
        pending.extend_from_slice(&buf[..read]);
    }
}

fn write_message<T: Serialize>(stream: &UnixStream, message: &T) -> io::Result<()> {
    (&*stream).write_all(&encode(message)?)
}

/// Only a process of this user may talk to a keeper or the server.
fn stream_peer_is_us(stream: &UnixStream) -> bool {
    // SAFETY: getuid has no preconditions.
    crate::platform::unix_stream_peer_is(stream, unsafe { libc::getuid() })
}

// ----- the keeper process -----

static PROGRAM: AtomicI32 = AtomicI32::new(0);
static STOPPED: AtomicBool = AtomicBool::new(false);

/// launchd ends a job with SIGTERM (a removed job, a logout). Interactive
/// shells ignore SIGTERM, so the program gets what a closed terminal sends.
extern "C" fn on_terminate(_signal: libc::c_int) {
    STOPPED.store(true, Ordering::SeqCst);
    let pid = PROGRAM.load(Ordering::SeqCst);
    if pid > 0 {
        // SAFETY: kill is async-signal-safe.
        unsafe { libc::kill(pid, libc::SIGHUP) };
    }
}

/// `herdr __pane-job --socket <path> --label <label>`, token in the environment.
pub(crate) fn run(args: &[String]) -> io::Result<()> {
    let (socket, label) = match args {
        [socket_flag, socket, label_flag, label]
            if socket_flag == "--socket" && label_flag == "--label" =>
        {
            (PathBuf::from(socket), label.clone())
        }
        _ => {
            return Err(io::Error::other(format!(
                "usage: herdr {SUBCOMMAND} --socket <path> --label <label>"
            )))
        }
    };
    let token = std::env::var(TOKEN_ENV)
        .map_err(|_| io::Error::other(format!("{TOKEN_ENV} is not set")))?;
    std::env::remove_var(TOKEN_ENV);
    // SAFETY: changes this process's own dispositions before any thread starts.
    unsafe {
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
        libc::signal(libc::SIGINT, libc::SIG_IGN);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        libc::signal(
            libc::SIGTERM,
            on_terminate as extern "C" fn(libc::c_int) as libc::sighandler_t,
        );
    }
    let stream = connect(&socket)?;
    std::process::exit(keep(stream, &socket, &label, &token));
}

fn connect(socket: &Path) -> io::Result<UnixStream> {
    let stream = UnixStream::connect(socket)?;
    if !stream_peer_is_us(&stream) {
        return Err(io::Error::other(
            "the pane-job socket belongs to another user",
        ));
    }
    Ok(stream)
}

/// The keeper's life on a connected stream; returns its exit code.
fn keep(stream: UnixStream, socket: &Path, label: &str, token: &str) -> i32 {
    let hello = FromKeeper::Hello {
        version: PROTOCOL,
        label: label.to_owned(),
        token: token.to_owned(),
    };
    if write_message(&stream, &hello).is_err() {
        return 1;
    }
    let mut pending = Vec::new();
    let mut fds = VecDeque::new();
    let command = match read_message::<ToKeeper>(&stream, &mut pending, Some(&mut fds)) {
        Ok(Some(ToKeeper::Spawn { command })) => command,
        _ => return 1,
    };
    // Exactly the slave, nothing else (security review H6).
    let slave = match (fds.pop_front(), fds.is_empty()) {
        (Some(slave), true) => slave,
        (slave, _) => {
            let arrived = usize::from(slave.is_some()) + fds.len();
            let _ = write_message(
                &stream,
                &FromKeeper::Failed {
                    error: format!("expected one pty descriptor, {arrived} arrived"),
                },
            );
            return 1;
        }
    };
    let started = start_program(&command, &slave);
    // The keeper must not keep the slave open, or the server never sees EOF.
    drop(slave);
    let pid = match started {
        Ok(pid) => pid,
        Err(err) => {
            let _ = write_message(
                &stream,
                &FromKeeper::Failed {
                    error: err.to_string(),
                },
            );
            return 1;
        }
    };
    let Ok(raw_pid) = libc::pid_t::try_from(pid) else {
        return 1;
    };
    PROGRAM.store(raw_pid, Ordering::SeqCst);
    if STOPPED.load(Ordering::SeqCst) {
        // SAFETY: signals the program this keeper just started.
        unsafe { libc::kill(raw_pid, libc::SIGHUP) };
    }
    if !accepted(&stream, &mut pending, pid) {
        // No pane shows this program (security review H3): end it and what
        // it started in its session, which it leads, before reaping it.
        // SAFETY: the group is led by the keeper's own child, not yet reaped.
        unsafe { libc::kill(-raw_pid, libc::SIGKILL) };
        wait_for(raw_pid, || PROGRAM.store(0, Ordering::SeqCst));
        return 1;
    }
    let status = wait_for(raw_pid, || PROGRAM.store(0, Ordering::SeqCst));
    let report = ExitReport {
        code: status.exit_code(),
        signal: status.signal().map(str::to_owned),
        stopped: STOPPED.load(Ordering::SeqCst),
    };
    report_exit(&stream, &mut pending, socket, label, token, &report);
    i32::try_from(report.code).unwrap_or(1)
}

/// Reports the program's pid and waits for the server to take the pane. A
/// server that gave up on the spawn hung up or never answers.
fn accepted(stream: &UnixStream, pending: &mut Vec<u8>, pid: u32) -> bool {
    write_message(stream, &FromKeeper::Spawned { pid }).is_ok()
        && stream.set_read_timeout(Some(REPLY_TIMEOUT)).is_ok()
        && matches!(
            read_message::<ToKeeper>(stream, pending, None),
            Ok(Some(ToKeeper::Ack))
        )
}

fn start_program(command: &CommandBuilder, slave: &OwnedFd) -> io::Result<u32> {
    let parts = command.unix_spawn_parts().map_err(io::Error::other)?;
    let tty = tty_path(slave)?;
    if let Some(mask) = parts.umask {
        // SAFETY: sets the keeper's mask, which the program inherits.
        unsafe { libc::umask(mask) };
    }
    super::disclaimed_spawn::spawn_disclaimed_on_tty(&parts, &tty)
}

fn tty_path(fd: &OwnedFd) -> io::Result<PathBuf> {
    let mut buf = [0u8; 128];
    // SAFETY: ttyname_r writes a NUL-terminated name into the buffer.
    let code = unsafe { libc::ttyname_r(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
    if code != 0 {
        return Err(io::Error::from_raw_os_error(code));
    }
    let name = std::ffi::CStr::from_bytes_until_nul(&buf).map_err(io::Error::other)?;
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(name.to_bytes())))
}

/// Waits for the program to exit and reaps it. `disarm` runs once the exit
/// is seen but before the reap, while the pid still names the exited program
/// and no other process can have it (security review H2).
fn wait_for(pid: libc::pid_t, disarm: impl FnOnce()) -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    let exited = loop {
        // SAFETY: waits for the keeper's own child into a zeroed siginfo,
        // leaving it waitable.
        let waited = unsafe {
            let mut info: libc::siginfo_t = std::mem::zeroed();
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOWAIT,
            )
        };
        if waited == 0 {
            break true;
        }
        if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            break false;
        }
    };
    disarm();
    if !exited {
        return ExitStatus::with_signal("unknown");
    }
    let mut status = 0;
    loop {
        // SAFETY: reaps the keeper's own child, which has exited.
        let waited = unsafe { libc::waitpid(pid, &mut status, 0) };
        if waited == pid {
            return std::process::ExitStatus::from_raw(status).into();
        }
        if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            return ExitStatus::with_signal("unknown");
        }
    }
}

/// Reports the exit on the keeper's connection, or, when its server went away
/// (a live handoff), to whichever server serves the socket path now.
fn report_exit(
    stream: &UnixStream,
    pending: &mut Vec<u8>,
    socket: &Path,
    label: &str,
    token: &str,
    report: &ExitReport,
) {
    if send_report(stream, pending, report).is_some() {
        return;
    }
    let deadline = Instant::now() + REPORT_WINDOW;
    while Instant::now() < deadline {
        if let Ok(stream) = connect(socket) {
            let hello = FromKeeper::Hello {
                version: PROTOCOL,
                label: label.to_owned(),
                token: token.to_owned(),
            };
            if write_message(&stream, &hello).is_ok() {
                // Ack or Unknown: either way no server will want it again.
                if send_report(&stream, &mut Vec::new(), report).is_some() {
                    return;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Sends the exit and waits for the server's answer.
fn send_report(
    stream: &UnixStream,
    pending: &mut Vec<u8>,
    report: &ExitReport,
) -> Option<ToKeeper> {
    write_message(stream, &FromKeeper::Exited(report.clone())).ok()?;
    stream.set_read_timeout(Some(ACK_TIMEOUT)).ok()?;
    match read_message::<ToKeeper>(stream, pending, None).ok()?? {
        answer @ (ToKeeper::Ack | ToKeeper::Unknown) => Some(answer),
        ToKeeper::Spawn { .. } => None,
    }
}

// ----- the server side -----

enum Waiting {
    /// A keeper this server submitted: its connection goes to the spawn.
    Start {
        token: String,
        connected: mpsc::Sender<UnixStream>,
    },
    /// A keeper an earlier server started: it reports its program's exit here.
    Exit {
        token: String,
        exit: mpsc::Sender<ExitReport>,
    },
}

impl Waiting {
    fn token(&self) -> &str {
        match self {
            Waiting::Start { token, .. } | Waiting::Exit { token, .. } => token,
        }
    }
}

#[derive(Default)]
struct Jobs {
    /// The inode of the socket this server listens on.
    bound: Option<u64>,
    /// Keepers expected to connect, by job label.
    waiting: HashMap<String, Waiting>,
    /// Each running job program's job, by program pid, for a live handoff.
    links: HashMap<u32, PaneJobLink>,
    retry_at: Option<Instant>,
    last_sweep: Option<Instant>,
}

fn jobs() -> &'static Mutex<Jobs> {
    static JOBS: OnceLock<Mutex<Jobs>> = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(Jobs::default()))
}

static ENABLED: AtomicBool = AtomicBool::new(false);

/// `[experimental] pane_jobs`; off by default.
pub(crate) fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Release);
}

/// Whether `[experimental] pane_jobs` is on in this server.
pub(crate) fn enabled() -> bool {
    ENABLED.load(Ordering::Acquire)
}

/// Whether a suspended pane may start again now (design D3): at once when
/// new panes do not start as jobs here, else once launchd accepts pane jobs
/// again (after a logout, the GUI domain is back).
// Tests inject their own probe: the real one asks launchd.
#[cfg_attr(test, allow(dead_code))]
pub(crate) fn ready_for_resume() -> bool {
    let over_ssh = std::env::var_os("SSH_CONNECTION").is_some();
    if unavailable(ENABLED.load(Ordering::Acquire), over_ssh).is_some() {
        return true;
    }
    if lock(jobs()).retry_at.is_some_and(|at| Instant::now() < at) {
        return false;
    }
    super::launchd_spawn::gui_domain_exists()
}

/// Why new panes do not start as jobs, if they do not.
fn unavailable(enabled: bool, over_ssh: bool) -> Option<&'static str> {
    if !enabled {
        Some("[experimental] pane_jobs is off")
    } else if over_ssh {
        Some("the server runs over SSH")
    } else {
        None
    }
}

/// The pane-job socket of the session this server serves.
fn socket_path() -> PathBuf {
    let api = crate::session::active_api_socket_path();
    let name = api
        .file_name()
        .map(|name| format!("{}{SOCKET_SUFFIX}", name.to_string_lossy()))
        .unwrap_or_else(|| format!("herdr.sock{SOCKET_SUFFIX}"));
    api.with_file_name(name)
}

/// Listens on `path` unless this server already does. A path that now holds
/// another server's socket (an importer re-bound it) is taken back, so a
/// server that keeps serving after a failed handoff still hears its keepers.
fn ensure_listening(path: &Path) -> io::Result<()> {
    let mut jobs = lock(jobs());
    let current = std::fs::metadata(path).map(|meta| meta.ino()).ok();
    if jobs.bound.is_some() && jobs.bound == current {
        return Ok(());
    }
    let _ = std::fs::remove_file(path);
    let listener = crate::platform::bind_owner_only(path)?;
    jobs.bound = std::fs::metadata(path).map(|meta| meta.ino()).ok();
    std::thread::Builder::new()
        .name("pane-job-listener".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = std::thread::Builder::new()
                    .name("pane-job-hello".into())
                    .spawn(move || serve(stream));
            }
        })?;
    Ok(())
}

/// Answers one keeper connection.
fn serve(stream: UnixStream) {
    if !stream_peer_is_us(&stream) {
        tracing::warn!("dropped a pane-job connection from another user");
        return;
    }
    if stream.set_read_timeout(Some(HELLO_TIMEOUT)).is_err() {
        return;
    }
    let mut pending = Vec::new();
    let Ok(Some(FromKeeper::Hello { label, token, .. })) =
        read_message::<FromKeeper>(&stream, &mut pending, None)
    else {
        return;
    };
    let waiting = {
        let mut jobs = lock(jobs());
        match jobs.waiting.get(&label) {
            Some(waiting) if waiting.token() == token => jobs.waiting.remove(&label),
            _ => None,
        }
    };
    match waiting {
        Some(Waiting::Start { connected, .. }) => {
            if stream.set_read_timeout(None).is_ok() {
                let _ = connected.send(stream);
            }
        }
        Some(Waiting::Exit { exit, .. }) => {
            if let Ok(Some(FromKeeper::Exited(report))) =
                read_message::<FromKeeper>(&stream, &mut pending, None)
            {
                let _ = exit.send(report);
                let _ = write_message(&stream, &ToKeeper::Ack);
            }
        }
        None => {
            tracing::debug!(label, "a pane keeper no pane waits for connected");
            let _ = write_message(&stream, &ToKeeper::Unknown);
        }
    }
}

fn new_label() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    format!(
        "{PANE_LABEL_PREFIX}{}.{}.{nanos}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// Removes finished pane jobs off the spawn path, at most once a minute.
fn sweep_stale_jobs() {
    {
        let mut jobs = lock(jobs());
        let now = Instant::now();
        if jobs
            .last_sweep
            .is_some_and(|at| now.duration_since(at) < STALE_SWEEP_EVERY)
        {
            return;
        }
        jobs.last_sweep = Some(now);
    }
    let _ = std::thread::Builder::new()
        .name("pane-job-sweep".into())
        .spawn(super::launchd_spawn::remove_stale_pane_jobs);
}

/// Starts `command` on the PTY whose slave is `tty` under a keeper job.
/// `None` means the caller starts the pane another way (the anchor, or
/// directly): the backend is off, unavailable, or the keeper never connected.
pub(crate) fn spawn(
    command: &CommandBuilder,
    tty: &Path,
) -> Option<io::Result<Box<dyn Child + Send + Sync>>> {
    if cfg!(test) {
        // A test binary would run its tests if started as the keeper.
        return None;
    }
    let over_ssh = std::env::var_os("SSH_CONNECTION").is_some();
    if let Some(reason) = unavailable(ENABLED.load(Ordering::Acquire), over_ssh) {
        tracing::trace!(reason, "pane starts without a launchd job");
        return None;
    }
    if lock(jobs()).retry_at.is_some_and(|at| Instant::now() < at) {
        return None;
    }
    let path = socket_path();
    if path.as_os_str().len() > super::pane_anchor::MAX_SOCKET_PATH {
        return fall_back(format!(
            "the pane-job socket path {} is too long",
            path.display()
        ));
    }
    if let Err(err) = ensure_listening(&path) {
        return fall_back(format!(
            "cannot listen on the pane-job socket {}: {err}",
            path.display()
        ));
    }
    let exe = match super::pane_anchor::helper_executable() {
        Ok(exe) => exe,
        Err(err) => return fall_back(format!("no keeper executable: {err}")),
    };
    let label = new_label();
    let token = match crate::platform::random_token() {
        Ok(token) => token,
        Err(err) => return fall_back(format!("no random token: {err}")),
    };
    let (connected_tx, connected) = mpsc::channel();
    lock(jobs()).waiting.insert(
        label.clone(),
        Waiting::Start {
            token: token.clone(),
            connected: connected_tx,
        },
    );
    let mut job = std::process::Command::new(&exe);
    job.arg(SUBCOMMAND)
        .arg("--socket")
        .arg(&path)
        .arg("--label")
        .arg(&label)
        .env(TOKEN_ENV, &token)
        .current_dir("/");
    let started = Instant::now();
    if let Err(err) = super::launchd_spawn::submit_pane_job(&label, &job) {
        lock(jobs()).waiting.remove(&label);
        return fall_back(format!("launchd did not take the pane job: {err}"));
    }
    sweep_stale_jobs();
    let timeout = if super::pane_anchor::is_serving() {
        START_TIMEOUT_SERVING
    } else {
        START_TIMEOUT
    };
    let stream = match connected.recv_timeout(timeout) {
        Ok(stream) => stream,
        Err(_) => {
            let mut jobs = lock(jobs());
            jobs.waiting.remove(&label);
            jobs.retry_at = Some(Instant::now() + RETRY_AFTER);
            drop(jobs);
            remove_job_in_background(label);
            return fall_back(format!(
                "a pane keeper did not connect within {} s; panes start without launchd \
                 jobs for {} s",
                timeout.as_secs(),
                RETRY_AFTER.as_secs()
            ));
        }
    };
    let child = start_on(
        stream,
        command,
        tty,
        PaneJobLink {
            label: label.clone(),
            token,
        },
    );
    match &child {
        Some(Ok(child)) => tracing::debug!(
            pid = child.process_id(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "pane started in its own launchd job"
        ),
        // A keeper no pane took ends its program; the job goes with it.
        _ => remove_job_in_background(label),
    }
    child
}

/// The first reason a pane started another way although pane jobs are on,
/// until the app takes it for its one notice.
static FALLBACK_NOTICE: Mutex<Option<String>> = Mutex::new(None);
static FALLBACK_NOTED: AtomicBool = AtomicBool::new(false);

/// A pane starts without a job although `[experimental] pane_jobs` is on.
fn fall_back<T>(reason: String) -> Option<T> {
    tracing::warn!(reason, "pane starts without a launchd job");
    if !FALLBACK_NOTED.swap(true, Ordering::AcqRel) {
        *lock(&FALLBACK_NOTICE) = Some(reason);
    }
    None
}

/// The reason for the one notice that panes start without jobs, once per
/// server.
pub(crate) fn take_fallback_notice() -> Option<String> {
    lock(&FALLBACK_NOTICE).take()
}

fn remove_job_in_background(label: String) {
    let _ = std::thread::Builder::new()
        .name("pane-job-remove".into())
        .spawn(move || {
            if let Err(err) = super::launchd_spawn::remove_pane_job(&label) {
                tracing::debug!(%err, label, "could not remove a pane job");
            }
        });
}

/// Hands a connected keeper its program and waits for the pid.
fn start_on(
    stream: UnixStream,
    command: &CommandBuilder,
    tty: &Path,
    link: PaneJobLink,
) -> Option<io::Result<Box<dyn Child + Send + Sync>>> {
    let slave = match open_slave(tty) {
        Ok(slave) => slave,
        Err(err) => {
            return fall_back(format!(
                "could not open the pane's terminal {} for its keeper: {err}",
                tty.display()
            ))
        }
    };
    let line = match encode(&ToKeeper::Spawn {
        command: command.clone(),
    }) {
        Ok(line) => line,
        Err(err) => return Some(Err(err)),
    };
    if let Err(err) = send_with_fds(&stream, &line, &[slave.as_raw_fd()]) {
        return fall_back(format!("a pane keeper did not take its program: {err}"));
    }
    drop(slave);
    let mut pending = Vec::new();
    let reply = stream
        .set_read_timeout(Some(REPLY_TIMEOUT))
        .and_then(|()| read_message::<FromKeeper>(&stream, &mut pending, None));
    match reply {
        Ok(Some(FromKeeper::Spawned { pid })) => {
            if let Err(err) = stream
                .set_read_timeout(None)
                .and_then(|()| write_message(&stream, &ToKeeper::Ack))
            {
                return Some(Err(err));
            }
            lock(jobs()).links.insert(pid, link);
            Some(Ok(Box::new(JobChild {
                pid,
                identity: ProcessIdentity::of(pid),
                stream,
                pending,
                done: None,
            })))
        }
        Ok(Some(FromKeeper::Failed { error })) => Some(Err(io::Error::other(error))),
        _ => Some(Err(io::Error::other(
            "the pane keeper did not answer a spawn",
        ))),
    }
}

/// The job of a running pane program, for a live handoff's state.
pub(crate) fn link(pid: u32) -> Option<PaneJobLink> {
    lock(jobs()).links.get(&pid).cloned()
}

fn forget(pid: u32) {
    lock(jobs()).links.remove(&pid);
}

/// Takes over a job pane from the server this one replaces: its keeper
/// reports the program's exit to this server. The returned wait, called once
/// the pane's terminal closed, gives the reported status within `grace`.
pub(crate) fn adopt(
    link: PaneJobLink,
    pid: u32,
) -> Option<impl FnOnce(Duration) -> Option<ExitStatus>> {
    let path = socket_path();
    if path.as_os_str().len() > super::pane_anchor::MAX_SOCKET_PATH {
        return None;
    }
    if let Err(err) = ensure_listening(&path) {
        tracing::warn!(%err, "cannot hear the keepers of handed-off panes");
        return None;
    }
    let (exit_tx, exit) = mpsc::channel();
    let label = link.label.clone();
    {
        let mut jobs = lock(jobs());
        jobs.waiting.insert(
            label.clone(),
            Waiting::Exit {
                token: link.token.clone(),
                exit: exit_tx,
            },
        );
        jobs.links.insert(pid, link);
    }
    Some(move |grace: Duration| {
        let report = exit.recv_timeout(grace).ok();
        let mut jobs = lock(jobs());
        jobs.waiting.remove(&label);
        jobs.links.remove(&pid);
        report.map(|report| report.status())
    })
}

/// A pane program started by its keeper.
#[derive(Debug)]
pub(crate) struct JobChild {
    pid: u32,
    identity: Option<ProcessIdentity>,
    stream: UnixStream,
    pending: Vec<u8>,
    done: Option<ExitStatus>,
}

impl JobChild {
    fn finish(&mut self, status: ExitStatus) -> ExitStatus {
        forget(self.pid);
        self.done = Some(status.clone());
        status
    }

    /// The keeper went away without reporting the exit: the program counts
    /// as running while its session is the one it was started in.
    fn wait_without_keeper(&mut self) -> ExitStatus {
        while ProcessIdentity::is(self.identity, self.pid) {
            std::thread::sleep(LOST_KEEPER_POLL);
        }
        self.finish(ExitStatus::with_signal(KEEPER_LOST))
    }
}

impl Child for JobChild {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if self.done.is_some() {
            return Ok(self.done.clone());
        }
        self.stream.set_nonblocking(true)?;
        let read = read_message::<FromKeeper>(&self.stream, &mut self.pending, None);
        self.stream.set_nonblocking(false)?;
        match read {
            Ok(Some(FromKeeper::Exited(report))) => {
                let _ = write_message(&self.stream, &ToKeeper::Ack);
                Ok(Some(self.finish(report.status())))
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Ok(Some(_)) => Ok(None),
            Ok(None) | Err(_) if !ProcessIdentity::is(self.identity, self.pid) => {
                Ok(Some(self.finish(ExitStatus::with_signal(KEEPER_LOST))))
            }
            Ok(None) | Err(_) => Ok(None),
        }
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        if let Some(status) = &self.done {
            return Ok(status.clone());
        }
        loop {
            match read_message::<FromKeeper>(&self.stream, &mut self.pending, None) {
                Ok(Some(FromKeeper::Exited(report))) => {
                    let _ = write_message(&self.stream, &ToKeeper::Ack);
                    return Ok(self.finish(report.status()));
                }
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => return Ok(self.wait_without_keeper()),
            }
        }
    }

    fn process_id(&self) -> Option<u32> {
        Some(self.pid)
    }
}

impl ChildKiller for JobChild {
    fn kill(&mut self) -> io::Result<()> {
        JobKiller {
            pid: self.pid,
            identity: self.identity,
        }
        .kill()?;
        // As the PTY library's own child: a grace period, then SIGKILL.
        for attempt in 0..5 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(50));
            }
            if let Ok(Some(_)) = self.try_wait() {
                return Ok(());
            }
        }
        if ProcessIdentity::is(self.identity, self.pid) {
            super::pane_anchor::signal(self.pid, libc::SIGKILL)?;
        }
        Ok(())
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(JobKiller {
            pid: self.pid,
            identity: self.identity,
        })
    }
}

impl Drop for JobChild {
    fn drop(&mut self) {
        if self.done.is_none() {
            forget(self.pid);
        }
    }
}

#[derive(Debug)]
struct JobKiller {
    pid: u32,
    identity: Option<ProcessIdentity>,
}

impl ChildKiller for JobKiller {
    fn kill(&mut self) -> io::Result<()> {
        if !ProcessIdentity::is(self.identity, self.pid) {
            return Ok(());
        }
        super::pane_anchor::signal(self.pid, libc::SIGHUP)
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(JobKiller {
            pid: self.pid,
            identity: self.identity,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn responsible_for(pid: u32) -> u32 {
        type ResponsibleFor = unsafe extern "C" fn(libc::pid_t) -> libc::pid_t;
        let name = c"responsibility_get_pid_responsible_for_pid";
        let address = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
        assert!(!address.is_null(), "responsibility lookup unavailable");
        let lookup = unsafe { std::mem::transmute::<*mut libc::c_void, ResponsibleFor>(address) };
        unsafe { lookup(pid as libc::pid_t) as u32 }
    }

    /// A keeper on one end of a socket pair, the server's hello handling on
    /// the other, as the listener would run them.
    fn keeper_connected(label: &str, token: &str) -> (UnixStream, std::thread::JoinHandle<i32>) {
        let (connected_tx, connected) = mpsc::channel();
        lock(jobs()).waiting.insert(
            label.to_owned(),
            Waiting::Start {
                token: token.to_owned(),
                connected: connected_tx,
            },
        );
        let (keeper_end, server_end) = UnixStream::pair().unwrap();
        std::thread::spawn(move || serve(server_end));
        let (label, token) = (label.to_owned(), token.to_owned());
        let keeper = std::thread::spawn(move || {
            keep(
                keeper_end,
                Path::new("/nonexistent/pane-jobs"),
                &label,
                &token,
            )
        });
        let stream = connected
            .recv_timeout(Duration::from_secs(5))
            .expect("the keeper's hello reached the spawn");
        (stream, keeper)
    }

    fn read_until(reader: &mut Box<dyn Read + Send>, output: &mut String, needle: &str) {
        let mut buf = [0u8; 512];
        let deadline = Instant::now() + Duration::from_secs(10);
        while !output.contains(needle) {
            assert!(Instant::now() < deadline, "no {needle:?} in {output:?}");
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(read) => output.push_str(&String::from_utf8_lossy(&buf[..read])),
            }
        }
    }

    #[test]
    fn a_keeper_starts_the_program_on_the_pty_responsible_for_itself_and_reports_its_exit() {
        let pair = portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize::default())
            .unwrap();
        let tty = pair.master.tty_name().unwrap();
        let mut reader = pair.master.try_clone_reader().unwrap();
        drop(pair.slave);
        let label = format!("{PANE_LABEL_PREFIX}test.{}.1", std::process::id());
        let (stream, keeper) = keeper_connected(&label, "secret");
        let link = PaneJobLink {
            label: label.clone(),
            token: "secret".into(),
        };
        let mut command = CommandBuilder::new("/bin/sh");
        command.args([
            "-c",
            "sleep 30 & echo jobbed $(ps -o tty= -p $$) $!; read -r line; kill $!; exit 3",
        ]);
        let mut child = start_on(stream, &command, &tty, link.clone())
            .expect("the keeper answered")
            .expect("spawned");
        let pid = child.process_id().unwrap();

        let mut output = String::new();
        read_until(&mut reader, &mut output, "jobbed");
        read_until(&mut reader, &mut output, "\n");
        let grandchild: u32 = output
            .lines()
            .find(|line| line.contains("jobbed"))
            .and_then(|line| line.split_whitespace().nth(2))
            .and_then(|pid| pid.parse().ok())
            .expect("grandchild pid");
        // Children are attributed to the program only when it is its own
        // responsible process; this test process may be one too.
        let responsible = responsible_for(grandchild);
        let session = crate::platform::process_session(pid);
        assert_eq!(super::link(pid), Some(link), "kept for a handoff");
        let mut writer = pair.master.take_writer().unwrap();
        writer.write_all(b"done\r").unwrap();
        // As a pane's reader does: a session leader's exit waits for the
        // terminal's output, here the echo, to drain.
        std::thread::spawn(move || {
            let mut sink = [0u8; 512];
            while matches!(reader.read(&mut sink), Ok(read) if read > 0) {}
        });
        let status = child.wait().unwrap();

        assert!(
            output.contains(&format!(
                "jobbed {}",
                tty.file_name().unwrap().to_string_lossy()
            )),
            "{output:?}"
        );
        assert_eq!(
            responsible, pid,
            "the program is responsible for itself, not the keeper"
        );
        assert_eq!(session, Some(pid), "the program leads its own session");
        assert_eq!(status.exit_code(), 3);
        assert!(status.signal().is_none());
        assert_eq!(
            keeper.join().unwrap(),
            3,
            "the keeper exits with the program's status"
        );
        assert_eq!(super::link(pid), None, "forgotten after the exit");
    }

    #[test]
    fn panes_that_start_without_their_job_give_one_notice_with_the_first_reason() {
        assert!(fall_back::<()>("launchd did not take the pane job".into()).is_none());
        assert!(fall_back::<()>("a pane keeper did not connect".into()).is_none());

        assert_eq!(
            take_fallback_notice().as_deref(),
            Some("launchd did not take the pane job")
        );
        assert_eq!(take_fallback_notice(), None, "once per server");
    }

    #[test]
    fn a_keeper_reports_a_program_that_cannot_start() {
        let pair = portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize::default())
            .unwrap();
        let tty = pair.master.tty_name().unwrap();
        drop(pair.slave);
        let label = format!("{PANE_LABEL_PREFIX}test.{}.2", std::process::id());
        let (stream, keeper) = keeper_connected(&label, "secret");
        let command = CommandBuilder::new("/nonexistent/program");
        let started = start_on(
            stream,
            &command,
            &tty,
            PaneJobLink {
                label,
                token: "secret".into(),
            },
        )
        .expect("the keeper answered");
        assert!(started.is_err());
        assert_eq!(keeper.join().unwrap(), 1);
    }

    #[test]
    fn a_wrong_token_gets_no_pane() {
        let label = format!("{PANE_LABEL_PREFIX}test.{}.3", std::process::id());
        let (connected_tx, connected) = mpsc::channel();
        lock(jobs()).waiting.insert(
            label.clone(),
            Waiting::Start {
                token: "right".into(),
                connected: connected_tx,
            },
        );
        let (keeper_end, server_end) = UnixStream::pair().unwrap();
        std::thread::spawn(move || serve(server_end));
        write_message(
            &keeper_end,
            &FromKeeper::Hello {
                version: PROTOCOL,
                label: label.clone(),
                token: "wrong".into(),
            },
        )
        .unwrap();
        let answer = read_message::<ToKeeper>(&keeper_end, &mut Vec::new(), None).unwrap();
        assert!(matches!(answer, Some(ToKeeper::Unknown)), "{answer:?}");
        assert!(connected.try_recv().is_err());
        assert!(
            lock(jobs()).waiting.contains_key(&label),
            "still waits for the real keeper"
        );
        lock(jobs()).waiting.remove(&label);
    }

    #[test]
    fn panes_fall_back_when_off_or_over_ssh() {
        assert!(unavailable(false, false).is_some(), "off by default");
        assert!(unavailable(true, true).is_some(), "SSH keeps the old path");
        assert_eq!(unavailable(true, false), None);
        assert!(
            !ENABLED.load(Ordering::Acquire),
            "off until the setting turns it on"
        );
    }

    /// Design D1 step 4: a keeper whose server went away reports the exit to
    /// whichever server took the socket over.
    #[test]
    fn an_adopted_pane_hears_its_keepers_exit_on_a_new_connection() {
        let label = format!("{PANE_LABEL_PREFIX}test.{}.4", std::process::id());
        let (exit_tx, exit) = mpsc::channel();
        lock(jobs()).waiting.insert(
            label.clone(),
            Waiting::Exit {
                token: "secret".into(),
                exit: exit_tx,
            },
        );
        let (keeper_end, server_end) = UnixStream::pair().unwrap();
        std::thread::spawn(move || serve(server_end));
        write_message(
            &keeper_end,
            &FromKeeper::Hello {
                version: PROTOCOL,
                label: label.clone(),
                token: "secret".into(),
            },
        )
        .unwrap();
        let report = ExitReport {
            code: 0,
            signal: Some("Hangup: 1".into()),
            stopped: true,
        };
        let answer = send_report(&keeper_end, &mut Vec::new(), &report);
        assert!(matches!(answer, Some(ToKeeper::Ack)), "{answer:?}");
        let received = exit.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(received, report);
        assert_eq!(received.status().signal(), Some(KEEPER_STOPPED));
        assert_eq!(
            crate::platform::classify_child_exit(&received.status()),
            crate::platform::ChildExitReason::Suspended(
                crate::platform::SuspendCause::PaneJobEnded
            ),
            "a program the stopped keeper hung up on suspends its pane"
        );
    }

    /// Design D2 and security review M2: with the keeper gone, the program
    /// counts as running only while its pid is still the process it was.
    #[test]
    fn a_lost_keeper_waits_for_the_program_itself_and_never_for_a_reused_pid() {
        use std::os::unix::process::CommandExt;
        let mut sleeper = std::process::Command::new("/bin/sleep");
        sleeper.arg("0.3");
        // SAFETY: setsid is async-signal-safe and touches only the child.
        unsafe {
            sleeper.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        let mut sleeper = sleeper.spawn().unwrap();
        let pid = sleeper.id();
        let (server_end, keeper_end) = UnixStream::pair().unwrap();
        drop(keeper_end);
        let mut child = JobChild {
            pid,
            identity: ProcessIdentity::of(pid),
            stream: server_end,
            pending: Vec::new(),
            done: None,
        };
        assert!(child.identity.is_some());
        let reaper = std::thread::spawn(move || sleeper.wait());
        let started = Instant::now();
        let status = child.wait().unwrap();
        assert!(
            started.elapsed() >= Duration::from_millis(200),
            "waited for the program"
        );
        assert_eq!(status.signal(), Some(KEEPER_LOST));
        let _ = reaper.join();

        let (server_end, keeper_end) = UnixStream::pair().unwrap();
        drop(keeper_end);
        let mut stranger = JobChild {
            pid: std::process::id(),
            identity: Some(ProcessIdentity {
                session: crate::platform::process_session(std::process::id()).unwrap(),
                started: 1,
            }),
            stream: server_end,
            pending: Vec::new(),
            done: None,
        };
        let started = Instant::now();
        assert_eq!(stranger.wait().unwrap().signal(), Some(KEEPER_LOST));
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "a reused pid is not the program"
        );
    }

    /// Security review H3: a program whose pane the server gave up on, even
    /// one that ignores a hangup, ends with its keeper.
    #[test]
    fn a_keeper_ends_a_program_no_server_took() {
        let pair = portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize::default())
            .unwrap();
        let tty = pair.master.tty_name().unwrap();
        drop(pair.slave);
        let (keeper_end, server_end) = UnixStream::pair().unwrap();
        let (done_tx, done) = mpsc::channel();
        std::thread::spawn(move || {
            let code = keep(
                keeper_end,
                Path::new("/nonexistent/pane-jobs"),
                "dev.herdr.pane.test",
                "secret",
            );
            let _ = done_tx.send(code);
        });
        let mut pending = Vec::new();
        let hello = read_message::<FromKeeper>(&server_end, &mut pending, None).unwrap();
        assert!(matches!(hello, Some(FromKeeper::Hello { .. })), "{hello:?}");
        let mut command = CommandBuilder::new("/bin/sh");
        command.args(["-c", "trap '' HUP; exec sleep 30"]);
        let slave = open_slave(&tty).unwrap();
        let line = encode(&ToKeeper::Spawn { command }).unwrap();
        send_with_fds(&server_end, &line, &[slave.as_raw_fd()]).unwrap();
        drop(slave);
        let pid = match read_message::<FromKeeper>(&server_end, &mut pending, None).unwrap() {
            Some(FromKeeper::Spawned { pid }) => pid,
            other => panic!("no pid: {other:?}"),
        };
        // The server timed out: it hangs up without taking the pane.
        drop(server_end);
        let code = done
            .recv_timeout(Duration::from_secs(5))
            .expect("the keeper did not wait for the program");
        assert_eq!(code, 1);
        // SAFETY: probes the pid the keeper reported, which it reaped.
        let gone = unsafe { libc::kill(pid as libc::pid_t, 0) } == -1;
        assert!(gone, "the program ended with its keeper");
    }

    /// Security review H6: a keeper takes exactly one descriptor for its
    /// program, and none of a transfer cut short.
    #[test]
    fn a_keeper_refuses_anything_but_one_descriptor() {
        let pair = portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize::default())
            .unwrap();
        let slave = open_slave(&pair.master.tty_name().unwrap()).unwrap();
        drop(pair.slave);
        let fd = slave.as_raw_fd();
        for count in [0, 2, 9] {
            let (keeper_end, server_end) = UnixStream::pair().unwrap();
            let keeper = std::thread::spawn(move || {
                keep(
                    keeper_end,
                    Path::new("/nonexistent/pane-jobs"),
                    "dev.herdr.pane.test",
                    "secret",
                )
            });
            let mut pending = Vec::new();
            let hello = read_message::<FromKeeper>(&server_end, &mut pending, None).unwrap();
            assert!(matches!(hello, Some(FromKeeper::Hello { .. })), "{hello:?}");
            let line = encode(&ToKeeper::Spawn {
                command: CommandBuilder::new("/usr/bin/true"),
            })
            .unwrap();
            // A keeper that refused the transfer may hang up before the rest.
            let _ = if count == 0 {
                (&server_end).write_all(&line)
            } else {
                send_with_fds(&server_end, &line, &vec![fd; count])
            };
            let reply = read_message::<FromKeeper>(&server_end, &mut pending, None);
            assert!(
                !matches!(reply, Ok(Some(FromKeeper::Spawned { .. }))),
                "{count} descriptors started a program"
            );
            assert_eq!(keeper.join().unwrap(), 1, "{count} descriptors");
        }
    }

    #[test]
    fn received_descriptors_close_on_exec_and_a_cut_transfer_is_refused() {
        let devnull = std::fs::File::open("/dev/null").unwrap();
        let (sender, receiver) = UnixStream::pair().unwrap();
        send_with_fds(&sender, b"x", &[devnull.as_raw_fd()]).unwrap();
        let mut fds = VecDeque::new();
        recv_with_fds(&receiver, &mut [0u8; 8], &mut fds).unwrap();
        assert_eq!(fds.len(), 1);
        // SAFETY: reads the flags of a descriptor this test owns.
        let flags = unsafe { libc::fcntl(fds[0].as_raw_fd(), libc::F_GETFD) };
        assert_ne!(flags & libc::FD_CLOEXEC, 0, "flags {flags:#x}");

        send_with_fds(&sender, b"x", &[devnull.as_raw_fd(); 9]).unwrap();
        let mut fds = VecDeque::new();
        assert!(
            recv_with_fds(&receiver, &mut [0u8; 8], &mut fds).is_err(),
            "more than fit is refused"
        );
        assert!(fds.is_empty());
    }

    /// Security review H2: signal forwarding is turned off while the exited
    /// program still holds its pid, so a SIGTERM never reaches a stranger.
    #[test]
    // `wait_for` reaps the child itself, which is what this test checks.
    #[allow(clippy::zombie_processes)]
    fn forwarding_is_disarmed_before_the_program_is_reaped() {
        let child = std::process::Command::new("/bin/sh")
            .args(["-c", "exit 7"])
            .spawn()
            .unwrap();
        let pid = child.id() as libc::pid_t;
        let mut held = None;
        let status = wait_for(pid, || {
            // SAFETY: asks about the test's own child without reaping it.
            let waitable = unsafe {
                let mut info: libc::siginfo_t = std::mem::zeroed();
                libc::waitid(
                    libc::P_PID,
                    pid as libc::id_t,
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                ) == 0
                    && info.si_pid == pid
            };
            held = Some(waitable);
        });
        assert_eq!(held, Some(true), "the exited program still held its pid");
        assert_eq!(status.exit_code(), 7);
        // SAFETY: probes the reaped child's pid.
        let reaped = unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) } == -1;
        assert!(reaped, "reaped after the disarm");
    }

    #[test]
    fn messages_round_trip() {
        let report = FromKeeper::Exited(ExitReport {
            code: 0,
            signal: Some("Hangup: 1".into()),
            stopped: false,
        });
        let line = encode(&report).unwrap();
        assert_eq!(
            String::from_utf8(line.clone()).unwrap().trim_end(),
            r#"{"type":"exited","code":0,"signal":"Hangup: 1","stopped":false}"#
        );
        let mut pending = line;
        let (a, b) = UnixStream::pair().unwrap();
        drop(b);
        assert!(matches!(
            read_message::<FromKeeper>(&a, &mut pending, None).unwrap(),
            Some(FromKeeper::Exited(ExitReport { code: 0, .. }))
        ));
        assert_eq!(
            read_message::<FromKeeper>(&a, &mut pending, None)
                .unwrap()
                .map(|_: FromKeeper| ()),
            None
        );
    }
}
