//! herdr-max's extras as the herdr-core document `herdr-max`
//! (openspec/changes/core-phase4, D1 to D4): what herdr-max keeps beyond the
//! `herdr-app` document, keyed by the same space, tab and pane ids, for
//! readers on other machines. One way: herdr never reads it back, and
//! `session.json` stays the home of every fact.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::terminal::todo::TodoPriority;
use crate::terminal::{TerminalId, TerminalState};
use crate::workspace::{public_pane_id_for_number, public_tab_id_for_number, Workspace};

use super::App;

/// The shape's version. A change an old reader would misread needs a new one.
const FORMAT: u32 = 1;

#[derive(Debug, Serialize)]
pub(super) struct ExtrasDocument {
    format: u32,
    writer: Writer,
    spaces: Vec<ExtrasSpace>,
}

#[derive(Debug, Serialize)]
struct Writer {
    session: String,
    version: String,
}

#[derive(Debug, Serialize)]
struct ExtrasSpace {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pin: Option<usize>,
    custom_name: Option<String>,
    tokens: BTreeMap<String, Token>,
    tabs: Vec<ExtrasTab>,
    panes: Vec<ExtrasPane>,
}

#[derive(Debug, Serialize, PartialEq)]
struct Token {
    value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
struct ExtrasTab {
    id: String,
    custom_name: Option<String>,
}

#[derive(Debug, Serialize)]
struct ExtrasPane {
    id: String,
    terminal: String,
    label: Option<String>,
    agent_name: Option<String>,
    todos: Vec<TodoRef>,
}

#[derive(Debug, Serialize, PartialEq)]
struct TodoRef {
    id: u64,
    priority: TodoPriority,
}

/// Both clocks at one moment, to turn a monotonic expiry into wall time.
#[derive(Debug, Clone, Copy)]
pub(super) struct Clock {
    pub(super) now: Instant,
    pub(super) wall: SystemTime,
}

impl Clock {
    pub(super) fn now() -> Self {
        Self {
            now: Instant::now(),
            wall: SystemTime::now(),
        }
    }

    fn wall_ms(&self, at: Instant) -> u64 {
        let ahead = at.saturating_duration_since(self.now);
        let at = self.wall + ahead;
        at.duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }
}

/// Builds the document. `core_address` gives a terminal's core address, or
/// `None` for a pane that is not in core, which is left out.
pub(super) fn extras_document(
    session: String,
    workspaces: &[Workspace],
    terminals: &HashMap<TerminalId, TerminalState>,
    core_address: impl Fn(&TerminalId) -> Option<String>,
    clock: Clock,
) -> ExtrasDocument {
    let mut pins: Vec<(u64, &str)> = workspaces
        .iter()
        .filter_map(|ws| Some((ws.pin_order?, ws.id.as_str())))
        .collect();
    pins.sort_unstable();
    let pin_place = |ws: &Workspace| {
        pins.iter()
            .position(|(_, id)| *id == ws.id)
            .map(|place| place + 1)
    };
    let spaces = workspaces
        .iter()
        .map(|ws| ExtrasSpace {
            id: ws.id.clone(),
            pin: pin_place(ws),
            custom_name: ws.custom_name.clone(),
            tokens: ws
                .metadata_tokens
                .entries()
                .filter(|(_, _, expires)| expires.is_none_or(|at| at > clock.now))
                .map(|(key, value, expires)| {
                    let token = Token {
                        value: value.to_string(),
                        expires_ms: expires.map(|at| clock.wall_ms(at)),
                    };
                    (key.to_string(), token)
                })
                .collect(),
            tabs: ws
                .tabs
                .iter()
                .map(|tab| ExtrasTab {
                    id: public_tab_id_for_number(&ws.id, tab.number),
                    custom_name: tab.custom_name.clone(),
                })
                .collect(),
            panes: ws
                .tabs
                .iter()
                .flat_map(|tab| tab.layout.pane_ids().into_iter().map(move |id| (tab, id)))
                .filter_map(|(tab, pane_id)| {
                    let terminal_id = tab.terminal_id(pane_id)?;
                    let terminal = terminals.get(terminal_id)?;
                    Some(ExtrasPane {
                        id: match ws.public_pane_number(pane_id) {
                            Some(number) => public_pane_id_for_number(&ws.id, number),
                            None => format!("{}:x{}", ws.id, pane_id.raw()),
                        },
                        terminal: core_address(terminal_id)?,
                        label: terminal.manual_label.clone(),
                        agent_name: terminal.agent_name.clone(),
                        todos: terminal
                            .todos()
                            .iter()
                            .filter(|todo| !todo.done)
                            .map(|todo| TodoRef {
                                id: todo.id,
                                priority: todo.priority,
                            })
                            .collect(),
                    })
                })
                .collect(),
        })
        .collect();
    ExtrasDocument {
        format: FORMAT,
        writer: Writer {
            session,
            version: crate::build_info::version(),
        },
        spaces,
    }
}

impl App {
    /// `[experimental] core_extras_doc`, on reload. Turning it on publishes
    /// with the next pass; turning it off removes the document.
    pub(super) fn set_core_extras_doc(&mut self, enabled: bool) {
        if self.core_extras_doc == enabled {
            return;
        }
        self.core_extras_doc = enabled;
        if enabled {
            self.core_spaces_deadline = Some(Instant::now());
        } else {
            (self.core_extras_publish)(serde_json::Value::Null);
        }
    }

    pub(super) fn publish_core_extras(&self) {
        if !self.core_extras_doc {
            return;
        }
        let runtimes = &self.terminal_runtimes;
        let primary = crate::pane::core_session_name();
        let document = extras_document(
            crate::pane::herdr_session_label(),
            &self.state.workspaces,
            &self.state.terminals,
            |id| {
                let link = runtimes.get(id)?.core_terminal()?;
                (link.session == primary).then_some(link.address)
            },
            Clock::now(),
        );
        match serde_json::to_value(&document) {
            Ok(value) => (self.core_extras_publish)(value),
            Err(err) => tracing::warn!(err = %err, "cannot encode the herdr-max document"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Workspace;
    use serde_json::json;

    fn clock() -> Clock {
        Clock {
            now: Instant::now(),
            wall: UNIX_EPOCH + Duration::from_secs(1_000_000),
        }
    }

    fn space_with_terminals() -> (Workspace, HashMap<TerminalId, TerminalState>) {
        let mut ws = Workspace::test_new("api");
        ws.test_split(ratatui::layout::Direction::Horizontal);
        let mut terminals = HashMap::new();
        for tab in &ws.tabs {
            for pane in tab.panes.values() {
                let id = pane.attached_terminal_id.clone();
                terminals.insert(id.clone(), TerminalState::new(id, "/tmp".into()));
            }
        }
        (ws, terminals)
    }

    fn by_order(ws: &Workspace) -> Vec<TerminalId> {
        ws.tabs[0]
            .layout
            .pane_ids()
            .into_iter()
            .map(|pane| ws.tabs[0].panes[&pane].attached_terminal_id.clone())
            .collect()
    }

    // core-phase4 verifier 16: the fields of D3, done todos left out, the
    // ids of the herdr-app document.
    #[test]
    fn spaces_tabs_and_panes_carry_the_extras() {
        let (mut ws, mut terminals) = space_with_terminals();
        ws.custom_name = Some("herdr".into());
        ws.pin_order = Some(7);
        ws.tabs[0].custom_name = Some("build".into());
        let order = by_order(&ws);
        let first = terminals.get_mut(&order[0]).unwrap();
        first.manual_label = Some("server".into());
        first.agent_name = Some("lead".into());
        first.add_todo("a", TodoPriority::High, None, 0).unwrap();
        let done = first.add_todo("b", TodoPriority::Low, None, 0).unwrap();
        first
            .update_todo(
                done.id,
                crate::terminal::todo::TodoUpdate {
                    done: Some(true),
                    ..Default::default()
                },
                0,
            )
            .unwrap();
        let third = first.add_todo("c", TodoPriority::Normal, None, 0).unwrap();
        let open_ids = [done.id - 1, third.id];
        let mut unpinned = Workspace::test_new("other");
        unpinned.pin_order = None;
        unpinned.custom_name = None;
        let address = |id: &TerminalId| {
            order
                .iter()
                .position(|known| known == id)
                .map(|n| format!("t{}", n + 1))
        };
        let ws_id = ws.id.clone();
        let tab_id = public_tab_id_for_number(&ws_id, ws.tabs[0].number);
        let pane_id = ws
            .public_pane_number(ws.tabs[0].layout.pane_ids()[0])
            .map(|number| public_pane_id_for_number(&ws_id, number))
            .unwrap();
        let document = extras_document(
            "herdr".into(),
            &[ws, unpinned],
            &terminals,
            address,
            clock(),
        );
        let value = serde_json::to_value(&document).unwrap();
        assert_eq!(value["format"], 1);
        assert_eq!(value["writer"]["session"], "herdr");
        let space = &value["spaces"][0];
        assert_eq!(space["id"], ws_id);
        assert_eq!(space["pin"], 1, "first among the pinned");
        assert_eq!(space["custom_name"], "herdr");
        assert_eq!(
            space["tabs"][0],
            json!({"id": tab_id, "custom_name": "build"})
        );
        assert_eq!(
            space["panes"][0],
            json!({"id": pane_id, "terminal": "t1", "label": "server",
                   "agent_name": "lead",
                   "todos": [{"id": open_ids[0], "priority": "high"},
                             {"id": open_ids[1], "priority": "normal"}]})
        );
        assert_eq!(space["panes"][1]["terminal"], "t2");
        assert_eq!(space["panes"][1]["label"], json!(null));
        assert!(value["spaces"][1].get("pin").is_none(), "unpinned");
        assert_eq!(value["spaces"][1]["custom_name"], json!(null));
        assert!(
            value["spaces"][1]["panes"].as_array().unwrap().is_empty(),
            "no core terminal, no pane"
        );
    }

    thread_local! {
        static PUBLISHED: std::cell::RefCell<Vec<serde_json::Value>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    fn record(document: serde_json::Value) {
        PUBLISHED.with(|published| published.borrow_mut().push(document));
    }

    fn published() -> Vec<serde_json::Value> {
        PUBLISHED.with(|published| std::mem::take(&mut *published.borrow_mut()))
    }

    // core-phase4 verifier 19: off by default; on, a metadata report and a
    // saved change publish on the shared schedule; off writes `null`.
    #[test]
    fn the_server_publishes_the_extras_only_with_the_flag_on() {
        let mut app = App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            tokio::sync::mpsc::unbounded_channel().1,
            crate::api::EventHub::default(),
        );
        app.core_extras_publish = record;
        app.state.workspaces.push(Workspace::test_new("api"));
        app.state.active = Some(0);
        assert!(!app.core_extras_doc, "off by default");
        app.state.mark_session_dirty();
        app.sync_core_spaces_schedule();
        app.publish_core_spaces_if_due(Instant::now() + Duration::from_secs(1));
        assert!(published().is_empty());

        app.set_core_extras_doc(true);
        app.publish_core_spaces_if_due(Instant::now());
        let first = published();
        assert_eq!(first.len(), 1, "turning it on publishes at once");
        assert_eq!(first[0]["format"], 1);

        let workspace_id = app.state.workspaces[0].id.clone();
        let reply = app.handle_api_request(crate::api::schema::Request {
            id: "r".into(),
            method: crate::api::schema::Method::WorkspaceReportMetadata(
                crate::api::schema::WorkspaceReportMetadataParams {
                    workspace_id,
                    source: "asks".into(),
                    tokens: HashMap::from([("asks".to_string(), Some("3".to_string()))]),
                    seq: None,
                    ttl_ms: Some(60_000),
                },
            ),
        });
        assert!(reply.contains("\"result\""), "{reply}");
        app.sync_core_spaces_schedule();
        let due = app
            .core_spaces_deadline
            .expect("a token report schedules a publish");
        app.publish_core_spaces_if_due(due);
        let with_token = published();
        assert_eq!(with_token[0]["spaces"][0]["tokens"]["asks"]["value"], "3");

        app.set_core_extras_doc(false);
        assert_eq!(published(), [serde_json::Value::Null], "off removes it");
    }

    // core-phase4 verifier 17: token expiry in wall time; expired tokens
    // are left out.
    #[test]
    fn tokens_carry_their_expiry_in_wall_time() {
        let (mut ws, terminals) = space_with_terminals();
        let clock = clock();
        let patch =
            |key: &str, value: &str| HashMap::from([(key.to_string(), Some(value.to_string()))]);
        ws.metadata_tokens
            .patch(patch("asks", "3"), Some(Duration::from_secs(60)), clock.now);
        ws.metadata_tokens
            .patch(patch("forever", "x"), None, clock.now);
        ws.metadata_tokens.patch(
            patch("gone", "y"),
            Some(Duration::from_secs(1)),
            clock.now - Duration::from_secs(5),
        );
        let document = extras_document("herdr".into(), &[ws], &terminals, |_| None, clock);
        let tokens = &serde_json::to_value(&document).unwrap()["spaces"][0]["tokens"];
        assert_eq!(
            *tokens,
            json!({"asks": {"value": "3", "expires_ms": 1_000_060_000_u64},
                   "forever": {"value": "x"}})
        );
    }
}
