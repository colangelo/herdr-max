//! core-phase5 C.5: the pane menu offers a split on each of this Mac's
//! core machines, with its gate, only while Local advertises
//! `spawn_on_machine`; choosing one sends `pane.split` with `on`.

use super::*;
use crate::api::schema::Method;
use crossterm::event::{MouseButton, MouseEventKind};

fn state(spawn_on_machine: bool) -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.set_local_core_machines(vec![
        ("ac-mbm5".into(), "open".into()),
        ("lab-vm".into(), "dormant".into()),
    ]);
    state.set_endpoint_spawn_on_machine(&ClientEndpointId::Local, spawn_on_machine);
    state.compose(106, 24).unwrap();
    state
}

fn open_pane_menu(state: &mut ClientShellState) {
    let pane = state.hits.panes[0].rect;
    state.handle_raw_events(vec![crate::raw_input::RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Right),
        column: pane.x + 1,
        row: pane.y,
        modifiers: KeyModifiers::empty(),
    })]);
}

fn labels(state: &ClientShellState) -> Vec<String> {
    match state.overlay.as_ref() {
        Some(ClientShellOverlay::ContextMenu(menu)) => menu
            .items()
            .iter()
            .map(|item| item.label.to_string())
            .collect(),
        other => panic!("expected the pane menu, got {other:?}"),
    }
}

#[test]
fn a_server_that_starts_panes_on_machines_offers_each_machine_with_its_gate() {
    let mut state = state(true);
    open_pane_menu(&mut state);
    let items = labels(&state);
    let split = items
        .iter()
        .position(|label| label == "Split down")
        .expect("split down");
    assert_eq!(
        &items[split + 1..split + 3],
        [
            "Split right on ac-mbm5 · open",
            "Split right on lab-vm · dormant"
        ]
    );
    // Rendered: the menu shows the rows, and their click targets choose them.
    let rows = frame_rows(&state.compose(106, 24).unwrap().frame);
    assert!(
        rows.iter()
            .any(|row| row.contains("Split right on lab-vm · dormant")),
        "{rows:#?}"
    );
    let row = state.hits.context_menu_rows[split + 2].0;
    let outcome =
        state.handle_raw_events(vec![crate::raw_input::RawInputEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: row.x + 1,
            row: row.y,
            modifiers: KeyModifiers::empty(),
        })]);
    let splits: Vec<&crate::api::schema::PaneSplitParams> = outcome
        .actions
        .iter()
        .filter_map(|action| match action {
            ClientShellAction::Endpoint { request, .. } => match &request.method {
                Method::PaneSplit(params) => Some(params),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(splits.len(), 1);
    assert_eq!(splits[0].on.as_deref(), Some("lab-vm"));
    assert_eq!(splits[0].target_pane_id.as_deref(), Some("pane_1"));
}

// V13's client part: without the capability an older server would start the
// pane here, so no machine is offered.
#[test]
fn a_server_without_spawn_on_machine_offers_no_machine() {
    let mut state = state(false);
    open_pane_menu(&mut state);
    assert!(
        labels(&state).iter().all(|label| !label.contains(" on ")),
        "{:?}",
        labels(&state)
    );
}
