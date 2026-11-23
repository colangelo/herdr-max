//! The view server's watcher (core-phase5 D6): one thread that follows the
//! viewed machine through the primary core and tells the server loop what
//! changed. It reads the far `herdr-app` document on start, on its `doc`
//! event, on `missed` and on wake; forwards the far terminals' sizes and
//! status; reads `machine.list` only while a client is attached and the
//! machine is away or a client typed lately (so an idle view sends nothing,
//! V30); and wakes the machine only while a lease is in force (D3), through
//! this process's dialer (D3a). It never starts a core and never dials
//! itself.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::doc::{self, ViewLayout};
use super::lifetime::ViewPresence;
use crate::core_machine::MachineName;
use crate::events::AppEvent;
use crate::pane::view_link::{self, FarTerminal, ViewFact, ViewSubscription};

/// How often the machine's state is read while that is worth it.
const STATE_EVERY: Duration = Duration::from_secs(5);
/// While the machine is away and a person types in the view, so a wake shows
/// within a second or two of the link (V4).
const STATE_EVERY_WAKING: Duration = Duration::from_secs(1);
/// Input this recent keeps the state read going while the machine is up,
/// so a drop under a typing person is seen within `STATE_EVERY`.
const TYPING_WINDOW: Duration = Duration::from_secs(30);
/// The pause before a lost subscription is opened again.
const RESUBSCRIBE: Duration = Duration::from_secs(2);
const TICK: Duration = Duration::from_millis(250);
/// The far hc version D9 asks for.
const MIN_FAR_VERSION: (u64, u64, u64) = (0, 1, 21);

/// What the watcher tells the server loop.
pub(crate) enum ViewEvent {
    /// The far layout, read and checked (D11).
    Layout(ViewLayout),
    /// The view's notice line: dormant, waking, nothing published, an old
    /// far core; `None` once all is well.
    Notice(Option<String>),
    /// The far terminal's size now (rows, cols).
    FarSize {
        address: String,
        rows: u16,
        cols: u16,
    },
    /// The far terminal's 7501 record, as the far herdr mirrors its verdict.
    Status { address: String, status: Value },
    /// A pane's far terminal attached, off the loop.
    Attached {
        address: String,
        runtime: Box<crate::terminal::TerminalRuntime>,
    },
    /// It could not attach; the pane says why and is asked for again on the
    /// next layout or wake.
    AttachFailed { address: String, reason: String },
}

impl std::fmt::Debug for ViewEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Layout(layout) => f.debug_tuple("Layout").field(&layout.spaces.len()).finish(),
            Self::Notice(notice) => f.debug_tuple("Notice").field(notice).finish(),
            Self::FarSize {
                address,
                rows,
                cols,
            } => {
                write!(f, "FarSize({address} {cols}x{rows})")
            }
            Self::Status { address, .. } => write!(f, "Status({address})"),
            Self::Attached { address, .. } => write!(f, "Attached({address})"),
            Self::AttachFailed { address, reason } => {
                write!(f, "AttachFailed({address}: {reason})")
            }
        }
    }
}

/// What the watcher needs from the world, behind a trait so its decisions
/// are tested without a core.
pub(crate) trait ViewSource: Send + 'static {
    fn subscribe(&mut self) -> Result<Vec<ViewFact>, String>;
    /// Facts that arrived within `wait`; `Err` once the subscription is lost.
    fn next(&mut self, wait: Duration) -> Result<Vec<ViewFact>, String>;
    fn layout(&mut self) -> Result<Value, String>;
    /// The machine's state and far hc version; `None` when core does not
    /// list it.
    fn machine(&mut self) -> Result<Option<(String, String)>, String>;
    /// A lease is in force for the machine (D3).
    fn leased(&mut self) -> bool;
    /// Posts a wake to this process's dialer (D3a); returns at once.
    fn wake(&mut self);
    /// Records the machine's link for the panes' input gate (D5).
    fn mark_connected(&mut self, connected: bool);
}

/// The real source: the primary core through the view's connection, the
/// lease files under the passwd home, and the process's dialer.
pub(crate) struct CoreSource {
    machine: MachineName,
    connector: crate::pane::CoreConnector,
    subscription: Option<ViewSubscription>,
}

impl CoreSource {
    pub(crate) fn new(machine: MachineName) -> Self {
        Self {
            machine,
            connector: crate::pane::view_connector(),
            subscription: None,
        }
    }
}

impl ViewSource for CoreSource {
    fn subscribe(&mut self) -> Result<Vec<ViewFact>, String> {
        let (subscription, first) = ViewSubscription::open(&self.connector, &self.machine)
            .map_err(|err| err.to_string())?;
        self.subscription = Some(subscription);
        Ok(vec![first])
    }

    fn next(&mut self, wait: Duration) -> Result<Vec<ViewFact>, String> {
        let Some(subscription) = self.subscription.as_mut() else {
            return Err("not subscribed".into());
        };
        subscription.next(wait).map_err(|err| {
            self.subscription = None;
            err.to_string()
        })
    }

    fn layout(&mut self) -> Result<Value, String> {
        view_link::layout_document(&self.connector, &self.machine).map_err(|err| err.to_string())
    }

    fn machine(&mut self) -> Result<Option<(String, String)>, String> {
        view_link::machine_record(&self.connector, &self.machine).map_err(|err| err.to_string())
    }

    fn leased(&mut self) -> bool {
        !crate::core_machine::panes::dormant(&self.machine)
    }

    fn wake(&mut self) {
        crate::core_machine::panes::link(&self.machine).wake();
    }

    fn mark_connected(&mut self, connected: bool) {
        crate::core_machine::panes::link(&self.machine).set_connected(connected);
    }
}

/// The watcher's state between ticks.
pub(crate) struct Watcher<S> {
    machine: MachineName,
    source: S,
    presence: Arc<ViewPresence>,
    subscribed: bool,
    /// The machine's last state, `None` before the first read.
    state: Option<String>,
    far_version: Option<String>,
    next_state_read: Instant,
    next_subscribe: Instant,
    /// The layout must be read again (start, `doc` event, `missed`, wake).
    layout_due: bool,
    /// The last good layout's text, so an unchanged document is not sent.
    last_layout: Option<ViewLayout>,
    /// A refused document is logged once until a good one comes.
    warned_bad: bool,
    notice: Option<String>,
    sent_notice: bool,
    clients_before: usize,
}

impl<S: ViewSource> Watcher<S> {
    pub(crate) fn new(
        machine: MachineName,
        source: S,
        presence: Arc<ViewPresence>,
        now: Instant,
    ) -> Self {
        Self {
            machine,
            source,
            presence,
            subscribed: false,
            state: None,
            far_version: None,
            next_state_read: now,
            next_subscribe: now,
            layout_due: true,
            last_layout: None,
            warned_bad: false,
            notice: None,
            sent_notice: false,
            clients_before: 0,
        }
    }

    fn connected(&self) -> bool {
        self.state.as_deref() == Some("connected")
    }

    /// One pass: what the server loop must hear now.
    pub(crate) fn tick(&mut self, now: Instant, wait: Duration) -> Vec<ViewEvent> {
        let mut out = Vec::new();
        let clients = self.presence.clients();
        let first_client = clients > 0 && self.clients_before == 0;
        self.clients_before = clients;
        if !self.subscribed && now >= self.next_subscribe {
            match self.source.subscribe() {
                Ok(facts) => {
                    self.subscribed = true;
                    self.apply(facts, &mut out);
                }
                Err(err) => {
                    tracing::debug!(machine = %self.machine, %err, "view: no subscription yet");
                    self.next_subscribe = now + RESUBSCRIBE;
                }
            }
        }
        if self.subscribed {
            match self.source.next(wait) {
                Ok(facts) => self.apply(facts, &mut out),
                Err(err) => {
                    tracing::debug!(machine = %self.machine, %err, "view: subscription lost");
                    self.subscribed = false;
                    self.next_subscribe = now + RESUBSCRIBE;
                }
            }
        } else {
            std::thread::sleep(wait);
        }
        // The machine's state: on start, on a first client, and while a
        // client is attached and the machine is away or someone types.
        let worth_reading = self.state.is_none()
            || first_client
            || (clients > 0
                && (!self.connected() || self.presence.typed_within(now, TYPING_WINDOW)));
        if worth_reading && (first_client || now >= self.next_state_read) {
            let waking = !self.connected() && self.presence.typed_within(now, TYPING_WINDOW);
            self.next_state_read = now
                + if waking {
                    STATE_EVERY_WAKING
                } else {
                    STATE_EVERY
                };
            self.read_state(clients);
        }
        if self.layout_due && self.connected() {
            self.layout_due = false;
            self.read_layout(&mut out);
        }
        if !self.sent_notice {
            self.sent_notice = true;
            out.push(ViewEvent::Notice(self.notice.clone()));
        }
        out
    }

    fn set_notice(&mut self, notice: Option<String>) {
        if self.notice != notice {
            self.notice = notice;
            self.sent_notice = false;
        }
    }

    fn apply(&mut self, facts: Vec<ViewFact>, out: &mut Vec<ViewEvent>) {
        for fact in facts {
            match fact {
                ViewFact::Snapshot(terminals) => {
                    // After a reconnect (`missed`, P6) the layout and the
                    // sizes may have changed while the link was down.
                    self.layout_due = true;
                    for FarTerminal {
                        address,
                        rows,
                        cols,
                        live,
                        status,
                    } in terminals
                    {
                        if live && rows > 0 && cols > 0 {
                            out.push(ViewEvent::FarSize {
                                address: address.clone(),
                                rows,
                                cols,
                            });
                        }
                        if !status.is_null() {
                            out.push(ViewEvent::Status { address, status });
                        }
                    }
                }
                ViewFact::LayoutChanged => self.layout_due = true,
                ViewFact::Size {
                    address,
                    rows,
                    cols,
                } => out.push(ViewEvent::FarSize {
                    address,
                    rows,
                    cols,
                }),
                ViewFact::Status { address, status } => {
                    out.push(ViewEvent::Status { address, status })
                }
                ViewFact::Ended { .. } => self.layout_due = true,
            }
        }
    }

    fn read_state(&mut self, clients: usize) {
        let record = match self.source.machine() {
            Ok(record) => record,
            Err(err) => {
                tracing::debug!(machine = %self.machine, %err, "view: machine.list failed");
                self.set_notice(Some(format!(
                    "Herdr Core is not answering; {} cannot be shown",
                    self.machine
                )));
                return;
            }
        };
        let Some((state, version)) = record else {
            self.state = Some("not_listed".into());
            self.set_notice(Some(format!(
                "{} is not a machine of this Herdr Core; add it with herdr machine add --core",
                self.machine
            )));
            return;
        };
        let was_connected = self.connected();
        self.state = Some(state.clone());
        self.far_version = Some(version);
        self.source.mark_connected(self.connected());
        if self.connected() {
            if !was_connected {
                // A link just came up: read the layout again (wake, D6).
                self.layout_due = true;
            }
            self.set_notice(self.far_version_notice());
            return;
        }
        // Away. Only a lease in force lets the view wake it (D3): a person's
        // client writes one on input, and at its start.
        let leased = self.source.leased();
        if leased && clients > 0 {
            self.source.wake();
        }
        self.set_notice(Some(away_notice(&self.machine, &state, leased)));
    }

    fn far_version_notice(&self) -> Option<String> {
        let version = self.far_version.as_deref()?;
        let parsed = crate::pane::core_machines::parse_core_version(version)?;
        (parsed < MIN_FAR_VERSION).then(|| {
            format!(
                "{} runs Herdr Core {version}, older than 0.1.21: run hc machine update {} (it restarts Herdr Core there; its terminals keep running)",
                self.machine, self.machine
            )
        })
    }

    fn read_layout(&mut self, out: &mut Vec<ViewEvent>) {
        let value = match self.source.layout() {
            Ok(value) => value,
            Err(err) => {
                tracing::debug!(machine = %self.machine, %err, "view: doc.get failed; keeping the layout");
                self.layout_due = true;
                return;
            }
        };
        match doc::parse(&value) {
            Ok(layout) => {
                self.warned_bad = false;
                if layout.spaces.is_empty() {
                    self.set_notice(Some(format!(
                        "{}'s herdr publishes no spaces: turn on [experimental] core_spaces_doc there",
                        self.machine
                    )));
                } else if self
                    .notice
                    .as_deref()
                    .is_some_and(|notice| notice.contains("publishes no spaces"))
                {
                    self.set_notice(self.far_version_notice());
                }
                if self.last_layout.as_ref() != Some(&layout) {
                    self.last_layout = Some(layout.clone());
                    out.push(ViewEvent::Layout(layout));
                }
            }
            Err(err) => {
                // D11: the last good layout stays; one warning.
                if !std::mem::replace(&mut self.warned_bad, true) {
                    tracing::warn!(machine = %self.machine, %err, "the far herdr-app document was refused; the view keeps its last good layout");
                }
            }
        }
    }
}

fn away_notice(machine: &MachineName, state: &str, leased: bool) -> String {
    match (state, leased) {
        ("needs_login", _) => format!("{machine} needs a login: run herdr machine login {machine}"),
        ("connecting", _) => format!("connecting to {machine}…"),
        (_, true) => format!("waking {machine}…"),
        (_, false) => format!("{machine} is dormant: press a key to wake it"),
    }
}

/// Starts the watcher thread for `machine`; it ends when the server loop's
/// event channel closes.
pub(crate) fn start(
    machine: MachineName,
    presence: Arc<ViewPresence>,
    events: tokio::sync::mpsc::Sender<AppEvent>,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("herdr-view-watch".into())
        .spawn(move || {
            let mut watcher = Watcher::new(
                machine.clone(),
                CoreSource::new(machine),
                presence,
                Instant::now(),
            );
            loop {
                for event in watcher.tick(Instant::now(), TICK) {
                    if events.blocking_send(AppEvent::View(event)).is_err() {
                        return;
                    }
                }
                if events.is_closed() {
                    return;
                }
            }
        })
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::VecDeque;

    #[derive(Default)]
    struct Fake {
        facts: VecDeque<Vec<ViewFact>>,
        doc: Value,
        state: Option<(String, String)>,
        leased: bool,
        wakes: usize,
        layout_reads: usize,
        state_reads: usize,
        marked: Vec<bool>,
    }

    impl ViewSource for Fake {
        fn subscribe(&mut self) -> Result<Vec<ViewFact>, String> {
            Ok(vec![ViewFact::Snapshot(Vec::new())])
        }
        fn next(&mut self, _wait: Duration) -> Result<Vec<ViewFact>, String> {
            Ok(self.facts.pop_front().unwrap_or_default())
        }
        fn layout(&mut self) -> Result<Value, String> {
            self.layout_reads += 1;
            Ok(self.doc.clone())
        }
        fn machine(&mut self) -> Result<Option<(String, String)>, String> {
            self.state_reads += 1;
            Ok(self.state.clone())
        }
        fn leased(&mut self) -> bool {
            self.leased
        }
        fn wake(&mut self) {
            self.wakes += 1;
        }
        fn mark_connected(&mut self, connected: bool) {
            self.marked.push(connected);
        }
    }

    fn space_doc(name: &str) -> Value {
        json!({"spaces": [{"id": "w1", "name": name, "herdr_max": 1, "tabs": [
            {"id": "w1:t1", "root": {"id": "w1:t1", "type": "row", "children": [
                {"id": "w1:p1", "type": "block", "content": {"terminal": "t1"}}]}}]}]})
    }

    fn watcher(fake: Fake) -> (Watcher<Fake>, Arc<ViewPresence>) {
        let presence = Arc::new(ViewPresence::default());
        let machine = MachineName::parse("loop").unwrap();
        (
            Watcher::new(machine, fake, Arc::clone(&presence), Instant::now()),
            presence,
        )
    }

    fn layouts(events: &[ViewEvent]) -> Vec<&ViewLayout> {
        events
            .iter()
            .filter_map(|event| match event {
                ViewEvent::Layout(layout) => Some(layout),
                _ => None,
            })
            .collect()
    }

    fn notices(events: &[ViewEvent]) -> Vec<Option<&str>> {
        events
            .iter()
            .filter_map(|event| match event {
                ViewEvent::Notice(notice) => Some(notice.as_deref()),
                _ => None,
            })
            .collect()
    }

    // V15's unit part (D6 layout): read at start, again on the machine's doc
    // event, and only a changed document reaches the loop.
    #[test]
    fn the_layout_is_read_at_start_and_again_on_its_doc_event() {
        let (mut watch, _) = watcher(Fake {
            doc: space_doc("api"),
            state: Some(("connected".into(), "0.1.21".into())),
            ..Fake::default()
        });
        let now = Instant::now();
        let first = watch.tick(now, Duration::ZERO);
        assert_eq!(layouts(&first).len(), 1);
        assert_eq!(layouts(&first)[0].spaces[0].name, "api");
        assert_eq!(notices(&first), [None]);

        assert!(
            layouts(&watch.tick(now, Duration::ZERO)).is_empty(),
            "nothing changed"
        );
        watch.source.doc = space_doc("api, renamed");
        watch.source.facts.push_back(vec![ViewFact::LayoutChanged]);
        let changed = watch.tick(now, Duration::ZERO);
        assert_eq!(layouts(&changed)[0].spaces[0].name, "api, renamed");

        watch.source.facts.push_back(vec![ViewFact::LayoutChanged]);
        assert!(
            layouts(&watch.tick(now, Duration::ZERO)).is_empty(),
            "same text: not sent"
        );
        assert_eq!(watch.source.layout_reads, 3);
    }

    // V16's watcher part (D11): a refused document leaves the last good one.
    #[test]
    fn a_refused_document_keeps_the_last_good_layout() {
        let (mut watch, _) = watcher(Fake {
            doc: space_doc("good"),
            state: Some(("connected".into(), "0.1.21".into())),
            ..Fake::default()
        });
        let now = Instant::now();
        assert_eq!(layouts(&watch.tick(now, Duration::ZERO)).len(), 1);
        watch.source.doc = json!({"spaces": [{"id": "../../x", "herdr_max": 1}]});
        watch.source.facts.push_back(vec![ViewFact::LayoutChanged]);
        assert!(layouts(&watch.tick(now, Duration::ZERO)).is_empty());
        assert!(watch.warned_bad);
        watch.source.doc = space_doc("good");
        watch.source.facts.push_back(vec![ViewFact::LayoutChanged]);
        assert!(
            layouts(&watch.tick(now, Duration::ZERO)).is_empty(),
            "back to the same good layout"
        );
    }

    // D3 / V33's view part: a dormant machine is woken only with a lease and
    // a client; the layout waits for the link.
    #[test]
    fn a_dormant_machine_is_woken_only_with_a_lease_and_a_client() {
        let (mut watch, presence) = watcher(Fake {
            doc: space_doc("api"),
            state: Some(("unreachable".into(), "0.1.21".into())),
            ..Fake::default()
        });
        let now = Instant::now();
        let first = watch.tick(now, Duration::ZERO);
        assert_eq!(
            notices(&first),
            [Some("loop is dormant: press a key to wake it")]
        );
        assert!(layouts(&first).is_empty(), "no layout read while away");
        assert_eq!(watch.source.wakes, 0, "no client, no lease: nothing dials");

        presence.set_clients(1);
        watch.tick(now, Duration::ZERO);
        assert_eq!(
            watch.source.wakes, 0,
            "a client but no lease: nothing dials"
        );

        watch.source.leased = true;
        let waking = watch.tick(now + STATE_EVERY, Duration::ZERO);
        assert_eq!(watch.source.wakes, 1);
        assert_eq!(notices(&waking), [Some("waking loop…")]);

        watch.source.state = Some(("connected".into(), "0.1.21".into()));
        let up = watch.tick(now + STATE_EVERY * 2, Duration::ZERO);
        assert_eq!(
            layouts(&up).len(),
            1,
            "the link came up: the layout is read"
        );
        assert_eq!(notices(&up), [None]);
        assert_eq!(watch.source.marked.last(), Some(&true));
    }

    // V30's watcher part: an idle view with its machine connected reads
    // nothing of the machine.
    #[test]
    fn an_idle_connected_view_stops_reading_the_machines_state() {
        let (mut watch, presence) = watcher(Fake {
            doc: space_doc("api"),
            state: Some(("connected".into(), "0.1.21".into())),
            ..Fake::default()
        });
        presence.set_clients(1);
        let now = Instant::now();
        watch.tick(now, Duration::ZERO);
        let reads = watch.source.state_reads;
        for second in 1..=60 {
            watch.tick(now + Duration::from_secs(second), Duration::ZERO);
        }
        assert_eq!(watch.source.state_reads, reads, "no input: no machine.list");
        presence.input(now + Duration::from_secs(60));
        watch.tick(now + Duration::from_secs(61), Duration::ZERO);
        assert_eq!(
            watch.source.state_reads,
            reads + 1,
            "typing: the link is watched again"
        );
    }

    #[test]
    fn an_old_far_core_is_named_with_its_update_command() {
        let (mut watch, _) = watcher(Fake {
            doc: space_doc("api"),
            state: Some(("connected".into(), "0.1.20".into())),
            ..Fake::default()
        });
        let events = watch.tick(Instant::now(), Duration::ZERO);
        let notice = notices(&events)[0].expect("a notice");
        assert!(notice.contains("hc machine update loop"), "{notice}");
    }

    #[test]
    fn sizes_and_status_of_the_snapshot_reach_the_loop() {
        let (mut watch, _) = watcher(Fake {
            doc: space_doc("api"),
            state: Some(("connected".into(), "0.1.21".into())),
            ..Fake::default()
        });
        watch.source.facts.push_back(vec![
            ViewFact::Snapshot(vec![FarTerminal {
                address: "loop/t1".into(),
                rows: 30,
                cols: 100,
                live: true,
                status: json!({"state": "working"}),
            }]),
            ViewFact::Size {
                address: "loop/t1".into(),
                rows: 25,
                cols: 90,
            },
        ]);
        let events = watch.tick(Instant::now(), Duration::ZERO);
        let sizes: Vec<(u16, u16)> = events
            .iter()
            .filter_map(|event| match event {
                ViewEvent::FarSize { rows, cols, .. } => Some((*rows, *cols)),
                _ => None,
            })
            .collect();
        assert_eq!(sizes, [(30, 100), (25, 90)]);
        assert!(events
            .iter()
            .any(|event| matches!(event, ViewEvent::Status { .. })));
    }
}
