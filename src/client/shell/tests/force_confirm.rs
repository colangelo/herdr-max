//! Confirmations for refused closes and respawns (fork c7500330, 82897fd1,
//! de8f3926): the count comes from the refusal, accepting resends with
//! `force`, cancelling sends nothing.

use super::*;
use crate::api::schema::{Method, ResponseResult};

fn state() -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.compose(106, 24).unwrap();
    state
}

fn bind(state: &mut ClientShellState, action: crate::input::KeybindAction) -> ClientShellInput {
    let mut outcome = ClientShellInput::default();
    state.record_binding(crate::input::KeybindMatch::Action(action), &mut outcome);
    outcome
}

fn only_method(outcome: &ClientShellInput) -> (String, Method) {
    let [ClientShellAction::Endpoint { request, .. }] = outcome.actions.as_slice() else {
        panic!("expected one endpoint request, got {:?}", outcome.actions);
    };
    (request.id.clone(), request.method.clone())
}

fn refuse(state: &mut ClientShellState, request_id: &str, message: &str) {
    state.handle_endpoint_result(
        "boot-1",
        request_id,
        Err(ClientShellEndpointError {
            code: Some("confirmation_required".into()),
            message: message.into(),
        }),
    );
}

fn confirm_text(state: &ClientShellState) -> (String, String) {
    let Some(ClientShellOverlay::ConfirmClose(confirm)) = state.overlay.as_ref() else {
        panic!("expected a close confirmation, got {:?}", state.overlay);
    };
    (confirm.title.clone(), confirm.detail.clone())
}

fn screen(state: &mut ClientShellState) -> String {
    let frame = state.compose(106, 24).expect("frame");
    frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_pane_close_refused_for_todos_asks_and_forces_on_enter() {
    let mut state = state();
    let (id, method) = only_method(&bind(&mut state, crate::input::KeybindAction::ClosePane));
    assert!(matches!(method, Method::PaneClose(ref params) if !params.force));

    refuse(
        &mut state,
        &id,
        "this pane still has 2 open todos (pane_1: a; pane_1: b); pass --force (force=true) to close it anyway",
    );

    let (title, detail) = confirm_text(&state);
    assert_eq!(title, "Close pane with unfinished todos?");
    assert_eq!(detail, "this pane - 2 outstanding todos");
    let text = screen(&mut state);
    assert!(text.contains("Close pane with unfinished todos?"), "{text}");
    assert!(text.contains("2 outstanding todos"), "{text}");

    let (_, forced) = only_method(&state.handle_input_bytes(b"\r"));
    assert!(matches!(forced, Method::PaneClose(ref params)
        if params.pane_id == "pane_1" && params.force));
    assert!(state.overlay.is_none());
}

#[test]
fn the_confirmation_names_the_pane_like_its_border() {
    let mut state = state();
    let mut projected = snapshot();
    projected.panes[0].label = Some("api".into());
    state.set_snapshot(Box::new(projected));
    let (id, _) = only_method(&bind(&mut state, crate::input::KeybindAction::ClosePane));

    refuse(
        &mut state,
        &id,
        "this pane still has 1 open todo (pane_1: a)",
    );

    assert_eq!(confirm_text(&state).1, "api - 1 outstanding todo");
}

#[test]
fn cancelling_a_forced_close_sends_nothing() {
    let mut state = state();
    let (id, _) = only_method(&bind(&mut state, crate::input::KeybindAction::ClosePane));
    refuse(
        &mut state,
        &id,
        "this pane still has 1 open todo (pane_1: a)",
    );

    let cancelled = state.handle_input_bytes(b"\x1b");

    assert!(cancelled.actions.is_empty());
    assert!(state.overlay.is_none());
}

#[test]
fn the_confirm_button_forces_the_close() {
    let mut state = state();
    let (id, _) = only_method(&bind(&mut state, crate::input::KeybindAction::ClosePane));
    refuse(
        &mut state,
        &id,
        "this pane still has 1 open todo (pane_1: a)",
    );
    state.compose(106, 24).unwrap();
    let ok = state.hits.overlay_primary;

    let clicked = state.handle_raw_events(vec![crate::raw_input::RawInputEvent::Mouse(
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: ok.x,
            row: ok.y,
            modifiers: KeyModifiers::empty(),
        },
    )]);

    let (_, forced) = only_method(&clicked);
    assert!(matches!(forced, Method::PaneClose(ref params) if params.force));
}

#[test]
fn a_respawn_with_live_work_asks_and_forces_on_enter() {
    let mut state = state();
    let (id, method) = only_method(&bind(&mut state, crate::input::KeybindAction::RespawnPane));
    assert!(matches!(method, Method::PaneRespawn(ref params)
        if params.pane_id == "pane_1" && !params.force));

    refuse(
        &mut state,
        &id,
        "this pane is still running claude (pid 4242) and has 1 open todo (pane_1: a); pass --force (force=true) to respawn it anyway",
    );

    let (title, detail) = confirm_text(&state);
    assert_eq!(title, "Respawn pane and kill what is running?");
    assert_eq!(
        detail,
        "this pane - restarts the process, 1 outstanding todo"
    );
    let (_, forced) = only_method(&state.handle_input_bytes(b"\r"));
    assert!(matches!(forced, Method::PaneRespawn(ref params) if params.force));
}

#[test]
fn a_respawn_refused_only_for_a_process_does_not_mention_todos() {
    let mut state = state();
    let (id, _) = only_method(&bind(&mut state, crate::input::KeybindAction::RespawnPane));

    refuse(
        &mut state,
        &id,
        "this pane is still running claude (pid 4242); pass --force (force=true) to respawn it anyway",
    );

    let detail = confirm_text(&state).1;
    assert!(detail.contains("restarts the process"), "{detail}");
    assert!(!detail.contains("outstanding"), "{detail}");
}

fn refuse_with(
    state: &mut ClientShellState,
    request_id: &str,
    message: &str,
) -> (bool, Vec<ClientShellAction>) {
    state.handle_endpoint_result(
        "boot-1",
        request_id,
        Err(ClientShellEndpointError {
            code: Some("confirmation_required".into()),
            message: message.into(),
        }),
    )
}

/// The fork never asked about todos on a tab close: a refusal that is only
/// about todos is answered with the forced close, no dialog.
#[test]
fn a_tab_close_refused_for_todos_is_forced_without_asking() {
    let mut state = state();
    let mut projected = snapshot();
    let mut tab = projected.tabs[0].clone();
    tab.tab_id = "tab_2".into();
    tab.focused = false;
    projected.tabs.push(tab);
    state.set_snapshot(Box::new(projected));
    let mut outcome = ClientShellInput::default();
    state.request_tab_close("tab_2".into(), &mut outcome);
    let (id, method) = only_method(&outcome);
    assert!(matches!(method, Method::TabClose(ref params) if !params.force));

    let (_, actions) = refuse_with(
        &mut state,
        &id,
        "this tab still has 3 open todos (pane_2: a)",
    );

    assert!(state.overlay.is_none());
    assert!(matches!(actions.as_slice(),
        [ClientShellAction::Endpoint { request, .. }]
            if matches!(&request.method, Method::TabClose(params)
                if params.tab_id == "tab_2" && params.force)));
}

#[test]
fn a_confirmed_workspace_close_refused_for_todos_is_forced_without_asking_again() {
    let mut state = state();
    let mut outcome = ClientShellInput::default();
    state.request_workspace_close("ws_1".into(), None, &mut outcome);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::ConfirmClose(_))
    ));
    let (id, method) = only_method(&state.handle_input_bytes(b"\r"));
    assert!(matches!(method, Method::WorkspaceClose(ref params) if !params.force));

    let (_, actions) = refuse_with(
        &mut state,
        &id,
        "this workspace still has 1 open todo (pane_1: a)",
    );

    assert!(state.overlay.is_none());
    assert!(matches!(actions.as_slice(),
        [ClientShellAction::Endpoint { request, .. }]
            if matches!(&request.method, Method::WorkspaceClose(params)
                if params.workspace_id == "ws_1" && params.force)));
}

#[test]
fn a_worktree_group_refusal_keeps_its_own_confirmation() {
    let mut state = state();
    let (id, _) = only_method(&bind(&mut state, crate::input::KeybindAction::ClosePane));

    refuse(
        &mut state,
        &id,
        "closing this pane would close a worktree group",
    );

    assert_eq!(confirm_text(&state).0, "Close workspace?");
}

#[test]
fn a_forced_close_whose_target_left_is_not_sent() {
    let mut state = state();
    let (id, _) = only_method(&bind(&mut state, crate::input::KeybindAction::ClosePane));
    refuse(
        &mut state,
        &id,
        "this pane still has 1 open todo (pane_1: a)",
    );
    let mut projected = snapshot();
    projected.panes.clear();
    state.set_snapshot(Box::new(projected));

    let accepted = state.handle_input_bytes(b"\r");

    assert!(accepted.actions.is_empty());
    assert!(state.overlay.is_none());
}

#[test]
fn a_forced_close_from_another_boot_is_not_sent() {
    let mut state = state();
    let (id, _) = only_method(&bind(&mut state, crate::input::KeybindAction::ClosePane));
    refuse(
        &mut state,
        &id,
        "this pane still has 1 open todo (pane_1: a)",
    );
    let mut projected = snapshot();
    projected.boot_id = "boot-2".into();
    state.set_snapshot(Box::new(projected));
    if state.overlay.is_none() {
        // A new boot already dropped the confirmation; nothing to accept.
        return;
    }

    let accepted = state.handle_input_bytes(b"\r");

    assert!(accepted.actions.is_empty());
}

#[test]
fn a_forced_close_result_leaves_no_overlay() {
    let mut state = state();
    let (id, _) = only_method(&bind(&mut state, crate::input::KeybindAction::ClosePane));
    refuse(
        &mut state,
        &id,
        "this pane still has 1 open todo (pane_1: a)",
    );
    let (forced_id, _) = only_method(&state.handle_input_bytes(b"\r"));

    state.handle_endpoint_result(
        "boot-1",
        &forced_id,
        Ok(ResponseResult::Closed {
            closed_todos: Vec::new(),
        }),
    );

    assert!(state.overlay.is_none());
}
