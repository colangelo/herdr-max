//! Writes herdr's layout into herdr-core as the `herdr-app` document the
//! Herdr apps read their spaces from (openspec/changes/core-spaces-doc, D5).
//! One thread, one write in flight, latest value wins, compare-and-set on the
//! document's version. herdr never reads the value back (D6).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};

use super::client::CoreError;

const DOC_NAME: &str = "herdr-app";
const CALL_TIMEOUT: Duration = Duration::from_secs(5);
/// How long an unwritten document waits before the next try.
const RETRY: Duration = Duration::from_secs(10);
/// Conflicts in a row before the writer waits for the next try.
const CONFLICT_RETRIES: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum StoreError {
    /// Someone wrote the document after the version we hold.
    Conflict,
    Unavailable(String),
}

/// The document in core, as the publisher needs it.
pub(super) trait DocStore {
    /// The document's current version.
    fn get(&mut self) -> Result<u64, StoreError>;
    /// Writes `value` if the document is still at `version`; the new version.
    fn set(&mut self, value: &Value, version: u64) -> Result<u64, StoreError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Flush {
    /// Nothing is left to write.
    Idle,
    /// A value is still pending; try again later.
    Retry,
}

pub(super) struct DocPublisher<S> {
    store: S,
    version: Option<u64>,
    written: Option<Value>,
    pending: Option<Value>,
}

impl<S: DocStore> DocPublisher<S> {
    pub(super) fn new(store: S) -> Self {
        Self {
            store,
            version: None,
            written: None,
            pending: None,
        }
    }

    pub(super) fn offer(&mut self, value: Value) {
        self.pending = (self.written.as_ref() != Some(&value)).then_some(value);
    }

    pub(super) fn flush(&mut self) -> Flush {
        let Some(value) = self.pending.take() else {
            return Flush::Idle;
        };
        for _ in 0..CONFLICT_RETRIES {
            let version = match self.version {
                Some(version) => Ok(version),
                None => self.store.get(),
            };
            match version.and_then(|version| self.store.set(&value, version)) {
                Ok(version) => {
                    self.version = Some(version);
                    self.written = Some(value);
                    return Flush::Idle;
                }
                Err(StoreError::Conflict) => self.version = None,
                Err(StoreError::Unavailable(reason)) => {
                    note_unavailable(&reason);
                    self.version = None;
                    break;
                }
            }
        }
        self.pending = Some(value);
        Flush::Retry
    }

    #[cfg(test)]
    pub(super) fn store(&self) -> &S {
        &self.store
    }
}

static WARNED: AtomicBool = AtomicBool::new(false);

fn note_unavailable(reason: &str) {
    if !WARNED.swap(true, Ordering::AcqRel) {
        tracing::warn!(
            reason,
            "cannot write the herdr-app document to herdr-core yet"
        );
    } else {
        tracing::debug!(reason, "herdr-app document not written");
    }
}

/// The real store: the server's one core connection (never starts a core).
struct CoreDocStore;

fn store_error(err: CoreError) -> StoreError {
    match err {
        CoreError::Rpc { kind, .. } if kind == "conflict" => StoreError::Conflict,
        err => StoreError::Unavailable(err.to_string()),
    }
}

impl DocStore for CoreDocStore {
    fn get(&mut self) -> Result<u64, StoreError> {
        let client = super::existing_core_connector()().map_err(store_error)?;
        let reply = client
            .call("doc.get", json!({"name": DOC_NAME}), CALL_TIMEOUT)
            .map_err(store_error)?;
        reply["version"]
            .as_u64()
            .ok_or_else(|| StoreError::Unavailable(format!("doc.get gave no version: {reply}")))
    }

    fn set(&mut self, value: &Value, version: u64) -> Result<u64, StoreError> {
        let client = super::existing_core_connector()().map_err(store_error)?;
        let reply = client
            .call(
                "doc.set",
                json!({"name": DOC_NAME, "value": value, "version": version}),
                CALL_TIMEOUT,
            )
            .map_err(store_error)?;
        reply["version"]
            .as_u64()
            .ok_or_else(|| StoreError::Unavailable(format!("doc.set gave no version: {reply}")))
    }
}

fn run(store: impl DocStore, offers: mpsc::Receiver<Value>) {
    let mut publisher = DocPublisher::new(store);
    let mut state = Flush::Idle;
    loop {
        let offer = match state {
            Flush::Idle => offers.recv().map_err(|_| RecvTimeoutError::Disconnected),
            Flush::Retry => offers.recv_timeout(RETRY),
        };
        match offer {
            Ok(value) => publisher.offer(value),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        while let Ok(value) = offers.try_recv() {
            publisher.offer(value);
        }
        state = publisher.flush();
    }
}

static OFFERS: OnceLock<Mutex<Option<mpsc::Sender<Value>>>> = OnceLock::new();

/// Hands the publisher thread the latest document (`Null` clears it),
/// starting the thread on first use. Never blocks on core.
pub(crate) fn publish(value: Value) {
    let slot = OFFERS.get_or_init(|| Mutex::new(None));
    let mut sender = super::lock(slot);
    if sender.is_none() {
        let (tx, rx) = mpsc::channel();
        match std::thread::Builder::new()
            .name("herdr-core-spaces".into())
            .spawn(move || run(CoreDocStore, rx))
        {
            Ok(_) => *sender = Some(tx),
            Err(err) => {
                tracing::warn!(err = %err, "cannot start the herdr-app document writer");
                return;
            }
        }
    }
    if let Some(tx) = sender.as_ref() {
        if tx.send(value).is_err() {
            *sender = None;
        }
    }
}

/// A raw document call on the server's core connection, for live tests.
#[cfg(test)]
pub(crate) fn call_for_test(method: &str, params: Value) -> Result<Value, CoreError> {
    super::existing_core_connector()()?.call(method, params, CALL_TIMEOUT)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store that answers from a script and records its calls.
    #[derive(Default)]
    struct FakeStore {
        version: u64,
        value: Value,
        /// Errors the next calls return, in order, before normal answers.
        fail: Vec<StoreError>,
        calls: Vec<String>,
    }

    impl FakeStore {
        fn scripted_failure(&mut self) -> Option<StoreError> {
            (!self.fail.is_empty()).then(|| self.fail.remove(0))
        }
    }

    impl DocStore for FakeStore {
        fn get(&mut self) -> Result<u64, StoreError> {
            self.calls.push(format!("get -> {}", self.version));
            Ok(self.version)
        }

        fn set(&mut self, value: &Value, version: u64) -> Result<u64, StoreError> {
            self.calls.push(format!("set {value} @{version}"));
            if let Some(err) = self.scripted_failure() {
                if err == StoreError::Conflict {
                    self.version += 1;
                }
                return Err(err);
            }
            if version != self.version {
                return Err(StoreError::Conflict);
            }
            self.version += 1;
            self.value = value.clone();
            Ok(self.version)
        }
    }

    fn publisher(version: u64) -> DocPublisher<FakeStore> {
        DocPublisher::new(FakeStore {
            version,
            ..FakeStore::default()
        })
    }

    #[test]
    fn writes_with_the_known_version_and_skips_what_it_wrote() {
        let mut p = publisher(4);
        p.offer(json!({"spaces": []}));
        assert_eq!(p.flush(), Flush::Idle);
        p.offer(json!({"spaces": []}));
        assert_eq!(
            p.flush(),
            Flush::Idle,
            "an equal document is not written again"
        );
        p.offer(json!({"spaces": [{"id": "w1"}]}));
        assert_eq!(p.flush(), Flush::Idle);
        assert_eq!(
            p.store().calls,
            [
                "get -> 4",
                r#"set {"spaces":[]} @4"#,
                r#"set {"spaces":[{"id":"w1"}]} @5"#,
            ]
        );
        assert_eq!(p.store().value, json!({"spaces": [{"id": "w1"}]}));
    }

    #[test]
    fn a_conflict_rereads_the_version_and_overwrites() {
        let mut p = publisher(1);
        p.offer(json!(1));
        p.flush();
        p.store.version = 7;
        p.offer(json!(2));
        assert_eq!(p.flush(), Flush::Idle);
        assert_eq!(
            p.store().calls[2..],
            ["set 2 @2", "get -> 7", "set 2 @7"],
            "the foreign write at 7 is overwritten"
        );
        assert_eq!(p.store().value, json!(2));
    }

    #[test]
    fn conflicts_in_a_row_keep_the_document_for_later() {
        let mut p = publisher(0);
        p.store.fail = vec![StoreError::Conflict; CONFLICT_RETRIES];
        p.offer(json!(1));
        assert_eq!(p.flush(), Flush::Retry);
        assert_eq!(p.store().calls.len(), 2 * CONFLICT_RETRIES);
        assert_eq!(p.flush(), Flush::Idle, "the next try lands");
        assert_eq!(p.store().value, json!(1));
    }

    #[test]
    fn an_unavailable_core_keeps_the_document_and_forgets_the_version() {
        let mut p = publisher(2);
        p.offer(json!(1));
        p.flush();
        p.store.fail = vec![StoreError::Unavailable("core restarting".into())];
        p.offer(json!(2));
        assert_eq!(p.flush(), Flush::Retry);
        assert_eq!(p.flush(), Flush::Idle);
        assert_eq!(
            p.store().calls[2..],
            ["set 2 @3", "get -> 3", "set 2 @3"],
            "after the failure the version is read again"
        );
        assert_eq!(p.flush(), Flush::Idle, "nothing left");
    }

    #[test]
    fn only_the_latest_offer_is_written() {
        let mut p = publisher(0);
        for n in 1..=3 {
            p.offer(json!(n));
        }
        p.flush();
        assert_eq!(p.store().calls, ["get -> 0", "set 3 @0"]);
    }

    #[test]
    fn clearing_writes_null_once() {
        let mut p = publisher(0);
        p.offer(json!({"spaces": []}));
        p.flush();
        p.offer(Value::Null);
        p.flush();
        p.offer(Value::Null);
        p.flush();
        assert_eq!(
            p.store().calls.last().map(String::as_str),
            Some("set null @1")
        );
        assert_eq!(p.store().calls.len(), 3);
    }

    #[test]
    fn only_cores_conflict_kind_is_a_conflict() {
        let rpc = |kind: &str| CoreError::Rpc {
            kind: kind.into(),
            message: "version 3".into(),
        };
        assert_eq!(store_error(rpc("conflict")), StoreError::Conflict);
        assert!(matches!(
            store_error(rpc("invalid_params")),
            StoreError::Unavailable(_)
        ));
        assert!(matches!(
            store_error(CoreError::Closed),
            StoreError::Unavailable(_)
        ));
    }
}
