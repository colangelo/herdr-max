#[cfg(unix)]
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::fd::{AsRawFd, RawFd};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(unix)]
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::{Child, Command};
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use tracing::{info, warn};

#[cfg(unix)]
const HANDOFF_VERSION: u32 = 1;
#[cfg(unix)]
const READY_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(unix)]
const OWNED_ACK_TIMEOUT: Duration = Duration::from_millis(500);
// Descriptors are transferred in batches of this size. A single SCM_RIGHTS
// control message caps out at 253 descriptors on Linux and 254 on macOS, so the
// batch stays well below both limits and the number of panes stays unbounded.
#[cfg(unix)]
const FDS_PER_MESSAGE: usize = 64;
// An importer that announces this capability receives descriptors in batches of
// any size. One that announces nothing predates batching: it reads them with a
// single recvmsg, so they must all arrive in one message.
#[cfg(unix)]
const FD_BATCHES_CAPABILITY: &str = "fd-batches";
// The most descriptors one SCM_RIGHTS message carries on every supported
// platform (Linux's SCM_MAX_FD), so the most panes a pre-batching importer can
// take.
#[cfg(unix)]
const SINGLE_MESSAGE_FD_LIMIT: usize = 253;
#[cfg(unix)]
pub(crate) const MAX_REPLAY_BYTES_PER_PANE: usize = 8 * 1024;
#[cfg(unix)]
pub(crate) const COMMIT_TIMEOUT: Duration = READY_TIMEOUT;

#[cfg(unix)]
#[derive(Serialize, Deserialize)]
pub(crate) struct HandoffManifest {
    pub version: u32,
    pub source_version: String,
    pub source_protocol: u32,
    pub expected_version: Option<String>,
    pub expected_protocol: Option<u32>,
    pub snapshot: crate::persist::SessionSnapshot,
    pub panes: Vec<crate::handoff_runtime::HandoffRuntimeState>,
    /// An outer window title set over the API outlives the server that took the
    /// call, so a handoff carries it rather than falling back to the config.
    /// Absent from manifests written before this field existed.
    #[serde(default)]
    pub api_window_title: Option<String>,
    /// The exporting server's effective size, (cols, rows): its attached
    /// client's, or the size it had itself carried over a handoff. The
    /// importing server keeps panes at this size until a client attaches,
    /// instead of shrinking them to the headless default. Absent from
    /// manifests written before this field existed.
    #[serde(default)]
    pub client_size: Option<(u16, u16)>,
    /// Asks the importer to list its transport capabilities when it validates
    /// the manifest. An importer that predates the field ignores it and
    /// answers a bare `validated`, which tells the exporter it is that old.
    #[serde(default)]
    pub announce_capabilities: bool,
}

/// How the exporter sends descriptors to the importer it validated with.
#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FdTransport {
    /// Batches of [`FDS_PER_MESSAGE`], so any number of panes moves.
    Batches,
    /// One message holding every descriptor, for an importer from before
    /// batching.
    SingleMessage,
}

/// The transport an importer's validation line allows for `panes` panes. A
/// pre-batching importer that could not take them all in one message is
/// refused here, before any descriptor moves, so the exporter rolls back with
/// every pane intact.
#[cfg(unix)]
fn fd_transport_for(validated: &str, panes: usize) -> io::Result<FdTransport> {
    let mut words = validated.split_whitespace();
    if words.next() != Some("validated") {
        return Err(io::Error::other("handoff import did not validate manifest"));
    }
    if words.any(|capability| capability == FD_BATCHES_CAPABILITY) {
        return Ok(FdTransport::Batches);
    }
    if panes > SINGLE_MESSAGE_FD_LIMIT {
        let excess = panes - SINGLE_MESSAGE_FD_LIMIT;
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "the target build can't receive more than {SINGLE_MESSAGE_FD_LIMIT} panes in a live handoff and this session has {panes}; close {excess} pane{} or hand off to a newer build",
                if excess == 1 { "" } else { "s" }
            ),
        ));
    }
    Ok(FdTransport::SingleMessage)
}

/// What the importer answers once the manifest checks out.
#[cfg(unix)]
fn validated_line(manifest: &HandoffManifest) -> String {
    if manifest.announce_capabilities {
        format!("validated {FD_BATCHES_CAPABILITY}\n")
    } else {
        "validated\n".to_owned()
    }
}

#[cfg(unix)]
pub(crate) struct ReceivedHandoff {
    pub manifest: HandoffManifest,
    pub fds: Vec<RawFd>,
    pub stream: UnixStream,
}

#[cfg(unix)]
pub(crate) fn handoff_socket_path() -> PathBuf {
    crate::session::data_dir().join(format!("herdr-handoff-{}.sock", std::process::id()))
}

#[cfg(unix)]
pub(crate) fn spawn_handoff_import(
    import_exe: Option<&Path>,
    socket_path: &Path,
    token: &str,
) -> io::Result<Child> {
    let fallback_exe;
    let exe = if let Some(import_exe) = import_exe {
        import_exe
    } else {
        fallback_exe = std::env::current_exe().map_err(|err| {
            io::Error::new(
                err.kind(),
                format!("failed to determine herdr executable path: {err}"),
            )
        })?;
        &fallback_exe
    };
    let mut command = Command::new(exe);
    command
        .arg("server")
        .arg("--handoff-import")
        .arg(socket_path)
        .arg(token)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if crate::session::explicit_session_requested() {
        // The import child no longer has the original `--session` argument, so
        // stale socket overrides must not mask the inherited HERDR_SESSION.
        command
            .env_remove(crate::api::SOCKET_PATH_ENV_VAR)
            .env_remove(crate::server::socket_paths::CLIENT_SOCKET_PATH_ENV_VAR);
    }
    crate::platform::detach_server_daemon_command(&mut command);
    command.spawn().map_err(|err| {
        io::Error::new(
            err.kind(),
            format!(
                "failed to spawn handoff import server at {}: {err}",
                exe.display()
            ),
        )
    })
}

#[cfg(unix)]
pub(crate) fn cleanup_failed_import_child(child: &mut Child) {
    let pid = child.id();
    match child.try_wait() {
        Ok(Some(status)) => {
            info!(pid, status = %status, "handoff import server exited during rollback");
            return;
        }
        Ok(None) => {}
        Err(err) => {
            warn!(pid, err = %err, "failed to inspect handoff import server before rollback");
        }
    }

    if let Err(err) = child.kill() {
        warn!(pid, err = %err, "failed to kill handoff import server during rollback");
    }
    match child.wait() {
        Ok(status) => {
            info!(pid, status = %status, "handoff import server reaped during rollback");
        }
        Err(err) => {
            warn!(pid, err = %err, "failed to reap handoff import server during rollback");
        }
    }
}

#[cfg(unix)]
pub(crate) fn bind_listener(socket_path: &Path) -> io::Result<UnixListener> {
    let _ = std::fs::remove_file(socket_path);
    let listener = UnixListener::bind(socket_path)?;
    listener.set_nonblocking(true)?;
    restrict_socket_permissions(socket_path)?;
    Ok(listener)
}

#[cfg(unix)]
pub(crate) fn accept_and_validate_on(
    listener: UnixListener,
    socket_path: &Path,
    token: &str,
    manifest: &HandoffManifest,
) -> io::Result<(UnixStream, FdTransport)> {
    let (mut stream, _) = accept_with_timeout(&listener, READY_TIMEOUT)?;
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(READY_TIMEOUT))?;
    stream.set_write_timeout(Some(READY_TIMEOUT))?;
    let token_line = read_line_unbuffered(&mut stream)?;
    if token_line.trim_end() != token {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "handoff import token mismatch",
        ));
    }

    serde_json::to_writer(&mut stream, manifest).map_err(io::Error::other)?;
    stream.write_all(b"\n")?;
    stream.flush()?;

    stream.set_read_timeout(Some(READY_TIMEOUT))?;
    let validated = read_line_unbuffered(&mut stream)?;
    let transport = fd_transport_for(&validated, manifest.panes.len())?;
    let _ = std::fs::remove_file(socket_path);
    Ok((stream, transport))
}

#[cfg(unix)]
pub(crate) fn send_fds_and_wait_restored(
    stream: &mut UnixStream,
    fds: &[RawFd],
    transport: FdTransport,
) -> io::Result<()> {
    send_fds(stream, fds, transport)?;

    stream.set_read_timeout(Some(READY_TIMEOUT))?;
    let restored = read_line_unbuffered(&mut *stream)?;
    if restored.trim_end() != "restored" {
        return Err(io::Error::other(
            "handoff import did not report restored runtimes",
        ));
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) fn wait_ready(stream: &mut UnixStream) -> io::Result<()> {
    stream.set_read_timeout(Some(READY_TIMEOUT))?;
    let ready = read_line_unbuffered(&mut *stream)?;
    if ready.trim_end() != "ready" {
        return Err(io::Error::other("handoff import did not report ready"));
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) fn report_committed(stream: &mut UnixStream) -> io::Result<()> {
    stream.write_all(b"committed\n")?;
    stream.flush()
}

#[cfg(unix)]
pub(crate) fn wait_owned_ack(stream: &mut UnixStream) {
    if let Err(err) = stream.set_read_timeout(Some(OWNED_ACK_TIMEOUT)) {
        warn!(err = %err, "failed to set handoff ownership ack timeout");
        return;
    }
    match read_line_unbuffered(&mut *stream) {
        Ok(owned) if owned.trim_end() == "owned" => {}
        Ok(other) => {
            warn!(
                response = %other.trim_end(),
                "handoff import sent unexpected ownership ack after commit"
            );
        }
        Err(err) => {
            warn!(err = %err, "handoff import ownership ack was not received after commit");
        }
    }
}

#[cfg(unix)]
pub(crate) fn receive(socket_path: &Path, token: &str) -> io::Result<ReceivedHandoff> {
    let mut stream = UnixStream::connect(socket_path)?;
    stream.write_all(token.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;

    let manifest_line = read_line_unbuffered(&mut stream)?;
    let manifest: HandoffManifest =
        serde_json::from_str(&manifest_line).map_err(io::Error::other)?;
    if manifest.version != HANDOFF_VERSION {
        return Err(io::Error::other(format!(
            "unsupported handoff version {}",
            manifest.version
        )));
    }
    if manifest
        .expected_protocol
        .is_some_and(|protocol| protocol != crate::protocol::PROTOCOL_VERSION)
    {
        return Err(io::Error::other(format!(
            "handoff expected protocol {}, but this server speaks protocol {}",
            manifest.expected_protocol.unwrap_or_default(),
            crate::protocol::PROTOCOL_VERSION
        )));
    }
    if manifest
        .expected_version
        .as_deref()
        .is_some_and(|version| version != crate::build_info::version())
    {
        return Err(io::Error::other(format!(
            "handoff expected herdr v{}, but this server is v{}",
            manifest.expected_version.as_deref().unwrap_or("unknown"),
            crate::build_info::version()
        )));
    }
    stream.write_all(validated_line(&manifest).as_bytes())?;
    stream.flush()?;
    let fds = recv_fds(&stream, manifest.panes.len())?;
    Ok(ReceivedHandoff {
        manifest,
        fds,
        stream,
    })
}

#[cfg(unix)]
pub(crate) fn report_restored(stream: &mut UnixStream) -> io::Result<()> {
    stream.write_all(b"restored\n")?;
    stream.flush()
}

#[cfg(unix)]
pub(crate) fn report_ready(stream: &mut UnixStream) -> io::Result<()> {
    stream.write_all(b"ready\n")?;
    stream.flush()
}

#[cfg(unix)]
pub(crate) fn wait_committed(stream: &mut UnixStream) -> io::Result<()> {
    stream.set_read_timeout(Some(READY_TIMEOUT))?;
    let committed = read_line_unbuffered(&mut *stream)?;
    if committed.trim_end() != "committed" {
        return Err(io::Error::other("handoff source did not commit"));
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) fn report_owned(stream: &mut UnixStream) -> io::Result<()> {
    stream.write_all(b"owned\n")?;
    stream.flush()
}

#[cfg(unix)]
pub(crate) fn manifest_for(
    snapshot: crate::persist::SessionSnapshot,
    panes: Vec<crate::handoff_runtime::HandoffRuntimeState>,
    expected_protocol: Option<u32>,
    expected_version: Option<String>,
    api_window_title: Option<String>,
    client_size: Option<(u16, u16)>,
) -> HandoffManifest {
    HandoffManifest {
        version: HANDOFF_VERSION,
        source_version: crate::build_info::version(),
        source_protocol: crate::protocol::PROTOCOL_VERSION,
        expected_version,
        expected_protocol,
        snapshot,
        panes,
        api_window_title,
        client_size,
        announce_capabilities: true,
    }
}

#[cfg(unix)]
fn restrict_socket_permissions(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(unix)]
fn accept_with_timeout(
    listener: &UnixListener,
    timeout: Duration,
) -> io::Result<(UnixStream, std::os::unix::net::SocketAddr)> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok(accepted) => return Ok(accepted),
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                if std::time::Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "timed out waiting for handoff import connection",
                    ));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err),
        }
    }
}

#[cfg(unix)]
fn read_line_unbuffered(stream: &mut UnixStream) -> io::Result<String> {
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let read = stream.read(&mut byte)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "handoff stream closed while reading line",
            ));
        }
        bytes.push(byte[0]);
        if byte[0] == b'\n' {
            return String::from_utf8(bytes)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err));
        }
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "handoff line exceeded maximum size",
            ));
        }
    }
}

#[cfg(unix)]
fn send_fds(stream: &UnixStream, fds: &[RawFd], transport: FdTransport) -> io::Result<()> {
    if transport == FdTransport::SingleMessage {
        return send_fd_batch(stream, fds);
    }
    for batch in fds.chunks(FDS_PER_MESSAGE) {
        send_fd_batch(stream, batch)?;
    }
    Ok(())
}

#[cfg(unix)]
fn send_fd_batch(stream: &UnixStream, fds: &[RawFd]) -> io::Result<()> {
    if fds.is_empty() {
        return Ok(());
    }
    let byte = *b"F";
    let iov = [libc::iovec {
        iov_base: byte.as_ptr() as *mut libc::c_void,
        iov_len: byte.len(),
    }];
    let fd_bytes = std::mem::size_of_val(fds);
    let mut control = vec![0u8; unsafe { libc::CMSG_SPACE(fd_bytes as u32) as usize }];
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = iov.as_ptr() as *mut libc::iovec;
    msg.msg_iovlen = iov.len() as _;
    msg.msg_control = control.as_mut_ptr() as *mut libc::c_void;
    msg.msg_controllen = control.len() as _;

    unsafe {
        let cmsg = libc::CMSG_FIRSTHDR(&msg);
        if cmsg.is_null() {
            return Err(io::Error::other("failed to allocate fd control message"));
        }
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(fd_bytes as u32) as _;
        std::ptr::copy_nonoverlapping(fds.as_ptr() as *const u8, libc::CMSG_DATA(cmsg), fd_bytes);
        if libc::sendmsg(stream.as_raw_fd(), &msg, 0) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(unix)]
fn close_raw_fds(fds: &[RawFd]) {
    for fd in fds {
        let _ = unsafe { libc::close(*fd) };
    }
}

#[cfg(unix)]
fn recv_fds(stream: &UnixStream, expected: usize) -> io::Result<Vec<RawFd>> {
    let mut out: Vec<RawFd> = Vec::with_capacity(expected);
    while out.len() < expected {
        // Room for every remaining descriptor: a pre-batching exporter sends
        // them all in one message, and a smaller buffer would truncate it.
        let wanted = expected - out.len();
        let batch = match recv_fd_batch(stream, wanted) {
            Ok(batch) => batch,
            Err(err) => {
                close_raw_fds(&out);
                return Err(err);
            }
        };
        if batch.is_empty() {
            let received = out.len();
            close_raw_fds(&out);
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!(
                    "handoff stream closed after {received} of {expected} pane file descriptors"
                ),
            ));
        }
        out.extend(batch);
    }
    Ok(out)
}

#[cfg(unix)]
fn recv_fd_batch(stream: &UnixStream, wanted: usize) -> io::Result<Vec<RawFd>> {
    let mut byte = [0u8; 1];
    let mut iov = [libc::iovec {
        iov_base: byte.as_mut_ptr() as *mut libc::c_void,
        iov_len: byte.len(),
    }];
    let fd_bytes = wanted * std::mem::size_of::<RawFd>();
    let mut control = vec![0u8; unsafe { libc::CMSG_SPACE(fd_bytes as u32) as usize }];
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = iov.as_mut_ptr();
    msg.msg_iovlen = iov.len() as _;
    msg.msg_control = control.as_mut_ptr() as *mut libc::c_void;
    msg.msg_controllen = control.len() as _;

    let read = unsafe { libc::recvmsg(stream.as_raw_fd(), &mut msg, 0) };
    if read < 0 {
        return Err(io::Error::last_os_error());
    }

    let mut out = Vec::new();
    unsafe {
        let control_end = control.as_ptr() as usize + msg.msg_controllen as usize;
        let mut cmsg = libc::CMSG_FIRSTHDR(&msg);
        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                let data = libc::CMSG_DATA(cmsg);
                // Bound the payload by both the header's own length and the
                // bytes the kernel wrote into `control`, so the read below can
                // never run past the buffer.
                let available = control_end.saturating_sub(data as usize);
                let data_len = ((*cmsg).cmsg_len as usize)
                    .saturating_sub(libc::CMSG_LEN(0) as usize)
                    .min(available);
                let count = data_len / std::mem::size_of::<RawFd>();
                let data = data as *const RawFd;
                for idx in 0..count {
                    out.push(*data.add(idx));
                }
            }
            cmsg = libc::CMSG_NXTHDR(&msg, cmsg);
        }
    }

    // Truncation means the kernel closed the descriptors that did not fit, so
    // the batch is unrecoverable rather than merely short.
    if msg.msg_flags & libc::MSG_CTRUNC != 0 {
        close_raw_fds(&out);
        return Err(io::Error::other("handoff fd control message was truncated"));
    }
    if read == 0 {
        close_raw_fds(&out);
        return Ok(Vec::new());
    }
    if out.len() > wanted {
        let received = out.len();
        close_raw_fds(&out);
        return Err(io::Error::other(format!(
            "handoff fd message carried {received} descriptors, expected at most {wanted}"
        )));
    }
    if out.is_empty() {
        return Err(io::Error::other("handoff fd message missing SCM_RIGHTS"));
    }
    Ok(out)
}

#[cfg(unix)]
pub(crate) fn log_import_result(panes: usize) {
    info!(panes, "handoff import ready");
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn empty_snapshot() -> crate::persist::SessionSnapshot {
        crate::persist::SessionSnapshot {
            version: 0,
            workspaces: Vec::new(),
            active: None,
            selected: 0,
            sidebar_width: None,
            sidebar_section_split: None,
            collapsed_space_keys: Default::default(),
            last_client_size: None,
        }
    }

    /// `n` real descriptors (all on /dev/null), as a handoff moves pane PTYs.
    /// Both ends of a transfer hold up to `n` at once, so callers keep `2 * n`
    /// under macOS's default limit of 256 open descriptors.
    fn dev_null_fds(n: usize) -> Vec<RawFd> {
        use std::os::fd::IntoRawFd;
        (0..n)
            .map(|_| {
                std::fs::File::open("/dev/null")
                    .expect("/dev/null should open")
                    .into_raw_fd()
            })
            .collect()
    }

    /// The pre-batching importer's receive (fork 26537d32), kept verbatim in
    /// behaviour: one recvmsg that must carry every descriptor.
    fn pre_batching_recv_fds(stream: &UnixStream, expected: usize) -> io::Result<Vec<RawFd>> {
        let mut byte = [0u8; 1];
        let mut iov = [libc::iovec {
            iov_base: byte.as_mut_ptr() as *mut libc::c_void,
            iov_len: byte.len(),
        }];
        let fd_bytes = expected * std::mem::size_of::<RawFd>();
        let mut control = vec![0u8; unsafe { libc::CMSG_SPACE(fd_bytes as u32) as usize }];
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_iov = iov.as_mut_ptr();
        msg.msg_iovlen = iov.len() as _;
        msg.msg_control = control.as_mut_ptr() as *mut libc::c_void;
        msg.msg_controllen = control.len() as _;
        if unsafe { libc::recvmsg(stream.as_raw_fd(), &mut msg, 0) } < 0 {
            return Err(io::Error::last_os_error());
        }
        if msg.msg_flags & libc::MSG_CTRUNC != 0 {
            return Err(io::Error::other("handoff fd control message was truncated"));
        }
        let mut out = Vec::new();
        unsafe {
            let cmsg = libc::CMSG_FIRSTHDR(&msg);
            if !cmsg.is_null() {
                let data_len =
                    ((*cmsg).cmsg_len as usize).saturating_sub(libc::CMSG_LEN(0) as usize);
                let data = libc::CMSG_DATA(cmsg) as *const RawFd;
                for idx in 0..data_len / std::mem::size_of::<RawFd>() {
                    out.push(*data.add(idx));
                }
            }
        }
        if out.len() != expected {
            close_raw_fds(&out);
            return Err(io::Error::other(format!(
                "expected {expected} handoff fds, received fewer"
            )));
        }
        Ok(out)
    }

    /// Send `n` descriptors with `send` and receive them with `recv` over a
    /// socket pair; the count received, or the receiver's error.
    fn transfer(
        n: usize,
        send: impl FnOnce(&UnixStream, &[RawFd]) -> io::Result<()> + Send + 'static,
        recv: impl FnOnce(&UnixStream, usize) -> io::Result<Vec<RawFd>>,
    ) -> io::Result<usize> {
        let (sender, receiver) = UnixStream::pair().expect("socket pair");
        let fds = dev_null_fds(n);
        let sending = std::thread::spawn(move || {
            let result = send(&sender, &fds);
            close_raw_fds(&fds);
            result
        });
        let received = recv(&receiver, n);
        sending
            .join()
            .expect("sender thread")
            .expect("send should succeed");
        let received = received?;
        close_raw_fds(&received);
        Ok(received.len())
    }

    #[test]
    fn a_pre_batching_importer_with_too_many_panes_is_refused_before_any_descriptor_moves() {
        let panes = SINGLE_MESSAGE_FD_LIMIT + 1;
        let err = fd_transport_for("validated\n", panes)
            .expect_err("an importer that takes one message cannot take this many");
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
        let reason = err.to_string();
        assert!(
            reason.contains("can't receive more than 253 panes")
                && reason.contains("this session has 254")
                && reason.contains("close 1 pane ")
                && reason.contains("hand off to a newer build"),
            "{reason}"
        );
        // The refusal is the validation step's answer: accept_and_validate_on
        // returns it, so the exporter rolls back before send_fds runs.
        assert!(fd_transport_for("validated\n", SINGLE_MESSAGE_FD_LIMIT).is_ok());
    }

    #[test]
    fn a_pre_batching_importer_takes_more_than_one_batch_in_a_single_message() {
        let panes = FDS_PER_MESSAGE + 1;
        assert_eq!(
            fd_transport_for("validated\n", panes).unwrap(),
            FdTransport::SingleMessage
        );
        let moved = transfer(
            panes,
            |stream, fds| send_fds(stream, fds, FdTransport::SingleMessage),
            pre_batching_recv_fds,
        )
        .expect("an old importer receives every pane in one message");
        assert_eq!(moved, panes);
    }

    #[test]
    fn the_batched_transport_is_what_breaks_a_pre_batching_importer() {
        // The mismatch this negotiation exists for: batches reach a single
        // recvmsg as only their first message.
        let err = transfer(
            FDS_PER_MESSAGE + 1,
            |stream, fds| send_fds(stream, fds, FdTransport::Batches),
            pre_batching_recv_fds,
        )
        .expect_err("an old importer cannot take batches");
        assert!(err.to_string().contains("received fewer"), "{err}");
    }

    #[test]
    fn importers_that_announce_batches_take_any_number_of_panes() {
        // Past the single-message limit the capability is all that matters.
        assert_eq!(
            fd_transport_for("validated fd-batches\n", SINGLE_MESSAGE_FD_LIMIT + 47).unwrap(),
            FdTransport::Batches
        );
        // Two batches and part of a third, staying under macOS's default
        // limit of 256 open descriptors for both ends together.
        let panes = FDS_PER_MESSAGE + 36;
        let moved = transfer(
            panes,
            |stream, fds| send_fds(stream, fds, FdTransport::Batches),
            recv_fds,
        )
        .expect("new to new moves every batch");
        assert_eq!(moved, panes);
    }

    #[test]
    fn a_pre_batching_exporter_reaches_a_new_importer_above_one_batch() {
        // The old exporter sent every descriptor in one message.
        let panes = FDS_PER_MESSAGE + 36;
        let moved = transfer(panes, send_fd_batch, recv_fds)
            .expect("a new importer has room for an old exporter's single message");
        assert_eq!(moved, panes);
    }

    #[test]
    fn up_to_one_batch_moves_both_ways_as_before() {
        let panes = FDS_PER_MESSAGE;
        let new_to_old = transfer(
            panes,
            |stream, fds| send_fds(stream, fds, FdTransport::SingleMessage),
            pre_batching_recv_fds,
        )
        .unwrap();
        let old_to_new = transfer(panes, send_fd_batch, recv_fds).unwrap();
        let new_to_new = transfer(
            panes,
            |stream, fds| send_fds(stream, fds, FdTransport::Batches),
            recv_fds,
        )
        .unwrap();
        assert_eq!((new_to_old, old_to_new, new_to_new), (panes, panes, panes));
    }

    #[test]
    fn the_importer_announces_batches_only_when_the_exporter_asks() {
        let manifest = manifest_for(empty_snapshot(), Vec::new(), None, None, None, None);
        assert!(manifest.announce_capabilities);
        assert_eq!(validated_line(&manifest), "validated fd-batches\n");

        // An exporter from before the field sends a manifest without it; the
        // importer answers the bare line that exporter checks for.
        let mut value = serde_json::to_value(&manifest).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("announce_capabilities");
        let older: HandoffManifest = serde_json::from_value(value).unwrap();
        assert!(!older.announce_capabilities);
        assert_eq!(validated_line(&older), "validated\n");

        assert!(fd_transport_for("ready\n", 1).is_err());
    }

    #[test]
    fn the_handshake_negotiates_the_transport_end_to_end() {
        for (announcing_importer, expected) in [
            (true, FdTransport::Batches),
            (false, FdTransport::SingleMessage),
        ] {
            let dir = std::env::temp_dir().join(format!(
                "herdr-handoff-190-{}-{announcing_importer}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let socket = dir.join("handoff.sock");
            let listener = bind_listener(&socket).unwrap();
            let manifest = manifest_for(empty_snapshot(), Vec::new(), None, None, None, None);
            let importer_socket = socket.clone();
            let importer = std::thread::spawn(move || {
                if announcing_importer {
                    receive(&importer_socket, "token").map(|received| received.fds.len())
                } else {
                    // An importer from before the capability: it reads the
                    // manifest and answers the bare line whatever it asked.
                    let mut stream = UnixStream::connect(&importer_socket)?;
                    stream.write_all(b"token\n")?;
                    read_line_unbuffered(&mut stream)?;
                    stream.write_all(b"validated\n")?;
                    Ok(0)
                }
            });
            let (_stream, transport) =
                accept_and_validate_on(listener, &socket, "token", &manifest).unwrap();
            assert_eq!(
                transport, expected,
                "announcing_importer={announcing_importer}"
            );
            assert_eq!(importer.join().unwrap().unwrap(), 0);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_handoff_carries_an_api_set_window_title() {
        let manifest = manifest_for(
            empty_snapshot(),
            Vec::new(),
            None,
            None,
            Some("deploying".to_string()),
            None,
        );

        assert_eq!(manifest.api_window_title.as_deref(), Some("deploying"));
    }

    #[test]
    fn a_manifest_written_before_the_title_field_still_loads() {
        let manifest = manifest_for(
            empty_snapshot(),
            Vec::new(),
            None,
            None,
            Some("deploying".to_string()),
            None,
        );
        let mut value = serde_json::to_value(&manifest).expect("manifest should serialize");
        value
            .as_object_mut()
            .expect("manifest should be a json object")
            .remove("api_window_title");

        let older: HandoffManifest =
            serde_json::from_value(value).expect("an older manifest should still load");

        assert!(older.api_window_title.is_none());
        assert!(older.client_size.is_none());
    }

    #[test]
    fn a_handoff_carries_the_client_size() {
        let manifest = manifest_for(
            empty_snapshot(),
            Vec::new(),
            None,
            None,
            None,
            Some((310, 56)),
        );
        let value = serde_json::to_value(&manifest).expect("manifest should serialize");

        let loaded: HandoffManifest =
            serde_json::from_value(value).expect("the manifest should load back");

        assert_eq!(loaded.client_size, Some((310, 56)));
    }
}
