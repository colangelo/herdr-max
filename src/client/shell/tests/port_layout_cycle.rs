use super::*;

#[test]
fn next_layout_cycles_presets_for_the_client_tab_and_exits_navigate() {
    use crate::api::schema::LayoutPreset;
    for (configuration, key) in [
        ("", KeyCode::Char(' ')),
        ("[keys]\nnext_layout = \"prefix+f11\"\n", KeyCode::F(11)),
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
        for expected in [
            LayoutPreset::EvenHorizontal,
            LayoutPreset::EvenVertical,
            LayoutPreset::Tiled,
            LayoutPreset::EvenHorizontal,
        ] {
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
                if matches!(&request.method, crate::api::schema::Method::LayoutSetPreset(params)
                    if params.tab_id.as_deref() == Some("tab_client")
                        && params.pane_id.is_none() && params.preset == expected))
            );
        }
        let groups = crate::input::keybind_help_groups(
            &state.config.keybinds.keybinds,
            &state.config.keybinds.prefix,
        );
        let help = groups
            .iter()
            .flat_map(|(_, rows)| rows)
            .find(|(_, label)| label.as_ref() == "cycle layout")
            .unwrap();
        assert_eq!(
            help.0,
            state.config.keybinds.keybinds.next_layout.label().unwrap()
        );
    }
}

#[test]
fn next_layout_unavailable_preserves_the_cycle_cursor() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_endpoint_methods(Some(vec!["pane.focus".into()]));
    let mut outcome = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::NextLayout),
        &mut outcome,
    );
    assert!(outcome.actions.is_empty());
    assert!(state.visible_endpoint_notice.is_some());
    assert_eq!(state.layout_cycle_index, 0);
    state.set_endpoint_methods(None);
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::NextLayout),
        &mut outcome,
    );
    assert_eq!(state.layout_cycle_index, 1);
    assert!(
        matches!(outcome.actions.as_slice(), [ClientShellAction::Endpoint { request, .. }]
        if matches!(&request.method, crate::api::schema::Method::LayoutSetPreset(params)
            if params.preset == crate::api::schema::LayoutPreset::EvenHorizontal))
    );
}
