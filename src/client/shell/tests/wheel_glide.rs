// Fork issue 273: `ui.smooth_scroll` paces a wheel notch over a few frames.
use super::*;
use crate::protocol::ClientMouseKind;

fn wheel_state(smooth_scroll: bool, edit: impl FnOnce(&mut PaneSurfaceFrame)) -> ClientShellState {
    let mut config = Config::default();
    config.ui.smooth_scroll = smooth_scroll;
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    state.set_snapshot(Box::new(snapshot()));
    let mut pane_surface = surface();
    edit(&mut pane_surface);
    state.set_pane_surface(pane_surface);
    state.compose(106, 20).expect("pane surface");
    state
}

fn wheel(state: &mut ClientShellState, kind: MouseEventKind) -> ClientShellInput {
    let inner = state.hits.panes[0].inner_rect;
    state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind,
        column: inner.x,
        row: inner.y,
        modifiers: KeyModifiers::empty(),
    })])
}

/// The wheel events sent to `pane_1`, as (kind, lines).
fn wheel_rows(outcome: &ClientShellInput) -> Vec<(ClientMouseKind, u16)> {
    outcome
        .requests
        .iter()
        .filter_map(|request| match request {
            ClientMessage::ClientShellPaneInput { pane_id, events } if pane_id == "pane_1" => {
                Some(events)
            }
            _ => None,
        })
        .flatten()
        .filter_map(|event| match event {
            ClientPaneInputEvent::Mouse { kind, lines, .. } => Some((*kind, *lines)),
            _ => None,
        })
        .collect()
}

/// Runs every due glide step and returns the rows each one sent.
fn drain_glide(state: &mut ClientShellState) -> Vec<(ClientMouseKind, u16)> {
    let mut sent = Vec::new();
    while let Some(deadline) = state.wheel_glide_deadline {
        let now = deadline - std::time::Duration::from_millis(1);
        assert!(state.timer_delay(now) <= std::time::Duration::from_millis(1));
        let mut early = ClientShellInput::default();
        state.tick_wheel_glide(now, &mut early);
        assert!(early.requests.is_empty(), "a step waits for its deadline");
        let mut outcome = ClientShellInput::default();
        state.tick_wheel_glide(deadline, &mut outcome);
        let rows = wheel_rows(&outcome);
        assert_eq!(rows.len(), 1, "one wheel event per step");
        sent.extend(rows);
    }
    sent
}

#[test]
fn smooth_scroll_off_sends_the_whole_notch_at_once() {
    let mut state = wheel_state(false, |_| {});
    let outcome = wheel(&mut state, MouseEventKind::ScrollUp);
    assert_eq!(wheel_rows(&outcome), [(ClientMouseKind::ScrollUp, 3)]);
    assert!(state.wheel_glide.is_none());
    assert!(state.wheel_glide_deadline.is_none());
}

#[test]
fn smooth_scroll_paces_one_notch_row_by_row() {
    let mut state = wheel_state(true, |_| {});
    let first = wheel(&mut state, MouseEventKind::ScrollUp);
    assert_eq!(
        wheel_rows(&first),
        [(ClientMouseKind::ScrollUp, 1)],
        "the first row moves at once"
    );
    let rest = drain_glide(&mut state);
    assert_eq!(
        rest,
        [
            (ClientMouseKind::ScrollUp, 1),
            (ClientMouseKind::ScrollUp, 1)
        ]
    );
    assert!(state.wheel_glide.is_none());
    let mut idle = ClientShellInput::default();
    state.tick_wheel_glide(std::time::Instant::now(), &mut idle);
    assert!(idle.requests.is_empty());
}

#[test]
fn smooth_scroll_burst_eases_out_and_keeps_every_row() {
    let mut state = wheel_state(true, |_| {});
    let mut sent = wheel_rows(&wheel(&mut state, MouseEventKind::ScrollDown));
    for _ in 0..4 {
        assert!(
            wheel_rows(&wheel(&mut state, MouseEventKind::ScrollDown)).is_empty(),
            "a running glide keeps its own pace"
        );
    }
    sent.extend(drain_glide(&mut state));
    let lines = sent.iter().map(|(_, lines)| *lines).collect::<Vec<_>>();
    assert_eq!(
        lines.iter().map(|lines| usize::from(*lines)).sum::<usize>(),
        15
    );
    assert!(sent
        .iter()
        .all(|(kind, _)| *kind == ClientMouseKind::ScrollDown));
    assert!(
        lines[1] > 1,
        "a burst catches up several rows a step: {lines:?}"
    );
    assert!(
        lines[1..].windows(2).all(|pair| pair[0] >= pair[1]),
        "steps after the first only shrink: {lines:?}"
    );
    assert_eq!(lines.last(), Some(&1), "the glide ends on single rows");
}

#[test]
fn smooth_scroll_direction_change_drops_the_other_way() {
    let mut state = wheel_state(true, |_| {});
    wheel(&mut state, MouseEventKind::ScrollUp);
    let reverse = wheel(&mut state, MouseEventKind::ScrollDown);
    assert_eq!(wheel_rows(&reverse), [(ClientMouseKind::ScrollDown, 1)]);
    let rest = drain_glide(&mut state);
    assert_eq!(
        rest,
        [
            (ClientMouseKind::ScrollDown, 1),
            (ClientMouseKind::ScrollDown, 1)
        ]
    );
}

#[test]
fn smooth_scroll_leaves_wheel_owning_apps_alone() {
    for edit in [
        (|surface: &mut PaneSurfaceFrame| surface.panes[0].mouse_reporting = true)
            as fn(&mut PaneSurfaceFrame),
        |surface: &mut PaneSurfaceFrame| surface.panes[0].alternate_screen_active = true,
    ] {
        let mut state = wheel_state(true, edit);
        let outcome = wheel(&mut state, MouseEventKind::ScrollUp);
        assert_eq!(wheel_rows(&outcome), [(ClientMouseKind::ScrollUp, 3)]);
        assert!(state.wheel_glide_deadline.is_none());
    }
}

#[test]
fn smooth_scroll_glide_stops_when_the_pane_starts_taking_the_wheel() {
    let mut state = wheel_state(true, |_| {});
    wheel(&mut state, MouseEventKind::ScrollUp);
    let deadline = state.wheel_glide_deadline.expect("glide running");
    let mut pane_surface = surface();
    pane_surface.surface_revision = 2;
    pane_surface.panes[0].mouse_reporting = true;
    state.set_pane_surface(pane_surface);
    state.compose(106, 20).expect("pane surface");
    let mut outcome = ClientShellInput::default();
    state.tick_wheel_glide(deadline, &mut outcome);
    assert!(wheel_rows(&outcome).is_empty());
    assert!(state.wheel_glide.is_none());
    assert!(state.wheel_glide_deadline.is_none());
}
