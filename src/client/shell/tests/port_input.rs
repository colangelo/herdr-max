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

fn copy_entry_state(config: &Config, offset: u64, max_offset: u64) -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(config));
    state.set_snapshot(Box::new(snapshot()));
    let mut projected = surface();
    projected.panes[0].scroll = Some(crate::protocol::PaneSurfaceScrollMetrics {
        offset_from_bottom: offset,
        max_offset_from_bottom: max_offset,
        viewport_rows: 2,
    });
    state.set_pane_surface(projected);
    state.compose(106, 20).unwrap();
    state
}

fn entry_prefix(state: &mut ClientShellState) {
    press(state, KeyCode::Char('b'), KeyModifiers::CONTROL);
    assert_eq!(state.mode, ClientShellMode::Prefix);
}

#[test]
fn prefix_page_up_enters_copy_mode_scrolled_and_repeats() {
    for (config, key) in [
        (Config::default(), KeyCode::PageUp),
        (
            toml::from_str("[keys]\ncopy_mode_page_up = \"prefix+f12\"\n").unwrap(),
            KeyCode::F(12),
        ),
    ] {
        let mut state = copy_entry_state(&config, 0, 20);
        entry_prefix(&mut state);
        let outcome = state.handle_raw_events(vec![RawInputEvent::Key(
            crate::input::TerminalKey::new(key, KeyModifiers::empty()).with_repeat_count(4),
        )]);
        assert_eq!(state.mode, ClientShellMode::Copy);
        assert_eq!(state.copy_mode.as_ref().unwrap().offset_from_bottom, 4);
        assert!(outcome.requests.is_empty());
        let outcome = state.handle_raw_events(vec![RawInputEvent::Key(
            crate::input::TerminalKey::new(key, KeyModifiers::empty())
                .with_kind(crossterm::event::KeyEventKind::Repeat)
                .with_repeat_count(2),
        )]);
        assert_eq!(state.copy_mode.as_ref().unwrap().offset_from_bottom, 6);
        assert!(outcome.requests.is_empty());
    }
}

#[test]
fn prefix_ctrl_u_and_ctrl_k_enter_copy_mode_scrolled() {
    for ch in ['u', 'k'] {
        let mut state = copy_entry_state(&Config::default(), 0, 20);
        entry_prefix(&mut state);
        press(&mut state, KeyCode::Char(ch), KeyModifiers::CONTROL);
        assert_eq!(state.mode, ClientShellMode::Copy);
        assert_eq!(state.copy_mode.as_ref().unwrap().offset_from_bottom, 1);
        assert_eq!(
            state.copy_mode.as_ref().unwrap().entry_offset_from_bottom,
            0
        );
    }
}

#[test]
fn prefix_ctrl_d_scrolls_an_open_copy_mode_back_down_without_losing_anchor() {
    let mut state = copy_entry_state(&Config::default(), 0, 20);
    entry_prefix(&mut state);
    press(&mut state, KeyCode::PageUp, KeyModifiers::empty());
    press(&mut state, KeyCode::Char('v'), KeyModifiers::empty());
    let anchor = state.copy_mode.as_ref().unwrap().selection.clone();
    entry_prefix(&mut state);
    press(&mut state, KeyCode::Char('d'), KeyModifiers::CONTROL);
    assert_eq!(state.mode, ClientShellMode::Copy);
    let copy = state.copy_mode.as_ref().unwrap();
    assert_eq!(copy.offset_from_bottom, 0);
    assert_eq!(copy.selection, anchor);
    assert_eq!(copy.entry_offset_from_bottom, 0);
}

#[test]
fn downward_gestures_do_not_open_copy_mode_on_an_unscrolled_pane() {
    for (key, modifiers) in [
        (KeyCode::PageDown, KeyModifiers::empty()),
        (KeyCode::Char('d'), KeyModifiers::CONTROL),
        (KeyCode::Char('j'), KeyModifiers::CONTROL),
    ] {
        let mut state = copy_entry_state(&Config::default(), 0, 20);
        entry_prefix(&mut state);
        let outcome = press(&mut state, key, modifiers);
        assert_eq!(state.mode, ClientShellMode::Terminal);
        assert!(state.copy_mode.is_none());
        assert!(outcome.requests.is_empty());
        assert!(outcome.actions.is_empty());
    }
}

#[test]
fn prefix_page_up_without_scrollback_still_enters_copy_mode() {
    let mut state = copy_entry_state(&Config::default(), 0, 0);
    entry_prefix(&mut state);
    press(&mut state, KeyCode::PageUp, KeyModifiers::empty());
    assert_eq!(state.mode, ClientShellMode::Copy);
    assert_eq!(state.copy_mode.as_ref().unwrap().offset_from_bottom, 0);
}

#[test]
fn scroll_entry_repeats_do_not_cross_a_prefix_transition_or_pane_change() {
    let mut state = copy_entry_state(&Config::default(), 0, 20);
    entry_prefix(&mut state);
    press(&mut state, KeyCode::PageUp, KeyModifiers::empty());
    entry_prefix(&mut state);
    let repeat = || {
        crate::input::TerminalKey::new(KeyCode::PageUp, KeyModifiers::empty())
            .with_kind(crossterm::event::KeyEventKind::Repeat)
    };
    state.handle_raw_events(vec![RawInputEvent::Key(repeat())]);
    assert_eq!(state.mode, ClientShellMode::Prefix);
    assert_eq!(state.copy_mode.as_ref().unwrap().offset_from_bottom, 1);
    press(&mut state, KeyCode::Esc, KeyModifiers::empty());
    state.handle_raw_events(vec![RawInputEvent::Key(repeat())]);
    assert_eq!(
        state.copy_mode.as_ref().unwrap().offset_from_bottom,
        1,
        "returning to Copy does not resurrect a suppressed lease"
    );
}

#[test]
fn the_downward_scroll_gestures_are_discoverable_in_the_help_panel() {
    let config = Config::default();
    let keybinds = config.keybinds();
    let groups = crate::input::keybind_help_groups(&keybinds, &config.prefix_keys());
    for (label, binding) in [
        ("scroll page down", "prefix+pagedown"),
        ("scroll half page down", "prefix+ctrl+d"),
        ("scroll line down", "prefix+ctrl+j"),
    ] {
        let entry = groups
            .iter()
            .flat_map(|(_, entries)| entries)
            .find(|(_, actual)| actual.as_ref() == label)
            .unwrap();
        assert_eq!(entry.0, binding);
    }
}
