//! A view's structural actions (core-phase5 D7, E.3): new tab, split, close
//! pane and rename, which `view::policy` classes `Far`, are not done here.
//! The view turns the client's ids into the far herdr's through its own
//! layout (so only what the far document showed can be named), builds the
//! far API request with the client's names as JSON strings, and sends it
//! off the loop through `core_machine::fleet::view`. The far herdr changes
//! its layout and republishes; the view follows the document. The far
//! reply, or why there is none, is the client's answer: an error is a
//! notice there, and the view stays connected.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use super::App;
use crate::api::schema::{
    ErrorBody, ErrorResponse, Method, PaneCloseParams, PaneRenameParams, PaneRightClickTarget,
    PaneSplitParams, Request, ResponseResult, SuccessResponse, TabCreateParams, TabRenameParams,
    WorkspaceRenameParams,
};
use crate::core_machine::MachineName;

/// The code of an action the view could not send or the far side did not
/// answer: nothing a client retries by itself.
const NOT_DONE: &str = "view_action_failed";

impl App {
    /// The far herdr's request for the structural action `method` of a view
    /// (D7): the same method with the far ids of its targets, the client's
    /// names unchanged, and nothing of this Mac (no folder, environment,
    /// machine or focus: a far folder is the far herdr's to pick, and the
    /// far screen keeps its focus). `Err` when a target is not one the far
    /// document showed, or the method is not one a view sends there.
    pub(crate) fn view_far_method(&self, method: &Method) -> Result<Method, String> {
        let Some(view) = self.view.as_ref() else {
            return Err("this is not a view".into());
        };
        let machine = &view.machine;
        let unknown =
            |what: &str, id: &str| format!("{what} {id} is not one {machine}'s herdr shows");
        let far_workspace = |id: Option<&str>| -> Result<(usize, String), String> {
            let index = match id {
                Some(id) => self
                    .parse_workspace_id(id)
                    .ok_or_else(|| unknown("workspace", id))?,
                None => self
                    .state
                    .active
                    .ok_or_else(|| format!("no workspace of {machine} is open in this view"))?,
            };
            let ws = self
                .state
                .workspaces
                .get(index)
                .ok_or_else(|| format!("no workspace of {machine} is open in this view"))?;
            Ok((index, ws.id.clone()))
        };
        let far_pane = |id: &str| -> Result<(usize, String), String> {
            let (ws, pane) = self.parse_pane_id(id).ok_or_else(|| unknown("pane", id))?;
            let far = view.far_pane(pane).ok_or_else(|| unknown("pane", id))?;
            Ok((ws, far.to_string()))
        };
        Ok(match method {
            Method::TabCreate(params) => {
                let (_, workspace_id) = far_workspace(params.workspace_id.as_deref())?;
                Method::TabCreate(TabCreateParams {
                    workspace_id: Some(workspace_id),
                    cwd: None,
                    focus: false,
                    label: params.label.clone(),
                    env: HashMap::new(),
                    on: None,
                })
            }
            Method::PaneSplit(params) => {
                let target = match params.target_pane_id.as_deref() {
                    Some(id) => far_pane(id)?,
                    None => {
                        let (index, _) = far_workspace(params.workspace_id.as_deref())?;
                        let ws = &self.state.workspaces[index];
                        let focused = ws
                            .tabs
                            .get(ws.active_tab)
                            .map(|tab| tab.layout.focused())
                            .and_then(|pane| view.far_pane(pane))
                            .ok_or_else(|| format!("no pane of {machine} to split"))?;
                        (index, focused.to_string())
                    }
                };
                let (ws, target_pane_id) = target;
                Method::PaneSplit(PaneSplitParams {
                    workspace_id: Some(self.state.workspaces[ws].id.clone()),
                    target_pane_id: Some(target_pane_id),
                    direction: params.direction.clone(),
                    ratio: params.ratio,
                    cwd: None,
                    focus: false,
                    right_click: PaneRightClickTarget::default(),
                    env: HashMap::new(),
                    on: None,
                })
            }
            Method::PaneClose(params) => Method::PaneClose(PaneCloseParams {
                pane_id: far_pane(&params.pane_id)?.1,
                force: params.force,
            }),
            Method::WorkspaceRename(params) => Method::WorkspaceRename(WorkspaceRenameParams {
                workspace_id: far_workspace(Some(&params.workspace_id))?.1,
                label: params.label.clone(),
            }),
            Method::TabRename(params) => {
                let (ws, tab) = self
                    .parse_tab_id(&params.tab_id)
                    .ok_or_else(|| unknown("tab", &params.tab_id))?;
                let tab_id = self
                    .public_tab_id(ws, tab)
                    .ok_or_else(|| unknown("tab", &params.tab_id))?;
                Method::TabRename(TabRenameParams {
                    tab_id,
                    label: params.label.clone(),
                })
            }
            Method::PaneRename(params) => Method::PaneRename(PaneRenameParams {
                pane_id: far_pane(&params.pane_id)?.1,
                label: params.label.clone(),
            }),
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
    /// or why there is none. Never dials: a machine that is not connected
    /// refuses at once.
    pub(crate) fn forward_view_action(
        &self,
        request: Request,
        respond_to: std::sync::mpsc::Sender<String>,
    ) {
        let id = request.id.clone();
        let method_name = request.method.operation().name;
        let Some(machine) = self.view.as_ref().map(|view| view.machine.clone()) else {
            let _ = respond_to.send(error_response(id, NOT_DONE, "this is not a view"));
            return;
        };
        let line = match self
            .view_far_method(&request.method)
            .and_then(far_request_line)
        {
            Ok(line) => line,
            Err(why) => {
                let _ = respond_to.send(error_response(id, NOT_DONE, why));
                return;
            }
        };
        let thread_respond = respond_to.clone();
        let thread_id = id.clone();
        let spawned = std::thread::Builder::new()
            .name("herdr-view-action".into())
            .spawn(move || {
                let reply = crate::core_machine::fleet::view::send(&machine, &line);
                let _ = thread_respond.send(client_reply(&machine, method_name, thread_id, reply));
            });
        if let Err(err) = spawned {
            let _ = respond_to.send(error_response(
                id,
                NOT_DONE,
                format!("could not start the action: {err}; nothing was sent"),
            ));
        }
    }
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

/// The client's answer to a far action: the far result (one this build
/// reads, else a plain `ok`), the far error with its code, or why there is
/// no reply. Far text is shown escaped (security review of PR E, M1). A
/// far herdr that does not know the method (an older build) is a missing
/// verb: that action is refused with a notice, nothing else changes (D7).
fn client_reply(
    machine: &MachineName,
    method_name: &str,
    id: String,
    reply: Result<Vec<u8>, String>,
) -> String {
    let reply = match reply {
        Ok(reply) => reply,
        Err(why) => return error_response(id, NOT_DONE, crate::remote::far_text(&why)),
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&reply) else {
        return error_response(
            id,
            NOT_DONE,
            format!(
                "{machine}'s herdr answered something that is no API reply; it may have run there"
            ),
        );
    };
    if let Some(error) = value.get("error") {
        let code = error["code"].as_str().unwrap_or_default();
        let message = crate::remote::far_text(error["message"].as_str().unwrap_or_default());
        if code == "unknown_method" {
            return error_response(
                id,
                "unknown_method",
                format!(
                    "{machine}'s herdr does not take {method_name}; update Herdr there to do this from a view"
                ),
            );
        }
        // A code is a plain word the client may act on (a close's
        // `confirmation_required` asks the person, then sends it forced).
        let code = if !code.is_empty()
            && code.len() <= 64
            && code
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
        {
            code
        } else {
            NOT_DONE
        };
        return error_response(id, code, format!("{machine}: {message}"));
    }
    let result = serde_json::from_value::<ResponseResult>(value["result"].clone())
        .unwrap_or(ResponseResult::Ok {});
    serde_json::to_string(&SuccessResponse { id, result }).unwrap_or_default()
}

#[cfg(test)]
mod tests;
