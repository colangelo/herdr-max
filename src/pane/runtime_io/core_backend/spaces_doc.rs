//! Writes herdr's layout into herdr-core's `herdr-app` document, the one the
//! Herdr apps read their spaces from (openspec/changes/core-spaces-doc, D5).
//! Other clients write the same document, so herdr-max owns only the spaces it
//! marks with `"herdr_max": 1` and merges them into what core holds (D6).
//! One thread, one write in flight, latest value wins, compare-and-set on the
//! document's version.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Map, Value};

use super::client::CoreError;

const DOC_NAME: &str = "herdr-app";
/// The space field that marks a space as herdr-max's own.
const MARK: &str = "herdr_max";
const CALL_TIMEOUT: Duration = Duration::from_secs(5);
/// How long an unwritten document waits before the next try.
const RETRY: Duration = Duration::from_secs(10);
/// Conflicts in a row before the writer waits for the next try.
const CONFLICT_RETRIES: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum StoreError {
    /// Someone wrote the document after the version we read.
    Conflict,
    Unavailable(String),
}

/// The document as core holds it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Doc {
    value: Value,
    version: u64,
}

/// The document in core, as the publisher needs it.
pub(super) trait DocStore {
    fn get(&mut self) -> Result<Doc, StoreError>;
    /// Writes `value` if the document is still at `version`; the new version.
    fn set(&mut self, value: &Value, version: u64) -> Result<u64, StoreError>;
}

/// A document herdr cannot read as the apps' `{"spaces": [{…}]}`, which it
/// leaves alone rather than replace someone else's data.
#[derive(Debug, PartialEq)]
struct Undecodable;

fn is_ours(space: &Value) -> bool {
    space.get(MARK).is_some()
}

/// `current` with its marked spaces replaced by `ours`, or `None` when that
/// changes nothing. Foreign spaces and other top-level fields stay as they
/// are. Ours go where the first marked space was, else after the rest. A
/// document left with no spaces and no other field becomes `null`.
fn merge(current: &Value, ours: &[Value]) -> Result<Option<Value>, Undecodable> {
    let empty = Map::new();
    let fields = match current {
        Value::Null => &empty,
        Value::Object(fields) => fields,
        _ => return Err(Undecodable),
    };
    let spaces = match fields.get("spaces") {
        None => &[][..],
        Some(Value::Array(spaces)) if spaces.iter().all(Value::is_object) => spaces.as_slice(),
        Some(_) => return Err(Undecodable),
    };
    let at = spaces.iter().position(is_ours).unwrap_or(spaces.len());
    let before = spaces[..at].iter();
    let after = spaces[at..].iter().filter(|space| !is_ours(space));
    let merged: Vec<Value> = before
        .cloned()
        .chain(ours.iter().cloned())
        .chain(after.cloned())
        .collect();
    if merged.as_slice() == spaces {
        return Ok(None);
    }
    if merged.is_empty() && fields.keys().all(|key| key == "spaces") {
        return Ok(Some(Value::Null));
    }
    let mut fields = fields.clone();
    fields.insert("spaces".into(), Value::Array(merged));
    Ok(Some(Value::Object(fields)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Flush {
    /// Nothing is left to write.
    Idle,
    /// Our spaces are still pending; try again later.
    Retry,
}

pub(super) struct DocPublisher<S> {
    store: S,
    written: Option<Vec<Value>>,
    pending: Option<Vec<Value>>,
}

impl<S: DocStore> DocPublisher<S> {
    pub(super) fn new(store: S) -> Self {
        Self {
            store,
            written: None,
            pending: None,
        }
    }

    /// herdr-max's spaces, marked here; an empty list removes ours.
    pub(super) fn offer(&mut self, mut spaces: Vec<Value>) {
        for space in &mut spaces {
            if let Value::Object(fields) = space {
                fields.insert(MARK.into(), json!(1));
            }
        }
        self.pending = (self.written.as_ref() != Some(&spaces)).then_some(spaces);
    }

    pub(super) fn flush(&mut self) -> Flush {
        let Some(ours) = self.pending.take() else {
            return Flush::Idle;
        };
        for _ in 0..CONFLICT_RETRIES {
            let doc = match self.store.get() {
                Ok(doc) => doc,
                Err(err) => {
                    note_unavailable(&format!("{err:?}"));
                    break;
                }
            };
            let merged = match merge(&doc.value, &ours) {
                Ok(Some(merged)) => merged,
                Ok(None) => {
                    self.written = Some(ours);
                    return Flush::Idle;
                }
                Err(Undecodable) => {
                    note_undecodable(&doc.value);
                    break;
                }
            };
            match self.store.set(&merged, doc.version) {
                Ok(_) => {
                    self.written = Some(ours);
                    return Flush::Idle;
                }
                Err(StoreError::Conflict) => {}
                Err(StoreError::Unavailable(reason)) => {
                    note_unavailable(&reason);
                    break;
                }
            }
        }
        self.pending = Some(ours);
        Flush::Retry
    }

    #[cfg(test)]
    pub(super) fn store(&self) -> &S {
        &self.store
    }
}

static WARNED_UNAVAILABLE: AtomicBool = AtomicBool::new(false);
static WARNED_UNDECODABLE: AtomicBool = AtomicBool::new(false);

fn note_unavailable(reason: &str) {
    if !WARNED_UNAVAILABLE.swap(true, Ordering::AcqRel) {
        tracing::warn!(
            reason,
            "cannot write the herdr-app document to herdr-core yet"
        );
    } else {
        tracing::debug!(reason, "herdr-app document not written");
    }
}

fn note_undecodable(value: &Value) {
    let kind = match value {
        Value::Object(_) => "an object whose spaces is not a list of objects",
        Value::Array(_) => "an array",
        _ => "a scalar",
    };
    if !WARNED_UNDECODABLE.swap(true, Ordering::AcqRel) {
        tracing::warn!(
            kind,
            "herdr-core's herdr-app document is not a spaces document; left as is, retrying later"
        );
    } else {
        tracing::debug!(kind, "herdr-app document still undecodable");
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
    fn get(&mut self) -> Result<Doc, StoreError> {
        let client = super::existing_core_connector()().map_err(store_error)?;
        let reply = client
            .call("doc.get", json!({"name": DOC_NAME}), CALL_TIMEOUT)
            .map_err(store_error)?;
        let version = reply["version"]
            .as_u64()
            .ok_or_else(|| StoreError::Unavailable(format!("doc.get gave no version: {reply}")))?;
        Ok(Doc {
            value: reply["value"].clone(),
            version,
        })
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

fn run(store: impl DocStore, offers: mpsc::Receiver<Vec<Value>>) {
    let mut publisher = DocPublisher::new(store);
    let mut state = Flush::Idle;
    loop {
        let offer = match state {
            Flush::Idle => offers.recv().map_err(|_| RecvTimeoutError::Disconnected),
            Flush::Retry => offers.recv_timeout(RETRY),
        };
        match offer {
            Ok(spaces) => publisher.offer(spaces),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        while let Ok(spaces) = offers.try_recv() {
            publisher.offer(spaces);
        }
        state = publisher.flush();
    }
}

static OFFERS: OnceLock<Mutex<Option<mpsc::Sender<Vec<Value>>>>> = OnceLock::new();

/// Hands the publisher thread herdr-max's latest spaces (empty removes them),
/// starting the thread on first use. Never blocks on core.
pub(crate) fn publish(spaces: Vec<Value>) {
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
        if tx.send(spaces).is_err() {
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

    /// A store that keeps one document and records its calls.
    #[derive(Default)]
    struct FakeStore {
        doc: Option<Doc>,
        /// Foreign values that land between our next reads and writes, in order.
        races: Vec<Value>,
        /// Errors the next calls return, in order, before normal answers.
        fail: Vec<StoreError>,
        calls: Vec<String>,
    }

    impl FakeStore {
        fn holding(value: Value) -> Self {
            Self {
                doc: Some(Doc { value, version: 4 }),
                ..Self::default()
            }
        }

        fn doc(&self) -> Doc {
            self.doc.clone().unwrap_or(Doc {
                value: Value::Null,
                version: 0,
            })
        }

        fn sets(&self) -> usize {
            self.calls
                .iter()
                .filter(|call| call.starts_with("set"))
                .count()
        }
    }

    impl DocStore for FakeStore {
        fn get(&mut self) -> Result<Doc, StoreError> {
            self.calls.push("get".into());
            Ok(self.doc())
        }

        fn set(&mut self, value: &Value, version: u64) -> Result<u64, StoreError> {
            self.calls.push(format!("set @{version}"));
            let current = self.doc();
            if !self.races.is_empty() {
                let foreign = self.races.remove(0);
                self.doc = Some(Doc {
                    value: foreign,
                    version: current.version + 1,
                });
                return Err(StoreError::Conflict);
            }
            if !self.fail.is_empty() {
                return Err(self.fail.remove(0));
            }
            if version != current.version {
                return Err(StoreError::Conflict);
            }
            self.doc = Some(Doc {
                value: value.clone(),
                version: version + 1,
            });
            Ok(version + 1)
        }
    }

    fn space(id: &str) -> Value {
        json!({"id": id, "name": id, "folder": "/w", "tabs": []})
    }

    fn marked(id: &str) -> Value {
        let mut space = space(id);
        space[MARK] = json!(1);
        space
    }

    fn foreign(id: &str) -> Value {
        json!({"id": id, "name": "phone", "folder": "/p", "color": "teal", "tabs": [{"id": "x"}]})
    }

    fn ids(value: &Value) -> Vec<String> {
        value["spaces"]
            .as_array()
            .map(|spaces| {
                spaces
                    .iter()
                    .map(|space| {
                        let mark = if is_ours(space) { "+" } else { "" };
                        format!("{mark}{}", space["id"].as_str().unwrap_or("?"))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn publish_to(store: FakeStore, ours: &[&str]) -> DocPublisher<FakeStore> {
        let mut p = DocPublisher::new(store);
        p.offer(ours.iter().copied().map(space).collect());
        assert_eq!(p.flush(), Flush::Idle);
        p
    }

    #[test]
    fn an_empty_document_gets_our_marked_spaces() {
        let p = publish_to(FakeStore::default(), &["w1", "w2"]);
        let doc = p.store().doc();
        assert_eq!(ids(&doc.value), ["+w1", "+w2"]);
        assert_eq!(doc.value["spaces"][0], marked("w1"));
        assert_eq!(p.store().calls, ["get", "set @0"]);
    }

    #[test]
    fn foreign_spaces_stay_untouched_and_ours_replace_ours_in_place() {
        let current =
            json!({"spaces": [foreign("f1"), marked("old"), foreign("f2"), marked("old2")]});
        let p = publish_to(FakeStore::holding(current.clone()), &["w1", "w2"]);
        let value = p.store().doc().value;
        assert_eq!(ids(&value), ["f1", "+w1", "+w2", "f2"]);
        assert_eq!(value["spaces"][0], current["spaces"][0], "every field kept");
        assert_eq!(value["spaces"][3], current["spaces"][2]);
    }

    #[test]
    fn with_none_of_ours_yet_ours_go_after_the_foreign_ones() {
        let current = json!({"spaces": [foreign("f1")], "other": true});
        let p = publish_to(FakeStore::holding(current), &["w1"]);
        let value = p.store().doc().value;
        assert_eq!(ids(&value), ["f1", "+w1"]);
        assert_eq!(value["other"], true, "top-level fields kept");
    }

    #[test]
    fn removing_ours_keeps_the_foreign_ones() {
        let current = json!({"spaces": [marked("w1"), foreign("f1"), marked("w2")]});
        let p = publish_to(FakeStore::holding(current.clone()), &[]);
        let value = p.store().doc().value;
        assert_eq!(value, json!({"spaces": [current["spaces"][1]]}));
    }

    #[test]
    fn removing_ours_from_our_own_document_clears_it() {
        let p = publish_to(FakeStore::holding(json!({"spaces": [marked("w1")]})), &[]);
        assert_eq!(p.store().doc().value, Value::Null);
    }

    #[test]
    fn a_foreign_only_document_is_left_alone_when_we_have_nothing() {
        for current in [
            json!({"spaces": []}),
            json!({"spaces": [foreign("f1")]}),
            Value::Null,
        ] {
            let p = publish_to(FakeStore::holding(current.clone()), &[]);
            assert_eq!(p.store().sets(), 0, "{current}");
            assert_eq!(p.store().doc().value, current);
        }
    }

    #[test]
    fn an_undecodable_document_is_never_overwritten_and_is_retried() {
        for current in [
            json!("text"),
            json!([1]),
            json!({"spaces": {"id": "f1"}}),
            json!({"spaces": [foreign("f1"), 7]}),
        ] {
            let mut p = DocPublisher::new(FakeStore::holding(current.clone()));
            p.offer(vec![space("w1")]);
            assert_eq!(p.flush(), Flush::Retry, "{current}");
            assert_eq!(p.store().sets(), 0);
            p.store.doc = Some(Doc {
                value: json!({"spaces": [foreign("f1")]}),
                version: 9,
            });
            assert_eq!(p.flush(), Flush::Idle, "a later try lands once it decodes");
            assert_eq!(ids(&p.store().doc().value), ["f1", "+w1"]);
        }
    }

    #[test]
    fn a_conflict_merges_again_into_the_foreign_write() {
        let mut store = FakeStore::holding(json!({"spaces": [foreign("f1"), marked("w1")]}));
        store.races = vec![json!({"spaces": [marked("w1"), foreign("f1"), foreign("f2")]})];
        let p = publish_to(store, &["w1", "w2"]);
        assert_eq!(p.store().calls, ["get", "set @4", "get", "set @5"]);
        assert_eq!(ids(&p.store().doc().value), ["+w1", "+w2", "f1", "f2"]);
    }

    #[test]
    fn conflicts_in_a_row_keep_our_spaces_for_later() {
        let mut p = DocPublisher::new(FakeStore {
            races: vec![json!({"spaces": [foreign("f1")]}); CONFLICT_RETRIES],
            ..FakeStore::default()
        });
        p.offer(vec![space("w1")]);
        assert_eq!(p.flush(), Flush::Retry);
        assert_eq!(p.store().calls.len(), 2 * CONFLICT_RETRIES);
        assert_eq!(p.flush(), Flush::Idle, "the next try lands");
        assert_eq!(ids(&p.store().doc().value), ["f1", "+w1"]);
    }

    #[test]
    fn an_unavailable_core_keeps_our_spaces_for_later() {
        let mut p = DocPublisher::new(FakeStore {
            fail: vec![StoreError::Unavailable("core restarting".into())],
            ..FakeStore::default()
        });
        p.offer(vec![space("w1")]);
        assert_eq!(p.flush(), Flush::Retry);
        assert_eq!(p.flush(), Flush::Idle);
        assert_eq!(ids(&p.store().doc().value), ["+w1"]);
        assert_eq!(p.flush(), Flush::Idle, "nothing left");
    }

    #[test]
    fn only_the_latest_offer_is_written_and_a_repeat_is_not() {
        let mut p = DocPublisher::new(FakeStore::default());
        for id in ["w1", "w2", "w3"] {
            p.offer(vec![space(id)]);
        }
        p.flush();
        p.offer(vec![space("w3")]);
        assert_eq!(p.flush(), Flush::Idle);
        assert_eq!(p.store().calls, ["get", "set @0"]);
        assert_eq!(ids(&p.store().doc().value), ["+w3"]);
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
