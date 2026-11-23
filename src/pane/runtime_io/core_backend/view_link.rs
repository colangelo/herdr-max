//! What a view server reads of the primary core (core-phase5 D6): the far
//! side's `herdr-app` document through `doc.get {on}`, the machine's state,
//! and one events subscription, which is where `doc` events of a machine,
//! `size` events of its terminals (within 2 ms of the far resize, R8) and
//! `missed` snapshots arrive. Core sends a machine's `doc` events only on a
//! subscription with no filter (measured 2026-11-23, hc 0.1.21: a label or
//! terminal filter gets none), so the view subscribes to everything and keeps
//! the facts of its own machine. Never starts a core.

use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use serde_json::{json, Value};

use super::client::{stream_channel, CoreClient, CoreError, StreamEvent, StreamEvents};
use super::frame::{kind, Frame};
use super::labels::Lines;
use super::Connector;
use crate::core_machine::MachineName;

const CALL_TIMEOUT: Duration = Duration::from_secs(5);
/// The document the far herdr publishes its spaces to (core-spaces-doc).
pub(crate) const LAYOUT_DOC: &str = "herdr-app";

/// One terminal of the viewed machine as core lists it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FarTerminal {
    /// `<machine>/t7`.
    pub address: String,
    pub rows: u16,
    pub cols: u16,
    pub live: bool,
    /// The terminal's 7501 record (`status`), `Null` when none.
    pub status: Value,
}

/// A fact of the viewed machine from the subscription.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ViewFact {
    /// The terminals core lists for the machine now: the subscription's
    /// first answer, or a `missed` after it fell behind or a reconnect.
    Snapshot(Vec<FarTerminal>),
    /// The machine's layout document changed.
    LayoutChanged,
    Size {
        address: String,
        rows: u16,
        cols: u16,
    },
    Status {
        address: String,
        status: Value,
    },
    /// The terminal finished or was removed there.
    Ended {
        address: String,
    },
}

/// The primary core's events subscription, as the view reads it.
pub(crate) struct ViewSubscription {
    client: CoreClient,
    channel: u32,
    events: StreamEvents,
    lines: Lines,
    machine: MachineName,
}

fn far_terminal(record: &Value, machine: &MachineName) -> Option<FarTerminal> {
    let address = record["address"].as_str()?;
    let parsed = super::address::CoreAddress::parse(address).ok()?;
    if parsed.machine.as_ref() != Some(machine) {
        return None;
    }
    let size = |key: &str| {
        record[key]
            .as_u64()
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or(0)
    };
    Some(FarTerminal {
        address: address.to_string(),
        rows: size("rows"),
        cols: size("cols"),
        live: record["state"] == "live",
        status: record["status"].clone(),
    })
}

fn snapshot(terminals: &Value, machine: &MachineName) -> Vec<FarTerminal> {
    terminals
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|record| far_terminal(record, machine))
        .collect()
}

/// One notification line as a fact of `machine`; `None` for anything else
/// (another machine, this Mac's terminals, heartbeats, titles, turns).
pub(crate) fn parse_fact(line: &[u8], machine: &MachineName) -> Option<ViewFact> {
    let message: Value = serde_json::from_slice(line).ok()?;
    let params = &message["params"];
    match message["method"].as_str()? {
        "missed" => Some(ViewFact::Snapshot(snapshot(&params["terminals"], machine))),
        "event" => {
            if params["type"] == "doc" {
                return (params["machine"] == machine.as_str()
                    && params["data"]["name"] == LAYOUT_DOC)
                    .then_some(ViewFact::LayoutChanged);
            }
            let address = params["terminal"].as_str()?;
            let parsed = super::address::CoreAddress::parse(address).ok()?;
            if parsed.machine.as_ref() != Some(machine) {
                return None;
            }
            let address = address.to_string();
            let data = &params["data"];
            match params["type"].as_str()? {
                "size" => {
                    let size = |key: &str| {
                        data[key]
                            .as_u64()
                            .and_then(|value| u16::try_from(value).ok())
                            .filter(|value| *value > 0)
                    };
                    Some(ViewFact::Size {
                        address,
                        rows: size("rows")?,
                        cols: size("cols")?,
                    })
                }
                "status" => Some(ViewFact::Status {
                    address,
                    status: data["status"].clone(),
                }),
                "finished" | "removed" => Some(ViewFact::Ended { address }),
                _ => None,
            }
        }
        _ => None,
    }
}

impl ViewSubscription {
    /// Subscribes on the view's connection to the primary core; the first
    /// fact is the snapshot.
    pub(crate) fn open(
        connector: &Connector,
        machine: &MachineName,
    ) -> Result<(Self, ViewFact), CoreError> {
        let client = connector()?;
        let (sink, events) = stream_channel();
        let reply = client.subscribe(json!({}), sink, CALL_TIMEOUT)?;
        let channel = reply["channel"].as_u64().unwrap_or(0) as u32;
        let first = ViewFact::Snapshot(snapshot(&reply["terminals"], machine));
        Ok((
            Self {
                client,
                channel,
                events,
                lines: Lines::default(),
                machine: machine.clone(),
            },
            first,
        ))
    }

    /// The facts that arrived within `wait`; `Err` once the subscription is
    /// gone (its connection closed, or core ended it).
    pub(crate) fn next(&mut self, wait: Duration) -> Result<Vec<ViewFact>, CoreError> {
        match self.events.recv_timeout(wait) {
            Ok(StreamEvent::Data(bytes)) => {
                let _ = self
                    .client
                    .send(&Frame::credit(self.channel, bytes.len() as u32));
                Ok(self
                    .lines
                    .push(&bytes)
                    .iter()
                    .filter_map(|line| parse_fact(line, &self.machine))
                    .collect())
            }
            Ok(StreamEvent::Close(_) | StreamEvent::ConnectionLost) => Err(CoreError::Closed),
            Ok(StreamEvent::Wake) | Err(RecvTimeoutError::Timeout) => Ok(Vec::new()),
            Err(RecvTimeoutError::Disconnected) => Err(CoreError::Closed),
        }
    }
}

impl Drop for ViewSubscription {
    fn drop(&mut self) {
        if self.channel != 0 {
            self.client.forget_stream(self.channel);
            let _ = self
                .client
                .send(&Frame::new(self.channel, kind::CLOSE, Vec::new()));
        }
    }
}

/// The room a `doc.get` reply takes beyond its document: the JSON-RPC
/// envelope and the reply's own fields.
const DOC_REPLY_ENVELOPE: usize = 16 * 1024;

/// The far side's layout document (`doc.get {name, on}`).
pub(crate) fn layout_document(
    connector: &Connector,
    machine: &MachineName,
) -> Result<Value, CoreError> {
    // Core refuses a document over 1 MiB at `doc.set`; the far core is not
    // trusted to have, so the reply is bounded before it is parsed (L5.1).
    let mut reply = connector()?.call_bounded(
        "doc.get",
        json!({"name": LAYOUT_DOC, "on": machine.as_str()}),
        CALL_TIMEOUT,
        crate::view::doc::MAX_DOC_BYTES + DOC_REPLY_ENVELOPE,
    )?;
    Ok(reply["value"].take())
}

/// Core's record of the far terminal `address` (`<machine>/t7`): its
/// state, pid and labels, which tell the program there (E.3 review, item
/// 1). Bounded as the document is: the far core is not trusted to keep a
/// record small.
pub(crate) fn terminal_record(connector: &Connector, address: &str) -> Result<Value, CoreError> {
    connector()?.call_bounded(
        "terminal.get",
        json!({"terminal": address}),
        CALL_TIMEOUT,
        TERMINAL_RECORD_BYTES,
    )
}

/// The largest `terminal.get` reply the aim check reads.
const TERMINAL_RECORD_BYTES: usize = 64 * 1024;

/// The machine's state and hc version in the primary core's `machine.list`;
/// `None` when core does not list it.
pub(crate) fn machine_record(
    connector: &Connector,
    machine: &MachineName,
) -> Result<Option<(String, String)>, CoreError> {
    let listed = connector()?.call("machine.list", json!({}), CALL_TIMEOUT)?;
    Ok(listed["machines"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|record| record["name"] == machine.as_str())
        .map(|record| {
            (
                record["state"].as_str().unwrap_or("unknown").to_string(),
                record["version"].as_str().unwrap_or_default().to_string(),
            )
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(value: Value) -> Vec<u8> {
        serde_json::to_vec(&value).unwrap()
    }

    #[test]
    fn only_the_viewed_machines_facts_are_kept() {
        let loop_ = MachineName::parse("loop").unwrap();
        let event = |kind: &str, terminal: &str, data: Value| {
            line(json!({"jsonrpc": "2.0", "method": "event",
                "params": {"seq": 3, "type": kind, "terminal": terminal, "data": data}}))
        };
        // As measured on hc 0.1.21: a far document change, a far resize.
        assert_eq!(
            parse_fact(
                &line(
                    json!({"jsonrpc":"2.0","method":"event","params":{"data":{"name":"herdr-app","version":1},"machine":"loop","seq":3,"type":"doc"}})
                ),
                &loop_
            ),
            Some(ViewFact::LayoutChanged)
        );
        assert_eq!(
            parse_fact(
                &event("size", "loop/t4", json!({"cols": 90, "rows": 25})),
                &loop_
            ),
            Some(ViewFact::Size {
                address: "loop/t4".into(),
                rows: 25,
                cols: 90
            })
        );
        // This Mac's own document, another document, another machine.
        assert_eq!(
            parse_fact(
                &line(
                    json!({"method":"event","params":{"data":{"name":"herdr-app"},"seq":4,"type":"doc"}})
                ),
                &loop_
            ),
            None
        );
        assert_eq!(
            parse_fact(
                &line(
                    json!({"method":"event","params":{"data":{"name":"herdr-max"},"machine":"loop","type":"doc"}})
                ),
                &loop_
            ),
            None
        );
        assert_eq!(
            parse_fact(
                &event("size", "t4", json!({"cols": 90, "rows": 25})),
                &loop_
            ),
            None
        );
        assert_eq!(
            parse_fact(
                &event("size", "other/t4", json!({"cols": 90, "rows": 25})),
                &loop_
            ),
            None
        );
        assert_eq!(
            parse_fact(
                &event("size", "loop/t4", json!({"cols": 0, "rows": 25})),
                &loop_
            ),
            None
        );
        assert_eq!(
            parse_fact(&event("finished", "loop/t4", json!({})), &loop_),
            Some(ViewFact::Ended {
                address: "loop/t4".into()
            })
        );
        assert_eq!(
            parse_fact(
                &event("status", "loop/t4", json!({"status": {"state": "working"}})),
                &loop_
            ),
            Some(ViewFact::Status {
                address: "loop/t4".into(),
                status: json!({"state": "working"})
            })
        );
        let missed = parse_fact(
            &line(json!({"method": "missed", "params": {"terminals": [
                {"address": "loop/t4", "rows": 30, "cols": 100, "state": "live"},
                {"address": "t1", "rows": 1, "cols": 1, "state": "live"}
            ]}})),
            &loop_,
        );
        assert_eq!(
            missed,
            Some(ViewFact::Snapshot(vec![FarTerminal {
                address: "loop/t4".into(),
                rows: 30,
                cols: 100,
                live: true,
                status: Value::Null,
            }]))
        );
        assert_eq!(parse_fact(b"not json", &loop_), None);
    }
}
