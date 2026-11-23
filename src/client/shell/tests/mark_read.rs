//! Fork issue 276: the "Mark space read" / "Mark all read" context-menu items.

use super::*;
use crate::api::schema::{AgentStatus, Method};

fn agent(pane: &str, workspace: &str, status: &str) -> crate::protocol::ClientShellAgent {
    serde_json::from_value(serde_json::json!({
        "pane_id": pane, "workspace_id": workspace, "tab_id": "tab_1", "name": null,
        "display_agent": null, "agent": null, "title": null, "terminal_title": null,
        "terminal_title_stripped": null, "agent_status": status, "state_change_seq": 0,
        "state_labels": [], "tokens": [], "focused": false
    }))
    .unwrap()
}

/// Two spaces; each agent is `working`, `idle` or `done`. A `done` agent is
/// driven through the client's real completion projection: it is shown working
/// in one snapshot and idle with a newer sequence in the next.
fn state_with(
    ws1_status: &str,
    ws2_status: &str,
    methods: Option<Vec<String>>,
) -> ClientShellState {
    let build = |second_pass: bool| {
        let mut projected = snapshot();
        let mut second = projected.workspaces[0].clone();
        second.workspace_id = "ws_2".into();
        second.focused = false;
        projected.workspaces.push(second);
        let wire = |status: &str| {
            match (status, second_pass) {
                ("done", false) => "working",
                ("done", true) => "idle",
                (other, _) => other,
            }
            .to_owned()
        };
        let mut agents = vec![
            agent("w1:p1", "ws_1", &wire(ws1_status)),
            agent("w2:p1", "ws_2", &wire(ws2_status)),
        ];
        for agent in &mut agents {
            agent.state_change_seq = if second_pass { 2 } else { 1 };
        }
        projected.agents = agents;
        projected
    };
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(build(false)));
    state.set_snapshot(Box::new(build(true)));
    state.set_endpoint_methods(methods);
    state
}

fn statuses(state: &ClientShellState) -> Vec<AgentStatus> {
    state
        .snapshot
        .as_deref()
        .unwrap()
        .agents
        .iter()
        .map(|agent| agent.agent_status)
        .collect()
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

fn menu_labels(state: &ClientShellState) -> Vec<String> {
    match state.overlay.as_ref() {
        Some(ClientShellOverlay::ContextMenu(menu)) => menu
            .items()
            .iter()
            .map(|item| item.label.to_string())
            .collect(),
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
        labels.ends_with(&["Mark space read".to_string(), "Mark all read".to_string()]),
        "{labels:?}"
    );
    // It renders as real rows.
    state.set_pane_surface(surface());
    assert!(state.overlay.is_some(), "overlay survives the surface");
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
    assert!(menu_labels(&state).ends_with(&["Mark all read".to_string()]));
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

#[test]
fn choosing_an_item_acknowledges_the_in_scope_agents_on_this_client() {
    use AgentStatus::{Done, Idle};
    // Both spaces hold a finished, unread agent.
    let cases = [
        (ClientContextMenuAction::MarkSpaceRead, vec![Idle, Done]),
        (ClientContextMenuAction::MarkAllRead, vec![Idle, Idle]),
    ];
    for (action, expected) in cases {
        let mut state = state_with("done", "done", advertised());
        // Setup: both agents finished work this client has not shown yet.
        assert_eq!(statuses(&state), vec![Done, Done]);
        state.open_workspace_context_menu("ws_1".into(), 5, 3);
        // "Mark space read" is for ws_1 here; ws_2 stays unread.
        let index = menu_actions(&state)
            .iter()
            .position(|a| *a == action)
            .unwrap();
        let mut outcome = ClientShellInput::default();
        state.activate_context_menu_item(index, &mut outcome);
        assert_eq!(statuses(&state), expected, "{action:?}");
        let workspace_statuses: Vec<_> = state
            .snapshot
            .as_deref()
            .unwrap()
            .workspaces
            .iter()
            .map(|workspace| workspace.agent_status)
            .collect();
        assert_eq!(workspace_statuses, expected, "{action:?} aggregates");
        assert_eq!(sent(&outcome).len(), 1, "{action:?} still asks the server");
    }
}
