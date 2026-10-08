use super::*;

#[test]
fn balance_panes_targets_the_client_tab_and_uses_its_effective_key() {
    for (configuration, key) in [
        ("", KeyCode::Char('=')),
        ("[keys]\nbalance_panes = \"prefix+f11\"\n", KeyCode::F(11)),
    ] {
        let config: Config = if configuration.is_empty() {
            Config::default()
        } else {
            toml::from_str(configuration).unwrap()
        };
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
        let mut projected = snapshot();
        projected.tabs[0].tab_id = "tab_client".into();
        projected.focused_tab_id = Some("tab_client".into());
        projected.workspaces[0].active_tab_id = "tab_client".into();
        projected.panes[0].tab_id = "tab_client".into();
        state.set_snapshot(Box::new(projected));
        state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
            KeyCode::Char('b'),
            KeyModifiers::CONTROL,
        ))]);
        let outcome = state.handle_raw_events(vec![RawInputEvent::Key(
            crate::input::TerminalKey::new(key, KeyModifiers::empty()),
        )]);
        assert_eq!(state.mode, ClientShellMode::Terminal);
        assert!(outcome.requests.is_empty());
        assert!(
            matches!(outcome.actions.as_slice(), [ClientShellAction::Endpoint { request, .. }]
            if matches!(&request.method, crate::api::schema::Method::LayoutBalance(params)
                if params.tab_id.as_deref() == Some("tab_client") && params.pane_id.is_none()))
        );
        let groups = crate::input::keybind_help_groups(
            &state.config.keybinds.keybinds,
            &state.config.keybinds.prefix,
        );
        let help = groups
            .iter()
            .flat_map(|(_, rows)| rows)
            .find(|(_, label)| label.as_ref() == "balance panes")
            .unwrap();
        assert_eq!(
            help.0,
            state
                .config
                .keybinds
                .keybinds
                .balance_panes
                .label()
                .unwrap()
        );
    }
}

#[test]
fn balance_unavailable_is_local_and_does_not_fall_through_to_typing() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_endpoint_methods(Some(vec!["pane.focus".into()]));
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::BalancePanes),
        &mut ClientShellInput::default(),
    );
    assert!(state.visible_endpoint_notice.is_some());
    assert_eq!(state.snapshot.as_deref().unwrap().boot_id, "boot-1");
}
