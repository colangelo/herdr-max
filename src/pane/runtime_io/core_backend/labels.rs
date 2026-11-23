//! Follows this herdr session's core terminals for `turn` and `herdr.seen`,
//! and writes `herdr.seen` back (openspec/changes/core-phase4, D11). One
//! thread, the server's one core connection (it never starts a core), one
//! framed subscription filtered on the `herdr.session` label. `status` events
//! are dropped unread: herdr never takes core's status (security review M3).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};

use super::client::{stream_channel, CoreError, StreamEvent, StreamEvents};
use super::frame::{kind, Frame};
use crate::events::AppEvent;
use crate::pane::runtime_io::{CoreAddress, CoreFact, LabelWrite, Observed};

const CALL_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a lost subscription waits before it subscribes again. Short: at
/// startup the core may start only with the first core pane.
const RETRY: Duration = Duration::from_secs(2);
/// How often the thread looks for writes while events are quiet.
const POLL: Duration = Duration::from_millis(100);

/// `[experimental] core_label_sync`: also read by `pane_labels`, so a new
/// terminal starts with `herdr.seen=0` only while the sync is on.
static ENABLED: AtomicBool = AtomicBool::new(false);
/// Bumped on every start and stop; a thread whose generation is old ends.
static GENERATION: AtomicU64 = AtomicU64::new(0);
static WRITES: Mutex<Option<mpsc::Sender<LabelWrite>>> = Mutex::new(None);

pub(crate) fn enabled() -> bool {
    ENABLED.load(Ordering::Acquire)
}

/// Starts or stops the label thread. Facts go to the server as
/// `AppEvent::CoreLabels`.
pub(crate) fn set_enabled(on: bool, events: tokio::sync::mpsc::Sender<AppEvent>) {
    if ENABLED.swap(on, Ordering::AcqRel) == on {
        return;
    }
    let generation = GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    let mut writes = super::lock(&WRITES);
    *writes = None;
    if !on {
        return;
    }
    let (tx, rx) = mpsc::channel();
    match std::thread::Builder::new()
        .name("herdr-core-labels".into())
        .spawn(move || run(generation, &rx, &events))
    {
        Ok(_) => *writes = Some(tx),
        Err(err) => tracing::warn!(err = %err, "cannot start the herdr-core label thread"),
    }
}

/// Hands a label write to the thread. Never blocks on core.
pub(crate) fn write(label: LabelWrite) {
    if let Some(tx) = super::lock(&WRITES).as_ref() {
        let _ = tx.send(label);
    }
}

fn current(generation: u64) -> bool {
    GENERATION.load(Ordering::Acquire) == generation
}

fn run(
    generation: u64,
    writes: &mpsc::Receiver<LabelWrite>,
    events: &tokio::sync::mpsc::Sender<AppEvent>,
) {
    let tell = |fact: CoreFact| {
        let _ = events.blocking_send(AppEvent::CoreLabels(fact));
    };
    let mut warned = false;
    while current(generation) {
        match follow(generation, writes, &tell) {
            Ok(()) => return,
            Err(err) => {
                if std::mem::replace(&mut warned, true) {
                    tracing::debug!(err = %err, "herdr-core labels: subscription lost");
                } else {
                    tracing::warn!(err = %err, "herdr-core labels: subscription lost");
                }
                tell(CoreFact::Disconnected);
            }
        }
        let deadline = std::time::Instant::now() + RETRY;
        while current(generation) && std::time::Instant::now() < deadline {
            // Writes while unsubscribed have no terminal to land on.
            while writes.try_recv().is_ok() {}
            std::thread::sleep(POLL);
        }
    }
}

/// One subscription: snapshot, then events and writes until it ends.
/// `Ok` when the thread was stopped, `Err` when the subscription was lost.
fn follow(
    generation: u64,
    writes: &mpsc::Receiver<LabelWrite>,
    tell: &impl Fn(CoreFact),
) -> Result<(), CoreError> {
    let client = super::existing_core_connector()()?;
    let (sink, stream) = stream_channel();
    let label = super::herdr_session_label();
    let reply = client.subscribe(
        json!({"labels": {"herdr.session": label}}),
        sink,
        CALL_TIMEOUT,
    )?;
    let channel = reply["channel"].as_u64().unwrap_or(0) as u32;
    tell(CoreFact::Snapshot(snapshot(&reply["terminals"])));
    let mut lines = Lines::default();
    let outcome = loop {
        if !current(generation) {
            break Ok(());
        }
        match next(&stream) {
            Ok(Some(StreamEvent::Data(bytes))) => {
                let _ = client.send(&Frame::credit(channel, bytes.len() as u32));
                for line in lines.push(&bytes) {
                    if let Some(fact) = parse_event(&line) {
                        tell(fact);
                    }
                }
            }
            Ok(Some(StreamEvent::Close(_) | StreamEvent::ConnectionLost)) => {
                break Err(CoreError::Closed)
            }
            Ok(Some(StreamEvent::Wake) | None) => {}
            Err(()) => break Err(CoreError::Closed),
        }
        while let Ok(label) = writes.try_recv() {
            let (terminal, labels) = label_call(&label);
            if let Err(err) = client.call(
                "terminal.label",
                json!({"terminal": terminal, "labels": labels}),
                CALL_TIMEOUT,
            ) {
                tracing::debug!(err = %err, terminal, "herdr-core labels: write failed");
            }
        }
    };
    if channel != 0 {
        client.forget_stream(channel);
        let _ = client.send(&Frame::new(channel, kind::CLOSE, Vec::new()));
    }
    outcome
}

fn next(stream: &StreamEvents) -> Result<Option<StreamEvent>, ()> {
    match stream.recv_timeout(POLL) {
        Ok(event) => Ok(Some(event)),
        Err(RecvTimeoutError::Timeout) => Ok(None),
        Err(RecvTimeoutError::Disconnected) => Err(()),
    }
}

fn label_call(label: &LabelWrite) -> (&str, Value) {
    match label {
        LabelWrite::Seen { address, turn } => {
            (address.as_str(), json!({"herdr.seen": turn.to_string()}))
        }
        LabelWrite::Pinned { address, pinned } => (
            address.as_str(),
            json!({"herdr.pinned": if *pinned { "1" } else { "" }}),
        ),
    }
}

/// From a core `Terminal` record (`turn`, `labels`).
fn observed(record: &Value) -> Observed {
    Observed {
        turn: record["turn"].as_u64().unwrap_or(0),
        seen: seen_label(&record["labels"]),
        pinned: pinned_label(&record["labels"]),
    }
}

/// `herdr.pinned` from a labels map: present and non-empty.
fn pinned_label(labels: &Value) -> bool {
    labels["herdr.pinned"]
        .as_str()
        .is_some_and(|value| !value.is_empty())
}

/// `herdr.seen` from a labels map: a decimal `u64`, anything else missing.
fn seen_label(labels: &Value) -> Option<u64> {
    labels["herdr.seen"].as_str()?.parse().ok()
}

fn snapshot(terminals: &Value) -> Vec<(CoreAddress, Observed)> {
    terminals
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|record| {
            let address = record["address"].as_str()?.to_string();
            Some((address, observed(record)))
        })
        .collect()
}

/// One notification line as a fact; `None` for what herdr does not follow
/// (`status`, `title`, `size`, `heartbeat`, ...).
fn parse_event(line: &[u8]) -> Option<CoreFact> {
    let message: Value = serde_json::from_slice(line).ok()?;
    let params = &message["params"];
    match message["method"].as_str()? {
        "missed" => Some(CoreFact::Snapshot(snapshot(&params["terminals"]))),
        "event" => {
            let address = params["terminal"].as_str()?.to_string();
            let data = &params["data"];
            match params["type"].as_str()? {
                "created" | "restarted" => Some(CoreFact::Upsert {
                    address,
                    observed: observed(data),
                }),
                "turn" => Some(CoreFact::Turn {
                    address,
                    turn: data["turn"].as_u64()?,
                }),
                "labels" if data["record"].is_object() => Some(CoreFact::Upsert {
                    address,
                    observed: observed(&data["record"]),
                }),
                "labels" => Some(CoreFact::Labels {
                    address,
                    seen: seen_label(&data["labels"]),
                    pinned: pinned_label(&data["labels"]),
                }),
                "removed" | "finished" => Some(CoreFact::Gone { address }),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Splits a stream's bytes into whole lines, keeping a cut line for the next
/// frame.
#[derive(Default)]
pub(super) struct Lines {
    partial: Vec<u8>,
}

impl Lines {
    /// The longest line kept while waiting for its end; a longer one is
    /// dropped (a snapshot of 1000 terminals is well under this).
    const MAX: usize = 16 * 1024 * 1024;

    pub(super) fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        self.partial.extend_from_slice(bytes);
        let mut lines = Vec::new();
        while let Some(at) = self.partial.iter().position(|byte| *byte == b'\n') {
            let rest = self.partial.split_off(at + 1);
            let mut line = std::mem::replace(&mut self.partial, rest);
            line.pop();
            if !line.is_empty() {
                lines.push(line);
            }
        }
        if self.partial.len() > Self::MAX {
            self.partial.clear();
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: &str, data: Value) -> Vec<u8> {
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "method": "event",
            "params": {"seq": 1, "type": kind, "terminal": "t7", "data": data}}))
        .unwrap()
    }

    // core-phase4 verifier 13: core's status never becomes a fact herdr reads.
    #[test]
    fn status_events_are_dropped_and_turns_and_labels_are_read() {
        assert_eq!(
            parse_event(&event(
                "status",
                json!({"status": {"state": "working"}, "records": {}, "reported": true})
            )),
            None
        );
        assert_eq!(
            parse_event(&event("turn", json!({"turn": 4, "state": "done"}))),
            Some(CoreFact::Turn {
                address: "t7".into(),
                turn: 4
            })
        );
        assert_eq!(
            parse_event(&event(
                "labels",
                json!({"labels": {"herdr.seen": "3", "herdr.session": "herdr"}})
            )),
            Some(CoreFact::Labels {
                address: "t7".into(),
                seen: Some(3),
                pinned: false,
            })
        );
        assert_eq!(
            parse_event(&event(
                "labels",
                json!({"labels": {"herdr.seen": "3", "herdr.pinned": "1"}})
            )),
            Some(CoreFact::Labels {
                address: "t7".into(),
                seen: Some(3),
                pinned: true,
            })
        );
        assert_eq!(
            parse_event(&event(
                "labels",
                json!({"labels": {}, "record": {"turn": 2, "labels": {}}})
            )),
            Some(CoreFact::Upsert {
                address: "t7".into(),
                observed: Observed {
                    turn: 2,
                    seen: None,
                    pinned: false,
                }
            })
        );
        assert_eq!(
            parse_event(&event("removed", json!({}))),
            Some(CoreFact::Gone {
                address: "t7".into()
            })
        );
        let missed = serde_json::to_vec(&json!({"jsonrpc": "2.0", "method": "missed",
            "params": {"seq": 9, "terminals": [{"address": "t1", "turn": 1, "labels": {"herdr.seen": "1"}}]}}))
        .unwrap();
        assert_eq!(
            parse_event(&missed),
            Some(CoreFact::Snapshot(vec![(
                "t1".into(),
                Observed {
                    turn: 1,
                    seen: Some(1),
                    pinned: false,
                }
            )]))
        );
        assert_eq!(
            parse_event(br#"{"jsonrpc":"2.0","method":"heartbeat","params":{"seq":3}}"#),
            None
        );
        assert_eq!(parse_event(b"not json"), None);
    }

    #[test]
    fn the_seen_label_must_be_a_number() {
        let record = serde_json::json!({"turn": 5, "labels": {"herdr.seen": "4"}});
        assert_eq!(
            observed(&record),
            Observed {
                turn: 5,
                seen: Some(4),
                pinned: false,
            }
        );
        for bad in ["", "-1", "four", "4.0"] {
            let labels = serde_json::json!({"herdr.seen": bad});
            assert_eq!(seen_label(&labels), None, "{bad:?}");
        }
        assert_eq!(seen_label(&serde_json::json!({})), None, "missing is not 0");
    }

    #[test]
    fn lines_survive_any_cut() {
        let text = b"{\"a\":1}\n{\"b\":2}\n";
        for cut in 0..=text.len() {
            let mut lines = Lines::default();
            let mut got = lines.push(&text[..cut]);
            got.extend(lines.push(&text[cut..]));
            assert_eq!(
                got,
                vec![b"{\"a\":1}".to_vec(), b"{\"b\":2}".to_vec()],
                "cut {cut}"
            );
        }
    }

    #[test]
    fn a_seen_write_is_a_decimal_label() {
        let write = LabelWrite::Seen {
            address: "t7".into(),
            turn: 12,
        };
        let (terminal, labels) = label_call(&write);
        assert_eq!(terminal, "t7");
        assert_eq!(labels, json!({"herdr.seen": "12"}));
        for (pinned, value) in [(true, "1"), (false, "")] {
            let write = LabelWrite::Pinned {
                address: "t7".into(),
                pinned,
            };
            assert_eq!(label_call(&write).1, json!({"herdr.pinned": value}));
        }
    }
}
