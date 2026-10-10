use super::*;

#[test]
fn sync_binding_uses_the_client_tab_and_effective_help() {
    for (configuration, code, modifiers, prefix) in [
        ("", KeyCode::Char('S'), KeyModifiers::SHIFT, true),
        ("", KeyCode::Char('s'), KeyModifiers::SHIFT, true),
        (
            "[keys]\ntoggle_sync_panes = \"prefix+f11\"\n",
            KeyCode::F(11),
            KeyModifiers::empty(),
            true,
        ),
        (
            "[keys]\ntoggle_sync_panes = \"alt+f11\"\n",
            KeyCode::F(11),
            KeyModifiers::ALT,
            false,
        ),
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
        if prefix {
            state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
                KeyCode::Char('b'),
                KeyModifiers::CONTROL,
            ))]);
        }
        let outcome = state.handle_raw_events(vec![RawInputEvent::Key(
            crate::input::TerminalKey::new(code, modifiers).with_repeat_count(3),
        )]);
        assert_eq!(state.mode, ClientShellMode::Terminal);
        assert!(outcome.requests.is_empty());
        assert!(
            matches!(outcome.actions.as_slice(), [ClientShellAction::Endpoint { request, .. }]
            if matches!(&request.method, crate::api::schema::Method::TabSync(params)
                if params.tab_id.as_deref() == Some("tab_client")
                && params.mode == crate::api::schema::SyncMode::Toggle))
        );
        for kind in [
            crossterm::event::KeyEventKind::Repeat,
            crossterm::event::KeyEventKind::Release,
        ] {
            let held = state.handle_raw_events(vec![RawInputEvent::Key(
                crate::input::TerminalKey::new(code, modifiers).with_kind(kind),
            )]);
            assert!(held.requests.is_empty());
            assert!(
                held.actions.is_empty(),
                "{configuration} {kind:?}: {:?}",
                held.actions
            );
        }
        let groups = crate::input::keybind_help_groups(
            &state.config.keybinds.keybinds,
            &state.config.keybinds.prefix,
        );
        let help = groups
            .iter()
            .flat_map(|(_, rows)| rows)
            .find(|(_, label)| label.as_ref() == "sync panes")
            .unwrap();
        assert_eq!(
            help.0,
            state
                .config
                .keybinds
                .keybinds
                .toggle_sync_panes
                .label()
                .unwrap()
        );
    }
}

#[test]
fn unbound_sync_remains_discoverable() {
    let config: Config = toml::from_str("[keys]\ntoggle_sync_panes = []\n").unwrap();
    let state = ClientShellState::new(ClientShellConfig::from_config(&config));
    let groups = crate::input::keybind_help_groups(
        &state.config.keybinds.keybinds,
        &state.config.keybinds.prefix,
    );
    assert!(groups
        .iter()
        .flat_map(|(_, rows)| rows)
        .any(|(binding, label)| binding == "unset" && label.as_ref() == "sync panes"));
}

#[test]
fn missing_sync_method_is_local_and_the_command_does_not_type() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_endpoint_methods(Some(vec!["pane.focus".into()]));
    state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        KeyCode::Char('b'),
        KeyModifiers::CONTROL,
    ))]);
    let outcome = state.handle_raw_events(vec![RawInputEvent::Key(
        crate::input::TerminalKey::new(KeyCode::Char('S'), KeyModifiers::SHIFT),
    )]);
    assert!(outcome.requests.is_empty());
    assert!(outcome.actions.is_empty());
    assert!(state.visible_endpoint_notice.is_some());
    assert_eq!(state.snapshot.as_deref().unwrap().boot_id, "boot-1");
    state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        KeyCode::Char('x'),
        KeyModifiers::empty(),
    ))]);
    assert!(state.visible_endpoint_notice.is_some());
}

// Fork issue 202: while the focused tab syncs, toggle_pane_sync (ctrl+space)
// takes the focused pane out of the group or puts it back.
fn syncing_snapshot(members: &[&str], ending: bool) -> ClientShellSnapshot {
    let mut projected = snapshot();
    let tab = projected.focused_tab_id.clone().unwrap();
    projected.resource_facts = Some(crate::protocol::ClientShellResourceFacts {
        tab_sync: Some(
            [(
                tab,
                crate::protocol::ClientTabSync {
                    members: members.iter().map(|member| (*member).to_owned()).collect(),
                    ending,
                },
            )]
            .into(),
        ),
        ..Default::default()
    });
    projected
}

fn ctrl_space() -> RawInputEvent {
    RawInputEvent::Key(crate::input::TerminalKey::new(
        KeyCode::Char(' '),
        KeyModifiers::CONTROL,
    ))
}

#[test]
fn pane_sync_key_toggles_the_focused_pane_while_the_tab_syncs() {
    // In the group, and in the grace of an emptied group: both toggle.
    for (members, ending) in [(&["pane_1"][..], false), (&[][..], true)] {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        state.set_snapshot(Box::new(syncing_snapshot(members, ending)));
        let outcome = state.handle_raw_events(vec![ctrl_space()]);
        assert!(outcome.requests.is_empty(), "{:?}", outcome.requests);
        assert!(
            matches!(outcome.actions.as_slice(), [ClientShellAction::Endpoint { request, .. }]
            if matches!(&request.method, crate::api::schema::Method::PaneSync(params)
                if params.pane_id.as_deref() == Some("pane_1")
                && params.mode == crate::api::schema::SyncMode::Toggle)),
            "{members:?} {ending}: {:?}",
            outcome.actions
        );
        assert_eq!(state.mode, ClientShellMode::Terminal);
        // A held key must not flip the pane back.
        let held = state.handle_raw_events(vec![RawInputEvent::Key(
            crate::input::TerminalKey::new(KeyCode::Char(' '), KeyModifiers::CONTROL)
                .with_kind(crossterm::event::KeyEventKind::Repeat),
        )]);
        assert!(held.actions.is_empty() && held.requests.is_empty());
    }
}

#[test]
fn pane_sync_key_goes_to_the_pane_when_the_tab_does_not_sync() {
    let mut other_tab = syncing_snapshot(&["pane_9"], false);
    let facts = other_tab.resource_facts.as_mut().unwrap();
    let sync = facts.tab_sync.take().unwrap().into_values().next().unwrap();
    facts.tab_sync = Some([("tab_other".to_owned(), sync)].into());
    for projected in [snapshot(), other_tab] {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        state.set_snapshot(Box::new(projected));
        let outcome = state.handle_raw_events(vec![ctrl_space()]);
        assert!(outcome.actions.is_empty(), "{:?}", outcome.actions);
        assert!(
            matches!(
                &outcome.requests[..],
                [ClientMessage::ClientShellPaneInput { events, .. }]
                    if matches!(
                        &events[..],
                        [ClientPaneInputEvent::Key {
                            code: crate::protocol::ClientKeyCode::Char(' '),
                            modifiers,
                            ..
                        }] if *modifiers == KeyModifiers::CONTROL.bits()
                    )
            ),
            "{:?}",
            outcome.requests
        );
    }
}

#[test]
fn pane_sync_key_is_in_the_help_panel() {
    for (configuration, label) in [
        ("", "ctrl+space"),
        ("[keys]\ntoggle_pane_sync = []\n", "unset"),
    ] {
        let config: Config = toml::from_str(configuration).unwrap();
        let state = ClientShellState::new(ClientShellConfig::from_config(&config));
        let groups = crate::input::keybind_help_groups(
            &state.config.keybinds.keybinds,
            &state.config.keybinds.prefix,
        );
        assert!(
            groups
                .iter()
                .flat_map(|(_, rows)| rows)
                .any(|(binding, row)| {
                    binding == label && row.as_ref() == "pane in/out of sync (sync mode)"
                }),
            "{configuration}"
        );
    }
}
