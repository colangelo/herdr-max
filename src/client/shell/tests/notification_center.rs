//! The notification center, re-expressed from the fork's oracles
//! (src/ui/notification_center.rs, src/app/state.rs, src/app/input/modal.rs at
//! sync-snapshot/2026-10-08) over snapshots and emitted endpoint requests,
//! plus the mouse rules the fork never tested.

use super::*;
use crate::api::schema::{Method, NotificationInfo, NotificationKind, ResponseResult};
use crate::input::KeybindAction;

const W: u16 = 106;
const H: u16 = 24;

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn entry(
    id: u64,
    title: &str,
    kind: NotificationKind,
    target: bool,
    read: bool,
) -> NotificationInfo {
    NotificationInfo {
        id,
        kind,
        title: title.into(),
        context: String::new(),
        workspace_id: target.then(|| "ws_1".into()),
        pane_id: target.then(|| "pane_1".into()),
        posted_at_unix: now(),
        read,
    }
}

fn state() -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.compose(W, H).unwrap();
    state
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

fn open(state: &mut ClientShellState, entries: Vec<NotificationInfo>) {
    let mut outcome = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(KeybindAction::OpenNotificationCenter),
        &mut outcome,
    );
    let [ClientShellAction::Endpoint { request, .. }] = outcome.actions.as_slice() else {
        panic!("opening lists the log: {:?}", outcome.actions);
    };
    assert!(matches!(request.method, Method::NotificationList(_)));
    let unread = entries.iter().filter(|entry| !entry.read).count() as u64;
    state.handle_endpoint_result(
        "boot-1",
        &request.id,
        Ok(ResponseResult::NotificationList {
            notifications: entries,
            unread_count: unread,
        }),
    );
    state.compose(W, H).unwrap();
}

fn screen(state: &mut ClientShellState) -> Vec<String> {
    let frame = state.compose(W, H).expect("frame");
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

fn layout(state: &ClientShellState) -> super::super::notification_center::NotificationCenterLayout {
    state.hits.notification_center.clone().expect("layout")
}

fn is_open(state: &ClientShellState) -> bool {
    matches!(
        state.overlay,
        Some(ClientShellOverlay::NotificationCenter(_))
    )
}

fn selected(state: &ClientShellState) -> usize {
    match state.overlay.as_ref() {
        Some(ClientShellOverlay::NotificationCenter(center)) => center.selected,
        other => panic!("expected the center, got {other:?}"),
    }
}

fn click(state: &mut ClientShellState, x: u16, y: u16) -> ClientShellInput {
    state.handle_raw_events(vec![crate::raw_input::RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::empty(),
    })])
}

fn two() -> Vec<NotificationInfo> {
    vec![
        entry(
            2,
            "tests failed",
            NotificationKind::NeedsAttention,
            true,
            false,
        ),
        entry(
            1,
            "build finished",
            NotificationKind::Finished,
            false,
            false,
        ),
    ]
}

#[test]
fn an_empty_center_is_just_the_box() {
    let mut state = state();
    open(&mut state, Vec::new());
    let text = screen(&mut state).join("\n");
    let rect = layout(&state).outer;

    assert!(text.contains("│ no notifications           │"), "{text}");
    assert_eq!((rect.width, rect.height), (30, 3));
    assert!(layout(&state).buttons.is_empty());
}

#[test]
fn entries_list_newest_first_with_dot_and_age_and_the_footer() {
    let mut state = state();
    open(&mut state, two());
    let text = screen(&mut state).join("\n");
    let rect = layout(&state).outer;

    assert!(
        text.contains("│ ● tests failed                      now │"),
        "{text}"
    );
    assert!(
        text.contains("│ ● build finished                    now │"),
        "{text}"
    );
    assert!(
        text.contains("│ r mark read    c clear all    esc close │"),
        "{text}"
    );
    assert_eq!((rect.width, rect.height), (43, 6));
    // Right-aligned under the tab bar.
    let tab_bar = state.layout(W, H).tab_bar;
    assert_eq!(rect.right(), tab_bar.right());
    assert_eq!(rect.y, tab_bar.bottom());
}

#[test]
fn a_wide_title_sizes_the_panel() {
    let mut state = state();
    open(
        &mut state,
        vec![entry(
            1,
            "the build finished on the release runner",
            NotificationKind::Finished,
            false,
            false,
        )],
    );
    let text = screen(&mut state).join("\n");

    assert_eq!(layout(&state).outer.width, 50);
    assert!(
        text.contains("│ ● the build finished on the release runner now │"),
        "{text}"
    );
    assert!(
        text.contains("│    r mark read    c clear all    esc close     │"),
        "{text}"
    );
}

#[test]
fn unread_rows_show_dot_and_bold_while_read_rows_dim() {
    let mut state = state();
    open(
        &mut state,
        vec![
            entry(2, "read one", NotificationKind::Finished, false, true),
            entry(1, "unread one", NotificationKind::Finished, false, false),
        ],
    );
    let frame = state.compose(W, H).unwrap();
    let list = layout(&state).list;
    let cell = |x: u16, y: u16| &frame.cells[usize::from(y) * usize::from(W) + usize::from(x)];
    let palette = &state.config.palette;
    let bold = crate::protocol::modifier_to_u16(Modifier::BOLD);

    // Row 0 is selected and read: no dot, the band, not bold.
    assert_eq!(cell(list.x + 1, list.y).symbol, " ");
    let title = cell(list.x + 3, list.y);
    assert_eq!(title.bg, crate::protocol::color_to_u32(palette.accent));
    assert_eq!(title.modifier & bold, 0);
    // Row 1 is unread: the kind's dot, a bold title.
    let dot = cell(list.x + 1, list.y + 1);
    assert_eq!(dot.symbol, "●");
    assert_eq!(dot.fg, crate::protocol::color_to_u32(palette.blue));
    assert_ne!(cell(list.x + 3, list.y + 1).modifier & bold, 0);
}

#[test]
fn the_center_moves_on_every_shared_chord_and_clamps() {
    let mut state = state();
    open(&mut state, two());
    for key in [&b"j"[..], b"\x1b[B", b"\x0e"] {
        state.handle_input_bytes(b"k");
        state.handle_input_bytes(key);
        assert_eq!(selected(&state), 1, "{key:?}");
    }
    state.handle_input_bytes(b"j");
    assert_eq!(selected(&state), 1, "clamps at the end");
    for key in [&b"k"[..], b"\x1b[A", b"\x10", b"\x0b"] {
        state.handle_input_bytes(b"j");
        state.handle_input_bytes(key);
        assert_eq!(selected(&state), 0, "{key:?}");
    }
}

#[test]
fn c_clears_and_keeps_the_panel_and_r_marks_all_read() {
    let mut state = state();
    open(&mut state, two());
    state.handle_input_bytes(b"j");

    let cleared = state.handle_input_bytes(b"c");
    assert!(matches!(
        methods(&cleared).as_slice(),
        [Method::NotificationClear(_)]
    ));
    assert!(is_open(&state));
    assert_eq!(selected(&state), 0);

    let read = state.handle_input_bytes(b"r");
    assert!(matches!(methods(&read).as_slice(),
        [Method::NotificationMarkSeen(params)] if params.id.is_none()));
    assert!(is_open(&state));
}

#[test]
fn enter_marks_only_the_activated_entry_read_and_jumps_to_its_pane() {
    let mut state = state();
    open(&mut state, two());

    let jumped = state.handle_input_bytes(b"\r");

    assert!(matches!(methods(&jumped).as_slice(),
        [Method::NotificationMarkSeen(params), Method::PaneFocus(target)]
            if params.id == Some(2) && target.pane_id == "pane_1"));
    assert!(!is_open(&state));
}

#[test]
fn enter_on_a_targetless_entry_is_inert() {
    let mut state = state();
    open(&mut state, two());
    state.handle_input_bytes(b"j");

    assert!(methods(&state.handle_input_bytes(b"\r")).is_empty());
    assert!(is_open(&state));
}

#[test]
fn a_target_whose_workspace_is_gone_is_inert_and_one_whose_pane_is_gone_only_reads() {
    let mut gone = entry(3, "old", NotificationKind::Finished, true, false);
    gone.workspace_id = Some("ws_gone".into());
    gone.pane_id = None;
    let mut state = state();
    open(&mut state, vec![gone]);
    assert!(methods(&state.handle_input_bytes(b"\r")).is_empty());
    assert!(is_open(&state));

    let mut pane_gone = entry(4, "pane left", NotificationKind::Finished, true, false);
    pane_gone.pane_id = None;
    let mut state = super::notification_center::state();
    open(&mut state, vec![pane_gone]);
    let read = state.handle_input_bytes(b"\r");
    assert!(matches!(methods(&read).as_slice(),
        [Method::NotificationMarkSeen(params)] if params.id == Some(4)));
    assert!(!is_open(&state));
}

#[test]
fn esc_and_q_close_without_jumping() {
    for key in [&b"\x1b"[..], b"q"] {
        let mut state = state();
        open(&mut state, two());
        assert!(methods(&state.handle_input_bytes(key)).is_empty());
        assert!(!is_open(&state));
    }
}

#[test]
fn a_moved_log_summary_fetches_the_log_again() {
    let mut state = state();
    open(&mut state, two());
    let mut quiet = ClientShellInput::default();
    state.tick_notification_center(&mut quiet);
    assert!(methods(&quiet).is_empty());

    let mut projected = snapshot();
    projected.revision = 2;
    projected.resource_facts = Some(crate::protocol::ClientShellResourceFacts {
        notifications: Some(crate::protocol::ClientNotificationSummary {
            total: 100,
            unread: 3,
            latest_id: 141,
        }),
        ..Default::default()
    });
    state.set_snapshot(Box::new(projected));
    let mut outcome = ClientShellInput::default();
    state.tick_notification_center(&mut outcome);

    assert!(matches!(
        methods(&outcome).as_slice(),
        [Method::NotificationList(_)]
    ));
}

#[test]
fn a_click_inside_off_rows_closes_but_a_footer_near_miss_is_inert() {
    let mut state = state();
    open(&mut state, two());
    let layout = layout(&state);
    let (first, _) = layout.buttons[0];

    click(&mut state, first.right(), first.y);
    assert!(is_open(&state), "a near miss on the footer row");
    click(
        &mut state,
        layout.inner.x + 1,
        layout.footer_row.unwrap().y - 1,
    );
    assert!(!is_open(&state), "the blank row inside the panel closes it");
}

#[test]
fn clicking_a_row_activates_it_and_the_mark_read_button_reads_all() {
    let mut state = state();
    open(&mut state, two());
    let layout = layout(&state);

    let clicked = click(&mut state, layout.list.x + 4, layout.list.y);
    assert!(matches!(methods(&clicked).as_slice(),
        [Method::NotificationMarkSeen(params), Method::PaneFocus(_)] if params.id == Some(2)));

    let mut state = super::notification_center::state();
    open(&mut state, two());
    let (mark, _) = super::notification_center::layout(&state).buttons[0];
    let read = click(&mut state, mark.x, mark.y);
    assert!(matches!(methods(&read).as_slice(),
        [Method::NotificationMarkSeen(params)] if params.id.is_none()));
    assert!(is_open(&state));
}

#[test]
fn the_bottom_right_center_opens_above_its_indicator_or_flush_with_the_bottom() {
    use super::super::notification_center::{
        notification_center_layout, ClientNotificationCenterOverlay,
    };
    use crate::config::NotificationCenterPositionConfig::{BottomRight, TopRight};
    let center = ClientNotificationCenterOverlay {
        boot_id: "boot-1".into(),
        entries: two(),
        loaded: Some((2, 2, 2)),
        list_in_flight: false,
        selected: 0,
        scroll: 0,
        hovered_button: None,
    };
    let screen = Rect::new(0, 0, 80, 25);
    let tab_bar = Rect::new(0, 0, 80, 1);
    let at = |indicator, position| {
        notification_center_layout(&center, tab_bar, indicator, position, screen)
            .unwrap()
            .outer
    };

    assert_eq!(at(Rect::default(), TopRight), Rect::new(37, 1, 43, 6));
    assert_eq!(at(Rect::default(), BottomRight), Rect::new(37, 19, 43, 6));
    assert_eq!(
        at(Rect::new(75, 24, 5, 1), BottomRight),
        Rect::new(37, 18, 43, 6)
    );
}

#[test]
fn keybind_help_lists_the_notification_center() {
    let (live, _) = Config::default().live_keybinds_with_diagnostics().unwrap();
    let groups = crate::input::keybind_help_groups(&live.keybinds, &live.prefix);
    let global = &groups.iter().find(|(name, _)| *name == "global").unwrap().1;

    assert!(global
        .iter()
        .any(|(key, label)| key == "prefix+ctrl+n" && label == "notification center"));
}
