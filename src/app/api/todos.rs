//! `todo.*` handlers: the server-owned view of the per-pane todo store.
//!
//! Every mutating handler marks the session dirty (that is what schedules the
//! snapshot save, so without it todos never persist) and emits `todo.changed`
//! so external consumers can follow the same state.

use super::responses::{encode_error, encode_success};
use crate::api::schema::{
    ResponseResult, TodoAddParams, TodoClearParams, TodoInfo, TodoListParams, TodoRemoveParams,
    TodoUpdateParams,
};
use crate::app::App;
use crate::terminal::todo::{PaneTodo, TodoLink, TodoUpdate};
use crate::terminal::TerminalId;

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn pane_not_found(id: String, pane_id: &str) -> String {
    encode_error(id, "pane_not_found", format!("pane {pane_id} not found"))
}

fn link_unresolved(id: String, target: &str) -> String {
    encode_error(
        id,
        "todo_link_unresolved",
        format!("link target {target} did not resolve to a single pane"),
    )
}

impl App {
    /// Resolves a pane the todo handlers can write to, returning the public id
    /// (for the response and the event) alongside the owning terminal.
    fn resolve_todo_target(&self, raw: &str) -> Option<(String, TerminalId)> {
        let (ws_idx, pane_id) = self.parse_pane_id(raw)?;
        let public_pane_id = self.public_pane_id(ws_idx, pane_id)?;
        let terminal_id = self
            .state
            .workspaces
            .get(ws_idx)
            .and_then(|ws| ws.terminal_id(pane_id))
            .cloned()?;
        Some((public_pane_id, terminal_id))
    }

    fn resolve_link(&self, raw: &str) -> Option<TodoLink> {
        let (ws_idx, pane_id) = self.parse_pane_id(raw)?;
        // Captured once and read back long afterwards, so this prefers a
        // stable name over the pane's live title: the user's own label, then
        // the agent, then the command the pane launched. Without that third
        // step a plain shell falls straight through to its identifier, which
        // locates the pane without naming it — `w1:p3` rather than `npm`.
        let label = self
            .pane_info(ws_idx, pane_id)
            .and_then(|pane| pane.label.or(pane.agent))
            .or_else(|| {
                crate::app::actions::launch_label(
                    self.state
                        .pane_terminal(pane_id)
                        .and_then(|terminal| terminal.launch_argv.as_ref()),
                )
            })
            .unwrap_or_else(|| raw.to_string());
        Some(TodoLink {
            pane: Some(pane_id),
            label,
        })
    }

    /// A link is alive while its target pane still resolves to a public id.
    pub(super) fn todo_info(&self, pane_id: &str, todo: &PaneTodo) -> TodoInfo {
        let link_pane_id = todo
            .link
            .as_ref()
            .and_then(|link| link.pane)
            .and_then(|linked| {
                self.find_pane(linked)
                    .and_then(|(ws_idx, _)| self.public_pane_id(ws_idx, linked))
            });
        TodoInfo {
            pane_id: pane_id.to_string(),
            id: todo.id,
            text: todo.text.clone(),
            done: todo.done,
            priority: todo.priority,
            link_label: todo.link.as_ref().map(|link| link.label.clone()),
            link_alive: link_pane_id.is_some(),
            link_pane_id,
            created_at_unix: todo.created_at_unix,
            updated_at_unix: todo.updated_at_unix,
        }
    }

    fn emit_todo_changed(&mut self, pane_id: &str) {
        self.emit_event(crate::api::schema::EventEnvelope {
            event: crate::api::schema::EventKind::TodoChanged,
            data: crate::api::schema::EventData::TodoChanged {
                pane_id: pane_id.to_string(),
            },
        });
    }

    /// Marks the snapshot stale — urgently, so the todo reaches disk on the
    /// next loop pass instead of after the save debounce — and tells
    /// subscribers which pane changed. Every mutating handler ends here.
    fn after_todo_mutation(&mut self, pane_id: &str) {
        self.state.mark_session_dirty_urgent();
        self.emit_todo_changed(pane_id);
    }

    pub(super) fn handle_todo_list(&mut self, id: String, params: TodoListParams) -> String {
        let todos = match params.pane_id.as_deref() {
            Some(raw) => {
                let Some((public_pane_id, terminal_id)) = self.resolve_todo_target(raw) else {
                    return pane_not_found(id, raw);
                };
                let Some(terminal) = self.state.terminals.get(&terminal_id) else {
                    return pane_not_found(id, raw);
                };
                terminal
                    .todos_in_display_order()
                    .into_iter()
                    .map(|todo| self.todo_info(&public_pane_id, todo))
                    .collect()
            }
            None => self.collect_all_todos(),
        };

        encode_success(id, ResponseResult::TodoList { todos })
    }

    /// Every pane's todos in workspace, tab, then layout order, each entry
    /// naming its own pane so a whole-session listing stays unambiguous.
    fn collect_all_todos(&self) -> Vec<TodoInfo> {
        let mut todos = Vec::new();
        for (ws_idx, ws) in self.state.workspaces.iter().enumerate() {
            for pane_id in ws.tabs.iter().flat_map(|tab| tab.layout.pane_ids()) {
                let Some(public_pane_id) = self.public_pane_id(ws_idx, pane_id) else {
                    continue;
                };
                let Some(terminal) = ws
                    .terminal_id(pane_id)
                    .and_then(|terminal_id| self.state.terminals.get(terminal_id))
                else {
                    continue;
                };
                todos.extend(
                    terminal
                        .todos_in_display_order()
                        .into_iter()
                        .map(|todo| self.todo_info(&public_pane_id, todo)),
                );
            }
        }
        todos
    }

    pub(super) fn handle_todo_add(&mut self, id: String, params: TodoAddParams) -> String {
        let Some((public_pane_id, terminal_id)) = self.resolve_todo_target(&params.pane_id) else {
            return pane_not_found(id, &params.pane_id);
        };
        let link = match params.link_pane_id.as_deref() {
            Some(raw) => match self.resolve_link(raw) {
                Some(link) => Some(link),
                None => return link_unresolved(id, raw),
            },
            None => None,
        };
        let Some(terminal) = self.state.terminals.get_mut(&terminal_id) else {
            return pane_not_found(id, &params.pane_id);
        };
        let todo = match terminal.add_todo(
            &params.text,
            params.priority.unwrap_or_default(),
            link,
            now_unix(),
        ) {
            Ok(todo) => todo,
            Err(err) => return encode_error(id, err.code(), err.message()),
        };

        self.after_todo_mutation(&public_pane_id);
        let todo = self.todo_info(&public_pane_id, &todo);
        encode_success(id, ResponseResult::Todo { todo })
    }

    pub(super) fn handle_todo_update(&mut self, id: String, params: TodoUpdateParams) -> String {
        let Some((public_pane_id, terminal_id)) = self.resolve_todo_target(&params.pane_id) else {
            return pane_not_found(id, &params.pane_id);
        };
        // `clear_link` wins over `link_pane_id`: the CLI exposes them as
        // mutually exclusive flags, and dropping the link is the safer reading.
        let link = if params.clear_link {
            Some(None)
        } else {
            match params.link_pane_id.as_deref() {
                Some(raw) => match self.resolve_link(raw) {
                    Some(link) => Some(Some(link)),
                    None => return link_unresolved(id, raw),
                },
                None => None,
            }
        };
        let Some(terminal) = self.state.terminals.get_mut(&terminal_id) else {
            return pane_not_found(id, &params.pane_id);
        };
        let todo = match terminal.update_todo(
            params.id,
            TodoUpdate {
                text: params.text,
                done: params.done,
                priority: params.priority,
                link,
            },
            now_unix(),
        ) {
            Ok(todo) => todo,
            Err(err) => return encode_error(id, err.code(), err.message()),
        };

        self.after_todo_mutation(&public_pane_id);
        let todo = self.todo_info(&public_pane_id, &todo);
        encode_success(id, ResponseResult::Todo { todo })
    }

    pub(super) fn handle_todo_remove(&mut self, id: String, params: TodoRemoveParams) -> String {
        let Some((public_pane_id, terminal_id)) = self.resolve_todo_target(&params.pane_id) else {
            return pane_not_found(id, &params.pane_id);
        };
        let Some(terminal) = self.state.terminals.get_mut(&terminal_id) else {
            return pane_not_found(id, &params.pane_id);
        };
        if let Err(err) = terminal.remove_todo(params.id) {
            return encode_error(id, err.code(), err.message());
        }

        self.after_todo_mutation(&public_pane_id);
        encode_success(id, ResponseResult::Ok {})
    }

    pub(super) fn handle_todo_clear(&mut self, id: String, params: TodoClearParams) -> String {
        let Some((public_pane_id, terminal_id)) = self.resolve_todo_target(&params.pane_id) else {
            return pane_not_found(id, &params.pane_id);
        };
        let Some(terminal) = self.state.terminals.get_mut(&terminal_id) else {
            return pane_not_found(id, &params.pane_id);
        };
        let removed = terminal.clear_todos(params.done_only);

        // A clear that removed nothing changed nothing: no save, no event.
        if removed > 0 {
            self.after_todo_mutation(&public_pane_id);
        }
        encode_success(
            id,
            ResponseResult::TodoCleared {
                removed: removed as u32,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::api::schema::{
        Method, Request, TodoAddParams, TodoClearParams, TodoListParams, TodoRemoveParams,
        TodoUpdateParams,
    };
    use crate::app::App;
    use crate::config::Config;
    use crate::terminal::todo::TodoPriority;
    use crate::workspace::Workspace;

    /// Same shape as `app_with_test_workspace()` in `src/app/api/panes.rs`
    /// (that one is private to its own test module).
    fn test_app_with_pane() -> (App, String) {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.workspaces = vec![Workspace::test_new("todos")];
        app.state.ensure_test_terminals();
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let public_pane_id = app.public_pane_id(0, pane_id).unwrap();
        (app, public_pane_id)
    }

    /// `handle_api_request` returns the encoded JSON response. Parsing to
    /// `Value` keeps success and error assertions in one shape.
    fn request_json(app: &mut App, method: Method) -> serde_json::Value {
        let raw = app.handle_api_request(Request {
            id: "test".into(),
            method,
        });
        serde_json::from_str(&raw).unwrap()
    }

    #[test]
    fn todo_add_creates_a_todo_on_the_named_pane() {
        let (mut app, pane_id) = test_app_with_pane();

        let response = request_json(
            &mut app,
            Method::TodoAdd(TodoAddParams {
                pane_id: pane_id.clone(),
                text: "fix the flaky test".into(),
                priority: Some(TodoPriority::High),
                link_pane_id: None,
            }),
        );

        let todo = &response["result"]["todo"];
        assert_eq!(todo["text"], "fix the flaky test");
        assert_eq!(todo["priority"], "high");
        assert_eq!(todo["done"], false);
        assert_eq!(todo["pane_id"], pane_id);
    }

    #[test]
    fn todo_add_rejects_an_unknown_pane() {
        let (mut app, _pane_id) = test_app_with_pane();

        let response = request_json(
            &mut app,
            Method::TodoAdd(TodoAddParams {
                pane_id: "w9:p9".into(),
                text: "nope".into(),
                priority: None,
                link_pane_id: None,
            }),
        );

        assert_eq!(response["error"]["code"], "pane_not_found");
    }

    #[test]
    fn todo_add_surfaces_store_errors_as_codes() {
        let (mut app, pane_id) = test_app_with_pane();

        let response = request_json(
            &mut app,
            Method::TodoAdd(TodoAddParams {
                pane_id: pane_id.clone(),
                text: "   ".into(),
                priority: None,
                link_pane_id: None,
            }),
        );

        assert_eq!(response["error"]["code"], "todo_text_empty");
    }

    #[test]
    fn todo_add_rejects_an_unresolvable_link() {
        let (mut app, pane_id) = test_app_with_pane();

        let response = request_json(
            &mut app,
            Method::TodoAdd(TodoAddParams {
                pane_id: pane_id.clone(),
                text: "rerun deploy".into(),
                priority: None,
                link_pane_id: Some("w9:p9".into()),
            }),
        );

        assert_eq!(response["error"]["code"], "todo_link_unresolved");
    }

    #[test]
    fn todo_list_returns_display_order_for_one_pane() {
        let (mut app, pane_id) = test_app_with_pane();
        for (text, priority) in [
            ("normal", TodoPriority::Normal),
            ("high", TodoPriority::High),
        ] {
            request_json(
                &mut app,
                Method::TodoAdd(TodoAddParams {
                    pane_id: pane_id.clone(),
                    text: text.into(),
                    priority: Some(priority),
                    link_pane_id: None,
                }),
            );
        }

        let response = request_json(
            &mut app,
            Method::TodoList(TodoListParams {
                pane_id: Some(pane_id.clone()),
            }),
        );

        let todos = response["result"]["todos"].as_array().unwrap();
        assert_eq!(todos.len(), 2);
        assert_eq!(todos[0]["text"], "high", "priority orders the list");
    }

    #[test]
    fn todo_list_without_a_pane_returns_every_pane() {
        let (mut app, pane_id) = test_app_with_pane();
        request_json(
            &mut app,
            Method::TodoAdd(TodoAddParams {
                pane_id: pane_id.clone(),
                text: "only one".into(),
                priority: None,
                link_pane_id: None,
            }),
        );

        let response = request_json(&mut app, Method::TodoList(TodoListParams { pane_id: None }));

        let todos = response["result"]["todos"].as_array().unwrap();
        assert_eq!(todos.len(), 1);
        assert_eq!(
            todos[0]["pane_id"], pane_id,
            "each entry identifies its pane"
        );
    }

    #[test]
    fn todo_update_changes_done_state() {
        let (mut app, pane_id) = test_app_with_pane();
        let created = request_json(
            &mut app,
            Method::TodoAdd(TodoAddParams {
                pane_id: pane_id.clone(),
                text: "check me off".into(),
                priority: None,
                link_pane_id: None,
            }),
        );
        let todo_id = created["result"]["todo"]["id"].as_u64().unwrap();

        let response = request_json(
            &mut app,
            Method::TodoUpdate(TodoUpdateParams {
                pane_id: pane_id.clone(),
                id: todo_id,
                text: None,
                done: Some(true),
                priority: None,
                link_pane_id: None,
                clear_link: false,
            }),
        );

        assert_eq!(response["result"]["todo"]["done"], true);
    }

    #[test]
    fn todo_update_reports_a_missing_todo() {
        let (mut app, pane_id) = test_app_with_pane();

        let response = request_json(
            &mut app,
            Method::TodoUpdate(TodoUpdateParams {
                pane_id: pane_id.clone(),
                id: 999,
                text: None,
                done: Some(true),
                priority: None,
                link_pane_id: None,
                clear_link: false,
            }),
        );

        assert_eq!(response["error"]["code"], "todo_not_found");
    }

    #[test]
    fn todo_remove_and_clear_report_counts() {
        let (mut app, pane_id) = test_app_with_pane();
        let created = request_json(
            &mut app,
            Method::TodoAdd(TodoAddParams {
                pane_id: pane_id.clone(),
                text: "remove me".into(),
                priority: None,
                link_pane_id: None,
            }),
        );
        let todo_id = created["result"]["todo"]["id"].as_u64().unwrap();

        let removed = request_json(
            &mut app,
            Method::TodoRemove(TodoRemoveParams {
                pane_id: pane_id.clone(),
                id: todo_id,
            }),
        );
        assert!(removed["result"].is_object());

        request_json(
            &mut app,
            Method::TodoAdd(TodoAddParams {
                pane_id: pane_id.clone(),
                text: "clear me".into(),
                priority: None,
                link_pane_id: None,
            }),
        );
        let cleared = request_json(
            &mut app,
            Method::TodoClear(TodoClearParams {
                pane_id: pane_id.clone(),
                done_only: false,
            }),
        );

        assert_eq!(cleared["result"]["removed"], 1);
    }

    /// Spec: every mutating call emits `todo.changed` naming the affected pane,
    /// and every mutating call marks the session dirty so todos actually persist.
    #[test]
    fn every_mutation_emits_todo_changed_and_marks_the_session_dirty() {
        let (mut app, pane_id) = test_app_with_pane();
        let created = request_json(
            &mut app,
            Method::TodoAdd(TodoAddParams {
                pane_id: pane_id.clone(),
                text: "watch me".into(),
                priority: None,
                link_pane_id: None,
            }),
        );
        let todo_id = created["result"]["todo"]["id"].as_u64().unwrap();

        let mutations = vec![
            Method::TodoUpdate(TodoUpdateParams {
                pane_id: pane_id.clone(),
                id: todo_id,
                text: None,
                done: Some(true),
                priority: None,
                link_pane_id: None,
                clear_link: false,
            }),
            Method::TodoRemove(TodoRemoveParams {
                pane_id: pane_id.clone(),
                id: todo_id,
            }),
            Method::TodoAdd(TodoAddParams {
                pane_id: pane_id.clone(),
                text: "and me".into(),
                priority: None,
                link_pane_id: None,
            }),
            Method::TodoClear(TodoClearParams {
                pane_id: pane_id.clone(),
                done_only: false,
            }),
        ];

        for method in mutations {
            let before = app.event_hub.current_sequence();
            app.state.session_dirty = false;
            app.state.session_save_urgent = false;

            let response = request_json(&mut app, method);
            assert!(
                response["result"].is_object(),
                "mutation failed: {response:?}"
            );
            assert!(
                app.state.session_dirty,
                "mutation must mark the session dirty or todos never persist: {response:?}"
            );
            assert!(
                app.state.session_save_urgent,
                "todo mutations must skip the save debounce: {response:?}"
            );

            let emitted = app
                .event_hub
                .events_after(before)
                .into_iter()
                .any(|(_, event)| {
                    matches!(
                        event.data,
                        crate::api::schema::EventData::TodoChanged { pane_id: ref emitted }
                            if *emitted == pane_id
                    ) && event.event == crate::api::schema::EventKind::TodoChanged
                });
            assert!(emitted, "expected todo.changed for {pane_id}");
        }
    }

    #[test]
    fn a_read_only_list_neither_emits_nor_dirties() {
        let (mut app, pane_id) = test_app_with_pane();
        app.state.session_dirty = false;
        let before = app.event_hub.current_sequence();

        request_json(
            &mut app,
            Method::TodoList(TodoListParams {
                pane_id: Some(pane_id),
            }),
        );

        assert!(!app.state.session_dirty);
        assert!(app.event_hub.events_after(before).is_empty());
    }

    #[test]
    fn todo_add_records_a_resolvable_link() {
        let (mut app, pane_id) = test_app_with_pane();

        let response = request_json(
            &mut app,
            Method::TodoAdd(TodoAddParams {
                pane_id: pane_id.clone(),
                text: "look over there".into(),
                priority: None,
                link_pane_id: Some(pane_id.clone()),
            }),
        );

        let todo = &response["result"]["todo"];
        assert_eq!(todo["link_pane_id"], pane_id);
        assert_eq!(todo["link_alive"], true);
        assert!(todo["link_label"].is_string());
    }

    /// The target pane's terminal, so a test can give it something to be
    /// named by.
    fn target_terminal(app: &mut App) -> &mut crate::terminal::TerminalState {
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.state.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&terminal_id)
            .expect("test terminal should exist")
    }

    fn link_label(app: &mut App, pane_id: &str) -> String {
        let response = request_json(
            app,
            Method::TodoAdd(TodoAddParams {
                pane_id: pane_id.to_string(),
                text: "look over there".into(),
                priority: None,
                link_pane_id: Some(pane_id.to_string()),
            }),
        );
        response["result"]["todo"]["link_label"]
            .as_str()
            .expect("a link always carries a label")
            .to_string()
    }

    /// A pane running a plain shell has neither a manual label nor an agent,
    /// so without the launched-command step the stored label is the pane's own
    /// identifier — it locates the target without naming it, which is the
    /// thing that made links unreadable.
    #[test]
    fn todo_link_to_a_shell_pane_captures_its_launched_command() {
        let (mut app, pane_id) = test_app_with_pane();
        target_terminal(&mut app).launch_argv = Some(vec!["/bin/zsh".into()]);

        assert_eq!(link_label(&mut app, &pane_id), "zsh");
    }

    #[test]
    fn todo_link_prefers_a_manual_label_over_the_launched_command() {
        let (mut app, pane_id) = test_app_with_pane();
        let terminal = target_terminal(&mut app);
        terminal.launch_argv = Some(vec!["/bin/zsh".into()]);
        terminal.set_manual_label("reviewer".into());

        assert_eq!(link_label(&mut app, &pane_id), "reviewer");
    }

    #[test]
    fn todo_link_prefers_the_agent_over_the_launched_command() {
        let (mut app, pane_id) = test_app_with_pane();
        let terminal = target_terminal(&mut app);
        terminal.launch_argv = Some(vec!["/bin/zsh".into()]);
        terminal.set_detected_state(
            Some(crate::detect::Agent::Claude),
            crate::detect::AgentState::Idle,
        );

        assert_eq!(link_label(&mut app, &pane_id), "claude");
    }

    /// A target offering no name at all still has to carry something, and the
    /// identifier is the only thing left.
    #[test]
    fn todo_link_falls_back_to_the_identifier_when_the_pane_offers_no_name() {
        let (mut app, pane_id) = test_app_with_pane();
        assert!(target_terminal(&mut app).launch_argv.is_none());

        assert_eq!(link_label(&mut app, &pane_id), pane_id);
    }

    #[test]
    fn todo_update_clears_the_link() {
        let (mut app, pane_id) = test_app_with_pane();
        let created = request_json(
            &mut app,
            Method::TodoAdd(TodoAddParams {
                pane_id: pane_id.clone(),
                text: "unlink me".into(),
                priority: None,
                link_pane_id: Some(pane_id.clone()),
            }),
        );
        let todo_id = created["result"]["todo"]["id"].as_u64().unwrap();

        let response = request_json(
            &mut app,
            Method::TodoUpdate(TodoUpdateParams {
                pane_id: pane_id.clone(),
                id: todo_id,
                text: None,
                done: None,
                priority: None,
                link_pane_id: None,
                clear_link: true,
            }),
        );

        let todo = &response["result"]["todo"];
        assert!(todo["link_pane_id"].is_null());
        assert_eq!(todo["link_alive"], false);
    }

    #[test]
    fn todo_clear_done_only_keeps_outstanding_todos() {
        let (mut app, pane_id) = test_app_with_pane();
        let created = request_json(
            &mut app,
            Method::TodoAdd(TodoAddParams {
                pane_id: pane_id.clone(),
                text: "done one".into(),
                priority: None,
                link_pane_id: None,
            }),
        );
        let todo_id = created["result"]["todo"]["id"].as_u64().unwrap();
        request_json(
            &mut app,
            Method::TodoAdd(TodoAddParams {
                pane_id: pane_id.clone(),
                text: "still open".into(),
                priority: None,
                link_pane_id: None,
            }),
        );
        request_json(
            &mut app,
            Method::TodoUpdate(TodoUpdateParams {
                pane_id: pane_id.clone(),
                id: todo_id,
                text: None,
                done: Some(true),
                priority: None,
                link_pane_id: None,
                clear_link: false,
            }),
        );

        let cleared = request_json(
            &mut app,
            Method::TodoClear(TodoClearParams {
                pane_id: pane_id.clone(),
                done_only: true,
            }),
        );
        assert_eq!(cleared["result"]["removed"], 1);

        let listed = request_json(
            &mut app,
            Method::TodoList(TodoListParams {
                pane_id: Some(pane_id),
            }),
        );
        let todos = listed["result"]["todos"].as_array().unwrap();
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0]["text"], "still open");
    }

    #[test]
    fn a_clear_that_removed_nothing_neither_emits_nor_dirties() {
        let (mut app, pane_id) = test_app_with_pane();
        app.state.session_dirty = false;
        let before = app.event_hub.current_sequence();

        let cleared = request_json(
            &mut app,
            Method::TodoClear(TodoClearParams {
                pane_id,
                done_only: false,
            }),
        );

        assert_eq!(cleared["result"]["removed"], 0);
        assert!(!app.state.session_dirty);
        assert!(app.event_hub.events_after(before).is_empty());
    }

    #[test]
    fn todo_list_reports_an_unknown_pane() {
        let (mut app, _pane_id) = test_app_with_pane();

        let response = request_json(
            &mut app,
            Method::TodoList(TodoListParams {
                pane_id: Some("w9:p9".into()),
            }),
        );

        assert_eq!(response["error"]["code"], "pane_not_found");
    }
}
