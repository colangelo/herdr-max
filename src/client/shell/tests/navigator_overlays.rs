//! The fork's navigator deltas (width and status column, search row, Esc
//! stages) and the navigator as the todo link picker, re-expressed from the
//! fork's oracles (src/ui/navigator.rs, src/app/input/modal.rs at
//! sync-snapshot/2026-10-08).

use super::super::render::{navigator_columns, navigator_status_measure};
use super::super::state_presentation::StatePresentation;
use super::super::todo_edit::TodoEditLink;
use super::*;
use crate::api::schema::{AgentStatus, Method};
use crate::input::KeybindAction;
use crate::protocol::{ClientShellAgent, ClientShellPane};

fn agent(pane_id: &str, agent: Option<&str>, status: AgentStatus) -> ClientShellAgent {
    ClientShellAgent {
        pane_id: pane_id.into(),
        workspace_id: "ws_1".into(),
        tab_id: "tab_1".into(),
        name: None,
        display_agent: None,
        agent: agent.map(str::to_owned),
        title: None,
        terminal_title: None,
        terminal_title_stripped: None,
        agent_status: status,
        state_change_seq: 0,
        state_labels: Vec::new(),
        tokens: Vec::new(),
        focused: false,
    }
}

/// The base session plus a second pane, `other`, in the same tab.
fn two_panes() -> ClientShellSnapshot {
    let mut snapshot = super::snapshot();
    snapshot.panes.push(ClientShellPane {
        pane_id: "pane_2".into(),
        workspace_id: "ws_1".into(),
        tab_id: "tab_1".into(),
        label: Some("other".into()),
        cwd: Some("/repo".into()),
        foreground_cwd: Some("/repo".into()),
        focused: false,
        right_click_passthrough: false,
    });
    snapshot
}

fn state_with(snapshot: ClientShellSnapshot, width: u16, height: u16) -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface());
    state.compose(width, height).unwrap();
    state
}

fn bind(state: &mut ClientShellState, action: KeybindAction) -> ClientShellInput {
    let mut outcome = ClientShellInput::default();
    state.record_binding(crate::input::KeybindMatch::Action(action), &mut outcome);
    outcome
}

fn navigator(state: &ClientShellState) -> &ClientNavigatorOverlay {
    match state.overlay.as_ref() {
        Some(ClientShellOverlay::Navigator(navigator)) => navigator,
        other => panic!("expected the navigator, got {other:?}"),
    }
}

fn rows(state: &ClientShellState) -> Vec<ClientNavigatorRow> {
    render::client_navigator_rows(
        &state.endpoints,
        &state.active_endpoint_id,
        navigator(state),
    )
}

fn screen(state: &mut ClientShellState, width: u16, height: u16) -> Vec<String> {
    let frame = state.compose(width, height).expect("frame");
    frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect()
}

fn edit(state: &ClientShellState) -> &super::super::todo_edit::ClientTodoEditOverlay {
    match state.overlay.as_ref() {
        Some(ClientShellOverlay::TodoEdit(edit)) => edit,
        other => panic!("expected the todo editor, got {other:?}"),
    }
}

fn row(label: &str, status: Option<AgentStatus>, status_text: &str) -> ClientNavigatorRow {
    ClientNavigatorRow {
        depth: 1,
        label: label.into(),
        meta: String::new(),
        detail: String::new(),
        status,
        status_text: status_text.into(),
        stale: false,
        current: false,
        target: ClientNavigatorTarget::Pane {
            endpoint_id: ClientEndpointId::Local,
            pane_id: label.into(),
        },
    }
}

// -- measuring --------------------------------------------------------------------

fn columns(rows: &[ClientNavigatorRow]) -> (u16, u16) {
    let presentation = StatePresentation::from_config(&crate::config::Config::default());
    navigator_columns(rows, ClientNavigatorPurpose::Goto, &presentation)
}

#[test]
fn navigator_status_width_counts_the_longest_state_word() {
    assert_eq!(navigator_status_measure("claude · idle"), 9 + 7);
    assert_eq!(navigator_status_measure("shell"), 5);
    let rows = [
        row("a", Some(AgentStatus::Idle), "claude · idle"),
        row("b", Some(AgentStatus::Unknown), "shell"),
    ];
    assert_eq!(columns(&rows).1, 16 + 2);
}

#[test]
fn navigator_status_width_is_capped_at_40() {
    let rows = [row("a", Some(AgentStatus::Idle), &"x".repeat(80))];
    assert_eq!(columns(&rows).1, 40);
}

#[test]
fn rows_without_a_status_leave_no_status_column() {
    let rows = [row("a", None, "")];
    assert_eq!(columns(&rows).1, 0);
}

#[test]
fn navigator_is_at_most_120_wide_and_at_least_73() {
    let mut snapshot = super::snapshot();
    snapshot.panes[0].label = Some("n".repeat(150));
    let mut state = state_with(snapshot, 310, 56);
    bind(&mut state, KeybindAction::OpenNavigator);
    state.compose(310, 56).unwrap();
    let popup = state.hits.navigator_popup;
    assert_eq!(popup.width, 120);
    assert_eq!(popup.x, (310 - 120) / 2);

    let mut state = state_with(super::snapshot(), 310, 56);
    bind(&mut state, KeybindAction::OpenNavigator);
    state.compose(310, 56).unwrap();
    assert_eq!(state.hits.navigator_popup.width, 73);
}

#[test]
fn navigator_draws_the_combined_status_column() {
    let mut snapshot = two_panes();
    snapshot
        .agents
        .push(agent("pane_1", Some("claude"), AgentStatus::Idle));
    let mut state = state_with(snapshot, 310, 56);
    bind(&mut state, KeybindAction::OpenNavigator);
    let text = screen(&mut state, 310, 56).join("\n");
    assert!(text.contains("claude · idle"), "{text}");
    assert!(text.contains(" shell"), "{text}");
}

#[test]
fn a_count_of_one_pane_is_singular() {
    let mut state = state_with(super::snapshot(), 120, 40);
    bind(&mut state, KeybindAction::OpenNavigator);
    let text = screen(&mut state, 120, 40).join("\n");
    assert!(text.contains("1 pane "), "{text}");
    let mut state = state_with(two_panes(), 120, 40);
    bind(&mut state, KeybindAction::OpenNavigator);
    let text = screen(&mut state, 120, 40).join("\n");
    assert!(text.contains("2 panes"), "{text}");
    assert!(text.contains("search panes"), "{text}");
}

// -- Esc --------------------------------------------------------------------------

#[test]
fn navigator_escape_blurs_then_clears_then_drops_the_chip_then_closes() {
    let mut state = state_with(two_panes(), 120, 40);
    bind(&mut state, KeybindAction::OpenNavigator);
    state.handle_input_bytes(b"/oth");
    state.handle_input_bytes(b"\x1b");
    assert!(!navigator(&state).search_focused);
    assert_eq!(navigator(&state).query.as_str(), "oth");
    state.handle_input_bytes(b"\x1b");
    assert_eq!(navigator(&state).query.as_str(), "");
    state.handle_input_bytes(b"w");
    assert_eq!(
        navigator(&state).filter,
        Some(ClientNavigatorFilter::Working)
    );
    state.handle_input_bytes(b"\x1b");
    assert_eq!(navigator(&state).filter, None);
    state.handle_input_bytes(b"\x1b");
    assert!(state.overlay.is_none());
}

// -- the link picker --------------------------------------------------------------

/// Open the editor on pane_1 and press ctrl+l.
fn open_picker(state: &mut ClientShellState) {
    bind(state, KeybindAction::AddPaneTodo);
    state.handle_input_bytes(b"fix it");
    state.handle_input_bytes(b"\x0c");
}

#[test]
fn ctrl_l_opens_the_link_picker_over_the_editor() {
    let mut state = state_with(two_panes(), 120, 40);
    open_picker(&mut state);
    let picker = navigator(&state);
    assert_eq!(picker.purpose, ClientNavigatorPurpose::TodoLink);
    assert_eq!(
        picker
            .suspended_todo_edit
            .as_ref()
            .map(|edit| edit.pane_id.as_str()),
        Some("pane_1")
    );
    let rows = rows(&state);
    assert_eq!(rows[0].target, ClientNavigatorTarget::ClearLink);
    assert!(
        !rows.iter().any(|row| matches!(
            &row.target,
            ClientNavigatorTarget::Pane { pane_id, .. } if pane_id == "pane_1"
        )),
        "the editor's own pane is not offered"
    );
    assert_ne!(
        picker.selected,
        Some(ClientNavigatorTarget::ClearLink),
        "never opens on the no-link row"
    );
    let text = screen(&mut state, 120, 40).join("\n");
    assert!(text.contains("no link"), "{text}");
    assert!(text.contains("pane_2 other"), "{text}");
    assert!(text.contains("search panes to link"), "{text}");
    assert!(text.contains("enter link"), "{text}");
}

#[test]
fn choosing_a_pane_row_stages_it_and_returns_to_the_editor() {
    let mut state = state_with(two_panes(), 120, 40);
    open_picker(&mut state);
    if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
        navigator.selected = Some(ClientNavigatorTarget::Pane {
            endpoint_id: ClientEndpointId::Local,
            pane_id: "pane_2".into(),
        });
    }
    state.handle_input_bytes(b"\r");
    assert!(matches!(
        &edit(&state).link,
        TodoEditLink::Set { pane_id, .. } if pane_id == "pane_2"
    ));
    assert_eq!(
        edit(&state).text.text(),
        "fix it",
        "the typed text survives"
    );
    // Saving sends the staged link.
    let outcome = state.handle_input_bytes(b"\x13");
    let sent = outcome.actions.iter().find_map(|action| match action {
        ClientShellAction::Endpoint { request, .. } => match &request.method {
            Method::TodoAdd(params) => Some(params.link_pane_id.clone()),
            _ => None,
        },
        _ => None,
    });
    assert_eq!(sent, Some(Some("pane_2".to_owned())));
}

#[test]
fn the_no_link_row_stages_clearing_the_link() {
    let mut state = state_with(two_panes(), 120, 40);
    open_picker(&mut state);
    if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
        navigator.selected = Some(ClientNavigatorTarget::ClearLink);
    }
    state.handle_input_bytes(b"\r");
    assert_eq!(edit(&state).link, TodoEditLink::Clear);
}

#[test]
fn workspace_rows_never_resolve_a_link() {
    let mut state = state_with(two_panes(), 120, 40);
    open_picker(&mut state);
    if let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() {
        navigator.selected = Some(ClientNavigatorTarget::Workspace {
            endpoint_id: ClientEndpointId::Local,
            workspace_id: "ws_1".into(),
        });
    }
    state.handle_input_bytes(b"\r");
    assert_eq!(navigator(&state).purpose, ClientNavigatorPurpose::TodoLink);
}

#[test]
fn dismissing_the_picker_keeps_the_link_it_had() {
    let mut state = state_with(two_panes(), 120, 40);
    open_picker(&mut state);
    state.handle_input_bytes(b"\x1b");
    assert_eq!(edit(&state).link, TodoEditLink::Keep);

    // A click outside the picker dismisses it the same way.
    open_picker(&mut state);
    state.compose(120, 40).unwrap();
    state.handle_raw_events(vec![crate::raw_input::RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 0,
        row: 0,
        modifiers: KeyModifiers::empty(),
    })]);
    assert_eq!(edit(&state).link, TodoEditLink::Keep);
}

#[test]
fn ctrl_j_and_ctrl_k_move_the_link_picker_while_searching() {
    let mut state = state_with(two_panes(), 120, 40);
    open_picker(&mut state);
    state.handle_input_bytes(b"/");
    let before = navigator(&state).selected.clone();
    state.handle_input_bytes(b"\x0b");
    assert_ne!(navigator(&state).selected, before);
    state.handle_input_bytes(b"\x0e");
    assert_eq!(navigator(&state).selected, before);
}

#[test]
fn the_goto_navigator_has_no_link_row() {
    let mut state = state_with(two_panes(), 120, 40);
    bind(&mut state, KeybindAction::OpenNavigator);
    assert!(!rows(&state)
        .iter()
        .any(|row| row.target == ClientNavigatorTarget::ClearLink));
    assert_eq!(navigator(&state).purpose, ClientNavigatorPurpose::Goto);
}
