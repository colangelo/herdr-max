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
use crate::workspace::{public_pane_id_for_number, public_tab_id_for_number, Tab, Workspace};

/// The most far attaches a view runs at once (review M5): each holds a
/// blocking thread for its `terminal.get` and attach, so a far layout that
/// churns its terminals cannot stack them up; the rest wait their turn.
const MAX_ATTACHING: usize = 8;

/// What the view knows beyond its workspaces.
pub(crate) struct ViewApp {
    pub(crate) machine: MachineName,
    /// The far pane id (`w3:p4`) of each pane, both ways.
    pane_ids: HashMap<String, PaneId>,
    far_panes: HashMap<PaneId, String>,
    /// The terminal shown for each far address (`<machine>/t7`).
    terminals: HashMap<String, TerminalId>,
    /// Addresses whose attach is under way, with the generation of that
    /// attach: a completion of another generation is an obsolete one
    /// (review M5), detached at once.
    attaching: HashMap<String, u64>,
    /// Generations of the attaches whose blocking thread has not reported
    /// yet, the layout's or not: a dropped address's attach still holds its
    /// thread, so the slots count these (#312 follow-up), not `attaching`.
    running: HashSet<u64>,
    next_attach: u64,
    /// Attaches waiting for one of the `MAX_ATTACHING` slots.
    waiting: Vec<(String, PaneId)>,
    /// Far sizes (rows, cols) as core last reported them.
    far_sizes: HashMap<String, (u16, u16)>,
    /// The view's notice line (dormant, waking, nothing published), shown
    /// beside the config diagnostic, never in its place.
    notice: Option<String>,
    /// The far address each pane showed in the last projection.
    bound: HashMap<PaneId, String>,
    /// Panes whose far terminal changed under them (security review M3): the
    /// far layout put another address under the pane's id, or the address
    /// holds another program after a far core reset. Input typed for the old
    /// one must reach neither, so such a pane takes none until a person
    /// clicks it or focuses it again. A pane whose public id (`w3:p4`)
    /// another pane had, or whose own public id changed (a number the view
    /// gave a far pane that has none, review M4), is held too: a client
    /// holding that id meant another pane.
    held: HashSet<PaneId>,
    /// Each pane's public id in the last projection.
    public_ids: HashMap<PaneId, String>,
    /// The far panes whose close the far herdr was asked for without
    /// `force`, with the pane and far terminal it was: a forced close (the
    /// person confirmed) goes only to that same one (review M2).
    close_asked: HashMap<String, (PaneId, String)>,
    /// Where the view's structural actions go (E.3); the primary core and
    /// herdr's own machine state unless a test points it elsewhere.
    route: Option<crate::core_machine::fleet::view::ViewRoute>,
}

/// The most closes waiting for a person's confirmation the view keeps.
const MAX_CLOSE_ASKED: usize = 32;

impl ViewApp {
    pub(crate) fn new(machine: MachineName) -> Self {
        Self {
            machine,
            pane_ids: HashMap::new(),
            far_panes: HashMap::new(),
            terminals: HashMap::new(),
            attaching: HashMap::new(),
            running: HashSet::new(),
            next_attach: 0,
            waiting: Vec::new(),
            far_sizes: HashMap::new(),
            notice: None,
            bound: HashMap::new(),
            held: HashSet::new(),
            public_ids: HashMap::new(),
            close_asked: HashMap::new(),
            route: None,
        }
    }

    /// The far public pane id (`w3:p4`) of the view's pane `pane_id`, as
    /// the far document named it.
    pub(crate) fn far_pane(&self, pane_id: PaneId) -> Option<&str> {
        self.far_panes.get(&pane_id).map(String::as_str)
    }

    /// The far core's address (`t7`) of the terminal `pane_id` showed in
    /// the last projection.
    pub(crate) fn far_terminal(&self, pane_id: PaneId) -> Option<&str> {
        let address = self.bound.get(&pane_id)?;
        address
            .strip_prefix(self.machine.as_str())
            .and_then(|rest| rest.strip_prefix('/'))
    }

    /// Whether `pane_id` takes no input now (review M3, M4): neither does
    /// it take a structural action (M2).
    pub(crate) fn is_held(&self, pane_id: PaneId) -> bool {
        self.held.contains(&pane_id)
    }

    pub(crate) fn route(&self) -> crate::core_machine::fleet::view::ViewRoute {
        self.route
            .clone()
            .unwrap_or_else(crate::core_machine::fleet::view::ViewRoute::primary)
    }

    #[cfg(test)]
    pub(crate) fn set_route(&mut self, route: crate::core_machine::fleet::view::ViewRoute) {
        self.route = Some(route);
    }

    /// The far herdr was asked to close `far_pane` (the view's `pane_id`,
    /// on far terminal `terminal`) without `force`.
    pub(crate) fn note_close_asked(&mut self, far_pane: &str, pane_id: PaneId, terminal: &str) {
        if self.close_asked.len() >= MAX_CLOSE_ASKED && !self.close_asked.contains_key(far_pane) {
            self.close_asked.clear();
        }
        self.close_asked
            .insert(far_pane.to_string(), (pane_id, terminal.to_string()));
    }

    /// The pane and far terminal whose close of `far_pane` was asked for,
    /// once.
    pub(crate) fn take_close_asked(&mut self, far_pane: &str) -> Option<(PaneId, String)> {
        self.close_asked.remove(far_pane)
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
                // Another far terminal under the same pane id (review M3).
                if let Some(before) = self.view.bound.insert(pane_id, address.clone()) {
                    if before != address {
                        self.view.held.insert(pane_id);
                    }
                }
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
                    && !self.view.attaching.contains_key(&address)
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
    /// The view's notice line, `None` in the primary server or while all is
    /// well.
    pub(crate) fn view_notice(&self) -> Option<&str> {
        self.view.as_ref()?.notice.as_deref()
    }

    /// Why an action that would run something on this Mac, with the far
    /// side's ids or folders, is refused in a view (core-phase5 D6, review
    /// M4.1, L4.3); `None` in the primary server.
    pub(crate) fn view_refusal(&self, what: &str) -> Option<String> {
        self.view.as_ref().map(|view| {
            let machine = &view.machine;
            format!("this is a view of {machine}: {what} runs on this Mac, not on {machine}; use {machine}'s own herdr")
        })
    }

    /// Why a view refuses the client method `name` (core-phase5 D6,
    /// security review M2: `view::policy`, the list its welcome
    /// advertises); `None` in the primary server or for a method the view
    /// takes.
    pub(crate) fn view_method_refusal(&self, name: &str) -> Option<String> {
        let machine = &self.view.as_ref()?.machine;
        match crate::view::policy::classify(name) {
            Some(_) => crate::view::policy::refusal(machine, name),
            None => Some(crate::view::policy::unclassified_refusal(machine, name)),
        }
    }

    /// Turns this server into the view of `machine` (D6): no layout until the
    /// far document is read; the watcher thread follows the machine.
    pub(crate) fn start_view(
        &mut self,
        machine: MachineName,
        presence: Arc<ViewPresence>,
    ) -> std::io::Result<()> {
        self.view = Some(ViewApp::new(machine.clone()));
        // This Mac's release notes and announcements are the primary's to
        // show and dismiss: a view advertises no dismissal (review M2).
        self.state.latest_release_notes = None;
        self.state.latest_release_notes_available = false;
        self.state.product_announcement = None;
        crate::view::watch::start(machine, presence, self.event_tx.clone())
    }

    pub(crate) fn handle_view_event(&mut self, event: ViewEvent) {
        match event {
            ViewEvent::Layout(layout) => self.apply_view_layout(&layout),
            ViewEvent::Notice(notice) => {
                if let Some(view) = self.view.as_mut() {
                    view.notice = notice;
                }
            }
            ViewEvent::FarSize {
                address,
                rows,
                cols,
            } => self.follow_view_size(address, rows, cols),
            ViewEvent::Status { address, status } => self.apply_view_status(&address, &status),
            ViewEvent::Attached {
                address,
                generation,
                runtime,
            } => {
                if self.finish_view_attach(&address, generation) {
                    self.install_view_runtime(address, *runtime);
                } else {
                    // An attach the layout has since dropped or asked for
                    // again (review M5): detached, never shown.
                    runtime.shutdown();
                }
                self.start_waiting_view_attaches();
            }
            ViewEvent::AttachFailed {
                address,
                generation,
                reason,
            } => {
                if self.finish_view_attach(&address, generation) {
                    if let Some(state) = self
                        .view
                        .as_ref()
                        .and_then(|view| view.terminals.get(&address))
                        .and_then(|terminal_id| self.state.terminals.get_mut(terminal_id))
                    {
                        state.restore_error = Some(reason);
                        state.revision = state.revision.saturating_add(1);
                    }
                }
                self.start_waiting_view_attaches();
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
        // Sizes of terminals the layout no longer shows go too, and so do
        // their waiting attaches (review M5).
        view.far_sizes.retain(|address, _| used.contains(address));
        view.waiting.retain(|(address, _)| used.contains(address));
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
        view.bound.retain(|pane_id, _| live_panes.contains(pane_id));
        view.held.retain(|pane_id| live_panes.contains(pane_id));
        view.close_asked
            .retain(|_, (pane_id, _)| live_panes.contains(pane_id));

        let find = |id: &Option<String>| {
            id.as_ref()
                .and_then(|id| workspaces.iter().position(|ws| &ws.id == id))
        };
        let active = find(&kept.active_workspace).or((!workspaces.is_empty()).then_some(0));
        let selected = find(&kept.selected_workspace).or(active).unwrap_or(0);
        self.state.workspaces = workspaces;
        hold_moved_public_ids(&mut view, &self.state.workspaces);
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
        self.sync_view_holds();
        self.full_redraw_pending = true;
        self.state.mark_session_dirty();
    }

    /// Marks the terminal of each held pane for its border, and clears the
    /// mark everywhere else.
    fn sync_view_holds(&mut self) {
        let Some(view) = self.view.as_ref() else {
            return;
        };
        let held: HashSet<TerminalId> = self
            .state
            .workspaces
            .iter()
            .flat_map(|ws| ws.tabs.iter())
            .flat_map(|tab| tab.panes.iter())
            .filter(|(pane_id, _)| view.held.contains(pane_id))
            .map(|(_, pane)| pane.attached_terminal_id.clone())
            .collect();
        for (terminal_id, state) in &mut self.state.terminals {
            let now = held.contains(terminal_id);
            if state.view_input_held != now {
                state.view_input_held = now;
                state.revision = state.revision.saturating_add(1);
            }
        }
    }

    /// Whether `pane_id` is a view's pane that takes no input until a
    /// person clicks or focuses it (security review M3).
    pub(crate) fn view_input_held(&self, pane_id: PaneId) -> bool {
        self.view
            .as_ref()
            .is_some_and(|view| view.held.contains(&pane_id))
    }

    /// Holds `pane_id`'s input as a changed far binding does.
    #[cfg(test)]
    pub(crate) fn hold_view_input_for_test(&mut self, pane_id: PaneId) {
        if let Some(view) = self.view.as_mut() {
            view.held.insert(pane_id);
        }
        self.sync_view_holds();
    }

    /// A person clicked or focused `pane_id`: it takes input again. True
    /// when it was held.
    pub(crate) fn release_view_hold(&mut self, pane_id: PaneId) -> bool {
        let released = self
            .view
            .as_mut()
            .is_some_and(|view| view.held.remove(&pane_id));
        if released {
            self.sync_view_holds();
            self.full_redraw_pending = true;
        }
        released
    }

    /// A view's pane whose stream ended because its far address holds
    /// another program now (review M3): the pane is bound afresh to that
    /// program, attached again, and takes no input until a person clicks or
    /// focuses it. True when `pane_id` was such a pane (its exit is not the
    /// far program's end).
    pub(crate) fn rebind_replaced_view_pane(&mut self, pane_id: PaneId) -> bool {
        let Some(terminal_id) = self
            .state
            .pane_terminal(pane_id)
            .map(|terminal| terminal.id.clone())
        else {
            return false;
        };
        if !self
            .terminal_runtimes
            .get(&terminal_id)
            .is_some_and(crate::terminal::TerminalRuntime::view_replaced)
        {
            return false;
        }
        let Some(mut view) = self.view.take() else {
            return false;
        };
        let address = view
            .terminals
            .iter()
            .find(|(_, id)| **id == terminal_id)
            .map(|(address, _)| address.clone());
        if let Some(runtime) = self.terminal_runtimes.remove(&terminal_id) {
            runtime.shutdown();
        }
        if let (Some(address), Some(state)) = (address, self.state.terminals.get_mut(&terminal_id))
        {
            state.restore_error = Some(format!(
                "{address} is another program now; attaching to it…"
            ));
            state.revision = state.revision.saturating_add(1);
            view.held.insert(pane_id);
            if !view.attaching.contains_key(&address) {
                self.request_view_attach(&mut view, address, pane_id);
            }
        }
        self.view = Some(view);
        self.sync_view_holds();
        self.full_redraw_pending = true;
        true
    }

    /// Whether an attach completion of `generation` is the one under way
    /// for `address`; if so, it is no longer under way.
    fn finish_view_attach(&mut self, address: &str, generation: u64) -> bool {
        let Some(view) = self.view.as_mut() else {
            return false;
        };
        // Its thread is done, whichever attach it was: the slot is free.
        view.running.remove(&generation);
        if view.attaching.get(address) != Some(&generation) {
            return false;
        }
        view.attaching.remove(address);
        true
    }

    /// Starts waiting attaches while slots are free, for terminals the
    /// layout still shows and that are still not attached.
    fn start_waiting_view_attaches(&mut self) {
        let Some(mut view) = self.view.take() else {
            return;
        };
        while view.running.len() < MAX_ATTACHING && !view.waiting.is_empty() {
            let (address, pane_id) = view.waiting.remove(0);
            let unattached = view
                .terminals
                .get(&address)
                .and_then(|terminal_id| self.state.terminals.get(terminal_id))
                .is_some_and(|state| state.restore_error.is_some());
            if unattached && !view.attaching.contains_key(&address) {
                self.request_view_attach(&mut view, address, pane_id);
            }
        }
        self.view = Some(view);
    }

    fn request_view_attach(&mut self, view: &mut ViewApp, address: String, pane_id: PaneId) {
        if view.attaching.contains_key(&address) {
            return;
        }
        if view.running.len() >= MAX_ATTACHING {
            if !view.waiting.iter().any(|(waiting, _)| *waiting == address) {
                view.waiting.push((address, pane_id));
            }
            return;
        }
        view.next_attach += 1;
        let generation = view.next_attach;
        view.attaching.insert(address.clone(), generation);
        view.running.insert(generation);
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            // The server loop always runs on its runtime; only a unit test
            // gets here, and its attach stays under way.
            tracing::warn!(%address, "no runtime to attach a view pane on");
            return;
        };
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
                    generation,
                    runtime: Box::new(runtime),
                },
                Err(err) => ViewEvent::AttachFailed {
                    address,
                    generation,
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
        // Over the grid budget: ignored, the pane keeps its last size
        // (security review M4).
        if !crate::view::far_size_fits(rows, cols) {
            tracing::debug!(%address, rows, cols, "a far size over the view's budget is ignored");
            return;
        }
        // Only for an address the layout shows (review M5): an attach reads
        // the size of a new one itself, so no size is kept for others.
        let Some(terminal_id) = view.terminals.get(&address) else {
            return;
        };
        view.far_sizes.insert(address.clone(), (rows, cols));
        if let Some(runtime) = self.terminal_runtimes.get(terminal_id) {
            if !runtime.follow_view_size(rows, cols) {
                return;
            }
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

/// Holds each pane whose public id changed, or that took an id another
/// pane had (review M4): a number the view gave a far pane that has none
/// moves when the far herdr numbers another pane, and a client that still
/// holds the old id must reach neither.
fn hold_moved_public_ids(view: &mut ViewApp, workspaces: &[Workspace]) {
    let public: HashMap<PaneId, String> = workspaces
        .iter()
        .flat_map(|ws| {
            ws.tabs.iter().flat_map(move |tab| {
                tab.layout.pane_ids().into_iter().filter_map(move |pane| {
                    ws.public_pane_number(pane)
                        .map(|number| (pane, public_pane_id_for_number(&ws.id, number)))
                })
            })
        })
        .collect();
    let owner_before: HashMap<&str, PaneId> = view
        .public_ids
        .iter()
        .map(|(pane, id)| (id.as_str(), *pane))
        .collect();
    let moved: Vec<PaneId> = public
        .iter()
        .filter(|(pane, id)| {
            view.public_ids
                .get(*pane)
                .is_some_and(|before| before != *id)
                || owner_before
                    .get(id.as_str())
                    .is_some_and(|before| before != *pane)
        })
        .map(|(pane, _)| *pane)
        .collect();
    view.held.extend(moved);
    view.public_ids = public;
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
        // The notice is the view's own: a config diagnostic stays.
        app.state.config_diagnostic = Some("config: unknown key".into());
        app.handle_view_event(ViewEvent::Notice(Some(
            "loop is dormant: press a key to wake it".into(),
        )));
        assert_eq!(
            app.view_notice(),
            Some("loop is dormant: press a key to wake it")
        );
        assert_eq!(
            app.state.config_diagnostic.as_deref(),
            Some("config: unknown key")
        );
        assert_eq!(
            crate::view::with_notice(app.state.config_diagnostic.clone(), app.view_notice())
                .as_deref(),
            Some("config: unknown key\nloop is dormant: press a key to wake it")
        );
        app.handle_view_event(ViewEvent::Notice(None));
        assert_eq!(app.view_notice(), None);
    }

    // M3 (security review): when the far layout puts another terminal under
    // a pane's id (here two panes swap terminals), the pane's input is held:
    // keys typed for the old terminal must reach neither. The border says
    // so, and a person's focus releases it. A pane that keeps its terminal
    // is not held.
    #[test]
    fn a_pane_whose_far_terminal_changes_holds_its_input_until_focused() {
        let mut app = app();
        let doc = |first: &str, second: &str, third: &str| {
            layout(one_space(json!([
                {"id": "w3:t1", "root": {"id": "w3:t1", "type": "row", "children": [
                    block("w3:p1", first), block("w3:p2", second), block("w3:p3", third)
                ]}}
            ])))
        };
        app.apply_view_layout(&doc("t1", "t2", "t3"));
        let panes = app.state.workspaces[0].tabs[0].layout.pane_ids();
        assert!(panes.iter().all(|pane| !app.view_input_held(*pane)));
        app.apply_view_layout(&doc("t2", "t1", "t3"));
        let held: Vec<bool> = panes
            .iter()
            .map(|pane| app.view_input_held(*pane))
            .collect();
        assert_eq!(held, [true, true, false], "the two swapped panes only");
        let marked = |app: &App, pane: PaneId| {
            app.state
                .pane_terminal(pane)
                .is_some_and(|terminal| terminal.view_input_held)
        };
        assert!(marked(&app, panes[0]) && marked(&app, panes[1]) && !marked(&app, panes[2]));
        // A projection of the same layout keeps the hold.
        app.apply_view_layout(&doc("t2", "t1", "t3"));
        assert!(app.view_input_held(panes[0]));
        assert!(app.release_view_hold(panes[0]));
        assert!(!app.view_input_held(panes[0]) && !marked(&app, panes[0]));
        assert!(app.view_input_held(panes[1]), "only the one focused");
    }

    // M4, M5: a far size over the grid budget is ignored (the pane keeps
    // its last size), and no size is kept for an address the layout does not
    // show, nor for one it no longer shows.
    #[test]
    fn far_sizes_are_bounded_and_kept_only_for_shown_terminals() {
        let mut app = app();
        app.apply_view_layout(&layout(one_space(json!([
            {"id": "w3:t1", "root": {"id": "w3:t1", "type": "row", "children": [block("w3:p1", "t1")]}}
        ]))));
        let size = |app: &App| app.state.terminals.values().next().unwrap().view_far_size;
        app.handle_view_event(ViewEvent::FarSize {
            address: "loop/t1".into(),
            rows: 30,
            cols: 100,
        });
        for (rows, cols) in [(65_535, 65_535), (2000, 10), (600, 600)] {
            app.handle_view_event(ViewEvent::FarSize {
                address: "loop/t1".into(),
                rows,
                cols,
            });
        }
        assert_eq!(
            size(&app),
            Some((30, 100)),
            "over the budget: the last size"
        );
        for n in 0..500 {
            app.handle_view_event(ViewEvent::FarSize {
                address: format!("loop/t{}", 100 + n),
                rows: 30,
                cols: 100,
            });
        }
        let view = app.view.as_ref().unwrap();
        assert_eq!(view.far_sizes.len(), 1, "only the shown terminal's size");
        app.apply_view_layout(&layout(one_space(json!([
            {"id": "w3:t1", "root": {"id": "w3:t1", "type": "row", "children": [block("w3:p1", "t2")]}}
        ]))));
        let view = app.view.as_ref().unwrap();
        assert!(
            !view.far_sizes.contains_key("loop/t1"),
            "a gone terminal's size goes with it"
        );
    }

    // M5 (security review): at most MAX_ATTACHING far attaches run at once;
    // the rest wait and start as slots free. A completion of an attach the
    // layout has since dropped or asked for again is detached, never shown,
    // and a dropped terminal's waiting attach goes with it.
    #[test]
    fn far_attaches_are_capped_and_obsolete_ones_are_detached() {
        let mut app = app();
        let children: Vec<serde_json::Value> = (1..=20)
            .map(|n| block(&format!("w3:p{n}"), &format!("t{n}")))
            .collect();
        app.apply_view_layout(&layout(one_space(json!([
            {"id": "w3:t1", "root": {"id": "w3:t1", "type": "row", "children": children}}
        ]))));
        let view = app.view.as_ref().unwrap();
        assert_eq!(view.attaching.len(), MAX_ATTACHING);
        assert_eq!(view.waiting.len(), 20 - MAX_ATTACHING);

        let first = |app: &App| {
            let view = app.view.as_ref().unwrap();
            let mut under_way: Vec<(String, u64)> = view
                .attaching
                .iter()
                .map(|(address, generation)| (address.clone(), *generation))
                .collect();
            under_way.sort();
            under_way.remove(0)
        };
        let (address, generation) = first(&app);
        app.handle_view_event(ViewEvent::AttachFailed {
            address,
            generation,
            reason: "far core away".into(),
        });
        let view = app.view.as_ref().unwrap();
        assert_eq!(view.attaching.len(), MAX_ATTACHING, "a waiting one started");
        assert_eq!(view.waiting.len(), 20 - MAX_ATTACHING - 1);

        let (address, generation) = first(&app);
        // A runtime for the test pane only; every slot is taken, so no attach
        // starts on it.
        let tokio = tokio::runtime::Runtime::new().expect("a runtime");
        let entered = tokio.enter();
        let (runtime, _input) = crate::terminal::TerminalRuntime::test_with_channel(80, 24);
        app.handle_view_event(ViewEvent::Attached {
            address: address.clone(),
            generation: generation + 1000,
            runtime: Box::new(runtime),
        });
        drop(entered);
        assert_eq!(
            app.terminal_runtimes.len(),
            0,
            "an obsolete attach is never shown"
        );
        assert_eq!(
            app.view.as_ref().unwrap().attaching.get(&address),
            Some(&generation),
            "the current attach is still under way"
        );

        app.apply_view_layout(&layout(one_space(json!([
            {"id": "w3:t1", "root": {"id": "w3:t1", "type": "row", "children": [block("w3:p1", "t1")]}}
        ]))));
        let view = app.view.as_ref().unwrap();
        // Only the shown terminal waits or attaches; the dropped attaches
        // still hold their slots until they report (#312 follow-up).
        assert!(
            view.waiting.iter().all(|(address, _)| address == "loop/t1"),
            "{:?}",
            view.waiting
        );
        assert!(view.attaching.keys().all(|address| address == "loop/t1"));
    }

    // #312 follow-up: the slots count attaches whose thread still runs. A
    // layout that drops its terminals while their attaches block leaves
    // those threads running, so new terminals wait until they report.
    #[test]
    fn dropped_attaches_hold_their_slots_until_they_report() {
        let mut app = app();
        let children = |from: usize| -> Vec<serde_json::Value> {
            (from..from + MAX_ATTACHING)
                .map(|n| block(&format!("w3:p{n}"), &format!("t{n}")))
                .collect()
        };
        app.apply_view_layout(&layout(one_space(json!([
            {"id": "w3:t1", "root": {"id": "w3:t1", "type": "row", "children": children(1)}}
        ]))));
        let dropped: Vec<(String, u64)> = app
            .view
            .as_ref()
            .unwrap()
            .attaching
            .iter()
            .map(|(address, generation)| (address.clone(), *generation))
            .collect();
        assert_eq!(dropped.len(), MAX_ATTACHING);
        app.apply_view_layout(&layout(one_space(json!([
            {"id": "w3:t1", "root": {"id": "w3:t1", "type": "row", "children": children(100)}}
        ]))));
        let view = app.view.as_ref().unwrap();
        assert!(
            view.attaching.is_empty(),
            "no slot is free: {:?}",
            view.attaching
        );
        assert_eq!(view.waiting.len(), MAX_ATTACHING);
        let (address, generation) = dropped[0].clone();
        app.handle_view_event(ViewEvent::AttachFailed {
            address,
            generation,
            reason: "far core away".into(),
        });
        let view = app.view.as_ref().unwrap();
        assert_eq!(view.attaching.len(), 1, "one dropped attach reported");
        assert_eq!(view.waiting.len(), MAX_ATTACHING - 1);
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
