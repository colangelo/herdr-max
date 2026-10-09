//! The command palette (fork issue 182, part 1): how it opens, what it lists,
//! how it is drawn at 80 columns, and that `enter` runs a command through the
//! path its key takes.

use super::super::palette::{ClientPaletteOverlay, PaletteItem, PaletteLayout};
use super::*;
use crate::api::schema::{
    AgentSendBusy, AgentSendMode, AgentSendNote, AgentStatus, Method, ResponseResult,
};
use crate::input::KeybindAction;
use crate::protocol::ClientShellAgent;

const W: u16 = 80;
const H: u16 = 25;

fn state() -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.compose(W, H).unwrap();
    state
}

fn open(state: &mut ClientShellState) {
    state.handle_input_bytes(b"\x02:");
    state.compose(W, H).unwrap();
}

fn palette(state: &ClientShellState) -> &ClientPaletteOverlay {
    match state.overlay.as_ref() {
        Some(ClientShellOverlay::Palette(palette)) => palette,
        other => panic!("expected the command palette, got {other:?}"),
    }
}

fn layout(state: &ClientShellState) -> PaletteLayout {
    state.hits.palette.clone().expect("palette layout")
}

fn rows(state: &mut ClientShellState) -> Vec<String> {
    frame_rows(&state.compose(W, H).expect("frame"))
}

/// The bar's box: its columns of its rows.
fn boxed(state: &mut ClientShellState) -> Vec<String> {
    let outer = layout(state).outer;
    rows(state)
        .iter()
        .skip(usize::from(outer.y))
        .take(usize::from(outer.height))
        .map(|row| {
            row.chars()
                .skip(usize::from(outer.x))
                .take(usize::from(outer.width))
                .collect()
        })
        .collect()
}

fn type_text(state: &mut ClientShellState, text: &str) {
    state.handle_input_bytes(text.as_bytes());
    state.compose(W, H).unwrap();
}

fn bind(state: &mut ClientShellState, action: KeybindAction) -> ClientShellInput {
    let mut outcome = ClientShellInput::default();
    state.record_binding(crate::input::KeybindMatch::Action(action), &mut outcome);
    outcome
}

#[test]
fn prefix_colon_opens_the_bar_and_esc_closes_it() {
    let mut state = state();
    open(&mut state);
    assert!(palette(&state).shown() > 40, "every command is listed");
    assert_eq!(palette(&state).query.text(), "");

    state.handle_input_bytes(b"\x1b");
    assert!(state.overlay.is_none());
    assert_eq!(state.mode, ClientShellMode::Terminal);
}

#[test]
fn the_bar_never_lists_itself() {
    let mut state = state();
    open(&mut state);
    assert!(!palette(&state).labels().contains(&"command palette"));
}

#[test]
fn the_open_bar_has_a_title_a_query_a_rule_headings_and_a_key_bar() {
    let mut state = state();
    open(&mut state);
    let lines = boxed(&mut state);
    let total = palette(&state).total();

    assert!(lines[0].starts_with('┌'), "{lines:?}");
    assert!(lines[1].contains(" commands"), "{}", lines[1]);
    assert!(
        lines[1]
            .trim_end_matches(['│', ' '])
            .ends_with(&format!("{total} commands")),
        "the count is dim on the right: {}",
        lines[1]
    );
    assert!(lines[2].contains(" : type a command"), "{}", lines[2]);
    assert!(
        lines[3].starts_with("│─")
            || lines[3].starts_with("│─".trim_end())
            || lines[3].contains("──────"),
        "a rule under the query: {}",
        lines[3]
    );
    assert!(lines[4].contains("global"), "{}", lines[4]);
    assert!(lines[5].contains("keybinds"), "{}", lines[5]);
    assert!(
        lines[5].contains("prefix+?"),
        "the shortcut is on the row: {}",
        lines[5]
    );
    let last = lines.len() - 2;
    for needle in ["enter run", "tab complete", "esc close"] {
        assert!(lines[last].contains(needle), "{}", lines[last]);
    }
}

#[test]
fn the_selected_row_is_the_accent_bar_and_nothing_else_is() {
    let mut state = state();
    open(&mut state);
    let layout = layout(&state);
    let accent = crate::protocol::color_to_u32(state.config.palette.accent);
    let selected_y = layout.list.y + (palette(&state).selected - layout.start) as u16;
    let frame = state.compose(W, H).unwrap();
    let at =
        |x: u16, y: u16| &frame.cells[usize::from(y) * usize::from(frame.width) + usize::from(x)];

    for x in layout.list.x..layout.list.right() {
        assert_eq!(at(x, selected_y).bg, accent, "x {x}");
    }
    let bars = (layout.list.y..layout.list.bottom())
        .filter(|y| at(layout.list.x + 3, *y).bg == accent)
        .count();
    assert_eq!(bars, 1, "one accent bar");
}

#[test]
fn typing_filters_and_the_count_reads_shown_of_total() {
    let mut state = state();
    open(&mut state);
    type_text(&mut state, "split");
    let labels = palette(&state).labels();
    assert_eq!(labels[..2], ["split vertical", "split horizontal"]);
    let lines = boxed(&mut state);
    assert!(
        lines[1].contains(&format!(
            "{} of {}",
            palette(&state).shown(),
            palette(&state).total()
        )),
        "{}",
        lines[1]
    );
    assert!(lines[2].contains(" : split"), "{}", lines[2]);
    assert!(
        !lines.iter().any(|line| line.contains("global")),
        "headings go while filtering"
    );
}

#[test]
fn esc_clears_the_query_first_and_closes_on_the_second_press() {
    let mut state = state();
    open(&mut state);
    type_text(&mut state, "zoom");
    state.handle_input_bytes(b"\x1b");
    assert_eq!(palette(&state).query.text(), "");
    assert!(palette(&state).shown() > 40);
    state.handle_input_bytes(b"\x1b");
    assert!(state.overlay.is_none());
}

#[test]
fn enter_runs_the_command_through_the_path_its_key_takes() {
    let mut state = state();
    open(&mut state);
    type_text(&mut state, "split vertical");
    let outcome = state.handle_input_bytes(b"\r");
    assert!(state.overlay.is_none(), "the bar closes before it runs");

    let mut reference = self::state();
    let expected = bind(&mut reference, KeybindAction::SplitVertical);
    let sent = |outcome: &ClientShellInput| -> Vec<Method> {
        outcome
            .actions
            .iter()
            .filter_map(|action| match action {
                ClientShellAction::Endpoint { request, .. } => Some(request.method.clone()),
                _ => None,
            })
            .collect()
    };
    assert!(
        !sent(&expected).is_empty(),
        "the reference key sends a split"
    );
    assert_eq!(sent(&outcome), sent(&expected));
}

#[test]
fn a_command_that_opens_a_prompt_opens_it() {
    let mut state = state();
    open(&mut state);
    type_text(&mut state, "rename tab");
    state.handle_input_bytes(b"\r");
    assert!(
        matches!(state.overlay, Some(ClientShellOverlay::Rename(_))),
        "{:?}",
        state.overlay
    );
}

#[test]
fn an_indexed_command_takes_its_digit_from_the_query() {
    let mut state = state();
    open(&mut state);
    type_text(&mut state, "switch tab 2");
    assert_eq!(palette(&state).argument, Some(1));
    let outcome = state.handle_input_bytes(b"\r");
    assert!(state.overlay.is_none());

    let mut reference = self::state();
    let expected = bind(&mut reference, KeybindAction::SwitchTab(1));
    assert_eq!(outcome.actions.len(), expected.actions.len());
}

#[test]
fn enter_on_an_indexed_command_without_a_digit_completes_its_words() {
    let mut state = state();
    open(&mut state);
    type_text(&mut state, "switch tab");
    state.handle_input_bytes(b"\r");
    assert_eq!(palette(&state).query.text(), "switch tab ");
    assert!(state.overlay.is_some(), "still open, waiting for the digit");
}

#[test]
fn tab_completes_the_selected_commands_words() {
    let mut state = state();
    open(&mut state);
    type_text(&mut state, "zm");
    state.handle_input_bytes(b"\t");
    assert_eq!(palette(&state).query.text(), "zoom pane");
}

#[test]
fn no_match_says_so_and_enter_does_nothing() {
    let mut state = state();
    open(&mut state);
    type_text(&mut state, "qqqq");
    assert!(boxed(&mut state)
        .iter()
        .any(|line| line.contains("no match")));
    let outcome = state.handle_input_bytes(b"\r");
    assert!(outcome.actions.is_empty());
    assert!(state.overlay.is_some());
}

#[test]
fn moving_skips_headings_and_clicking_a_row_runs_it() {
    let mut state = state();
    open(&mut state);
    let before = palette(&state).selected;
    state.handle_input_bytes(b"\x0e"); // ctrl+n
    let after = palette(&state).selected;
    assert!(after > before);
    assert!(matches!(
        palette(&state).items[after],
        PaletteItem::Command(_)
    ));

    let layout = layout(&state);
    let (row, _) = palette(&state)
        .items
        .iter()
        .enumerate()
        .skip(layout.start)
        .find(|(_, item)| matches!(item, PaletteItem::Command(_)))
        .unwrap();
    let y = layout.list.y + (row - layout.start) as u16;
    state.handle_raw_events(vec![crate::raw_input::RawInputEvent::Mouse(
        crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: layout.list.x + 4,
            row: y,
            modifiers: crossterm::event::KeyModifiers::empty(),
        },
    )]);
    assert!(state.overlay.is_none(), "a click on a command runs it");
}

#[test]
fn a_narrow_screen_keeps_the_bar_inside_it() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.compose(40, 12).unwrap();
    state.handle_input_bytes(b"\x02:");
    state.compose(40, 12).unwrap();
    let outer = layout(&state).outer;
    assert!(outer.right() <= 40 && outer.bottom() <= 12, "{outer:?}");
    assert!(outer.width >= 38);
}

#[test]
fn the_help_panel_documents_the_bar_and_its_key() {
    let mut state = state();
    bind(&mut state, KeybindAction::Help);
    let text = rows(&mut state).join("\n");
    assert!(text.contains("command palette"), "{text}");
}

// -- the send flow ----------------------------------------------------------------

fn agent(pane_id: &str, name: Option<&str>, status: AgentStatus) -> ClientShellAgent {
    ClientShellAgent {
        pane_id: pane_id.into(),
        workspace_id: "ws_1".into(),
        tab_id: "tab_1".into(),
        name: name.map(str::to_owned),
        display_agent: None,
        agent: Some("claude".into()),
        title: None,
        terminal_title: None,
        terminal_title_stripped: None,
        agent_status: status,
        state_change_seq: 1,
        state_labels: Vec::new(),
        tokens: Vec::new(),
        focused: false,
    }
}

fn state_with_agents(config: Config) -> ClientShellState {
    let mut snapshot = snapshot();
    snapshot.agents = vec![
        agent("pane_1", Some("reviewer"), AgentStatus::Idle),
        agent("pane_2", Some("builder"), AgentStatus::Working),
        agent("pane_3", Some("gate"), AgentStatus::Blocked),
    ];
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface());
    state.compose(W, H).unwrap();
    state
}

fn sends(outcome: &ClientShellInput) -> Vec<(String, Method)> {
    outcome
        .actions
        .iter()
        .filter_map(|action| match action {
            ClientShellAction::Endpoint { request, .. } => {
                Some((request.id.clone(), request.method.clone()))
            }
            _ => None,
        })
        .collect()
}

fn one(outcome: &ClientShellInput) -> (String, Method) {
    let mut requests = sends(outcome);
    assert_eq!(requests.len(), 1, "one request: {requests:?}");
    requests.remove(0)
}

fn one_send(outcome: &ClientShellInput) -> (String, crate::api::schema::AgentSendParams) {
    match one(outcome) {
        (id, Method::AgentSend(params)) => (id, params),
        other => panic!("expected an agent.send, got {other:?}"),
    }
}

fn pick(state: &mut ClientShellState, name: &str) {
    open(state);
    type_text(state, &format!("send {name}"));
    state.handle_input_bytes(b"\r");
    state.compose(W, H).unwrap();
}

fn sent_result(mode: AgentSendMode) -> ResponseResult {
    let note = (mode == AgentSendMode::Note).then(|| AgentSendNote {
        state: "held".into(),
        detail: "held until morning: if urgent, tell your gestore".into(),
    });
    ResponseResult::AgentSent {
        agent: serde_json::from_value(serde_json::json!({
            "terminal_id": "t1",
            "agent_status": "idle",
            "workspace_id": "ws_1",
            "tab_id": "tab_1",
            "pane_id": "pane_1",
            "focused": false,
            "revision": 1,
        }))
        .unwrap(),
        delivery: mode,
        pieces: 2,
        enter_sent: mode == AgentSendMode::Typed,
        interrupted: false,
        note,
    }
}

#[test]
fn the_commands_list_has_a_way_into_the_send() {
    let mut state = state_with_agents(Config::default());
    open(&mut state);
    let lines = boxed(&mut state).join("\n");
    assert!(lines.contains("send to a session…"), "{lines}");
}

#[test]
fn send_and_a_name_lists_the_sessions_with_their_state() {
    let mut state = state_with_agents(Config::default());
    open(&mut state);
    type_text(&mut state, "send ");
    assert_eq!(palette(&state).labels(), ["reviewer", "builder", "gate"]);
    let lines = boxed(&mut state);
    assert!(lines[1].contains("send to a session"), "{}", lines[1]);
    assert!(
        lines[1].contains("3 of 3") || lines[1].contains("3 sessions"),
        "{}",
        lines[1]
    );
    let joined = lines.join("\n");
    for needle in ["reviewer", "idle", "builder", "working", "gate", "blocked"] {
        assert!(joined.contains(needle), "{needle}: {joined}");
    }

    type_text(&mut state, "bui");
    assert_eq!(palette(&state).labels(), ["builder"]);
}

#[test]
fn esc_from_the_session_list_goes_back_to_the_commands() {
    let mut state = state_with_agents(Config::default());
    open(&mut state);
    type_text(&mut state, "send ");
    state.handle_input_bytes(b"\x1b");
    assert!(state.overlay.is_some());
    assert!(palette(&state).shown() > 40, "the commands are back");
}

#[test]
fn picking_a_session_shows_who_how_and_what_its_state_means() {
    let mut state = state_with_agents(Config::default());
    pick(&mut state, "reviewer");
    let lines = boxed(&mut state);
    assert!(lines[1].contains("send to reviewer"), "{}", lines[1]);
    assert!(lines[2].contains("a message, one line"), "{}", lines[2]);
    let joined = lines.join("\n");
    assert!(joined.contains("to   reviewer"), "{joined}");
    assert!(joined.contains("idle"), "{joined}");
    assert!(
        joined.contains("as   you  typed into the pane"),
        "typed is the default: {joined}"
    );
    assert!(joined.contains("tab switches"), "{joined}");
}

#[test]
fn enter_sends_the_message_typed_as_you_and_the_receipt_says_what_it_means() {
    let mut state = state_with_agents(Config::default());
    pick(&mut state, "reviewer");
    type_text(&mut state, "take a look at the diff");
    let outcome = state.handle_input_bytes(b"\r");
    let (id, method) = one(&outcome);
    assert_eq!(
        method,
        Method::AgentSend(crate::api::schema::AgentSendParams {
            target: "pane_1".into(),
            text: "take a look at the diff".into(),
            mode: AgentSendMode::Typed,
            busy: AgentSendBusy::Queue,
            answer: None,
        })
    );

    state.handle_endpoint_result("boot-1", &id, Ok(sent_result(AgentSendMode::Typed)));
    let joined = boxed(&mut state).join("\n");
    assert!(
        joined.contains("typed to reviewer in 2 pieces, enter pressed"),
        "{joined}"
    );
    assert!(
        joined.contains("This means it arrived, not that it was read or done."),
        "{joined}"
    );
    state.handle_input_bytes(b"\r");
    assert!(state.overlay.is_none(), "enter closes the receipt");
}

#[test]
fn tab_sends_it_as_a_note_and_a_held_note_is_shown_in_its_own_words() {
    let mut state = state_with_agents(Config::default());
    pick(&mut state, "reviewer");
    type_text(&mut state, "fyi");
    state.handle_input_bytes(b"\t");
    let outcome = state.handle_input_bytes(b"\r");
    let (id, params) = one_send(&outcome);
    assert_eq!(params.mode, AgentSendMode::Note);

    state.handle_endpoint_result("boot-1", &id, Ok(sent_result(AgentSendMode::Note)));
    let joined = boxed(&mut state).join("\n");
    assert!(joined.contains("note to reviewer: held"), "{joined}");
    assert!(
        joined.contains("held until morning: if urgent, tell your gestore"),
        "{joined}"
    );
}

#[test]
fn the_default_mode_is_a_config_switch() {
    let config: Config = toml::from_str("[palette.send]\ndefault_mode = \"note\"\n").unwrap();
    let mut state = state_with_agents(config);
    pick(&mut state, "reviewer");
    let joined = boxed(&mut state).join("\n");
    assert!(joined.contains("as   a note"), "{joined}");
}

#[test]
fn a_working_session_queues_and_alt_enter_interrupts() {
    let mut state = state_with_agents(Config::default());
    pick(&mut state, "builder");
    let joined = boxed(&mut state).join("\n");
    assert!(
        joined.contains("working now: enter queues it, alt+enter interrupts first."),
        "{joined}"
    );

    type_text(&mut state, "stop and rebase");
    let queued = state.handle_input_bytes(b"\r");
    let (_, params) = one_send(&queued);
    assert_eq!(params.busy, AgentSendBusy::Queue);

    // A fresh bar: alt+enter is escape then enter.
    let mut state = state_with_agents(Config::default());
    pick(&mut state, "builder");
    type_text(&mut state, "stop and rebase");
    let interrupt = state.handle_input_bytes(b"\x1b\r");
    let (_, params) = one_send(&interrupt);
    assert_eq!(params.busy, AgentSendBusy::Interrupt);
}

#[test]
fn the_busy_policy_ask_asks_before_sending_to_a_working_session() {
    let config: Config = toml::from_str("[palette.send]\nbusy = \"ask\"\n").unwrap();
    let mut state = state_with_agents(config);
    pick(&mut state, "builder");
    type_text(&mut state, "hello");
    let first = state.handle_input_bytes(b"\r");
    assert!(sends(&first).is_empty(), "nothing sent yet");
    let joined = boxed(&mut state).join("\n");
    assert!(
        joined.contains("working now: enter queues it, alt+enter interrupts first, esc cancels."),
        "{joined}"
    );

    let second = state.handle_input_bytes(b"\r");
    assert_eq!(sends(&second).len(), 1, "the second enter queues it");
}

#[test]
fn a_blocked_session_is_offered_its_pane_not_free_text() {
    let mut state = state_with_agents(Config::default());
    pick(&mut state, "gate");
    let joined = boxed(&mut state).join("\n");
    assert!(joined.contains("blocked"), "{joined}");
    assert!(
        joined.contains("only a note can go to it"),
        "a named blocked session can still get a note: {joined}"
    );
    // The default is a note, never typed text.
    type_text(&mut state, "ping");
    let outcome = state.handle_input_bytes(b"\r");
    let (_, params) = one_send(&outcome);
    assert_eq!(params.mode, AgentSendMode::Note);
}

#[test]
fn a_refusal_is_shown_under_the_message_and_the_message_is_kept() {
    let mut state = state_with_agents(Config::default());
    pick(&mut state, "reviewer");
    type_text(&mut state, "hello there");
    let outcome = state.handle_input_bytes(b"\r");
    let (id, _) = one(&outcome);
    state.handle_endpoint_result(
        "boot-1",
        &id,
        Err(ClientShellEndpointError {
            code: Some("agent_blocked".into()),
            message: "agent reviewer is blocked on a prompt".into(),
        }),
    );
    let joined = boxed(&mut state).join("\n");
    assert!(
        joined.contains("agent reviewer is blocked on a prompt"),
        "{joined}"
    );
    assert_eq!(palette(&state).query.text(), "hello there");
    assert!(state.overlay.is_some());
}

#[test]
fn an_empty_message_is_not_sent() {
    let mut state = state_with_agents(Config::default());
    pick(&mut state, "reviewer");
    let outcome = state.handle_input_bytes(b"\r");
    assert!(sends(&outcome).is_empty());
    assert!(boxed(&mut state)
        .join("\n")
        .contains("type a message first"));
}

#[test]
fn esc_from_a_message_goes_back_to_the_sessions() {
    let mut state = state_with_agents(Config::default());
    pick(&mut state, "reviewer");
    state.handle_input_bytes(b"\x1b");
    assert_eq!(palette(&state).labels(), ["reviewer", "builder", "gate"]);
}
