//! An in-process stand-in for herdr-core, speaking the subset of its protocol
//! the core backend uses, so tests run without `hc` (core-backend design.md
//! § Verifier, "fake core").

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use super::frame::{kind, Frame, FrameDecoder};

pub(crate) struct FakeConfig {
    pub generation: u64,
    pub version: String,
    pub features: Vec<&'static str>,
    /// Bytes sent on a stream right after its attach reply.
    pub first_paint: Vec<u8>,
    pub pid: u32,
    /// Sent instead of the hello reply when set.
    pub hello_line: Option<Vec<u8>>,
    /// False: after hello, never read again, as a stalled core.
    pub reads_after_hello: bool,
    /// How long `terminal.run` takes to answer, as a slow core; other calls
    /// on the connection are answered meanwhile.
    pub run_delay: std::time::Duration,
    /// `terminal.run` starts the terminal, then the connection drops before
    /// the answer, as a core that restarts mid-call (security review M3).
    pub lose_run_reply: bool,
    /// These calls answer with an `internal` error.
    pub failing: Vec<&'static str>,
    /// `terminal.kill` answers with the terminal still `live`, as a core
    /// whose kill did not take.
    pub kill_leaves_live: bool,
    /// After a `terminal.kill` that finished, ends this stream with the
    /// finished record, as core ends the streams attached to the terminal.
    pub kill_closes_stream: Option<u32>,
    /// What `machine.exec` answers (core's `ExecResult`); an empty run
    /// with exit code 0 when unset.
    pub exec_reply: Option<Value>,
    /// These calls are recorded and then the connection drops before any
    /// answer, as a link lost after the request was sent.
    pub hang_up_on: Vec<&'static str>,
    /// These calls are recorded and never answered, the connection left
    /// open, as a far program that runs on.
    pub stall_on: Vec<&'static str>,
    /// The error `machine.exec` answers instead of a result.
    pub exec_error: Option<Value>,
    /// `doc.get`'s `value`, whatever the name asked for.
    pub doc_value: Value,
}

impl Default for FakeConfig {
    fn default() -> Self {
        Self {
            generation: 1,
            version: "0.1.21".into(),
            features: vec!["framed", "attach", "events", "status", "automatic_input"],
            first_paint: Vec::new(),
            pid: 4242,
            hello_line: None,
            reads_after_hello: true,
            run_delay: std::time::Duration::ZERO,
            lose_run_reply: false,
            failing: Vec::new(),
            kill_leaves_live: false,
            kill_closes_stream: None,
            exec_reply: None,
            hang_up_on: Vec::new(),
            stall_on: Vec::new(),
            exec_error: None,
            doc_value: Value::Null,
        }
    }
}

#[derive(Default)]
struct Shared {
    stream: Option<UnixStream>,
    /// Every frame the client sent, in order.
    received: Vec<Frame>,
    /// Every JSON-RPC request the client sent.
    requests: Vec<Value>,
    /// Terminal records by address: run here, or left by an earlier server.
    terminals: std::collections::HashMap<String, Value>,
    /// Addresses core no longer knows: `terminal.get` answers `not_found`.
    gone: std::collections::HashSet<String>,
    /// Machines by name and the record `machine.list` gives them.
    machines: std::collections::BTreeMap<String, Value>,
    /// The client closed its connection.
    client_gone: bool,
    /// `terminal.run` calls not answered yet, and the most there were.
    runs_pending: usize,
    runs_pending_max: usize,
}

pub(crate) struct FakeCore {
    pub socket: PathBuf,
    dir: PathBuf,
    shared: Arc<Mutex<Shared>>,
}

impl FakeCore {
    pub(crate) fn start() -> Self {
        Self::start_with(|_| {})
    }

    pub(crate) fn start_with(configure: impl FnOnce(&mut FakeConfig)) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        // Short: Unix socket paths have a small length limit.
        let dir = std::env::temp_dir().join(format!("hcf-{}-{nanos}", std::process::id()));
        Self::start_in(dir, "c.sock", configure)
    }

    /// A fake core whose socket is `dir/name`, as core lays out a session's
    /// socket, so a test can stand it in for a named session.
    pub(crate) fn start_in(
        dir: PathBuf,
        name: &str,
        configure: impl FnOnce(&mut FakeConfig),
    ) -> Self {
        let mut config = FakeConfig::default();
        configure(&mut config);
        std::fs::create_dir_all(&dir).expect("fake core dir");
        // As core's own: a socket folder others can write to is refused.
        std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .expect("fake core dir mode");
        let socket = dir.join(name);
        let listener = UnixListener::bind(&socket).expect("fake core bind");
        let shared = Arc::new(Mutex::new(Shared::default()));
        let thread_shared = Arc::clone(&shared);
        let config = Arc::new(config);
        std::thread::spawn(move || {
            // Each client is served on its own thread; `send` and `hang_up`
            // act on the latest one.
            for stream in listener.incoming().flatten() {
                let config = Arc::clone(&config);
                let shared = Arc::clone(&thread_shared);
                std::thread::spawn(move || serve(stream, &config, shared));
            }
        });
        Self {
            socket,
            dir,
            shared,
        }
    }

    /// Sends a channel-0 notification (an event, a heartbeat) to the client.
    pub(crate) fn notify(&self, message: Value) {
        let payload = serde_json::to_vec(&message).expect("encode notification");
        self.send(Frame::new(0, kind::DATA, payload));
    }

    pub(crate) fn send(&self, frame: Frame) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            {
                let mut shared = lock(&self.shared);
                if let Some(stream) = shared.stream.as_mut() {
                    let mut bytes = Vec::new();
                    frame.encode_into(&mut bytes).expect("encode frame");
                    let _ = stream.write_all(&bytes);
                    return;
                }
            }
            assert!(std::time::Instant::now() < deadline, "no client connected");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Ends stream `channel` with `reason` (a JSON finished record, or empty
    /// for a stream that only dropped).
    pub(crate) fn close_stream(&self, channel: u32, reason: &[u8]) {
        self.send(Frame::new(channel, kind::CLOSE, reason.to_vec()));
    }

    /// Whether the client closed its connection.
    pub(crate) fn client_hung_up(&self) -> bool {
        lock(&self.shared).client_gone
    }

    /// Closes the connection, as a core restart does.
    pub(crate) fn hang_up(&self) {
        if let Some(stream) = lock(&self.shared).stream.take() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }

    pub(crate) fn received(&self) -> Vec<Frame> {
        lock(&self.shared).received.clone()
    }

    pub(crate) fn requests(&self) -> Vec<Value> {
        lock(&self.shared).requests.clone()
    }

    /// The methods the client called, in order.
    pub(crate) fn methods(&self) -> Vec<String> {
        self.requests()
            .iter()
            .filter_map(|request| request["method"].as_str().map(str::to_string))
            .collect()
    }

    /// Adds a terminal as an earlier server would have left it; `terminal.get`
    /// answers with `record`.
    pub(crate) fn add_terminal(&self, record: Value) {
        let address = record["address"].as_str().unwrap_or("t1").to_string();
        let mut shared = lock(&self.shared);
        shared.gone.remove(&address);
        shared.terminals.insert(address, record);
    }

    /// The most `terminal.run` calls that waited for an answer at once.
    pub(crate) fn most_runs_at_once(&self) -> usize {
        lock(&self.shared).runs_pending_max
    }

    /// Lists `name` in `machine.list` with `state` (`connected`,
    /// `connecting`, `unreachable`, ...).
    /// The record is one herdr's `machine add --core` makes: reached
    /// through `herdr core-link <name>`.
    pub(crate) fn set_machine(&self, name: &str, state: &str) {
        self.set_machine_record(
            name,
            json!({"state": state, "via": true, "target": format!("/bin/herdr core-link {name}")}),
        );
    }

    /// Lists `name` in `machine.list` with `record` as its fields.
    pub(crate) fn set_machine_record(&self, name: &str, mut record: Value) {
        record["name"] = json!(name);
        lock(&self.shared).machines.insert(name.to_string(), record);
    }

    /// Core forgets the terminal, as after its holder was lost and swept.
    pub(crate) fn forget_terminal(&self, address: &str) {
        let mut shared = lock(&self.shared);
        shared.terminals.remove(address);
        shared.gone.insert(address.to_string());
    }
}

impl Drop for FakeCore {
    fn drop(&mut self) {
        self.hang_up();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn serve(stream: UnixStream, config: &FakeConfig, shared: Arc<Mutex<Shared>>) {
    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(read_half);
    let mut hello = String::new();
    if reader.read_line(&mut hello).is_err() {
        return;
    }
    let id = serde_json::from_str::<Value>(hello.trim())
        .ok()
        .and_then(|message| message.get("id").cloned())
        .unwrap_or(Value::Null);
    let reply = json!({"jsonrpc": "2.0", "id": id, "result": {
        "generation": config.generation,
        "features": config.features,
        "framed": true,
        "core": {"id": "fake", "name": "fake"},
    }});
    let mut writer = match stream.try_clone() {
        Ok(writer) => writer,
        Err(_) => return,
    };
    let line = config.hello_line.clone().unwrap_or_else(|| {
        let mut line = serde_json::to_vec(&reply).unwrap_or_default();
        line.push(b'\n');
        line
    });
    if writer.write_all(&line).is_err() {
        return;
    }
    lock(&shared).stream = Some(stream);
    if !config.reads_after_hello {
        loop {
            std::thread::park();
        }
    }

    let mut decoder = FrameDecoder::default();
    decoder.push(reader.buffer());
    let mut socket = reader.into_inner();
    let mut buf = vec![0u8; 64 * 1024];
    let mut next_channel = 1u32;
    loop {
        while let Ok(Some(frame)) = decoder.next_frame() {
            lock(&shared).received.push(frame.clone());
            if frame.channel == 0 && frame.kind == kind::DATA {
                if let Ok(request) = serde_json::from_slice::<Value>(&frame.payload) {
                    lock(&shared).requests.push(request.clone());
                    answer(&request, config, &shared, &mut next_channel, &mut writer);
                }
            }
        }
        match socket.read(&mut buf) {
            Ok(0) | Err(_) => {
                lock(&shared).client_gone = true;
                return;
            }
            Ok(n) => decoder.push(&buf[..n]),
        }
    }
}

fn answer(
    request: &Value,
    config: &FakeConfig,
    shared: &Arc<Mutex<Shared>>,
    next_channel: &mut u32,
    out: &mut UnixStream,
) {
    let id = request["id"].clone();
    let method = request["method"].as_str().unwrap_or_default();
    let params = &request["params"];
    if config.hang_up_on.contains(&method) {
        let _ = out.shutdown(std::net::Shutdown::Both);
        return;
    }
    if config.stall_on.contains(&method) {
        return;
    }
    let (result, then): (Result<Value, Value>, Option<Frame>) = match method {
        _ if config.failing.contains(&method) => (
            Err(
                json!({"code": -32000, "message": format!("{method} failed"),
                        "data": {"kind": "internal", "hint": ""}}),
            ),
            None,
        ),
        "server.status" => (
            Ok(json!({"version": config.version, "pid": 1, "holders": []})),
            None,
        ),
        // Core's API takes labels as a map (the CLI's `k=v` form is CLI only).
        "terminal.run" if !(params["labels"].is_null() || params["labels"].is_object()) => (
            Err(
                json!({"code": -32602, "message": "invalid type: sequence, expected a map",
                        "data": {"kind": "invalid_params", "hint": ""}}),
            ),
            None,
        ),
        "terminal.run" => {
            // `on` runs it on a machine: core names it `<machine>/t1`.
            let address = match params["on"].as_str() {
                Some(machine) => format!("{machine}/t1"),
                None => "t1".into(),
            };
            let record = json!({"address": address, "pid": config.pid, "state": "live",
                                 "labels": params["labels"], "argv": params["argv"]});
            lock(shared).terminals.insert(address, record.clone());
            (Ok(record), None)
        }
        "terminal.attach" => {
            let channel = *next_channel;
            *next_channel += 1;
            let paint = (!config.first_paint.is_empty())
                .then(|| Frame::new(channel, kind::DATA, config.first_paint.clone()));
            // As core's own, the answer names the attached terminal's record.
            let address = params["terminal"].as_str().unwrap_or("t1");
            let terminal = lock(shared)
                .terminals
                .get(address)
                .cloned()
                .unwrap_or_else(|| json!({"address": address}));
            (
                Ok(json!({"channel": channel, "automatic_input": true, "terminal": terminal})),
                paint,
            )
        }
        "events.subscribe" => {
            let channel = *next_channel;
            *next_channel += 1;
            (
                Ok(json!({"channel": channel, "seq": 0, "terminals": []})),
                None,
            )
        }
        "terminal.get" if params["terminal"] == "t404" => (
            Err(json!({"code": -32000, "message": "no terminal t404",
                        "data": {"kind": "not_found", "hint": ""}})),
            None,
        ),
        "terminal.get"
            if lock(shared)
                .gone
                .contains(params["terminal"].as_str().unwrap_or_default()) =>
        {
            (
                Err(json!({"code": -32000, "message": "no such terminal",
                        "data": {"kind": "not_found", "hint": ""}})),
                None,
            )
        }
        "terminal.get" => {
            let address = params["terminal"].as_str().unwrap_or_default();
            let known = lock(shared).terminals.get(address).cloned();
            (
                Ok(known.unwrap_or_else(|| json!({"address": address, "state": "live"}))),
                None,
            )
        }
        "terminal.list" => {
            let want = params["labels"].as_object().cloned().unwrap_or_default();
            let mut terminals: Vec<Value> = lock(shared)
                .terminals
                .values()
                .filter(|record| {
                    want.iter()
                        .all(|(key, value)| &record["labels"][key] == value)
                })
                .cloned()
                .collect();
            terminals.sort_by_key(|record| record["address"].to_string());
            (Ok(json!({"terminals": terminals})), None)
        }
        "machine.exec" if config.exec_error.is_some() => {
            (Err(config.exec_error.clone().unwrap_or_default()), None)
        }
        "machine.exec" => (
            Ok(config.exec_reply.clone().unwrap_or_else(|| {
                json!({"stdout": "", "stderr": "", "truncated": false, "timed_out": false,
                       "ms": 0, "code": 0})
            })),
            None,
        ),
        "doc.get" => (Ok(json!({"value": config.doc_value})), None),
        "machine.list" => {
            let machines: Vec<Value> = lock(shared).machines.values().cloned().collect();
            (Ok(json!({"machines": machines})), None)
        }
        "terminal.kill" => {
            let address = params["terminal"].as_str().unwrap_or_default();
            let state = if config.kill_leaves_live {
                "live"
            } else {
                "finished"
            };
            let mut shared = lock(shared);
            let mut fallback = json!({"address": address});
            let record = shared.terminals.get_mut(address).unwrap_or(&mut fallback);
            record["state"] = json!(state);
            if state == "finished" {
                record["reason"] = json!("killed");
            }
            let close = (state == "finished")
                .then_some(config.kill_closes_stream)
                .flatten()
                .map(|channel| {
                    Frame::new(
                        channel,
                        kind::CLOSE,
                        serde_json::to_vec(&*record).unwrap_or_default(),
                    )
                });
            (Ok(record.clone()), close)
        }
        _ => (Ok(json!({})), None),
    };
    let reply = match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(error) => json!({"jsonrpc": "2.0", "id": id, "error": error}),
    };
    let mut bytes = Vec::new();
    let payload = serde_json::to_vec(&reply).unwrap_or_default();
    let _ = Frame::new(0, kind::DATA, payload).encode_into(&mut bytes);
    if let Some(frame) = then {
        let _ = frame.encode_into(&mut bytes);
    }
    if method == "terminal.run" && config.lose_run_reply {
        let _ = out.shutdown(std::net::Shutdown::Both);
        return;
    }
    if method == "terminal.run" && !config.run_delay.is_zero() {
        let Ok(mut out) = out.try_clone() else {
            return;
        };
        {
            let mut state = lock(shared);
            state.runs_pending += 1;
            state.runs_pending_max = state.runs_pending_max.max(state.runs_pending);
        }
        let shared = Arc::clone(shared);
        let delay = config.run_delay;
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            let mut state = lock(&shared);
            state.runs_pending -= 1;
            let _ = out.write_all(&bytes);
        });
        return;
    }
    // The requester's own connection: with several clients, the latest one
    // is not necessarily it.
    let _guard = lock(shared);
    let _ = out.write_all(&bytes);
}
