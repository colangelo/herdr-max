//! Info mode (prefix+i, alt+i) and resize mode (prefix+r) share one overlay: the same number,
//! size and version label on every pane, and only the colour tells them apart (fork issue 331).
//! https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/331
use super::*;
use crate::client::frame_output::ComposedFrame;
use crossterm::event::{KeyEventKind, KeyModifiers};

const PANE_WIDTH: u16 = 48;

/// Three side-by-side panes of the server's layout order, `pane_2` focused, on a 200x30 window.
fn three_panes() -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_endpoint_server_version(&ClientEndpointId::Local, Some("0.11.0".into()));
    let mut snapshot = snapshot();
    let pane = snapshot.panes[0].clone();
    snapshot.panes = (1..=3)
        .map(|index| ClientShellPane {
            pane_id: format!("pane_{index}"),
            focused: index == 2,
            ..pane.clone()
        })
        .collect();
    snapshot.focused_pane_id = Some("pane_2".into());
    state.set_snapshot(Box::new(snapshot));
    let mut surface = surface();
    let pane = surface.panes[0].clone();
    surface.frame = FrameData::from_ratatui_buffer_with_hyperlinks(
        &Buffer::empty(Rect::new(0, 0, PANE_WIDTH * 3, 24)),
        None,
        &[],
    );
    surface.panes = (1..=3)
        .map(|index| {
            let rect = SurfaceRect {
                x: (index - 1) * PANE_WIDTH,
                y: 0,
                width: PANE_WIDTH,
                height: 24,
            };
            PaneSurfacePane {
                pane_id: format!("pane_{index}"),
                rect,
                inner_rect: rect,
                focused: index == 2,
                ..pane.clone()
            }
        })
        .collect();
    state.set_pane_surface(surface);
    state.compose(200, 30).unwrap();
    state
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> RawInputEvent {
    RawInputEvent::Key(crate::input::TerminalKey::new(code, modifiers))
}

fn press(state: &mut ClientShellState, c: char) -> ClientShellInput {
    state.handle_raw_events(vec![key(KeyCode::Char(c), KeyModifiers::NONE)])
}

fn focus_target(input: &ClientShellInput) -> Option<&str> {
    match &input.actions[..] {
        [ClientShellAction::Endpoint { request, .. }] => match &request.method {
            crate::api::schema::Method::PaneFocus(target) => Some(target.pane_id.as_str()),
            _ => None,
        },
        _ => None,
    }
}

fn resize_target(input: &ClientShellInput) -> Option<(&str, crate::api::schema::PaneDirection)> {
    match &input.actions[..] {
        [ClientShellAction::Endpoint { request, .. }] => match &request.method {
            crate::api::schema::Method::PaneResize(params) => {
                Some((params.pane_id.as_deref()?, params.direction))
            }
            _ => None,
        },
        _ => None,
    }
}

/// Rows of the composed frame that carry a label, as plain text.
fn label_rows(state: &mut ClientShellState) -> (ComposedFrame, Vec<String>) {
    let frame = state.compose(200, 30).unwrap();
    let rows = frame_rows(&frame);
    (frame, rows)
}

#[test]
fn info_and_resize_overlays_both_show_number_size_and_version_per_pane() {
    let mut state = three_panes();
    // The unified overlay's text capture, printed for the PR (cargo test -- --nocapture).
    for mode in ["info", "resize"] {
        if mode == "info" {
            state.enter_info_mode(std::time::Instant::now());
        } else {
            state.enter_resize_mode();
        }
        let (_, rows) = label_rows(&mut state);
        let text = rows.join("\n");
        for pane in 1..=3 {
            // Each pane's label: the number, then the 48x24 size chip and the version.
            let label = format!("{pane}  terminal   48x24   0.11.0  pane_{pane}");
            assert!(
                text.contains(&label),
                "{mode}: pane {pane} label {label:?}:\n{text}"
            );
        }
        println!("--- {mode} mode ---");
        for row in rows.iter().take(14) {
            println!("{}", row.trim_end());
        }
        let bar = rows.iter().find(|row| row.contains("window")).unwrap();
        println!("{}", bar.trim_end());
    }
}

#[test]
fn alt_i_opens_info_mode_without_the_prefix_and_the_help_panel_lists_it() {
    let mut state = three_panes();
    assert!(!state.pane_labels_visible());
    state.handle_raw_events(vec![key(KeyCode::Char('i'), KeyModifiers::ALT)]);
    assert!(state.pane_labels_explicit, "alt+i opens info mode");
    assert_eq!(
        state.mode,
        ClientShellMode::Terminal,
        "no prefix was pressed"
    );
    assert!(state.pane_labels_until.is_some());

    let (live, _) = Config::default().live_keybinds_with_diagnostics().unwrap();
    let groups = crate::input::keybind_help_groups(&live.keybinds, &live.prefix);
    let panes = &groups.iter().find(|(name, _)| *name == "panes").unwrap().1;
    assert!(
        panes
            .iter()
            .any(|(key, label)| key == "alt+i" && label.contains("info mode")),
        "{panes:?}"
    );
    // prefix+i still works.
    let mut state = three_panes();
    state.handle_input_bytes(&[0x02]);
    state.handle_input_bytes(b"i");
    assert!(state.pane_labels_explicit);
}

#[test]
fn an_empty_info_mode_binding_gives_alt_i_back_to_the_pane() {
    // The documented way to unbind it (keyboard.mdx): info_mode = "".
    let config: Config = toml::from_str("[keys]\ninfo_mode = \"\"").unwrap();
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.handle_raw_events(vec![key(KeyCode::Char('i'), KeyModifiers::ALT)]);
    assert!(!state.pane_labels_visible(), "alt+i is the pane's again");
}

#[test]
fn r_in_info_mode_enters_resize_mode_and_resizes_the_focused_pane() {
    let mut state = three_panes();
    state.enter_info_mode(std::time::Instant::now());
    press(&mut state, 'r');
    assert_eq!(state.mode, ClientShellMode::Resize);
    assert!(!state.pane_labels_explicit, "info mode is over");
    assert!(state.pane_labels_until.is_none(), "resize has no deadline");
    let resized = press(&mut state, 'l');
    assert_eq!(
        resize_target(&resized),
        Some(("pane_2", crate::api::schema::PaneDirection::Right))
    );
}

#[test]
fn a_digit_in_resize_mode_focuses_that_pane_and_the_next_resize_acts_on_it() {
    let mut state = three_panes();
    state.enter_resize_mode();
    let focus = press(&mut state, '3');
    assert_eq!(focus_target(&focus), Some("pane_3"));
    assert_eq!(state.mode, ClientShellMode::Resize, "resize stays on");
    // The snapshot has not caught up yet: the resize still goes to the new pane.
    let resized = press(&mut state, 'l');
    assert_eq!(
        resize_target(&resized),
        Some(("pane_3", crate::api::schema::PaneDirection::Right))
    );
    // Once the server's snapshot shows the focus, it is the snapshot's again.
    let mut moved = snapshot();
    moved.panes = state.snapshot.as_ref().unwrap().panes.clone();
    moved.focused_pane_id = Some("pane_3".into());
    moved.revision += 1;
    state.set_snapshot(Box::new(moved));
    assert!(state.resize_focus.is_none());
    let resized = press(&mut state, 'k');
    assert_eq!(
        resize_target(&resized),
        Some(("pane_3", crate::api::schema::PaneDirection::Up))
    );
    assert_eq!(state.mode, ClientShellMode::Resize);
}

#[test]
fn i_in_resize_mode_goes_to_info_mode_which_expires_while_resize_never_does() {
    let mut state = three_panes();
    let now = std::time::Instant::now();
    // Resize alone never expires, however long it sits and whatever the tick says.
    state.enter_resize_mode();
    let later = now + state.config.display_panes_duration * 10;
    assert!(!state.tick_pane_labels(later));
    assert_eq!(state.mode, ClientShellMode::Resize);
    assert!(state.pane_labels_visible());

    press(&mut state, 'i');
    assert_eq!(state.mode, ClientShellMode::Terminal, "resize is left");
    assert!(state.pane_labels_explicit, "info mode is on");
    let deadline = state.pane_labels_until.expect("info mode has a deadline");
    assert!(!state.tick_pane_labels(deadline - std::time::Duration::from_millis(1)));
    assert!(state.pane_labels_explicit);
    assert!(state.tick_pane_labels(deadline), "info mode times out");
    assert!(!state.pane_labels_visible());
    // The resize keys still end resize mode as before.
    state.enter_resize_mode();
    state.handle_raw_events(vec![key(KeyCode::Esc, KeyModifiers::NONE)]);
    assert_eq!(state.mode, ClientShellMode::Terminal);
    assert!(!state.pane_labels_visible());
}

#[test]
fn the_resize_overlay_uses_the_violet_token_and_info_keeps_red() {
    let mut state = three_panes();
    let p = state.config.palette.clone();
    assert_ne!(p.resize_violet(), p.mauve, "the violet is its own colour");
    let saturation = |colour: ratatui::style::Color| match colour {
        ratatui::style::Color::Rgb(r, g, b) => {
            let max = r.max(g).max(b);
            let min = r.min(g).min(b);
            f32::from(max - min) / f32::from(max)
        }
        _ => 0.0,
    };
    assert!(saturation(p.resize_violet()) > saturation(p.mauve));
    let code = crate::protocol::color_to_u32;

    state.enter_resize_mode();
    let (frame, _) = label_rows(&mut state);
    let (x, y) = cell_symbol_position(&frame, Rect::new(0, 0, 200, 30), " 48x24 ");
    let cell = &frame.cells[usize::from(y) * 200 + usize::from(x)];
    assert_eq!(cell.bg, code(p.resize_violet()), "size chip");
    let (x, y) = cell_symbol_position(&frame, Rect::new(0, 0, 200, 30), " RESIZE ");
    assert_eq!(
        frame.cells[usize::from(y) * 200 + usize::from(x)].bg,
        code(p.resize_violet())
    );

    state.enter_info_mode(std::time::Instant::now());
    let (frame, _) = label_rows(&mut state);
    let (x, y) = cell_symbol_position(&frame, Rect::new(0, 0, 200, 30), " 48x24 ");
    assert_eq!(
        frame.cells[usize::from(y) * 200 + usize::from(x)].bg,
        code(p.red),
        "info mode stays red"
    );
}

#[test]
fn key_releases_do_not_switch_modes() {
    let mut state = three_panes();
    state.enter_info_mode(std::time::Instant::now());
    state.handle_raw_events(vec![RawInputEvent::Key(
        crate::input::TerminalKey::new(KeyCode::Char('r'), KeyModifiers::NONE)
            .with_kind(KeyEventKind::Release),
    )]);
    assert!(state.pane_labels_explicit);
    assert_eq!(state.mode, ClientShellMode::Terminal);
}
