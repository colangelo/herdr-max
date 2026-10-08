use super::*;

#[test]
fn out_of_range_agent_jump_logs_the_displayed_count_and_sends_no_input() {
    #[derive(Clone)]
    struct TraceWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for TraceWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    for mode in [
        ClientShellMode::Terminal,
        ClientShellMode::Prefix,
        ClientShellMode::Navigate,
    ] {
        let config: Config = toml::from_str(
            "[keys]\nswitch_tab = []\nswitch_workspace = []\nfocus_agent = [\"alt+1..9\", \"prefix+alt+1..9\"]\n",
        ).unwrap();
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
        state.set_snapshot(Box::new(snapshot()));
        state.mode = mode;
        let entries = super::super::aggregate_navigation::online_agent_targets(
            &state.endpoints,
            &state.active_endpoint_id,
            state.config.agent_panel_sort,
        )
        .len();
        assert!(entries < 8);
        let bytes = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let writer = bytes.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_ansi(false)
            .without_time()
            .with_writer(move || TraceWriter(writer.clone()))
            .finish();
        let outcome = tracing::subscriber::with_default(subscriber, || {
            state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
                KeyCode::Char('8'),
                KeyModifiers::ALT,
            ))])
        });
        assert!(outcome.requests.is_empty());
        assert!(outcome.actions.is_empty());
        assert_eq!(
            state.mode,
            if mode == ClientShellMode::Navigate {
                mode
            } else {
                ClientShellMode::Terminal
            }
        );
        let text = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
        assert!(
            text.contains("focus_agent: no agent panel entry at index"),
            "{mode:?}: {text}"
        );
        assert!(text.contains("idx=7"), "{text}");
        assert!(text.contains("jump_symbol="), "{text}");
        assert!(text.contains(&format!("entries={entries}")), "{text}");
    }
}

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
