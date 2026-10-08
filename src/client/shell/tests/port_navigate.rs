use super::*;

#[test]
fn ctrl_j_and_ctrl_k_move_the_navigator_in_both_states() {
    for search_focused in [false, true] {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        let mut projected = snapshot();
        let mut another = projected.panes[0].clone();
        another.pane_id = "pane_2".into();
        another.focused = false;
        projected.panes.push(another);
        state.set_snapshot(Box::new(projected));
        state.open_navigator_overlay();
        let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() else {
            panic!("navigator");
        };
        navigator.search_focused = search_focused;
        navigator.query = "client-shell".into();
        let rows =
            render::client_navigator_rows(&state.endpoints, &state.active_endpoint_id, navigator);
        assert!(rows.len() > 1);
        navigator.selected = Some(rows[0].target.clone());

        for (ch, expected) in [('j', &rows[1].target), ('k', &rows[0].target)] {
            let outcome = state.handle_raw_events(vec![RawInputEvent::Key(
                crate::input::TerminalKey::new(KeyCode::Char(ch), KeyModifiers::CONTROL),
            )]);
            let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_ref() else {
                panic!("navigator retained");
            };
            assert_eq!(navigator.selected.as_ref(), Some(expected));
            assert_eq!(navigator.query.as_str(), "client-shell");
            assert!(outcome.requests.is_empty());
            assert!(outcome.actions.is_empty());
        }
    }
}

#[test]
fn modified_navigator_arrows_and_unbound_chords_do_not_move() {
    for search_focused in [false, true] {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        state.set_snapshot(Box::new(snapshot()));
        state.open_navigator_overlay();
        let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_mut() else {
            panic!("navigator");
        };
        navigator.search_focused = search_focused;
        let before = navigator.selected.clone();
        for code in [
            KeyCode::Char('j'),
            KeyCode::Char('k'),
            KeyCode::Up,
            KeyCode::Down,
        ] {
            state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
                code,
                KeyModifiers::ALT,
            ))]);
            let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_ref() else {
                panic!("navigator retained");
            };
            assert_eq!(navigator.selected, before);
        }
    }
}
