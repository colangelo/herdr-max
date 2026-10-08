use super::*;
use crossterm::event::{KeyCode, KeyModifiers};

fn press(state: &mut ClientShellState, code: KeyCode, modifiers: KeyModifiers) -> ClientShellInput {
    state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        code, modifiers,
    ))])
}

#[test]
fn ac_prefix_list_parses_and_both_prefixes_own_the_command() {
    let config: Config = toml::from_str("[keys]\nprefix = [\"ctrl+s\", \"ctrl+;\"]\n").unwrap();
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    state.set_snapshot(Box::new(snapshot()));

    for ch in ['s', ';'] {
        let outcome = press(&mut state, KeyCode::Char(ch), KeyModifiers::CONTROL);
        assert_eq!(state.mode, ClientShellMode::Prefix);
        assert!(outcome.requests.is_empty());
        assert!(outcome.actions.is_empty());
        let outcome = press(&mut state, KeyCode::Char('?'), KeyModifiers::empty());
        assert!(matches!(state.overlay, Some(ClientShellOverlay::Help(_))));
        assert!(outcome.requests.is_empty());
        state.overlay = None;
    }
}

#[test]
fn letter_range_indexed_bindings_dispatch_past_nine_and_compress_in_help() {
    let config: Config =
        toml::from_str("[keys]\nswitch_workspace = [\"alt+1..9\", \"alt+a..z\"]\n").unwrap();
    let mut projected = snapshot();
    projected.workspaces = (1..=12)
        .map(|number| {
            let mut workspace = projected.workspaces[0].clone();
            workspace.workspace_id = format!("ws_{number}");
            workspace.number = number;
            workspace.focused = number == 1;
            workspace
        })
        .collect();
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    state.set_snapshot(Box::new(projected));

    let outcome = press(&mut state, KeyCode::Char('a'), KeyModifiers::ALT);
    assert!(
        outcome.requests.is_empty(),
        "the shortcut is never pane input"
    );
    assert!(matches!(
        outcome.actions.as_slice(),
        [ClientShellAction::Endpoint { request, .. }]
            if matches!(&request.method,
                crate::api::schema::Method::WorkspaceFocus(target)
                if target.workspace_id == "ws_10")
    ));
    let groups = crate::input::keybind_help_groups(
        &state.config.keybinds.keybinds,
        &state.config.keybinds.prefix,
    );
    let binding = groups
        .iter()
        .flat_map(|(_, entries)| entries)
        .find(|(_, label)| label.as_ref() == "switch workspace 1-9")
        .unwrap();
    assert_eq!(binding.0, "alt+1..9 / alt+a..z");
}
