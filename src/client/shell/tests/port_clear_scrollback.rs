use super::*;

#[test]
fn clear_scrollback_is_discoverable_and_unbound_by_default() {
    let config = Config::default();
    let state = ClientShellState::new(ClientShellConfig::from_config(&config));
    assert!(state
        .config
        .keybinds
        .keybinds
        .clear_scrollback
        .label()
        .is_none());
    let groups = crate::input::keybind_help_groups(
        &state.config.keybinds.keybinds,
        &state.config.keybinds.prefix,
    );
    let help = groups
        .iter()
        .flat_map(|(_, rows)| rows)
        .find(|(_, label)| label.as_ref() == "clear scrollback")
        .unwrap();
    assert_eq!(help.0, "unset");
}

#[test]
fn clear_scrollback_binding_preserves_the_distinct_clear_screen_action() {
    for (configuration, code, modifiers) in [
        (
            "[keys]\nclear_scrollback = \"prefix+f11\"\n",
            KeyCode::F(11),
            KeyModifiers::empty(),
        ),
        (
            "[keys]\nclear_scrollback = \"alt+f11\"\n",
            KeyCode::F(11),
            KeyModifiers::ALT,
        ),
    ] {
        let config: Config = toml::from_str(configuration).unwrap();
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
        state.set_snapshot(Box::new(snapshot()));
        if modifiers.is_empty() {
            state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
                KeyCode::Char('b'),
                KeyModifiers::CONTROL,
            ))]);
        }
        let outcome = state.handle_raw_events(vec![RawInputEvent::Key(
            crate::input::TerminalKey::new(code, modifiers),
        )]);
        assert!(outcome.requests.is_empty());
        assert!(
            matches!(outcome.actions.as_slice(), [ClientShellAction::Endpoint { request, .. }]
            if matches!(&request.method, crate::api::schema::Method::PaneClearScrollback(params)
                if params.pane_id == "pane_1"))
        );
        assert!(matches!(
            state.endpoint_method_for_action(crate::input::KeybindAction::ClearPane),
            Some(crate::api::schema::Method::PaneClear(_))
        ));
    }
}

#[test]
fn missing_clear_scrollback_method_keeps_the_client_connected() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_endpoint_methods(Some(vec!["pane.clear".into()]));
    let mut outcome = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::ClearScrollback),
        &mut outcome,
    );
    assert!(outcome.actions.is_empty());
    assert!(state.visible_endpoint_notice.is_some());
    assert_eq!(state.snapshot.as_deref().unwrap().boot_id, "boot-1");
}
