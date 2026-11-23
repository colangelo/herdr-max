//! The view server's layout (core-phase5 D6): the far herdr's spaces, tabs
//! and splits, projected from its `herdr-app` document into this server's
//! workspaces, with one pane per far terminal. Pane, tab and workspace ids
//! are the far side's own (D4), so a client of the view names panes as the
//! far herdr does. A pane keeps its place, its id and its stream across
//! layout changes; panes the far side no longer lists are detached, never
//! ended. Attaches run off the loop and report back.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use ratatui::layout::Direction;

use super::App;
use crate::core_machine::MachineName;
use crate::layout::{Node, PaneId, TileLayout};
use crate::pane::PaneState;
use crate::terminal::{TerminalId, TerminalState};
use crate::view::doc::{ViewLayout, ViewNode, ViewSpace};
use crate::view::lifetime::ViewPresence;
use crate::view::watch::ViewEvent;
use crate::workspace::{public_tab_id_for_number, Tab, Workspace};

/// What the view knows beyond its workspaces.
pub(crate) struct ViewApp {
    pub(crate) machine: MachineName,
    /// The far pane id (`w3:p4`) of each pane, both ways.
    pane_ids: HashMap<String, PaneId>,
    far_panes: HashMap<PaneId, String>,
    /// The terminal shown for each far address (`<machine>/t7`).
    terminals: HashMap<String, TerminalId>,
    /// Addresses whose attach is under way.
    attaching: HashSet<String>,
    /// Far sizes (rows, cols) as core last reported them.
    far_sizes: HashMap<String, (u16, u16)>,
}

impl ViewApp {
    pub(crate) fn new(machine: MachineName) -> Self {
        Self {
            machine,
            pane_ids: HashMap::new(),
            far_panes: HashMap::new(),
            terminals: HashMap::new(),
            attaching: HashSet::new(),
            far_sizes: HashMap::new(),
        }
    }

    fn address(&self, terminal: &str) -> String {
        format!("{}/{terminal}", self.machine)
    }

    fn pane_id(&mut self, far: &str) -> PaneId {
        if let Some(pane_id) = self.pane_ids.get(far) {
            return *pane_id;
        }
        let pane_id = PaneId::alloc();
        self.pane_ids.insert(far.to_string(), pane_id);
        self.far_panes.insert(pane_id, far.to_string());
        pane_id
    }
}

/// What one projection needs to build panes.
struct Build<'a> {
    view: &'a mut ViewApp,
    terminals: &'a mut HashMap<TerminalId, TerminalState>,
    folder: PathBuf,
    /// Addresses in this layout, and the ones that need an attach.
    used: HashSet<String>,
    to_attach: Vec<(String, PaneId)>,
}

impl Build<'_> {
    fn node(&mut self, node: &ViewNode, panes: &mut HashMap<PaneId, PaneState>) -> Node {
        match node {
            ViewNode::Pane { id, terminal, .. } => {
                let pane_id = self.view.pane_id(id);
                let address = self.view.address(terminal);
                let terminal_id = match self.view.terminals.get(&address) {
                    Some(terminal_id) if self.terminals.contains_key(terminal_id) => {
                        terminal_id.clone()
                    }
                    _ => {
                        let terminal_id = TerminalId::alloc();
                        let mut state =
                            TerminalState::new(terminal_id.clone(), self.folder.clone());
                        state.restore_error = Some(format!("Attaching to {address}…"));
                        state.view_far_size = self.view.far_sizes.get(&address).copied();
                        self.terminals.insert(terminal_id.clone(), state);
                        self.view
                            .terminals
                            .insert(address.clone(), terminal_id.clone());
                        terminal_id
                    }
                };
                if self
                    .terminals
                    .get(&terminal_id)
                    .is_some_and(|state| state.restore_error.is_some())
                    && !self.view.attaching.contains(&address)
                {
                    self.to_attach.push((address.clone(), pane_id));
                }
                self.used.insert(address);
                panes.insert(pane_id, PaneState::new(terminal_id));
                Node::Pane(pane_id)
            }
            ViewNode::Split {
                side_by_side,
                ratio,
                first,
                second,
            } => Node::Split {
                direction: if *side_by_side {
                    Direction::Horizontal
                } else {
                    Direction::Vertical
                },
                ratio: *ratio,
                first: Box::new(self.node(first, panes)),
                second: Box::new(self.node(second, panes)),
            },
        }
    }
}

fn first_pane(node: &Node) -> PaneId {
    match node {
        Node::Pane(pane_id) => *pane_id,
        Node::Split { first, .. } => first_pane(first),
    }
}

/// Where each workspace's focus was, by far ids, to keep it across a
/// projection: the active tab, and each tab's focused pane and zoom.
struct Kept {
    active_workspace: Option<String>,
    selected_workspace: Option<String>,
    active_tab: HashMap<String, String>,
    focus: HashMap<String, (String, bool)>,
}

impl App {
    /// Why an action that would run something on this Mac, with the far
    /// side's ids or folders, is refused in a view (core-phase5 D6, review
    /// M4.1, L4.3); `None` in the primary server.
    pub(crate) fn view_refusal(&self, what: &str) -> Option<String> {
        self.view.as_ref().map(|view| {
            let machine = &view.machine;
            format!("this is a view of {machine}: {what} runs on this Mac, not on {machine}; use {machine}'s own herdr")
        })
    }

    /// Turns this server into the view of `machine` (D6): no layout until the
    /// far document is read; the watcher thread follows the machine.
    /// Turns this server into the view of `machine` (D6): no layout until the
    /// far document is read; the watcher thread follows the machine.
    pub(crate) fn start_view(
        &mut self,
        machine: MachineName,
        presence: Arc<ViewPresence>,
    ) -> std::io::Result<()> {
        self.view = Some(ViewApp::new(machine.clone()));
        crate::view::watch::start(machine, presence, self.event_tx.clone())
    }

    pub(crate) fn handle_view_event(&mut self, event: ViewEvent) {
        match event {
            ViewEvent::Layout(layout) => self.apply_view_layout(&layout),
            ViewEvent::Notice(notice) => {
                self.config_diagnostic_deadline = None;
                self.state.config_diagnostic = notice;
            }
            ViewEvent::FarSize {
                address,
                rows,
                cols,
            } => self.follow_view_size(address, rows, cols),
            ViewEvent::Status { address, status } => self.apply_view_status(&address, &status),
            ViewEvent::Attached { address, runtime } => {
                self.install_view_runtime(address, *runtime)
            }
            ViewEvent::AttachFailed { address, reason } => {
                let Some(view) = self.view.as_mut() else {
                    return;
                };
                view.attaching.remove(&address);
                if let Some(state) = view
                    .terminals
                    .get(&address)
                    .and_then(|terminal_id| self.state.terminals.get_mut(terminal_id))
                {
                    state.restore_error = Some(reason);
                    state.revision = state.revision.saturating_add(1);
                }
            }
        }
    }

    fn kept_focus(&self) -> Kept {
        let far = |pane_id: PaneId| {
            self.view
                .as_ref()
                .and_then(|view| view.far_panes.get(&pane_id).cloned())
        };
        let mut kept = Kept {
            active_workspace: self
                .state
                .active
                .and_then(|idx| self.state.workspaces.get(idx))
                .map(|ws| ws.id.clone()),
            selected_workspace: self
                .state
                .workspaces
                .get(self.state.selected)
                .map(|ws| ws.id.clone()),
            active_tab: HashMap::new(),
            focus: HashMap::new(),
        };
        for ws in &self.state.workspaces {
            if let Some(tab) = ws.tabs.get(ws.active_tab) {
                kept.active_tab
                    .insert(ws.id.clone(), public_tab_id_for_number(&ws.id, tab.number));
            }
            for tab in &ws.tabs {
                if let Some(focused) = far(tab.layout.focused()) {
                    kept.focus.insert(
                        public_tab_id_for_number(&ws.id, tab.number),
                        (focused, tab.zoomed),
                    );
                }
            }
        }
        kept
    }

    /// Projects `layout` (D6): the far spaces become this view's workspaces,
    /// in the far order, with their ids, names and splits. A tab with no pane
    /// in core, and a space with no such tab, are left out, as the far herdr
    /// leaves PTY panes out of the document.
    pub(crate) fn apply_view_layout(&mut self, layout: &ViewLayout) {
        let kept = self.kept_focus();
        let Some(mut view) = self.view.take() else {
            return;
        };
        let wiring = Wiring {
            events: self.event_tx.clone(),
            render_notify: self.render_notify.clone(),
            render_dirty: self.render_dirty.clone(),
        };
        let mut build = Build {
            view: &mut view,
            terminals: &mut self.state.terminals,
            folder: PathBuf::new(),
            used: HashSet::new(),
            to_attach: Vec::new(),
        };
        let mut workspaces = Vec::with_capacity(layout.spaces.len());
        for space in &layout.spaces {
            if let Some(ws) = view_workspace(space, &mut build, &kept, &wiring) {
                workspaces.push(ws);
            }
        }
        let used = std::mem::take(&mut build.used);
        let to_attach = std::mem::take(&mut build.to_attach);
        drop(build);

        // Far panes no longer listed: their streams detach (a view never
        // ends a far program, D6).
        let gone: Vec<(String, TerminalId)> = view
            .terminals
            .iter()
            .filter(|(address, _)| !used.contains(*address))
            .map(|(address, id)| (address.clone(), id.clone()))
            .collect();
        for (address, terminal_id) in gone {
            view.terminals.remove(&address);
            view.attaching.remove(&address);
            self.state.terminals.remove(&terminal_id);
            if let Some(runtime) = self.terminal_runtimes.remove(&terminal_id) {
                runtime.shutdown();
            }
        }
        let live_panes: HashSet<PaneId> = workspaces
            .iter()
            .flat_map(|ws| ws.tabs.iter().flat_map(|tab| tab.panes.keys().copied()))
            .collect();
        view.far_panes
            .retain(|pane_id, _| live_panes.contains(pane_id));
        let far_panes = &view.far_panes;
        view.pane_ids
            .retain(|_, pane_id| far_panes.contains_key(pane_id));

        let find = |id: &Option<String>| {
            id.as_ref()
                .and_then(|id| workspaces.iter().position(|ws| &ws.id == id))
        };
        let active = find(&kept.active_workspace).or((!workspaces.is_empty()).then_some(0));
        let selected = find(&kept.selected_workspace).or(active).unwrap_or(0);
        self.state.workspaces = workspaces;
        self.state.active = active;
        self.state.selected = selected;
        self.state.mode = if active.is_some() {
            super::Mode::Terminal
        } else {
            super::Mode::Navigate
        };
        for (address, pane_id) in to_attach {
            self.request_view_attach(&mut view, address, pane_id);
        }
        tracing::info!(
            machine = %view.machine,
            spaces = self.state.workspaces.len(),
            tabs = self.state.workspaces.iter().map(|ws| ws.tabs.len()).sum::<usize>(),
            panes = view.terminals.len(),
            "view layout"
        );
        self.view = Some(view);
        self.full_redraw_pending = true;
        self.state.mark_session_dirty();
    }

    fn request_view_attach(&mut self, view: &mut ViewApp, address: String, pane_id: PaneId) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::warn!(%address, "no runtime to attach a view pane on");
            return;
        };
        view.attaching.insert(address.clone());
        let events = self.event_tx.clone();
        let render_notify = self.render_notify.clone();
        let render_dirty = self.render_dirty.clone();
        let scrollback = self.state.pane_scrollback_limit_bytes;
        handle.spawn_blocking(move || {
            let event = match crate::terminal::TerminalRuntime::view_core(
                pane_id,
                address.clone(),
                scrollback,
                events.clone(),
                render_notify,
                render_dirty,
            ) {
                Ok(runtime) => ViewEvent::Attached {
                    address,
                    runtime: Box::new(runtime),
                },
                Err(err) => ViewEvent::AttachFailed {
                    address,
                    reason: format!("{err}; it is asked for again when the far layout changes or the link comes back"),
                },
            };
            let _ = events.blocking_send(crate::events::AppEvent::View(event));
        });
    }

    fn install_view_runtime(&mut self, address: String, runtime: crate::terminal::TerminalRuntime) {
        let Some(view) = self.view.as_mut() else {
            runtime.shutdown();
            return;
        };
        view.attaching.remove(&address);
        let Some(terminal_id) = view.terminals.get(&address).cloned() else {
            // Gone from the layout meanwhile: detach.
            runtime.shutdown();
            return;
        };
        let Some(state) = self.state.terminals.get_mut(&terminal_id) else {
            runtime.shutdown();
            return;
        };
        if let Some(&(rows, cols)) = view.far_sizes.get(&address) {
            runtime.follow_view_size(rows, cols);
        }
        state.restore_error = None;
        state.view_far_size = runtime.view_size();
        state.view_read_only = runtime.view_read_only();
        state.revision = state.revision.saturating_add(1);
        if let Some(previous) = self.terminal_runtimes.insert(terminal_id, runtime) {
            previous.shutdown();
        }
        self.full_redraw_pending = true;
    }

    fn follow_view_size(&mut self, address: String, rows: u16, cols: u16) {
        let Some(view) = self.view.as_mut() else {
            return;
        };
        view.far_sizes.insert(address.clone(), (rows, cols));
        let Some(terminal_id) = view.terminals.get(&address) else {
            return;
        };
        if let Some(runtime) = self.terminal_runtimes.get(terminal_id) {
            runtime.follow_view_size(rows, cols);
        }
        if let Some(state) = self.state.terminals.get_mut(terminal_id) {
            if state.view_far_size != Some((rows, cols)) {
                state.view_far_size = Some((rows, cols));
                state.revision = state.revision.saturating_add(1);
            }
        }
    }

    /// The far herdr's verdict, as it mirrors it into core (OSC 7501,
    /// core-status-mirror): the view runs no detection of its own (D6).
    fn apply_view_status(&mut self, address: &str, status: &serde_json::Value) {
        let Some(terminal_id) = self
            .view
            .as_ref()
            .and_then(|view| view.terminals.get(address).cloned())
        else {
            return;
        };
        let Some(state) = self.state.terminals.get_mut(&terminal_id) else {
            return;
        };
        let (agent, agent_state) = view_status(status);
        let blocked = agent_state == crate::detect::AgentState::Blocked;
        let _ = state.set_detected_state_with_screen_signals_at(
            agent,
            agent_state,
            blocked,
            false,
            false,
            false,
            std::time::Instant::now(),
        );
        state.revision = state.revision.saturating_add(1);
    }
}

/// The channels a tab is built with.
struct Wiring {
    events: tokio::sync::mpsc::Sender<crate::events::AppEvent>,
    render_notify: Arc<tokio::sync::Notify>,
    render_dirty: Arc<crate::render_signal::RenderSignal>,
}

fn view_workspace(
    space: &ViewSpace,
    build: &mut Build<'_>,
    kept: &Kept,
    wiring: &Wiring,
) -> Option<Workspace> {
    build.folder = PathBuf::from(&space.folder);
    let mut tabs = Vec::new();
    let mut public_pane_numbers = HashMap::new();
    let mut next_pane = 1;
    let mut next_tab = 1;
    let mut active_tab = 0;
    for view_tab in &space.tabs {
        let Some(root) = &view_tab.root else {
            continue;
        };
        let mut panes = HashMap::new();
        let node = build.node(root, &mut panes);
        let (focus, zoomed) = kept
            .focus
            .get(&view_tab.id)
            .and_then(|(far, zoomed)| {
                let pane_id = build.view.pane_ids.get(far)?;
                panes.contains_key(pane_id).then_some((*pane_id, *zoomed))
            })
            .unwrap_or((first_pane(&node), false));
        collect_numbers(root, build.view, &mut public_pane_numbers, &mut next_pane);
        if kept.active_tab.get(&space.id) == Some(&view_tab.id) {
            active_tab = tabs.len();
        }
        next_tab = next_tab.max(view_tab.number + 1);
        tabs.push(Tab {
            custom_name: (!view_tab.name.is_empty()).then(|| view_tab.name.clone()),
            number: view_tab.number,
            root_pane: first_pane(&node),
            layout: TileLayout::from_saved(node, focus),
            panes,
            #[cfg(test)]
            runtimes: HashMap::new(),
            zoomed,
            sync: None,
            events: wiring.events.clone(),
            render_notify: wiring.render_notify.clone(),
            render_dirty: wiring.render_dirty.clone(),
        });
    }
    // Panes the far herdr numbers none for get numbers past its own.
    for tab in &tabs {
        for pane_id in tab.layout.pane_ids() {
            public_pane_numbers.entry(pane_id).or_insert_with(|| {
                let number = next_pane;
                next_pane += 1;
                number
            });
        }
    }
    (!tabs.is_empty()).then(|| {
        Workspace::for_view(
            space.id.clone(),
            (!space.name.is_empty()).then(|| space.name.clone()),
            PathBuf::from(&space.folder),
            tabs,
            active_tab,
            public_pane_numbers,
            next_pane,
            next_tab,
        )
    })
}

/// A 7501 record as an agent and its state; an unknown program or state is
/// none.
fn view_status(
    status: &serde_json::Value,
) -> (Option<crate::detect::Agent>, crate::detect::AgentState) {
    use crate::detect::AgentState;
    let agent = status["app"]
        .as_str()
        .and_then(crate::detect::parse_agent_label);
    let state = match status["state"].as_str() {
        Some("working") => AgentState::Working,
        Some("blocked") => AgentState::Blocked,
        Some("idle" | "done" | "error") => AgentState::Idle,
        _ => AgentState::Unknown,
    };
    (agent, state)
}

/// The far public numbers of a tab's panes, so a pane id here is the far one
/// (D4); `next` ends past the highest.
fn collect_numbers(
    node: &ViewNode,
    view: &ViewApp,
    numbers: &mut HashMap<PaneId, usize>,
    next: &mut usize,
) {
    match node {
        ViewNode::Pane { id, number, .. } => {
            if let (Some(number), Some(pane_id)) = (number, view.pane_ids.get(id)) {
                numbers.insert(*pane_id, *number);
                *next = (*next).max(number + 1);
            }
        }
        ViewNode::Split { first, second, .. } => {
            collect_numbers(first, view, numbers, next);
            collect_numbers(second, view, numbers, next);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn app() -> App {
        let mut app = App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            tokio::sync::mpsc::unbounded_channel().1,
            crate::api::EventHub::default(),
        );
        app.view = Some(ViewApp::new(MachineName::parse("loop").unwrap()));
        app
    }

    fn layout(value: serde_json::Value) -> ViewLayout {
        crate::view::doc::parse(&value).expect("a good document")
    }

    fn block(id: &str, terminal: &str) -> serde_json::Value {
        json!({"id": id, "type": "block", "content": {"terminal": terminal}, "size": {"weight": 0.5}})
    }

    fn one_space(tabs: serde_json::Value) -> serde_json::Value {
        json!({"spaces": [{"id": "w3", "name": "api", "folder": "/far/api", "herdr_max": 1, "tabs": tabs}]})
    }

    fn public_ids(app: &App) -> Vec<String> {
        let mut ids: Vec<String> = app
            .state
            .workspaces
            .iter()
            .flat_map(|ws| {
                ws.tabs.iter().flat_map(move |tab| {
                    tab.layout.pane_ids().into_iter().map(move |pane| {
                        crate::workspace::public_pane_id_for_number(
                            &ws.id,
                            ws.public_pane_number(pane).unwrap(),
                        )
                    })
                })
            })
            .collect();
        ids.sort();
        ids
    }

    // V15's projection part (D6, D4): the far spaces, tabs and splits become
    // the view's, with the far ids; a new tab and a split show on the next
    // document, and panes that stay keep their pane and terminal.
    #[test]
    fn the_far_layout_becomes_the_views_with_the_far_ids() {
        let mut app = app();
        app.apply_view_layout(&layout(one_space(json!([
            {"id": "w3:t1", "root": {"id": "w3:t1", "type": "row", "children": [block("w3:p1", "t1")]}}
        ]))));
        assert_eq!(app.state.workspaces.len(), 1);
        let ws = &app.state.workspaces[0];
        assert_eq!(ws.id, "w3");
        assert_eq!(ws.custom_name.as_deref(), Some("api"));
        assert_eq!(public_ids(&app), ["w3:p1"]);
        let first_pane = app.state.workspaces[0].tabs[0].root_pane;
        let first_terminal = app.state.workspaces[0].tabs[0]
            .terminal_id(first_pane)
            .unwrap()
            .clone();
        assert_eq!(
            app.state.terminals[&first_terminal]
                .restore_error
                .as_deref(),
            Some("Attaching to loop/t1…")
        );

        app.apply_view_layout(&layout(one_space(json!([
            {"id": "w3:t1", "root": {"id": "w3:t1", "type": "row", "children": [block("w3:p1", "t1"), block("w3:p2", "t2")]}},
            {"id": "w3:t2", "name": "logs", "root": {"id": "w3:t2", "type": "row", "children": [block("w3:p3", "t3")]}}
        ]))));
        let ws = &app.state.workspaces[0];
        assert_eq!(ws.tabs.len(), 2, "the new tab");
        assert_eq!(ws.tabs[1].custom_name.as_deref(), Some("logs"));
        assert_eq!(public_tab_id_for_number(&ws.id, ws.tabs[1].number), "w3:t2");
        assert_eq!(ws.tabs[0].layout.pane_count(), 2, "the split");
        assert_eq!(
            ws.tabs[0].root_pane, first_pane,
            "the pane that stayed kept its id"
        );
        assert_eq!(
            ws.tabs[0].terminal_id(first_pane),
            Some(&first_terminal),
            "and its terminal"
        );
        assert_eq!(public_ids(&app), ["w3:p1", "w3:p2", "w3:p3"]);

        // The split closed there: its pane goes, its terminal is let go.
        app.apply_view_layout(&layout(one_space(json!([
            {"id": "w3:t1", "root": {"id": "w3:t1", "type": "row", "children": [block("w3:p1", "t1")]}}
        ]))));
        assert_eq!(public_ids(&app), ["w3:p1"]);
        assert_eq!(app.state.terminals.len(), 1);
    }

    #[test]
    fn focus_and_the_active_tab_survive_a_projection() {
        let mut app = app();
        let doc = layout(one_space(json!([
            {"id": "w3:t1", "root": {"id": "w3:t1", "type": "row", "children": [block("w3:p1", "t1"), block("w3:p2", "t2")]}},
            {"id": "w3:t2", "root": {"id": "w3:t2", "type": "row", "children": [block("w3:p3", "t3")]}}
        ])));
        app.apply_view_layout(&doc);
        let second = app.state.workspaces[0].tabs[0].layout.pane_ids()[1];
        app.state.workspaces[0].tabs[0].layout.focus_pane(second);
        app.state.workspaces[0].active_tab = 1;
        app.apply_view_layout(&doc);
        assert_eq!(app.state.workspaces[0].active_tab, 1);
        assert_eq!(app.state.workspaces[0].tabs[0].layout.focused(), second);
    }

    #[test]
    fn an_empty_layout_leaves_no_workspace() {
        let mut app = app();
        app.apply_view_layout(&layout(one_space(json!([
            {"id": "w3:t1", "root": {"id": "w3:t1", "type": "row", "children": [block("w3:p1", "t1")]}}
        ]))));
        app.apply_view_layout(&layout(json!({"spaces": []})));
        assert!(app.state.workspaces.is_empty());
        assert!(app.state.terminals.is_empty());
        assert_eq!(app.state.active, None);
    }

    #[test]
    fn a_far_size_reaches_the_pane_and_its_border() {
        let mut app = app();
        app.apply_view_layout(&layout(one_space(json!([
            {"id": "w3:t1", "root": {"id": "w3:t1", "type": "row", "children": [block("w3:p1", "t1")]}}
        ]))));
        app.handle_view_event(ViewEvent::FarSize {
            address: "loop/t1".into(),
            rows: 30,
            cols: 100,
        });
        let terminal = app.state.terminals.values().next().unwrap();
        assert_eq!(terminal.view_far_size, Some((30, 100)));
        app.handle_view_event(ViewEvent::Notice(Some(
            "loop is dormant: press a key to wake it".into(),
        )));
        assert_eq!(
            app.state.config_diagnostic.as_deref(),
            Some("loop is dormant: press a key to wake it")
        );
    }

    #[test]
    fn far_status_records_become_agent_states() {
        use crate::detect::AgentState;
        assert_eq!(
            view_status(&json!({"state": "working", "app": "claude"})).1,
            AgentState::Working
        );
        assert!(view_status(&json!({"state": "working", "app": "claude"}))
            .0
            .is_some());
        assert_eq!(
            view_status(&json!({"state": "blocked"})).1,
            AgentState::Blocked
        );
        assert_eq!(view_status(&json!({"state": "done"})).1, AgentState::Idle);
        assert_eq!(view_status(&json!(null)).1, AgentState::Unknown);
    }
}
