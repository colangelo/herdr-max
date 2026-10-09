//! Clicking chrome's indicators opens the overlays' panels, as in the fork
//! (src/app/input/mouse.rs at sync-snapshot/2026-10-08): the notification
//! indicator toggles the notification center, a pane's todo mark toggles
//! that pane's todo panel.

use super::*;
use crate::api::schema::Method;

const W: u16 = 106;
const H: u16 = 24;

fn facts() -> crate::protocol::ClientShellResourceFacts {
    crate::protocol::ClientShellResourceFacts {
        notifications: Some(crate::protocol::ClientNotificationSummary {
            total: 2,
            unread: 2,
            ..Default::default()
        }),
        pane_todos: Some(
            [(
                "pane_1".to_owned(),
                crate::protocol::ClientPaneTodoSummary {
                    total: 2,
                    open: 2,
                    highest_priority: Some("high".into()),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
        ),
        ..Default::default()
    }
}

/// One bordered pane, so its todo mark has a top border to sit on.
fn state() -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut snapshot = snapshot();
    snapshot.resource_facts = Some(facts());
    state.set_snapshot(Box::new(snapshot));
    let layout = state.layout(W, H);
    let mut surface = surface();
    let area = Rect::new(0, 0, layout.pane_surface.width, layout.pane_surface.height);
    surface.frame = FrameData::from_ratatui_buffer_with_hyperlinks(&Buffer::empty(area), None, &[]);
    surface.panes[0].rect = SurfaceRect {
        x: 0,
        y: 0,
        width: area.width,
        height: area.height,
    };
    surface.panes[0].inner_rect = SurfaceRect {
        x: 1,
        y: 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    state.set_pane_surface(surface);
    state.compose(W, H).unwrap();
    state
}

fn click(state: &mut ClientShellState, (x, y): (u16, u16)) -> ClientShellInput {
    let outcome =
        state.handle_raw_events(vec![crate::raw_input::RawInputEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::empty(),
        })]);
    state.compose(W, H).unwrap();
    outcome
}

fn methods(outcome: &ClientShellInput) -> Vec<Method> {
    outcome
        .actions
        .iter()
        .filter_map(|action| match action {
            ClientShellAction::Endpoint { request, .. } => Some(request.method.clone()),
            _ => None,
        })
        .collect()
}

fn indicator(state: &ClientShellState) -> (u16, u16) {
    let rect = state.hits.notification_indicator;
    assert!(rect.width > 0, "chrome draws the notification indicator");
    (rect.x + 1, rect.y)
}

fn todo_mark(state: &ClientShellState) -> (u16, u16) {
    let (rect, pane_id) = state
        .hits
        .pane_todos
        .first()
        .cloned()
        .expect("the pane draws its todo mark");
    assert_eq!(pane_id, "pane_1");
    (rect.x, rect.y)
}

#[test]
fn clicking_the_notification_indicator_toggles_the_center() {
    let mut state = state();
    let at = indicator(&state);

    let opened = click(&mut state, at);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::NotificationCenter(_))
    ));
    assert!(matches!(
        methods(&opened).as_slice(),
        [Method::NotificationList(_)]
    ));

    let at = indicator(&state);
    click(&mut state, at);
    assert!(state.overlay.is_none(), "a second click closes it");
}

#[test]
fn clicking_a_pane_todo_mark_opens_that_panes_todo_panel() {
    let mut state = state();
    let at = todo_mark(&state);

    let opened = click(&mut state, at);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::TodoPanel(_))
    ));
    assert!(matches!(methods(&opened).as_slice(), [Method::TodoList(_)]));
}

#[test]
fn the_indicators_do_nothing_outside_terminal_navigate_and_resize() {
    let mut state = state();
    state.mode = ClientShellMode::Copy;

    let at = indicator(&state);
    click(&mut state, at);
    assert!(!matches!(
        state.overlay,
        Some(ClientShellOverlay::NotificationCenter(_))
    ));
    let at = todo_mark(&state);
    click(&mut state, at);
    assert!(!matches!(
        state.overlay,
        Some(ClientShellOverlay::TodoPanel(_))
    ));
}
