//! Fork issue 276: the "Mark space read" / "Mark all read" context-menu items.

use super::*;
use crate::api::schema::Method;

fn agent(pane: &str, workspace: &str, status: &str) -> crate::protocol::ClientShellAgent {
    serde_json::from_value(serde_json::json!({
        "pane_id": pane, "workspace_id": workspace, "tab_id": "tab_1", "name": null,
        "display_agent": null, "agent": null, "title": null, "terminal_title": null,
        "terminal_title_stripped": null, "agent_status": status, "state_change_seq": 0,
        "state_labels": [], "tokens": [], "focused": false
    }))
    .unwrap()
}

/// Two spaces, `ws_1` with a working agent and `ws_2` with agents in `ws2_status`.
fn state_with(
    ws1_status: &str,
    ws2_status: &str,
    methods: Option<Vec<String>>,
) -> ClientShellState {
    let mut projected = snapshot();
    let mut second = projected.workspaces[0].clone();
    second.workspace_id = "ws_2".into();
    second.focused = false;
    projected.workspaces.push(second);
    projected.agents = vec![
        agent("w1:p1", "ws_1", ws1_status),
        agent("w2:p1", "ws_2", ws2_status),
    ];
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(projected));
    // Setting a snapshot re-projects `done` from the client's own completion
    // bookkeeping; the menu reads the projected status, so pin it directly.
    for agent in &mut state.snapshot.as_deref_mut().unwrap().agents {
        agent.agent_status =
            serde_json::from_value(serde_json::json!(if agent.workspace_id == "ws_1" {
                ws1_status
            } else {
                ws2_status
            }))
            .unwrap();
    }
    state.set_endpoint_methods(methods);
    state
}

fn advertised() -> Option<Vec<String>> {
    Some(vec!["pane.mark_seen".into()])
}

fn menu_actions(state: &ClientShellState) -> Vec<ClientContextMenuAction> {
    match state.overlay.as_ref() {
        Some(ClientShellOverlay::ContextMenu(menu)) => {
            menu.items().iter().map(|item| item.action).collect()
        }
        other => panic!("expected a context menu, got {other:?}"),
    }
}

fn menu_labels(state: &ClientShellState) -> Vec<&'static str> {
    match state.overlay.as_ref() {
        Some(ClientShellOverlay::ContextMenu(menu)) => {
            menu.items().iter().map(|item| item.label).collect()
        }
        other => panic!("expected a context menu, got {other:?}"),
    }
}

fn sent(outcome: &ClientShellInput) -> Vec<Method> {
    outcome
        .actions
        .iter()
        .filter_map(|action| match action {
            ClientShellAction::Endpoint { request, .. } => Some(request.method.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn space_menu_offers_both_items_when_its_space_is_unread() {
    let mut state = state_with("working", "done", advertised());
    state.open_workspace_context_menu("ws_2".into(), 5, 3);
    let labels = menu_labels(&state);
    assert!(
        labels.ends_with(&["Mark space read", "Mark all read"]),
        "{labels:?}"
    );
    // It renders as real rows.
    state.set_pane_surface(surface());
    state.compose(106, 24).unwrap();
    assert_eq!(state.hits.context_menu_rows.len(), labels.len());
}

#[test]
fn space_menu_offers_only_mark_all_when_another_space_is_unread() {
    let mut state = state_with("working", "done", advertised());
    state.open_workspace_context_menu("ws_1".into(), 5, 3);
    let actions = menu_actions(&state);
    assert!(!actions.contains(&ClientContextMenuAction::MarkSpaceRead));
    assert!(actions.contains(&ClientContextMenuAction::MarkAllRead));
}

#[test]
fn menus_offer_nothing_when_no_agent_is_unread() {
    let mut state = state_with("working", "idle", advertised());
    state.open_workspace_context_menu("ws_2".into(), 5, 3);
    let actions = menu_actions(&state);
    assert!(!actions.contains(&ClientContextMenuAction::MarkSpaceRead));
    assert!(!actions.contains(&ClientContextMenuAction::MarkAllRead));
    state.open_agent_context_menu("w1:p1".into(), 5, 3);
    assert!(!menu_actions(&state).contains(&ClientContextMenuAction::MarkAllRead));
}

#[test]
fn agent_menu_offers_mark_all_read_only_when_something_is_unread() {
    let mut state = state_with("working", "done", advertised());
    state.open_agent_context_menu("w1:p1".into(), 5, 3);
    assert!(menu_labels(&state).ends_with(&["Mark all read"]));
    assert!(!menu_actions(&state).contains(&ClientContextMenuAction::MarkSpaceRead));
}

#[test]
fn items_are_hidden_when_the_server_lacks_the_method() {
    for methods in [Some(vec!["pane.focus".to_owned()]), None] {
        let mut state = state_with("working", "done", methods.clone());
        state.open_workspace_context_menu("ws_2".into(), 5, 3);
        let labels = menu_labels(&state);
        assert!(
            !labels.iter().any(|label| label.contains("read")),
            "{methods:?}: {labels:?}"
        );
        state.open_agent_context_menu("w2:p1".into(), 5, 3);
        let labels = menu_labels(&state);
        assert!(
            !labels.iter().any(|label| label.contains("read")),
            "{methods:?}: {labels:?}"
        );
    }
}

#[test]
fn choosing_an_item_sends_pane_mark_seen_with_the_matching_scope() {
    let cases = [
        (ClientContextMenuAction::MarkSpaceRead, Some("ws_2")),
        (ClientContextMenuAction::MarkAllRead, None),
    ];
    for (action, workspace) in cases {
        let mut state = state_with("working", "done", advertised());
        state.open_workspace_context_menu("ws_2".into(), 5, 3);
        let index = menu_actions(&state)
            .iter()
            .position(|a| *a == action)
            .unwrap_or_else(|| panic!("{action:?} in {:?}", menu_labels(&state)));
        let mut outcome = ClientShellInput::default();
        state.activate_context_menu_item(index, &mut outcome);
        assert_eq!(
            sent(&outcome),
            vec![Method::PaneMarkSeen(
                crate::api::schema::PaneMarkSeenParams {
                    workspace_id: workspace.map(str::to_owned),
                }
            )],
            "{action:?}"
        );
    }
}

#[test]
fn agent_menu_mark_all_read_sends_the_session_scope() {
    let mut state = state_with("working", "done", advertised());
    state.open_agent_context_menu("w1:p1".into(), 5, 3);
    let index = menu_actions(&state)
        .iter()
        .position(|a| *a == ClientContextMenuAction::MarkAllRead)
        .unwrap();
    let mut outcome = ClientShellInput::default();
    state.activate_context_menu_item(index, &mut outcome);
    assert_eq!(
        sent(&outcome),
        vec![Method::PaneMarkSeen(Default::default())]
    );
}
