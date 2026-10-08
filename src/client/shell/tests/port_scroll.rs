use super::*;
use crate::api::schema::{Method, PaneApplicationScrollIntent as Intent, ResponseResult};

fn state(effective: Option<&str>, display: Option<&str>) -> ClientShellState {
    let mut projected = snapshot();
    if effective.is_some() || display.is_some() {
        projected.agents.push(ClientShellAgent {
            pane_id: "pane_1".into(),
            workspace_id: "ws_1".into(),
            tab_id: "tab_1".into(),
            name: None,
            display_agent: display.map(str::to_owned),
            agent: effective.map(str::to_owned),
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            agent_status: AgentStatus::Idle,
            state_change_seq: 0,
            state_labels: Vec::new(),
            tokens: Vec::new(),
            focused: true,
        });
    }
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(projected));
    let mut pane_surface = surface();
    pane_surface.panes[0].alternate_screen_active = true;
    pane_surface.panes[0].scroll = Some(crate::protocol::PaneSurfaceScrollMetrics {
        offset_from_bottom: 0,
        max_offset_from_bottom: 0,
        viewport_rows: 2,
    });
    state.set_pane_surface(pane_surface);
    state.compose(106, 20).unwrap();
    state
}

fn key(state: &mut ClientShellState, code: KeyCode, modifiers: KeyModifiers) -> ClientShellInput {
    state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        code, modifiers,
    ))])
}

fn enter(state: &mut ClientShellState) -> ClientShellInput {
    key(state, KeyCode::Char('b'), KeyModifiers::CONTROL);
    let outcome = key(state, KeyCode::Char('u'), KeyModifiers::CONTROL);
    assert_eq!(state.mode, ClientShellMode::Scroll);
    outcome
}

fn scroll_request(actions: &[ClientShellAction]) -> (&str, Intent, u16) {
    let [ClientShellAction::Endpoint { request, .. }] = actions else {
        panic!("one isolated request");
    };
    let Method::PaneScrollApplication(params) = &request.method else {
        panic!("application scroll");
    };
    assert_eq!(params.pane_id, "pane_1");
    (&request.id, params.intent, params.count)
}

fn finish(state: &mut ClientShellState, actions: &[ClientShellAction]) -> Vec<ClientShellAction> {
    let (request_id, _, _) = scroll_request(actions);
    state
        .handle_endpoint_result("boot-1", request_id, Ok(ResponseResult::Ok {}))
        .1
}

#[test]
fn scroll_top_and_bottom_follow_effective_agent_precedence() {
    for (effective, display, claude) in [
        (Some("claude"), Some("codex"), true),
        (Some("codex"), Some("claude"), false),
        (None, Some("claude"), true),
        (Some("custom"), Some("claude"), true),
        (None, None, false),
    ] {
        let mut state = state(effective, display);
        let first = enter(&mut state);
        assert_eq!(scroll_request(&first.actions).1, Intent::PageUp);
        assert!(finish(&mut state, &first.actions).is_empty());
        for (code, modifiers, top) in [
            (KeyCode::Char('g'), KeyModifiers::empty(), true),
            (KeyCode::Home, KeyModifiers::empty(), true),
            (KeyCode::Char('G'), KeyModifiers::SHIFT, false),
            (KeyCode::Char('g'), KeyModifiers::SHIFT, false),
            (KeyCode::Char('g'), KeyModifiers::CONTROL, false),
            (KeyCode::End, KeyModifiers::empty(), false),
        ] {
            let outcome = key(&mut state, code, modifiers);
            let expected = match (claude, top) {
                (true, true) => Intent::CtrlHome,
                (true, false) => Intent::CtrlEnd,
                (false, true) => Intent::Home,
                (false, false) => Intent::End,
            };
            assert_eq!(scroll_request(&outcome.actions).1, expected);
            assert!(outcome.requests.is_empty(), "never use typed pane input");
            assert!(finish(&mut state, &outcome.actions).is_empty());
        }
    }
}

#[test]
fn held_scroll_entry_repeats_are_batched_behind_one_request() {
    let mut state = state(None, None);
    key(&mut state, KeyCode::Char('b'), KeyModifiers::CONTROL);
    let first = state.handle_raw_events(vec![RawInputEvent::Key(
        crate::input::TerminalKey::new(KeyCode::Char('u'), KeyModifiers::CONTROL)
            .with_repeat_count(4),
    )]);
    assert_eq!(scroll_request(&first.actions).2, 1);
    let repeated = state.handle_raw_events(vec![RawInputEvent::Key(
        crate::input::TerminalKey::new(KeyCode::Char('u'), KeyModifiers::CONTROL)
            .with_kind(crossterm::event::KeyEventKind::Repeat)
            .with_repeat_count(2),
    )]);
    assert!(repeated.actions.is_empty());
    assert!(repeated.requests.is_empty());
    let next = finish(&mut state, &first.actions);
    let (_, intent, count) = scroll_request(&next);
    assert_eq!((intent, count), (Intent::PageUp, 5));
    assert!(finish(&mut state, &next).is_empty());
}

#[test]
fn scroll_line_keys_are_wheel_intents_and_unrelated_input_is_swallowed() {
    let mut state = state(None, None);
    let first = enter(&mut state);
    finish(&mut state, &first.actions);
    for (code, modifiers, expected) in [
        (KeyCode::Up, KeyModifiers::empty(), Intent::WheelUp),
        (KeyCode::Down, KeyModifiers::empty(), Intent::WheelDown),
        (KeyCode::Char('k'), KeyModifiers::CONTROL, Intent::WheelUp),
        (KeyCode::Char('j'), KeyModifiers::CONTROL, Intent::WheelDown),
    ] {
        let outcome = key(&mut state, code, modifiers);
        assert_eq!(scroll_request(&outcome.actions).1, expected);
        assert!(outcome.requests.is_empty());
        finish(&mut state, &outcome.actions);
    }
    let unrelated = state.handle_raw_events(vec![
        RawInputEvent::Text(crate::input::TextCommit::new("text")),
        RawInputEvent::Paste("paste".into()),
        RawInputEvent::Key(crate::input::TerminalKey::new(
            KeyCode::Char('x'),
            KeyModifiers::empty(),
        )),
    ]);
    assert!(unrelated.requests.is_empty());
    assert!(unrelated.actions.is_empty());
    assert_eq!(state.mode, ClientShellMode::Scroll);
}

#[test]
fn scroll_exit_cancels_unsent_work_and_ignores_late_completion() {
    for exit in [KeyCode::Esc, KeyCode::Enter, KeyCode::Char('q')] {
        let mut state = state(None, None);
        let first = enter(&mut state);
        key(&mut state, KeyCode::PageDown, KeyModifiers::empty());
        let outcome = key(&mut state, exit, KeyModifiers::empty());
        assert_eq!(state.mode, ClientShellMode::Terminal);
        assert!(state.application_scroll.is_none());
        assert!(outcome.requests.is_empty());
        assert!(matches!(
            outcome.actions.as_slice(),
            [ClientShellAction::CancelQueuedEndpoint { .. }]
        ));
        assert!(finish(&mut state, &first.actions).is_empty());
    }
}

#[test]
fn scroll_focus_loss_and_target_changes_never_type_into_a_new_pane() {
    for change in ["focus", "screen", "outer"] {
        let mut state = state(None, None);
        let first = enter(&mut state);
        key(&mut state, KeyCode::PageDown, KeyModifiers::empty());
        let outcome = if change == "outer" {
            state.handle_raw_events(vec![RawInputEvent::OuterFocusLost])
        } else {
            if change == "focus" {
                let mut projected = state.snapshot.as_deref().unwrap().clone();
                let mut another = projected.panes[0].clone();
                another.pane_id = "pane_2".into();
                projected.panes.push(another);
                projected.focused_pane_id = Some("pane_2".into());
                state.set_snapshot(Box::new(projected));
            } else {
                let mut pane_surface = surface();
                pane_surface.panes[0].alternate_screen_active = false;
                state.set_pane_surface(pane_surface);
            }
            state.handle_raw_events(vec![RawInputEvent::Key(
                crate::input::TerminalKey::new(KeyCode::Char('u'), KeyModifiers::CONTROL)
                    .with_kind(crossterm::event::KeyEventKind::Repeat),
            )])
        };
        assert_eq!(state.mode, ClientShellMode::Terminal);
        assert!(state.application_scroll.is_none());
        assert!(outcome
            .requests
            .iter()
            .all(|request| !matches!(request, ClientMessage::ClientShellPaneInput { .. })));
        assert!(finish(&mut state, &first.actions).is_empty());
        let repeated = state.handle_raw_events(vec![RawInputEvent::Key(
            crate::input::TerminalKey::new(KeyCode::Char('u'), KeyModifiers::CONTROL)
                .with_kind(crossterm::event::KeyEventKind::Repeat),
        )]);
        assert!(repeated.requests.is_empty());
    }
}

#[test]
fn scroll_missing_method_leaves_other_actions_available() {
    let mut state = state(None, None);
    state.set_endpoint_methods(Some(
        crate::server::client_commands::supported_client_shell_method_names()
            .iter()
            .filter(|method| **method != "pane.scroll_application")
            .map(|method| (*method).to_owned())
            .collect(),
    ));
    key(&mut state, KeyCode::Char('b'), KeyModifiers::CONTROL);
    let outcome = key(&mut state, KeyCode::Char('u'), KeyModifiers::CONTROL);
    assert_eq!(state.mode, ClientShellMode::Terminal);
    assert!(state.application_scroll.is_none());
    assert!(outcome.actions.is_empty());
    assert!(state.visible_endpoint_notice.is_some());
    key(&mut state, KeyCode::Char('b'), KeyModifiers::CONTROL);
    key(&mut state, KeyCode::Char('['), KeyModifiers::empty());
    assert_eq!(state.mode, ClientShellMode::Copy);
}

#[test]
fn scroll_instruction_bar_matches_the_fork() {
    let config = ClientShellConfig::from_config(&Config::default());
    let mut buffer = Buffer::empty(Rect::new(0, 0, 120, 3));
    render::render_mode_bar(
        &mut buffer,
        Rect::new(0, 0, 120, 3),
        ClientShellMode::Scroll,
        None,
        None,
        false,
        &config.keybinds,
        &config.palette,
    )
    .unwrap();
    let text: String = (0..120).map(|x| buffer[(x, 2)].symbol()).collect();
    assert!(text.starts_with(
        " SCROLL  ^u/^d page  ^k/^j line  g/G top/bottom  q/esc exit  keys scroll the app"
    ));
    assert_eq!(buffer[(0, 2)].bg, config.palette.accent);
}

#[test]
fn scroll_projection_loss_cancels_even_when_focus_or_screen_comes_back() {
    for changed in ["focus", "screen"] {
        let mut state = state(None, None);
        let first = enter(&mut state);
        key(&mut state, KeyCode::PageDown, KeyModifiers::empty());
        if changed == "focus" {
            let before = state.snapshot.as_deref().unwrap().clone();
            let mut away = before.clone();
            let mut another = away.panes[0].clone();
            another.pane_id = "pane_2".into();
            away.panes.push(another);
            away.focused_pane_id = Some("pane_2".into());
            state.set_snapshot(Box::new(away));
            assert_eq!(state.mode, ClientShellMode::Terminal);
            state.set_snapshot(Box::new(before));
        } else {
            let mut lost = surface();
            lost.surface_revision = 2;
            lost.panes[0].alternate_screen_active = false;
            state.set_pane_surface(lost);
            assert_eq!(state.mode, ClientShellMode::Terminal);
            let mut returned = surface();
            returned.surface_revision = 3;
            returned.panes[0].alternate_screen_active = true;
            state.set_pane_surface(returned);
        }
        assert!(state.application_scroll.is_none());
        assert_eq!(state.take_application_scroll_cancellations().len(), 1);
        assert!(finish(&mut state, &first.actions).is_empty());
    }
}

#[test]
fn scroll_held_keys_stay_suppressed_after_boot_or_connection_reset() {
    for reset in ["boot", "generation"] {
        let mut state = state(None, None);
        let first = enter(&mut state);
        let mut projected = state.snapshot.as_deref().unwrap().clone();
        if reset == "boot" {
            projected.boot_id = "boot-2".into();
            state.set_snapshot(Box::new(projected));
        } else {
            state.set_endpoint_snapshot_for_generation(
                &ClientEndpointId::Local,
                2,
                Box::new(projected),
            );
        }
        assert_eq!(state.mode, ClientShellMode::Terminal);
        assert!(state.application_scroll.is_none());
        let repeated = state.handle_raw_events(vec![RawInputEvent::Key(
            crate::input::TerminalKey::new(KeyCode::Char('u'), KeyModifiers::CONTROL)
                .with_kind(crossterm::event::KeyEventKind::Repeat),
        )]);
        assert!(repeated.requests.is_empty());
        assert!(finish(&mut state, &first.actions).is_empty());
        state.handle_raw_events(vec![RawInputEvent::Key(
            crate::input::TerminalKey::new(KeyCode::Char('u'), KeyModifiers::CONTROL)
                .with_kind(crossterm::event::KeyEventKind::Release),
        )]);
        let fresh = key(&mut state, KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert!(matches!(
            fresh.requests.as_slice(),
            [ClientMessage::ClientShellPaneInput { .. }]
        ));
    }
}

#[test]
fn legacy_remapped_entry_keeps_fresh_mode_command_semantics() {
    let config: Config =
        toml::from_str("[keys]\ncopy_mode_half_page_up = \"prefix+ctrl+g\"\n").unwrap();
    let mut state = state(None, None);
    state.config.keybinds = config.live_keybinds_with_diagnostics().unwrap().0;
    key(&mut state, KeyCode::Char('b'), KeyModifiers::CONTROL);
    let first = key(&mut state, KeyCode::Char('g'), KeyModifiers::CONTROL);
    assert_eq!(scroll_request(&first.actions).1, Intent::PageUp);
    finish(&mut state, &first.actions);
    // Legacy terminals cannot distinguish a held repeat from a fresh press.
    // A new Press retains the fork's SCROLL chord, rather than stealing Ctrl-G.
    let fresh = key(&mut state, KeyCode::Char('g'), KeyModifiers::CONTROL);
    assert_eq!(scroll_request(&fresh.actions).1, Intent::End);
}
