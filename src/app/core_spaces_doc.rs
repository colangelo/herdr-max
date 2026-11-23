//! herdr's workspaces, tabs and splits as spaces of the `herdr-app` document
//! the Herdr apps read (openspec/changes/core-spaces-doc). One way: herdr's
//! layout is the source of its own spaces, which the writer merges into the
//! document beside other clients' spaces (D6).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use ratatui::layout::Direction;
use serde::Serialize;

use crate::layout::Node;
use crate::pane::CoreTerminalLink;
use crate::terminal::{TerminalId, TerminalState};
use crate::workspace::{public_pane_id_for_number, public_tab_id_for_number, Tab, Workspace};

use super::App;

/// How long after a layout change the document is built (D4).
pub(super) const CORE_SPACES_DEBOUNCE: Duration = Duration::from_millis(500);

/// The app's `mirror::Document`.
#[derive(Debug, Serialize)]
struct SpacesDocument {
    spaces: Vec<DocSpace>,
}

/// The app's `mirror::SharedSpace`.
#[derive(Debug, Serialize)]
struct DocSpace {
    id: String,
    name: String,
    folder: String,
    tabs: Vec<DocTab>,
}

/// The app's `mirror::SharedTab`.
#[derive(Debug, Serialize)]
struct DocTab {
    id: String,
    name: String,
    root: DocNode,
}

/// The app's `layout::Node`, with the fields herdr sets.
#[derive(Debug, Serialize)]
struct DocNode {
    id: String,
    #[serde(flatten)]
    kind: DocNodeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<Weight>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum DocNodeKind {
    Block { content: TerminalBlock },
    Row { children: Vec<DocNode> },
    Column { children: Vec<DocNode> },
}

/// The app's `Content::Terminal`: a core address (`t7`).
#[derive(Debug, Serialize)]
struct TerminalBlock {
    terminal: String,
}

/// The app's `Size::Weight`.
#[derive(Debug, Clone, Copy, Serialize)]
struct Weight {
    weight: f32,
}

/// A pane's leaf address: only a terminal in the primary session, the one the
/// document is written to, has an address that means something there. A pane
/// in another core session is left out like a PTY pane (core-phase3 D7).
fn primary_address(link: CoreTerminalLink, primary: &str) -> Option<String> {
    (link.session == primary).then_some(link.address)
}

/// Builds the document. `core_address` gives a terminal's core address, or
/// `None` for a pane that is not in core, which is left out (D2).
fn spaces_document(
    workspaces: &[Workspace],
    terminals: &HashMap<TerminalId, TerminalState>,
    core_address: impl Fn(&TerminalId) -> Option<String>,
) -> SpacesDocument {
    let spaces = workspaces
        .iter()
        .map(|ws| DocSpace {
            id: ws.id.clone(),
            name: ws.display_name_from_terminals(terminals),
            folder: space_folder(ws, terminals),
            tabs: ws
                .tabs
                .iter()
                .enumerate()
                .map(|(tab_idx, tab)| {
                    let id = public_tab_id_for_number(&ws.id, tab.number);
                    let leaves = Leaves {
                        ws,
                        tab,
                        tab_id: &id,
                        core_address: &core_address,
                    };
                    let root = tab_root(&id, leaves.node(tab.layout.root(), "", None));
                    DocTab {
                        name: ws.tab_display_name(tab_idx).unwrap_or_default(),
                        id,
                        root,
                    }
                })
                .collect(),
        })
        .collect();
    SpacesDocument { spaces }
}

/// The folder herdr names the workspace after: the first tab's root pane's
/// cwd, else the workspace's identity cwd.
fn space_folder(ws: &Workspace, terminals: &HashMap<TerminalId, TerminalState>) -> String {
    ws.tabs
        .first()
        .and_then(|tab| tab.terminal_id(tab.root_pane))
        .and_then(|terminal_id| terminals.get(terminal_id))
        .map(|terminal| &terminal.cwd)
        .unwrap_or(&ws.identity_cwd)
        .to_string_lossy()
        .into_owned()
}

/// A tab's root is a container with the tab's id, as the Mac app writes it.
fn tab_root(tab_id: &str, tree: Option<DocNode>) -> DocNode {
    let kind = match tree {
        Some(DocNode {
            kind: kind @ (DocNodeKind::Row { .. } | DocNodeKind::Column { .. }),
            ..
        }) => kind,
        Some(block) => DocNodeKind::Row {
            children: vec![block],
        },
        None => DocNodeKind::Row {
            children: Vec::new(),
        },
    };
    DocNode {
        id: tab_id.to_owned(),
        kind,
        size: None,
    }
}

struct Leaves<'a, F> {
    ws: &'a Workspace,
    tab: &'a Tab,
    tab_id: &'a str,
    core_address: &'a F,
}

impl<F: Fn(&TerminalId) -> Option<String>> Leaves<'_, F> {
    /// The subtree at `path` (`a` first, `b` second child), sized `size` in
    /// its parent. A split with one side left out is the other side.
    fn node(&self, tree: &Node, path: &str, size: Option<Weight>) -> Option<DocNode> {
        match tree {
            Node::Pane(pane_id) => {
                let terminal = (self.core_address)(self.tab.terminal_id(*pane_id)?)?;
                let id = match self.ws.public_pane_number(*pane_id) {
                    Some(number) => public_pane_id_for_number(&self.ws.id, number),
                    None => format!("{}:x{}", self.ws.id, pane_id.raw()),
                };
                Some(DocNode {
                    id,
                    kind: DocNodeKind::Block {
                        content: TerminalBlock { terminal },
                    },
                    size,
                })
            }
            Node::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let first = self.node(first, &format!("{path}a"), Some(Weight { weight: *ratio }));
                let second = self.node(
                    second,
                    &format!("{path}b"),
                    Some(Weight {
                        weight: 1.0 - *ratio,
                    }),
                );
                match (first, second) {
                    (Some(first), Some(second)) => {
                        let children = vec![first, second];
                        Some(DocNode {
                            id: format!("{}:s{path}", self.tab_id),
                            kind: match direction {
                                Direction::Horizontal => DocNodeKind::Row { children },
                                Direction::Vertical => DocNodeKind::Column { children },
                            },
                            size,
                        })
                    }
                    (Some(only), None) | (None, Some(only)) => Some(DocNode { size, ..only }),
                    (None, None) => None,
                }
            }
        }
    }
}

impl App {
    /// `[experimental] core_spaces_doc`, on reload. Turning it on publishes
    /// at once; turning it off removes herdr-max's spaces.
    pub(super) fn set_core_spaces_doc(&mut self, enabled: bool) {
        if self.core_spaces_doc == enabled {
            return;
        }
        self.core_spaces_doc = enabled;
        if enabled {
            self.core_spaces_deadline = Some(Instant::now());
        } else {
            self.core_spaces_deadline = None;
            (self.core_spaces_publish)(Vec::new());
        }
    }

    /// After a change the session would save: publish within the debounce.
    pub(crate) fn sync_core_spaces_schedule(&mut self) {
        if std::mem::take(&mut self.state.core_spaces_dirty)
            && (self.core_spaces_doc || self.core_extras_doc)
            && self.core_spaces_deadline.is_none()
        {
            self.core_spaces_deadline = Some(Instant::now() + CORE_SPACES_DEBOUNCE);
        }
    }

    pub(crate) fn publish_core_spaces_if_due(&mut self, now: Instant) {
        if self.core_spaces_deadline.is_none_or(|due| now < due) {
            return;
        }
        self.core_spaces_deadline = None;
        self.publish_core_extras();
        if !self.core_spaces_doc {
            return;
        }
        let runtimes = &self.terminal_runtimes;
        let primary = crate::pane::core_session_name();
        let document = spaces_document(&self.state.workspaces, &self.state.terminals, |id| {
            primary_address(runtimes.get(id)?.core_terminal()?, &primary)
        });
        let spaces: Result<Vec<_>, _> = document.spaces.iter().map(serde_json::to_value).collect();
        match spaces {
            Ok(spaces) => (self.core_spaces_publish)(spaces),
            Err(err) => tracing::warn!(err = %err, "cannot encode the herdr-app spaces"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The Herdr app's shapes for this document, copied from herdr-app
    /// `src/mirror.rs` and `src/layout.rs` (fields and variants herdr does not
    /// write omitted), so a test can decode what herdr writes as the app does.
    mod app {
        use serde::Deserialize;

        #[derive(Debug, Default, Deserialize)]
        pub struct Document {
            #[serde(default)]
            pub spaces: Vec<SharedSpace>,
        }

        #[derive(Debug, Default, Deserialize)]
        #[serde(default)]
        pub struct SharedSpace {
            pub id: String,
            pub name: String,
            pub folder: String,
            pub tabs: Vec<SharedTab>,
            #[serde(flatten)]
            pub rest: serde_json::Map<String, serde_json::Value>,
        }

        #[derive(Debug, Deserialize)]
        pub struct SharedTab {
            #[serde(default)]
            pub id: String,
            #[serde(default)]
            pub name: String,
            #[serde(default = "empty_root")]
            pub root: Node,
        }

        fn empty_root() -> Node {
            Node {
                id: String::new(),
                kind: Kind::Row {
                    children: Vec::new(),
                },
                size: None,
            }
        }

        #[derive(Debug, PartialEq, Deserialize)]
        #[serde(untagged)]
        pub enum Content {
            Terminal {
                terminal: String,
            },
            Mod {
                #[serde(rename = "mod")]
                name: String,
                #[serde(default)]
                props: serde_json::Value,
            },
        }

        #[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
        #[serde(untagged)]
        pub enum Size {
            Px { px: f32 },
            Weight { weight: f32 },
        }

        #[derive(Debug, Deserialize)]
        #[serde(tag = "type", rename_all = "lowercase")]
        pub enum Kind {
            Block { content: Content },
            Row { children: Vec<Node> },
            Column { children: Vec<Node> },
        }

        #[derive(Debug, Deserialize)]
        pub struct Node {
            #[serde(default)]
            pub id: String,
            #[serde(flatten)]
            pub kind: Kind,
            #[serde(default)]
            pub size: Option<Size>,
        }
    }

    fn decode(document: SpacesDocument) -> (serde_json::Value, app::Document) {
        let value = serde_json::to_value(document).expect("encodes");
        let decoded = serde_json::from_value(value.clone()).expect("the app decodes it");
        (value, decoded)
    }

    fn core_everywhere(id: &TerminalId) -> Option<String> {
        Some(format!("core-{id}"))
    }

    fn address(ws: &Workspace, tab_idx: usize, pane: crate::layout::PaneId) -> String {
        core_everywhere(ws.tabs[tab_idx].terminal_id(pane).expect("pane")).expect("core")
    }

    fn terminal_of(node: &app::Node) -> &str {
        match &node.kind {
            app::Kind::Block {
                content: app::Content::Terminal { terminal },
            } => terminal,
            other => panic!("not a terminal block: {other:?}"),
        }
    }

    fn children(node: &app::Node) -> &[app::Node] {
        match &node.kind {
            app::Kind::Row { children } | app::Kind::Column { children } => children,
            other => panic!("not a container: {other:?}"),
        }
    }

    fn weight(node: &app::Node) -> f32 {
        match node.size {
            Some(app::Size::Weight { weight }) => weight,
            other => panic!("no weight: {other:?}"),
        }
    }

    #[test]
    fn the_copied_shapes_read_the_apps_own_document() {
        // herdr-app src/mirror.rs, unknown_space_and_tab_fields_survive_a_read_and_write.
        let text = json!({"spaces":[{"id":"s","name":"work","folder":"/w","color":"teal","tabs":[{"id":"t","name":"","root":
            {"id":"t","type":"row","children":[{"id":"p","type":"block","content":{"terminal":"t1"}}]},
            "floating":[{"pane":{"id":"q","type":"block","content":{"terminal":"t2"}},"rect":[0.2,0.2,0.6,0.6]}]}]}]});
        let doc: app::Document = serde_json::from_value(text).expect("decodes");
        let root = &doc.spaces[0].tabs[0].root;
        assert_eq!(root.id, "t");
        assert_eq!(terminal_of(&children(root)[0]), "t1");
        assert_eq!(doc.spaces[0].rest["color"], "teal");
        let empty: app::Document =
            serde_json::from_value(json!({"spaces":[{"id":"s","tabs":[{"id":"t"}]}]}))
                .expect("omitted fields decode");
        assert!(children(&empty.spaces[0].tabs[0].root).is_empty());
    }

    #[test]
    fn workspaces_and_tabs_become_spaces_and_tabs() {
        let mut first = Workspace::test_new("api");
        first.test_add_tab(Some("logs"));
        let mut second = Workspace::test_new("auto");
        second.custom_name = None;
        let terminals = HashMap::new();
        let workspaces = vec![first, second];

        let (_, doc) = decode(spaces_document(&workspaces, &terminals, core_everywhere));

        assert_eq!(doc.spaces.len(), 2);
        let space = &doc.spaces[0];
        assert_eq!(space.id, workspaces[0].id);
        assert_eq!(space.name, "api");
        assert_eq!(
            space.folder,
            workspaces[0].identity_cwd.to_string_lossy(),
            "no terminal state: the identity cwd"
        );
        let tabs: Vec<_> = space
            .tabs
            .iter()
            .map(|tab| (tab.id.as_str(), tab.name.as_str()))
            .collect();
        let tab_id = |n| public_tab_id_for_number(&workspaces[0].id, n);
        assert_eq!(
            tabs,
            [(tab_id(1).as_str(), "1"), (tab_id(2).as_str(), "logs")]
        );
        assert_eq!(
            doc.spaces[1].name,
            workspaces[1].display_name_from_terminals(&terminals),
            "the automatic name herdr shows"
        );
        for space in &doc.spaces {
            assert!(!space.id.contains('/'));
            for tab in &space.tabs {
                assert!(!tab.id.contains('/'), "the app qualifies ids with `/`");
            }
        }
    }

    #[test]
    fn the_folder_is_the_first_panes_cwd() {
        let ws = Workspace::test_new("api");
        let terminal_id = ws.tabs[0]
            .terminal_id(ws.tabs[0].root_pane)
            .unwrap()
            .clone();
        let mut terminals = HashMap::new();
        terminals.insert(
            terminal_id.clone(),
            TerminalState::new(terminal_id, "/work/api".into()),
        );
        let (_, doc) = decode(spaces_document(&[ws], &terminals, core_everywhere));
        assert_eq!(doc.spaces[0].folder, "/work/api");
    }

    #[test]
    fn splits_become_rows_and_columns_with_weights() {
        let mut ws = Workspace::test_new("api");
        let left = ws.tabs[0].root_pane;
        let right = ws.test_split(Direction::Horizontal);
        let below = ws.test_split(Direction::Vertical);
        assert!(ws.tabs[0].layout.set_ratio_at(&[true], 0.7));
        let tab_id = public_tab_id_for_number(&ws.id, 1);
        let pane_id =
            |pane| public_pane_id_for_number(&ws.id, ws.public_pane_number(pane).unwrap());

        let (_, doc) = decode(spaces_document(
            std::slice::from_ref(&ws),
            &HashMap::new(),
            core_everywhere,
        ));

        let root = &doc.spaces[0].tabs[0].root;
        assert_eq!(root.id, tab_id, "the root is the tab");
        assert!(matches!(root.kind, app::Kind::Row { .. }), "side by side");
        assert!(root.size.is_none());
        let [a, b] = children(root) else {
            panic!("two sides")
        };
        assert_eq!(a.id, pane_id(left));
        assert_eq!(terminal_of(a), address(&ws, 0, left));
        assert_eq!(weight(a), 0.5);
        assert_eq!(weight(b), 0.5);
        assert!(matches!(b.kind, app::Kind::Column { .. }), "stacked");
        assert_eq!(b.id, format!("{tab_id}:sb"));
        let [top, bottom] = children(b) else {
            panic!("two stacked panes")
        };
        assert_eq!(terminal_of(top), address(&ws, 0, right));
        assert_eq!(terminal_of(bottom), address(&ws, 0, below));
        assert_eq!(bottom.id, pane_id(below));
        assert_eq!(weight(top), 0.7);
        assert!((weight(bottom) - 0.3).abs() < 1e-6);
    }

    #[test]
    fn panes_not_in_core_are_left_out() {
        let mut ws = Workspace::test_new("api");
        let left = ws.tabs[0].root_pane;
        let right = ws.test_split(Direction::Horizontal);
        let below = ws.test_split(Direction::Vertical);
        ws.test_add_tab(Some("pty only"));
        let pty = [
            ws.tabs[0].terminal_id(right).unwrap().clone(),
            ws.tabs[1]
                .terminal_id(ws.tabs[1].root_pane)
                .unwrap()
                .clone(),
        ];
        let only_some =
            |id: &TerminalId| (!pty.contains(id)).then(|| core_everywhere(id)).flatten();

        let (_, doc) = decode(spaces_document(
            std::slice::from_ref(&ws),
            &HashMap::new(),
            only_some,
        ));

        let root = &doc.spaces[0].tabs[0].root;
        let [a, b] = children(root) else {
            panic!("two sides")
        };
        assert_eq!(terminal_of(a), address(&ws, 0, left));
        assert_eq!(terminal_of(b), address(&ws, 0, below));
        assert_eq!(weight(b), 0.5, "the core side keeps the split's slot");
        let pty_tab = &doc.spaces[0].tabs[1];
        assert_eq!(pty_tab.name, "pty only");
        assert_eq!(pty_tab.root.id, pty_tab.id);
        assert!(children(&pty_tab.root).is_empty(), "an empty row");
    }

    #[test]
    fn a_lone_pane_is_a_row_of_one() {
        let ws = Workspace::test_new("api");
        let (value, doc) = decode(spaces_document(
            std::slice::from_ref(&ws),
            &HashMap::new(),
            core_everywhere,
        ));
        let root = &doc.spaces[0].tabs[0].root;
        let [only] = children(root) else {
            panic!("one pane")
        };
        assert!(only.size.is_none());
        assert_eq!(
            value["spaces"][0]["tabs"][0]["root"]["children"][0]["content"],
            json!({"terminal": address(&ws, 0, ws.tabs[0].root_pane)})
        );
    }

    thread_local! {
        static PUBLISHED: std::cell::RefCell<Vec<Vec<serde_json::Value>>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    fn record(spaces: Vec<serde_json::Value>) {
        PUBLISHED.with(|published| published.borrow_mut().push(spaces));
    }

    fn published() -> Vec<Vec<serde_json::Value>> {
        PUBLISHED.with(|published| std::mem::take(&mut *published.borrow_mut()))
    }

    fn app_with_workspace() -> App {
        let mut app = App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            tokio::sync::mpsc::unbounded_channel().1,
            crate::api::EventHub::default(),
        );
        app.core_spaces_publish = record;
        app.state.workspaces.push(Workspace::test_new("api"));
        app.state.active = Some(0);
        app
    }

    #[test]
    fn the_server_publishes_only_with_the_flag_on() {
        let mut app = app_with_workspace();
        assert!(!app.core_spaces_doc, "off by default");
        app.state.mark_session_dirty();
        app.sync_core_spaces_schedule();
        assert!(app.core_spaces_deadline.is_none(), "off: no publish");
        assert!(!app.state.core_spaces_dirty, "the change is dropped");

        app.set_core_spaces_doc(true);
        app.publish_core_spaces_if_due(Instant::now());
        let first = published();
        assert_eq!(first.len(), 1, "turning it on publishes at once");
        assert_eq!(first[0][0]["name"], "api");

        app.state.workspaces[0].custom_name = Some("renamed".into());
        app.state.mark_session_dirty();
        let before = Instant::now();
        app.sync_core_spaces_schedule();
        let due = app.core_spaces_deadline.expect("a publish is scheduled");
        assert!(due >= before + CORE_SPACES_DEBOUNCE);
        app.state.mark_session_dirty();
        app.sync_core_spaces_schedule();
        assert_eq!(
            app.core_spaces_deadline,
            Some(due),
            "a burst keeps one deadline"
        );
        app.publish_core_spaces_if_due(due - Duration::from_millis(1));
        assert!(published().is_empty(), "not before the debounce");
        app.publish_core_spaces_if_due(due);
        let renamed = published();
        assert_eq!(renamed.len(), 1);
        assert_eq!(renamed[0][0]["name"], "renamed");

        app.set_core_spaces_doc(false);
        assert_eq!(
            published(),
            [Vec::<serde_json::Value>::new()],
            "off withdraws our spaces"
        );
        app.state.mark_session_dirty();
        app.sync_core_spaces_schedule();
        app.publish_core_spaces_if_due(Instant::now() + Duration::from_secs(1));
        assert!(published().is_empty());
    }

    #[test]
    fn a_scheduled_session_save_also_schedules_a_publish() {
        let mut app = app_with_workspace();
        app.set_core_spaces_doc(true);
        app.publish_core_spaces_if_due(Instant::now());
        published();
        app.schedule_session_save();
        app.sync_core_spaces_schedule();
        assert!(app.core_spaces_deadline.is_some());
    }

    fn link(session: &str, address: &str) -> CoreTerminalLink {
        CoreTerminalLink {
            agent: None,
            session: session.into(),
            address: address.into(),
            program: None,
            launch: None,
            machine_cwd: None,
        }
    }

    #[test]
    fn only_primary_session_panes_are_leaves() {
        let mut ws = Workspace::test_new("api");
        let left = ws.tabs[0].root_pane;
        let right = ws.test_split(Direction::Horizontal);
        let below = ws.test_split(Direction::Vertical);
        let legacy = ws.tabs[0].terminal_id(right).unwrap().clone();
        let links: HashMap<TerminalId, CoreTerminalLink> = ws.tabs[0]
            .layout
            .pane_ids()
            .into_iter()
            .map(|pane| {
                let id = ws.tabs[0].terminal_id(pane).unwrap().clone();
                let session = if id == legacy { "herdr" } else { "default" };
                (id, link(session, &format!("t{}", pane.raw())))
            })
            .collect();

        let (_, doc) = decode(spaces_document(
            std::slice::from_ref(&ws),
            &HashMap::new(),
            |id| primary_address(links.get(id)?.clone(), "default"),
        ));

        let root = &doc.spaces[0].tabs[0].root;
        let [a, b] = children(root) else {
            panic!("two sides")
        };
        assert_eq!(terminal_of(a), format!("t{}", left.raw()));
        assert_eq!(terminal_of(b), format!("t{}", below.raw()));
        assert_eq!(
            weight(b),
            0.5,
            "the legacy side is left out, its split collapsed"
        );
    }

    #[test]
    fn a_link_names_the_primary_or_is_not_a_leaf() {
        assert_eq!(
            primary_address(link("default", "t8"), "default").as_deref(),
            Some("t8")
        );
        assert_eq!(primary_address(link("herdr", "t8"), "default"), None);
    }
}
