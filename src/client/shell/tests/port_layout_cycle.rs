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

#[test]
fn next_layout_advertised_cycles_four_presets_and_confirms_with_toast() {
    use crate::api::schema::{AppliedLayoutPreset, LayoutPreset, Method};
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_endpoint_methods(Some(vec![
        "layout.set_preset".into(),
        "layout.apply_preset".into(),
    ]));
    let boot_id = state.snapshot.as_ref().unwrap().boot_id.clone();
    for (index, name) in [
        "even-horizontal",
        "even-vertical",
        "tiled",
        "main-vertical",
        "even-horizontal",
    ]
    .into_iter()
    .enumerate()
    {
        let mut outcome = ClientShellInput::default();
        state.cycle_layout(&mut outcome);
        let [ClientShellAction::Endpoint { request, .. }] = outcome.actions.as_slice() else {
            panic!("one endpoint action");
        };
        match index {
            0 | 4 => assert!(
                matches!(&request.method, Method::LayoutSetPreset(p) if p.preset == LayoutPreset::EvenHorizontal)
            ),
            1 => assert!(
                matches!(&request.method, Method::LayoutSetPreset(p) if p.preset == LayoutPreset::EvenVertical)
            ),
            2 => assert!(
                matches!(&request.method, Method::LayoutSetPreset(p) if p.preset == LayoutPreset::Tiled)
            ),
            3 => assert!(
                matches!(&request.method, Method::LayoutApplyPreset(p) if p.preset == AppliedLayoutPreset::MainVertical)
            ),
            _ => unreachable!(),
        }
        // No applied toast until the endpoint has confirmed success.
        assert!(state.visible_notification.is_none());
        state.handle_endpoint_result(
            &boot_id,
            &request.id,
            Ok(crate::api::schema::ResponseResult::LayoutPresetApplied {
                layout: crate::api::schema::LayoutDescription {
                    workspace_id: "w1".into(),
                    tab_id: "w1:t1".into(),
                    zoomed: false,
                    focused_pane_id: "w1:p1".into(),
                    root: crate::api::schema::LayoutNode::Pane {
                        pane: crate::api::schema::LayoutPane::default(),
                    },
                },
            }),
        );
        let toast = state
            .visible_notification
            .as_ref()
            .expect("applied feedback");
        assert_eq!(toast.event.title, "layout applied");
        assert_eq!(toast.event.body.as_deref(), Some(name));
        assert!(toast
            .deadline
            .is_some_and(|deadline| deadline > std::time::Instant::now()));
        state.visible_notification = None;
    }
}

#[test]
fn next_layout_explicit_old_server_stays_three_steps() {
    use crate::api::schema::{LayoutPreset, Method};
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_endpoint_methods(Some(vec!["layout.set_preset".into()]));
    for expected in [
        LayoutPreset::EvenHorizontal,
        LayoutPreset::EvenVertical,
        LayoutPreset::Tiled,
        LayoutPreset::EvenHorizontal,
    ] {
        let mut outcome = ClientShellInput::default();
        state.cycle_layout(&mut outcome);
        assert!(
            matches!(outcome.actions.as_slice(), [ClientShellAction::Endpoint { request, .. }]
            if matches!(&request.method, Method::LayoutSetPreset(p) if p.preset == expected))
        );
        assert!(state.visible_endpoint_notice.is_none());
    }
}

#[test]
fn next_layout_rejected_request_does_not_show_applied_feedback() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    let mut outcome = ClientShellInput::default();
    state.cycle_layout(&mut outcome);
    let [ClientShellAction::Endpoint { request, .. }] = outcome.actions.as_slice() else {
        panic!("one request");
    };
    let boot_id = state.snapshot.as_ref().unwrap().boot_id.clone();
    state.handle_endpoint_result(
        &boot_id,
        &request.id,
        Err(ClientShellEndpointError {
            code: Some("layout_not_found".into()),
            message: "tab disappeared".into(),
        }),
    );
    assert!(state.visible_notification.is_none());
    assert!(state.visible_endpoint_notice.is_some());
}
