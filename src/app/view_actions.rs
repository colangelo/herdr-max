//! A view's structural actions (core-phase5 D7, E.3): new tab, split, close
//! pane and rename, which `view::policy` classes `Far`, are not done here.
//! The view turns the client's ids into the far herdr's through its own
//! layout (so only what the far document showed can be named), builds the
//! far API request with the client's names as JSON strings, and sends it
//! off the loop through `core_machine::fleet::view`. The far herdr changes
//! its layout and republishes; the view follows the document. The far
//! reply, or why there is none, is the client's answer: an error is a
//! notice there, and the view stays connected.
//!
//! An action on a pane goes to the far program the person aimed at, or
//! nowhere (review M2, M4; E.3 review item 1): a pane the view holds
//! (another terminal under its id, a far core reset, a public id that
//! moved) is refused, so is a pane not attached (no far program bound
//! yet) and one the far herdr has no number for; a forced close goes only
//! to the pane, terminal and program whose close was asked for; and just
//! before the request the far document is read again (the far pane still
//! shows that terminal) and core is asked for the terminal (it is still the
//! program the pane attached to). A view never closes the last pane of a
//! far workspace: the far herdr would close the workspace, and a worktree
//! group's parent with it (security review of E.3, M2 P2).
//!
//! Fork issue 316 (M1, M2): the far herdr checks the program again, in the
//! same turn of its loop as the action. A pane's close, split and rename go
//! as `pane.close_if_terminal`, `pane.split_if_terminal` and
//! `pane.rename_if_terminal`, naming the far terminal, pid and launch label
//! the pane attached to; a far herdr whose pane shows another program
//! refuses (`terminal_mismatch`), and a far guarded close never closes a
//! workspace (`would_close_workspace`). An older far herdr does not know
//! those methods and refuses them unparsed, so no pane action reaches a far
//! herdr that cannot check it: the person is told to update Herdr there.
//! A pane action goes only for a pane of the tab the sending client's window
//! shows, and a client's request id is taken once (M5). A forced close goes
//! only with a one-use confirmation minted when the far herdr answered
//! `confirmation_required` for the same pane and program, within
//! `CONFIRM_FOR` (M4).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::App;
use crate::api::schema::{
    ErrorBody, ErrorResponse, Method, PaneCloseIfTerminalParams, PaneRenameIfTerminalParams,
    PaneSplitIfTerminalParams, PaneTerminalExpectation, Request, ResponseResult, SuccessResponse,
    TabCreateParams, TabRenameParams, WorkspaceRenameParams,
};
use crate::core_machine::fleet::view::{AimCheck, Commit, ViewRoute, VIEW_ACTION_BUDGET};
use crate::core_machine::MachineName;
use crate::layout::PaneId;
use crate::view::instance::FarInstance;

/// The code of an action the view could not send, the far side did not
/// answer, or refused for a reason the client has no handling of its own
/// for: nothing a client retries by itself.
const NOT_DONE: &str = "view_action_failed";

/// The far error codes passed on to the client as they are, because this
/// build knows what the client does with them: a close's
/// `confirmation_required` asks the person, then sends the close forced
/// (review L2). Every other far code becomes `NOT_DONE`, shown.
const PASSED_ON: &[&str] = &["confirmation_required"];

/// The far code that asks the person to confirm a close.
const CONFIRMATION_REQUIRED: &str = "confirmation_required";

/// How long the person's answer to a far `confirmation_required` may take:
/// a forced close after it is not sent (M4).
pub(crate) const CONFIRM_FOR: std::time::Duration = std::time::Duration::from_secs(120);

/// The far pane, terminal and program a pane action aims at (review M2,
/// E.3 review item 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Aimed {
    /// The far public pane id (`w3:p4`).
    pub(crate) far_pane: String,
    /// The far core's address of its terminal (`t7`).
    pub(crate) terminal: String,
    /// The far program the pane attached to there.
    pub(crate) instance: FarInstance,
    /// What the far herdr checks before it acts (fork issue 316, M1): the
    /// terminal, pid and launch label of `instance`.
    pub(crate) expect: PaneTerminalExpectation,
    pane_id: PaneId,
}

/// The most closes waiting for a person's confirmation a view keeps.
const MAX_CLOSE_ASKED: usize = 32;

/// The person's confirmations of far closes, by client and far pane (review
/// M2, M4): one is minted only when the far herdr answered
/// `confirmation_required` to a close without `force` of that pane and
/// program by that client, lasts `CONFIRM_FOR`, and is spent by that
/// client's one forced close it lets through, at the point the request is
/// sent (`ViewRoute`'s `send` calls the spend last of all). Another client
/// of the same view cannot spend it (fork issue 316, review item 3). It is
/// bound to the request it answers (fork issue 317 M3): the forced close
/// names the client's id of the close the far herdr asked about, so the
/// answer to an earlier prompt cannot spend a later one's, and it goes when
/// its client disconnects (`forget_client`). Shared with an action's
/// thread: a send refused before the request (busy, not connected, the aim
/// check, a timeout, the lease) leaves it in place (E.3 review, item 9).
#[derive(Clone, Default)]
pub(crate) struct CloseAsked(Arc<Mutex<AskedMap>>);

/// A confirmation by (client, far pane): the aim, when it lapses, and the
/// client's id of the close it answers.
type AskedMap = HashMap<(u64, String), (Aimed, Instant, String)>;

impl CloseAsked {
    fn map(&self) -> std::sync::MutexGuard<'_, AskedMap> {
        match self.0.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// The far herdr asked `client` to confirm the close of `aimed` at
    /// `now`, which the client sent as its request `request`.
    fn mint(&self, client: u64, aimed: &Aimed, now: Instant, request: &str) {
        let mut asked = self.map();
        let key = (client, aimed.far_pane.clone());
        asked.retain(|_, (_, until, _)| *until > now);
        if asked.len() >= MAX_CLOSE_ASKED && !asked.contains_key(&key) {
            asked.clear();
        }
        asked.insert(key, (aimed.clone(), now + CONFIRM_FOR, request.to_string()));
    }

    /// Whether a confirmation of exactly `aimed` by `client`, for its
    /// request `confirms`, is there at `now`.
    fn was_asked(&self, client: u64, aimed: &Aimed, now: Instant, confirms: Option<&str>) -> bool {
        self.map()
            .get(&(client, aimed.far_pane.clone()))
            .is_some_and(|(asked, until, request)| {
                asked == aimed && *until > now && confirms == Some(request.as_str())
            })
    }

    /// Spends `client`'s confirmation of `aimed` for its request `confirms`,
    /// if it is there at `now`.
    fn spend(&self, client: u64, aimed: &Aimed, now: Instant, confirms: Option<&str>) -> bool {
        let mut asked = self.map();
        let key = (client, aimed.far_pane.clone());
        let there = asked.get(&key).is_some_and(|(asked, until, request)| {
            asked == aimed && *until > now && confirms == Some(request.as_str())
        });
        if there {
            asked.remove(&key);
        }
        there
    }

    /// Keeps the records of the panes in `live`.
    pub(crate) fn retain(&self, live: &HashSet<PaneId>) {
        self.map()
            .retain(|_, (aimed, _, _)| live.contains(&aimed.pane_id));
    }

    /// Drops `client`'s confirmations: it disconnected (fork issue 317 M3).
    pub(crate) fn forget_client(&self, client: u64) {
        self.map().retain(|(asked_by, _), _| *asked_by != client);
    }
}

/// The most request ids a view remembers per client (M5).
pub(crate) const MAX_SEEN_ACTIONS: usize = 256;

/// The request ids of the actions a view took, per client and newest last
/// (M5): a client's request id is taken once, so a request sent again (by
/// any program that can reach the view's socket) is refused, not done twice.
/// The window (fork issue 317 M5): the last `MAX_SEEN_ACTIONS` of each
/// connected client, so another client cannot push a client's ids out; a
/// client's ids go when it disconnects, and its next connection is another
/// client id (a server never gives one twice), and a view restarted has
/// another boot id, which the server refuses an older request for
/// (`stale_boot`). Within one connection, an id older than its last 256
/// could be taken again: only a program that could send a new id anyway.
#[derive(Default)]
pub(crate) struct SeenActions(HashMap<u64, std::collections::VecDeque<String>>);

impl SeenActions {
    /// Takes `client`'s `id`: false when it was taken before.
    fn take(&mut self, client: u64, id: &str) -> bool {
        let seen = self.0.entry(client).or_default();
        if seen.iter().any(|seen_id| seen_id == id) {
            return false;
        }
        if seen.len() >= MAX_SEEN_ACTIONS {
            seen.pop_front();
        }
        seen.push_back(id.to_string());
        true
    }

    /// Drops what `client` took: it disconnected.
    pub(crate) fn forget_client(&mut self, client: u64) {
        self.0.remove(&client);
    }
}

/// A view's request for the far herdr, and the far pane it aims at.
#[derive(Debug)]
pub(crate) struct FarAction {
    pub(crate) method: Method,
    pub(crate) aimed: Option<Aimed>,
}

/// The tab a client's own window shows, as (workspace, tab) indexes.
pub(crate) type ClientTab = Option<(usize, usize)>;

/// Who sends a view's structural action, and what their window showed
/// (fork issue 317 M2, M3).
#[derive(Debug, Clone)]
pub(crate) struct ViewSender {
    /// The sending client shell's connection.
    pub(crate) client: u64,
    /// The tab its window shows.
    pub(crate) tab: ClientTab,
    /// The view's binding revision in the snapshot the window showed when
    /// the person acted; `None` from a client that does not say (an older
    /// build), which takes no pane action.
    pub(crate) binding: Option<u64>,
    /// For a forced close: the client's id of the unforced close whose
    /// confirmation the person answered.
    pub(crate) confirms: Option<String>,
    /// The client's own id of this request.
    pub(crate) request: String,
}

impl App {
    /// The far herdr's request for the structural action `method` of a view
    /// (D7): the same method with the far ids of its targets, the client's
    /// names unchanged, and nothing of this Mac (no folder, environment,
    /// machine or focus: a far folder is the far herdr's to pick, and the
    /// far screen keeps its focus; a request that names a folder, an
    /// environment or a machine is refused, review L3). Ids left out
    /// resolve against `client_tab`, the sending client's own tab (review
    /// L4). A pane's close, split and rename go as their guarded methods,
    /// naming the far program for the far herdr to check (fork issue 316,
    /// M1), and only for a pane of `client_tab` (M5), as the sending
    /// window saw it (fork issue 317 M2): not one the view holds for that
    /// client (its far binding changed and that client's person has not
    /// clicked or focused it since), nor one whose binding changed after the
    /// revision of the snapshot that window showed, and not for a client
    /// that says no revision. `Err` when a target is not one the far
    /// document showed, is held, has no far number, is not in the client's
    /// tab, not attached to a program known in full or not as the window
    /// showed it, or the method is not one a view sends there.
    pub(crate) fn view_far_action(
        &self,
        method: &Method,
        sender: &ViewSender,
    ) -> Result<FarAction, String> {
        let client_tab = sender.tab;
        let Some(view) = self.view.as_ref() else {
            return Err("this is not a view".into());
        };
        let machine = &view.machine;
        let unknown =
            |what: &str, id: &str| format!("{what} {id} is not one {machine}'s herdr shows");
        let no_tab = || {
            format!(
                "this window shows no tab of {machine}; click a pane of {machine} and do this again"
            )
        };
        let far_workspace = |id: Option<&str>| -> Result<(usize, String), String> {
            let index = match id {
                Some(id) => self
                    .parse_workspace_id(id)
                    .ok_or_else(|| unknown("workspace", id))?,
                None => client_tab.ok_or_else(no_tab)?.0,
            };
            let ws = self
                .state
                .workspaces
                .get(index)
                .ok_or_else(|| format!("no workspace of {machine} is open in this view"))?;
            Ok((index, ws.id.clone()))
        };
        let far_pane = |id: &str, ws: usize, pane: PaneId| -> Result<Aimed, String> {
            // M5: a person acts on a pane the window shows; a request for
            // another (from any program that reaches the view's socket) is
            // not one.
            let shown = client_tab.is_some_and(|(shown_ws, shown_tab)| {
                shown_ws == ws
                    && self
                        .state
                        .workspaces
                        .get(ws)
                        .and_then(|workspace| workspace.tabs.get(shown_tab))
                        .is_some_and(|tab| tab.panes.contains_key(&pane))
            });
            if !shown {
                return Err(format!(
                    "pane {id} is not in the tab this window shows; click it, then do this again"
                ));
            }
            let far = view.far_pane(pane).ok_or_else(|| unknown("pane", id))?;
            if !far_numbered(far) {
                return Err(format!(
                    "pane {id} has no number in {machine}'s herdr, so a view cannot name it there; do this in {machine}'s herdr"
                ));
            }
            if view.is_held_for(pane, sender.client) {
                return Err(format!(
                    "pane {id} shows another terminal of {machine} than before: click it to see it, then do this again"
                ));
            }
            // Only the far core's own terminal (`t7`) goes into an address
            // (`<machine>/t7`) and a request (security delta re-check of PR
            // 100, item 2).
            let terminal = view
                .far_terminal(pane)
                .filter(|terminal| crate::view::doc::is_local_terminal(terminal))
                .ok_or_else(|| unknown("pane", id))?;
            let not_attached = || {
                format!(
                    "pane {id} is not attached to its program on {machine} yet, so a view cannot tell which program it would reach; wait until it shows, then do this again"
                )
            };
            let replaced = self
                .state
                .pane_terminal(pane)
                .and_then(|state| self.terminal_runtimes.get(&state.id))
                .is_some_and(crate::terminal::TerminalRuntime::view_replaced);
            let ended = view.far_instance(pane).is_some_and(FarInstance::has_ended);
            if ended && !replaced {
                return Err(format!(
                    "core shows no process for the program of pane {id} on {machine}: it has ended (or core did not say), so a view cannot act on it; do this in {machine}'s herdr"
                ));
            }
            let instance = view
                .far_instance(pane)
                .filter(|instance| instance.is_known() && !replaced)
                .ok_or_else(not_attached)?;
            // M4 of fork issue 317: the far herdr's own id of the tab the
            // window shows the pane in, as its document named it.
            let far_tab = client_tab
                .and_then(|(_, shown_tab)| self.public_tab_id(ws, shown_tab))
                .and_then(|tab_id| view.far_tab(&tab_id))
                .ok_or_else(|| unknown("pane", id))?;
            let expect = instance
                .expectation(terminal, far_tab)
                .ok_or_else(not_attached)?;
            let Some(seen) = sender.binding else {
                return Err(format!(
                    "this window's Herdr does not say which program of {machine} it showed in pane {id}, so a view does not act on it; update Herdr here"
                ));
            };
            if view.rebound_since(pane, seen) {
                return Err(format!(
                    "pane {id} of {machine} shows another program than this window showed when you did this; look at it again, then do this again"
                ));
            }
            Ok(Aimed {
                far_pane: far.to_string(),
                terminal: terminal.to_string(),
                instance: instance.clone(),
                expect,
                pane_id: pane,
            })
        };
        let far_pane_id = |id: &str| -> Result<(usize, Aimed), String> {
            let (ws, pane) = self.parse_pane_id(id).ok_or_else(|| unknown("pane", id))?;
            Ok((ws, far_pane(id, ws, pane)?))
        };
        let nothing_of_this_mac = |cwd: bool, env: bool, on: bool| -> Result<(), String> {
            if cwd || env || on {
                return Err(format!(
                    "a view sends no folder, environment or machine to {machine}; do this in {machine}'s herdr"
                ));
            }
            Ok(())
        };
        Ok(match method {
            Method::TabCreate(params) => {
                nothing_of_this_mac(
                    params.cwd.is_some(),
                    !params.env.is_empty(),
                    params.on.is_some(),
                )?;
                let (_, workspace_id) = far_workspace(params.workspace_id.as_deref())?;
                FarAction {
                    method: Method::TabCreate(TabCreateParams {
                        workspace_id: Some(workspace_id),
                        cwd: None,
                        focus: false,
                        label: params.label.clone(),
                        env: HashMap::new(),
                        on: None,
                    }),
                    aimed: None,
                }
            }
            Method::PaneSplit(params) => {
                nothing_of_this_mac(
                    params.cwd.is_some(),
                    !params.env.is_empty(),
                    params.on.is_some(),
                )?;
                let (ws, aimed) = match params.target_pane_id.as_deref() {
                    Some(id) => far_pane_id(id)?,
                    None => {
                        // The focused pane of the client's own tab, in the
                        // workspace the request names, if it names one.
                        let (ws, tab) = client_tab.ok_or_else(no_tab)?;
                        if let Some(id) = params.workspace_id.as_deref() {
                            if far_workspace(Some(id))?.0 != ws {
                                return Err(format!(
                                    "this window shows another workspace of {machine}; name the pane to split"
                                ));
                            }
                        }
                        let pane = self
                            .state
                            .workspaces
                            .get(ws)
                            .and_then(|workspace| workspace.tabs.get(tab))
                            .map(|tab| tab.layout.focused())
                            .ok_or_else(no_tab)?;
                        let id = self.public_pane_id(ws, pane).unwrap_or_default();
                        (ws, far_pane(&id, ws, pane)?)
                    }
                };
                if let Some(id) = params.workspace_id.as_deref() {
                    if far_workspace(Some(id))?.0 != ws {
                        return Err(format!(
                            "pane {} is not in workspace {id} of {machine}",
                            aimed.far_pane
                        ));
                    }
                }
                FarAction {
                    method: Method::PaneSplitIfTerminal(PaneSplitIfTerminalParams {
                        pane_id: aimed.far_pane.clone(),
                        direction: params.direction.clone(),
                        ratio: params.ratio,
                        expect: aimed.expect.clone(),
                    }),
                    aimed: Some(aimed),
                }
            }
            Method::PaneClose(params) => {
                let (ws, aimed) = far_pane_id(&params.pane_id)?;
                let workspace = &self.state.workspaces[ws];
                let panes: usize = workspace
                    .tabs
                    .iter()
                    .map(|tab| tab.layout.pane_count())
                    .sum();
                if panes <= 1 {
                    return Err(last_pane_refusal(
                        workspace.custom_name.as_deref().unwrap_or(&workspace.id),
                        machine,
                    ));
                }
                FarAction {
                    method: Method::PaneCloseIfTerminal(PaneCloseIfTerminalParams {
                        pane_id: aimed.far_pane.clone(),
                        force: params.force,
                        expect: aimed.expect.clone(),
                    }),
                    aimed: Some(aimed),
                }
            }
            Method::WorkspaceRename(params) => FarAction {
                method: Method::WorkspaceRename(WorkspaceRenameParams {
                    workspace_id: far_workspace(Some(&params.workspace_id))?.1,
                    label: params.label.clone(),
                }),
                aimed: None,
            },
            Method::TabRename(params) => {
                let (ws, tab) = self
                    .parse_tab_id(&params.tab_id)
                    .ok_or_else(|| unknown("tab", &params.tab_id))?;
                // M8: the far herdr's own id of the tab, as its document
                // named it, never one this view derives.
                let tab_id = self
                    .public_tab_id(ws, tab)
                    .and_then(|tab_id| view.far_tab(&tab_id))
                    .map(str::to_string)
                    .ok_or_else(|| unknown("tab", &params.tab_id))?;
                FarAction {
                    method: Method::TabRename(TabRenameParams {
                        tab_id,
                        label: params.label.clone(),
                    }),
                    aimed: None,
                }
            }
            Method::PaneRename(params) => {
                let (_, aimed) = far_pane_id(&params.pane_id)?;
                FarAction {
                    method: Method::PaneRenameIfTerminal(PaneRenameIfTerminalParams {
                        pane_id: aimed.far_pane.clone(),
                        label: params.label.clone(),
                        expect: aimed.expect.clone(),
                    }),
                    aimed: Some(aimed),
                }
            }
            other => {
                return Err(format!(
                    "{} is not an action a view sends to {machine}",
                    other.operation().name
                ))
            }
        })
    }

    /// Sends the structural action `request` of a view to the far herdr,
    /// off the loop (D7, E.3), and answers `respond_to` with the far reply
    /// or why there is none, within `VIEW_ACTION_BUDGET` of now (review
    /// M3). Never dials: a machine that is not connected refuses at once.
    /// `sender` is the sending client: its own tab (review L4), its request
    /// id, taken once (M5), and what its window showed (fork issue 317 M2).
    pub(crate) fn forward_view_action(
        &mut self,
        request: Request,
        respond_to: std::sync::mpsc::Sender<String>,
        sender: ViewSender,
    ) {
        let deadline = Instant::now() + VIEW_ACTION_BUDGET;
        let id = request.id.clone();
        let client = sender.client;
        let method_name = request.method.operation().name;
        let Some((machine, route, close_asked, fresh)) = self.view.as_mut().map(|view| {
            let fresh = view.seen_actions.take(client, &id);
            (
                view.machine.clone(),
                view.route(),
                view.close_asked.clone(),
                fresh,
            )
        }) else {
            let _ = respond_to.send(error_response(id, NOT_DONE, "this is not a view"));
            return;
        };
        if !fresh {
            let _ = respond_to.send(error_response(
                id.clone(),
                NOT_DONE,
                format!("request {id} was taken before; nothing was sent to {machine}"),
            ));
            return;
        }
        let action = match self
            .view_far_action(&request.method, &sender)
            .and_then(|action| self.confirmed_close(action, &sender))
        {
            Ok(action) => action,
            Err(why) => {
                let _ =
                    respond_to.send(error_response(id, NOT_DONE, crate::remote::far_text(&why)));
                return;
            }
        };
        let closing = match &action.method {
            Method::PaneCloseIfTerminal(params) => Some(params.force),
            _ => None,
        };
        let far_method = action.method.operation().name;
        let line = match far_request_line(action.method) {
            Ok(line) => line,
            Err(why) => {
                let _ = respond_to.send(error_response(id, NOT_DONE, why));
                return;
            }
        };
        let aimed = action.aimed;
        let thread_respond = respond_to.clone();
        let thread_id = id.clone();
        let (confirms, asked_request) = (sender.confirms, sender.request);
        let spawned = std::thread::Builder::new()
            .name("herdr-view-action".into())
            .spawn(move || {
                let check: AimCheck = {
                    let (route, machine, aimed) = (route.clone(), machine.clone(), aimed.clone());
                    Box::new(move || {
                        still_aimed(&route, &machine, aimed.as_ref(), closing.is_some())
                    })
                };
                // M4: the person's confirmation is spent where the request
                // is sent, once: a timeout, a lease that could not be
                // written or a run too short to send leave it for the next
                // try (E.3 review, item 9; issue 316 review, item 2).
                let spend: Commit = {
                    let (close_asked, machine, aimed) =
                        (close_asked.clone(), machine.clone(), aimed.clone());
                    Box::new(move || match (closing, aimed.as_ref()) {
                        (Some(true), Some(aimed))
                            if !close_asked.spend(
                                client,
                                aimed,
                                Instant::now(),
                                confirms.as_deref(),
                            ) =>
                        {
                            Err(format!(
                                "the close you confirmed of pane {} on {machine} was sent already or waited over {} s; close it again",
                                aimed.far_pane,
                                CONFIRM_FOR.as_secs()
                            ))
                        }
                        _ => Ok(()),
                    })
                };
                let reply = crate::core_machine::fleet::view::send(
                    &route, &machine, &line, deadline, check, spend,
                );
                // M4: a confirmation is minted only when the far herdr asks
                // for one, for the pane and program the close aimed at.
                if let (Ok(reply), Some(false), Some(aimed)) = (&reply, closing, aimed.as_ref()) {
                    if far_error_code(reply).as_deref() == Some(CONFIRMATION_REQUIRED) {
                        close_asked.mint(client, aimed, Instant::now(), &asked_request);
                    }
                }
                let _ = thread_respond.send(client_reply(
                    &machine,
                    method_name,
                    far_method,
                    thread_id,
                    reply,
                ));
            });
        if let Err(err) = spawned {
            let _ = respond_to.send(error_response(
                id,
                NOT_DONE,
                format!("could not start the action: {err}; nothing was sent"),
            ));
        }
    }

    /// A forced close (the person confirmed what the far herdr said would
    /// be lost) goes only to the far pane, terminal and program whose
    /// confirmation the far herdr asked for, within `CONFIRM_FOR` (review
    /// M2, M4), and only as the answer to that request (fork issue 317 M3).
    /// The confirmation is spent just before the request.
    fn confirmed_close(
        &mut self,
        action: FarAction,
        sender: &ViewSender,
    ) -> Result<FarAction, String> {
        let (Method::PaneCloseIfTerminal(params), Some(aimed)) = (&action.method, &action.aimed)
        else {
            return Ok(action);
        };
        let Some(view) = self.view.as_ref() else {
            return Ok(action);
        };
        if !params.force
            || view.close_asked.was_asked(
                sender.client,
                aimed,
                Instant::now(),
                sender.confirms.as_deref(),
            )
        {
            return Ok(action);
        }
        Err(format!(
            "the close you confirmed was for another pane or program than {} of {} shows now; close it again",
            aimed.far_pane, view.machine
        ))
    }
}

/// Whether the far pane id `far` (`w3:p4`, or `w3:x12` for a pane the far
/// herdr has no number for) is one the far API can be asked about.
fn far_numbered(far: &str) -> bool {
    far.rsplit_once(':')
        .is_some_and(|(_, pane)| pane.starts_with('p'))
}

/// Why a view does not close the last pane of the far workspace
/// `workspace` (security review of E.3, M2 P2): the far herdr would close
/// the workspace with it, and a worktree group's parent workspace closes
/// its whole group, sibling workspaces' agents included.
fn last_pane_refusal(workspace: &str, machine: &MachineName) -> String {
    format!(
        "closing the last pane of {workspace} on {machine} is not done from a view; close it there"
    )
}

/// Just before the request (review M2, E.3 review item 1): the far
/// document still shows the aimed-at terminal under the aimed-at far pane,
/// and core still has the program the pane attached to at that terminal
/// (a far core reset gives an address to another program, which the
/// document cannot tell). For a close (`closing`), the pane is still not
/// the last of its far workspace. Nothing to check for an action that
/// names no pane.
fn still_aimed(
    route: &ViewRoute,
    machine: &MachineName,
    aimed: Option<&Aimed>,
    closing: bool,
) -> Result<(), String> {
    let Some(aimed) = aimed else {
        return Ok(());
    };
    let far_pane = &aimed.far_pane;
    // Only the far core's own terminal goes into a core address (security
    // delta re-check of PR 100, item 2), and only a program known in full is
    // one to check (item 1).
    if !crate::view::doc::is_local_terminal(&aimed.terminal) || !aimed.instance.is_known() {
        return Err(format!(
            "pane {far_pane} of {machine} is not bound to a program the view can name"
        ));
    }
    let document = route.far_layout(machine).map_err(|err| {
        format!("could not read {machine}'s layout to check pane {far_pane}: {err}")
    })?;
    let layout = crate::view::doc::parse(&document).map_err(|err| {
        format!("{machine}'s layout does not read ({err}), so pane {far_pane} could not be checked")
    })?;
    match layout.terminal_of(far_pane) {
        Some(terminal) if terminal == aimed.terminal => {}
        Some(_) => {
            return Err(format!(
                "pane {far_pane} of {machine} shows another terminal now than the one you aimed at"
            ))
        }
        None => {
            return Err(format!(
                "pane {far_pane} is no longer in {machine}'s layout"
            ))
        }
    }
    if closing {
        if let Some((space, 0..=1)) = layout.space_of(far_pane) {
            let name = if space.name.is_empty() {
                &space.id
            } else {
                &space.name
            };
            return Err(last_pane_refusal(name, machine));
        }
    }
    let address = format!("{machine}/{}", aimed.terminal);
    let record = route.far_terminal(&address).map_err(|err| {
        format!("could not ask Herdr Core which program pane {far_pane} of {machine} runs: {err}")
    })?;
    if record["state"] != "live" || !aimed.instance.matches(&record) {
        return Err(format!(
            "pane {far_pane} of {machine} runs another program now than the one you aimed at (its Herdr Core was reset, or the program ended)"
        ));
    }
    Ok(())
}

/// One far API request line for `method`, with an id of its own.
fn far_request_line(method: Method) -> Result<Vec<u8>, String> {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let request = Request {
        id: format!("view:{}", NEXT.fetch_add(1, Ordering::Relaxed)),
        method,
    };
    let mut line = serde_json::to_vec(&request)
        .map_err(|err| format!("could not encode the action: {err}; nothing was sent"))?;
    line.push(b'\n');
    Ok(line)
}

fn error_response(id: String, code: &str, message: impl Into<String>) -> String {
    serde_json::to_string(&ErrorResponse {
        id,
        error: ErrorBody {
            code: code.into(),
            message: message.into(),
        },
    })
    .unwrap_or_default()
}

/// The longest far message the client is shown, in characters.
const FAR_MESSAGE_CHARS: usize = 300;

/// A far error's text as the client shows it: escaped (security review of
/// PR E, M1), without a parser's list of the methods it takes, and short.
fn far_message(text: &str) -> String {
    let text = text.split(", expected one of").next().unwrap_or_default();
    let text = crate::remote::far_text(text);
    if text.chars().count() <= FAR_MESSAGE_CHARS {
        return text;
    }
    let mut short: String = text.chars().take(FAR_MESSAGE_CHARS).collect();
    short.push('…');
    short
}

/// Whether a far error says the far herdr does not know `method_name`: an
/// older herdr's API answers an unknown method with `invalid_request` and
/// serde's "unknown variant `<method>`, expected one of ..." (review M1),
/// a retired one with `unknown_method`.
fn missing_verb(code: &str, message: &str, method_name: &str) -> bool {
    code == "unknown_method"
        || (code == "invalid_request"
            && message.contains(&format!("unknown variant `{method_name}`")))
}

/// The far error code of `reply`, if it is an API error.
fn far_error_code(reply: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(reply).ok()?;
    value["error"]["code"].as_str().map(str::to_string)
}

/// The client's answer to a far action: the far result (one this build
/// reads, else a plain `ok`), the far error, or why there is no reply. A
/// far herdr that does not know the method sent there (`far_method`, an
/// older build) is a missing verb: that action is refused with a notice,
/// nothing else changes (D7); for a guarded pane action, the far herdr
/// cannot check the program, so nothing was done (fork issue 316, M1).
/// A far error passes its code on only when the client's handling of it is
/// known (`PASSED_ON`); a reply that is neither a result nor an error, or
/// an error that is no object, may have run there (review L1, L2).
fn client_reply(
    machine: &MachineName,
    method_name: &str,
    far_method: &str,
    id: String,
    reply: Result<Vec<u8>, String>,
) -> String {
    let reply = match reply {
        Ok(reply) => reply,
        Err(why) => return error_response(id, NOT_DONE, crate::remote::far_text(&why)),
    };
    let may_have_run = |what: &str| {
        error_response(
            id.clone(),
            NOT_DONE,
            format!("{machine}'s herdr answered {what} to {method_name}; it may have run there"),
        )
    };
    let Ok(serde_json::Value::Object(value)) = serde_json::from_slice::<serde_json::Value>(&reply)
    else {
        return may_have_run("something that is no API reply");
    };
    if let Some(error) = value.get("error") {
        let Some(error) = error.as_object() else {
            return may_have_run("an error that is no error object");
        };
        let code = error
            .get("code")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let message = error
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if missing_verb(code, message, far_method) {
            let what = if far_method == method_name {
                format!("{machine}'s herdr does not take {method_name}")
            } else {
                format!("{machine}'s herdr cannot check which program a pane runs before {method_name}, so nothing was done")
            };
            return error_response(
                id,
                "unknown_method",
                format!("{what}; update Herdr there to do this from a view"),
            );
        }
        if PASSED_ON.contains(&code) {
            return error_response(id, code, format!("{machine}: {}", far_message(message)));
        }
        let code = if !code.is_empty()
            && code.len() <= 64
            && code
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
        {
            format!(" ({code})")
        } else {
            String::new()
        };
        return error_response(
            id,
            NOT_DONE,
            format!(
                "{machine}'s herdr refused {method_name}{code}: {}",
                far_message(message)
            ),
        );
    }
    let Some(result) = value.get("result").filter(|result| result.is_object()) else {
        return may_have_run("neither a result nor an error");
    };
    let result =
        serde_json::from_value::<ResponseResult>(result.clone()).unwrap_or(ResponseResult::Ok {});
    serde_json::to_string(&SuccessResponse { id, result }).unwrap_or_default()
}

/// The client's answer the view gives for the far herdr's `reply` to
/// `method_name`, for a client test of what the person then sees.
#[cfg(test)]
pub(crate) fn client_answer_for_test(
    machine: &MachineName,
    method_name: &str,
    reply: &[u8],
) -> String {
    client_reply(
        machine,
        method_name,
        method_name,
        "endpoint:test".into(),
        Ok(reply.to_vec()),
    )
}

#[cfg(test)]
mod tests;
