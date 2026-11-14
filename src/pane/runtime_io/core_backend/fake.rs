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
}

impl Default for FakeConfig {
    fn default() -> Self {
        Self {
            generation: 1,
            version: "0.1.21".into(),
            features: vec!["framed", "attach", "events", "status", "automatic_input"],
            first_paint: Vec::new(),
            pid: 4242,
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
        let mut config = FakeConfig::default();
        configure(&mut config);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        // Short: Unix socket paths have a small length limit.
        let dir = std::env::temp_dir().join(format!("hcf-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("fake core dir");
        let socket = dir.join("c.sock");
        let listener = UnixListener::bind(&socket).expect("fake core bind");
        let shared = Arc::new(Mutex::new(Shared::default()));
        let thread_shared = Arc::clone(&shared);
        std::thread::spawn(move || {
            // One client per fake core is all the tests need.
            if let Ok((stream, _)) = listener.accept() {
                serve(stream, config, thread_shared);
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
        lock(&self.shared).terminals.insert(address, record);
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

fn serve(stream: UnixStream, config: FakeConfig, shared: Arc<Mutex<Shared>>) {
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
    let mut line = serde_json::to_vec(&reply).unwrap_or_default();
    line.push(b'\n');
    if writer.write_all(&line).is_err() {
        return;
    }
    lock(&shared).stream = Some(stream);

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
                    answer(&request, &config, &shared, &mut next_channel);
                }
            }
        }
        match socket.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => decoder.push(&buf[..n]),
        }
    }
}

fn answer(request: &Value, config: &FakeConfig, shared: &Mutex<Shared>, next_channel: &mut u32) {
    let id = request["id"].clone();
    let method = request["method"].as_str().unwrap_or_default();
    let params = &request["params"];
    let (result, then): (Result<Value, Value>, Option<Frame>) = match method {
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
            let record = json!({"address": "t1", "pid": config.pid, "state": "live",
                                 "labels": params["labels"], "argv": params["argv"]});
            lock(shared).terminals.insert("t1".into(), record.clone());
            (Ok(record), None)
        }
        "terminal.attach" => {
            let channel = *next_channel;
            *next_channel += 1;
            let paint = (!config.first_paint.is_empty())
                .then(|| Frame::new(channel, kind::DATA, config.first_paint.clone()));
            let address = params["terminal"].as_str().unwrap_or("t1");
            (
                Ok(
                    json!({"channel": channel, "automatic_input": true, "terminal": {"address": address}}),
                ),
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
        "terminal.get" => {
            let address = params["terminal"].as_str().unwrap_or_default();
            let known = lock(shared).terminals.get(address).cloned();
            (
                Ok(known.unwrap_or_else(|| json!({"address": address, "state": "live"}))),
                None,
            )
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
    if let Some(stream) = lock(shared).stream.as_mut() {
        let _ = stream.write_all(&bytes);
    }
}
