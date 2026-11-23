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
use std::time::Instant;

use super::App;
use crate::api::schema::{
    ErrorBody, ErrorResponse, Method, PaneCloseParams, PaneRenameParams, PaneRightClickTarget,
    PaneSplitParams, Request, ResponseResult, SuccessResponse, TabCreateParams, TabRenameParams,
    WorkspaceRenameParams,
};
use crate::core_machine::fleet::view::VIEW_ACTION_BUDGET;
use crate::core_machine::MachineName;

/// The code of an action the view could not send, the far side did not
/// answer, or refused for a reason the client has no handling of its own
/// for: nothing a client retries by itself.
const NOT_DONE: &str = "view_action_failed";

/// The far error codes passed on to the client as they are, because this
/// build knows what the client does with them: a close's
/// `confirmation_required` asks the person, then sends the close forced
/// (review L2). Every other far code becomes `NOT_DONE`, shown.
const PASSED_ON: &[&str] = &["confirmation_required"];

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
    /// or why there is none, within `VIEW_ACTION_BUDGET` of now (review
    /// M3). Never dials: a machine that is not connected refuses at once.
    pub(crate) fn forward_view_action(
        &self,
        request: Request,
        respond_to: std::sync::mpsc::Sender<String>,
    ) {
        let deadline = Instant::now() + VIEW_ACTION_BUDGET;
        let id = request.id.clone();
        let method_name = request.method.operation().name;
        let Some((machine, route)) = self
            .view
            .as_ref()
            .map(|view| (view.machine.clone(), view.route()))
        else {
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
                let reply = crate::core_machine::fleet::view::send(
                    &route,
                    &machine,
                    &line,
                    deadline,
                    &|| Ok(()),
                );
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

/// The client's answer to a far action: the far result (one this build
/// reads, else a plain `ok`), the far error, or why there is no reply. A
/// far herdr that does not know the method (an older build) is a missing
/// verb: that action is refused with a notice, nothing else changes (D7).
/// A far error passes its code on only when the client's handling of it is
/// known (`PASSED_ON`); a reply that is neither a result nor an error, or
/// an error that is no object, may have run there (review L1, L2).
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
        if missing_verb(code, message, method_name) {
            return error_response(
                id,
                "unknown_method",
                format!(
                    "{machine}'s herdr does not take {method_name}; update Herdr there to do this from a view"
                ),
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
        "endpoint:test".into(),
        Ok(reply.to_vec()),
    )
}

#[cfg(test)]
mod tests;
