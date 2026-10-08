//! The pane todo panel and the todo editor, re-expressed from the fork's
//! oracles (src/ui/todo_panel.rs, src/app/input/modal.rs, src/app/input/mouse.rs,
//! src/ui/dialogs.rs at sync-snapshot/2026-10-08) over snapshots and emitted
//! endpoint requests.

use super::*;
use crate::api::schema::{Method, ResponseResult, TodoInfo};
use crate::input::KeybindAction;
use crate::terminal::todo::TodoPriority;

const W: u16 = 106;
const H: u16 = 24;

fn todo(id: u64, text: &str, priority: TodoPriority, done: bool) -> TodoInfo {
    TodoInfo {
        pane_id: "pane_1".into(),
        id,
        text: text.into(),
        done,
        priority,
        link_pane_id: None,
        link_label: None,
        link_alive: false,
        created_at_unix: 0,
        updated_at_unix: 0,
    }
}

fn state() -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.compose(W, H).unwrap();
    state
}

fn methods(outcome: &ClientShellInput) -> Vec<(String, Method)> {
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

fn only(outcome: &ClientShellInput) -> (String, Method) {
    let methods = methods(outcome);
    let [method] = methods.as_slice() else {
        panic!("expected one endpoint request, got {methods:?}");
    };
    method.clone()
}

fn bind(state: &mut ClientShellState, action: KeybindAction) -> ClientShellInput {
    let mut outcome = ClientShellInput::default();
    state.record_binding(crate::input::KeybindMatch::Action(action), &mut outcome);
    outcome
}

/// Open the panel on the focused pane and answer its list with `todos`.
fn open_panel(state: &mut ClientShellState, todos: Vec<TodoInfo>) {
    let (id, method) = only(&bind(state, KeybindAction::OpenPaneTodos));
    assert!(matches!(&method, Method::TodoList(params)
        if params.pane_id.as_deref() == Some("pane_1")));
    state.handle_endpoint_result("boot-1", &id, Ok(ResponseResult::TodoList { todos }));
    state.compose(W, H).unwrap();
}

fn screen(state: &mut ClientShellState) -> Vec<String> {
    let frame = state.compose(W, H).expect("frame");
    frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect()
}

fn panel(state: &ClientShellState) -> &super::super::todo_panel::ClientTodoPanelOverlay {
    match state.overlay.as_ref() {
        Some(ClientShellOverlay::TodoPanel(panel)) => panel,
        other => panic!("expected the todo panel, got {other:?}"),
    }
}

fn edit(state: &ClientShellState) -> &super::super::todo_edit::ClientTodoEditOverlay {
    match state.overlay.as_ref() {
        Some(ClientShellOverlay::TodoEdit(edit)) => edit,
        other => panic!("expected the todo editor, got {other:?}"),
    }
}

fn click(state: &mut ClientShellState, x: u16, y: u16) -> ClientShellInput {
    state.handle_raw_events(vec![crate::raw_input::RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::empty(),
    })])
}

fn layout(state: &ClientShellState) -> super::super::todo_panel::TodoPanelLayout {
    state.hits.todo_panel.clone().expect("panel layout")
}

fn populated() -> Vec<TodoInfo> {
    vec![
        todo(
            1,
            "ship the overlay kit before the todo board lands",
            TodoPriority::High,
            false,
        ),
        todo(
            2,
            "write the rendered-layout tests first",
            TodoPriority::Normal,
            false,
        ),
        todo(3, "read the plan", TodoPriority::Normal, true),
    ]
}

// -- panel --------------------------------------------------------------------

#[test]
fn the_panel_draws_one_row_per_todo_in_presentation_order() {
    let mut state = state();
    open_panel(&mut state, populated());
    let text = screen(&mut state).join("\n");

    assert!(
        text.contains("│ ▲ ship the overlay kit before the todo board lands  #1│"),
        "{text}"
    );
    assert!(
        text.contains(" ● write the rendered-layout tests first"),
        "{text}"
    );
    assert!(text.contains(" ✓ read the plan"), "{text}");
    assert!(text.contains("spc toggle"), "{text}");
    assert!(text.contains("c clear done"), "{text}");
    // The panel has no title, and it does not dim the screen behind it.
    let rect = layout(&state).outer;
    assert_eq!(rect.width, 57);
}

#[test]
fn an_empty_pane_shows_the_empty_state_and_can_still_add() {
    let mut state = state();
    open_panel(&mut state, Vec::new());
    let text = screen(&mut state).join("\n");

    assert!(text.contains("│ no todos"), "{text}");
    assert!(text.contains("a add"), "{text}");
    assert!(text.contains("esc close"), "{text}");
    assert!(!text.contains("toggle"), "{text}");
    let rect = layout(&state).outer;
    assert_eq!((rect.width, rect.height), (30, 5));
}

#[test]
fn the_panel_hangs_from_the_pane_it_belongs_to() {
    use super::super::todo_panel::{todo_panel_layout, ClientTodoPanelOverlay};
    let mut panel = ClientTodoPanelOverlay::new("pane_1".into(), "boot-1".into());
    panel.todos = vec![todo(1, "a", TodoPriority::Normal, false)];

    let rect = todo_panel_layout(&panel, Rect::new(20, 4, 40, 10), Rect::new(0, 0, 80, 25))
        .unwrap()
        .outer;

    assert_eq!(rect.x + rect.width, 60, "right-aligned to the pane");
    assert_eq!(rect.y, 5, "one row inside the pane's top border");
    // A pane narrower than the panel: the panel stops at the screen's edge.
    let rect = todo_panel_layout(&panel, Rect::new(0, 4, 20, 10), Rect::new(0, 0, 80, 25))
        .unwrap()
        .outer;
    assert_eq!(rect.x, 0);
}

#[test]
fn a_multi_line_todo_occupies_one_row_and_its_text_shows_in_the_detail_box() {
    let mut state = state();
    open_panel(
        &mut state,
        vec![
            todo(
                1,
                "the plan\nsecond line\nthird line",
                TodoPriority::High,
                false,
            ),
            todo(2, "one line", TodoPriority::Normal, false),
        ],
    );
    let rows = screen(&mut state);
    let text = rows.join("\n");
    let layout = layout(&state);
    let detail = layout.detail.expect("detail box");

    assert!(text.contains("the plan ⏎"), "{text}");
    assert!(
        detail.bottom() <= layout.list.y,
        "the box sits above the list"
    );
    let list_row = &rows[usize::from(layout.list.y)];
    assert!(list_row.contains("the plan ⏎"), "{list_row}");
    let next_row = &rows[usize::from(layout.list.y) + 1];
    assert!(next_row.contains("one line"), "{next_row}");
    assert!(text.contains("second line"), "{text}");
    assert!(text.contains("third line"), "{text}");
}

#[test]
fn panel_height_is_steady_as_the_selection_moves() {
    let mut state = state();
    open_panel(
        &mut state,
        vec![
            todo(1, "the plan\nsecond line", TodoPriority::High, false),
            todo(2, "one line", TodoPriority::Normal, false),
        ],
    );
    let before = layout(&state).outer;
    state.handle_input_bytes(b"j");
    let text = screen(&mut state).join("\n");

    assert_eq!(layout(&state).outer, before);
    assert!(text.contains("full text in the row"), "{text}");
}

#[test]
fn panel_selection_moves_with_arrows_and_j_k_and_clamps() {
    let mut state = state();
    open_panel(&mut state, populated());

    state.handle_input_bytes(b"j");
    assert_eq!(panel(&state).selected, 1);
    state.handle_input_bytes(b"\x1b[B");
    assert_eq!(panel(&state).selected, 2);
    state.handle_input_bytes(b"j");
    assert_eq!(panel(&state).selected, 2, "movement clamps, never wraps");
    state.handle_input_bytes(b"k");
    assert_eq!(panel(&state).selected, 1);
    // ctrl+d moves half a page; it never removes.
    let moved = state.handle_input_bytes(b"\x04");
    assert!(methods(&moved).is_empty());
    assert_eq!(panel(&state).selected, 2);
}

#[test]
fn space_toggles_the_selected_todo_and_the_list_is_fetched_again() {
    let mut state = state();
    open_panel(&mut state, populated());

    let (id, method) = only(&state.handle_input_bytes(b" "));
    assert!(matches!(&method, Method::TodoUpdate(params)
        if params.id == 1 && params.done == Some(true) && params.text.is_none()));
    let (_, actions) = state.handle_endpoint_result("boot-1", &id, Ok(ResponseResult::Ok {}));

    assert!(matches!(actions.as_slice(),
        [ClientShellAction::Endpoint { request, .. }]
            if matches!(request.method, Method::TodoList(_))));
}

#[test]
fn d_removes_and_c_clears_only_done_todos() {
    let mut state = state();
    open_panel(&mut state, populated());

    let (_, removed) = only(&state.handle_input_bytes(b"d"));
    assert!(matches!(removed, Method::TodoRemove(ref params) if params.id == 1));
    let (_, cleared) = only(&state.handle_input_bytes(b"c"));
    assert!(matches!(cleared, Method::TodoClear(ref params) if params.done_only));
    // A shifted letter does what its lowercase form does.
    let (_, shifted) = only(&state.handle_input_bytes(b"C"));
    assert!(matches!(shifted, Method::TodoClear(ref params) if params.done_only));
}

#[test]
fn esc_and_q_close_the_panel() {
    for key in [&b"\x1b"[..], b"q"] {
        let mut state = state();
        open_panel(&mut state, populated());
        state.handle_input_bytes(key);
        assert!(state.overlay.is_none());
    }
}

#[test]
fn g_follows_a_live_link_and_closes_the_panel_and_a_dead_one_is_inert() {
    let mut live = todo(1, "deploy the release candidate", TodoPriority::High, false);
    live.link_pane_id = Some("w2:p3".into());
    live.link_label = Some("infra".into());
    live.link_alive = true;
    let mut state = state();
    open_panel(&mut state, vec![live.clone()]);
    let text = screen(&mut state).join("\n");
    assert!(text.contains(" → w2:p3 · infra "), "{text}");
    assert!(text.contains("g go"), "{text}");

    let (_, method) = only(&state.handle_input_bytes(b"g"));
    assert!(matches!(method, Method::PaneFocus(ref target) if target.pane_id == "w2:p3"));
    assert!(state.overlay.is_none());

    let mut dead = live;
    dead.link_pane_id = None;
    dead.link_alive = false;
    let mut state = super::todo_overlays::state();
    open_panel(&mut state, vec![dead]);
    let text = screen(&mut state).join("\n");
    assert!(text.contains(" → infra "), "{text}");
    assert!(!text.contains("g go"), "{text}");
    assert!(methods(&state.handle_input_bytes(b"g")).is_empty());
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::TodoPanel(_))
    ));
}

#[test]
fn a_new_todo_revision_in_the_snapshot_fetches_the_list_again() {
    let mut state = state();
    open_panel(&mut state, populated());
    let mut quiet = ClientShellInput::default();
    state.tick_todo_panel(&mut quiet);
    assert!(
        methods(&quiet).is_empty(),
        "nothing changed, nothing to fetch"
    );

    let mut projected = snapshot();
    projected.revision = 2;
    projected.resource_facts = Some(crate::protocol::ClientShellResourceFacts {
        pane_todos: Some(
            [(
                "pane_1".to_owned(),
                crate::protocol::ClientPaneTodoSummary {
                    total: 3,
                    open: 2,
                    highest_priority: Some("high".into()),
                    revision: 7,
                },
            )]
            .into_iter()
            .collect(),
        ),
        ..Default::default()
    });
    state.set_snapshot(Box::new(projected));
    let mut outcome = ClientShellInput::default();
    state.tick_todo_panel(&mut outcome);

    assert!(matches!(only(&outcome).1, Method::TodoList(_)));
}

#[test]
fn the_panel_closes_when_its_pane_is_gone() {
    let mut state = state();
    open_panel(&mut state, populated());
    let mut projected = snapshot();
    projected.revision = 2;
    projected.panes.clear();
    state.set_snapshot(Box::new(projected));
    let mut outcome = ClientShellInput::default();
    state.tick_todo_panel(&mut outcome);

    assert!(state.overlay.is_none());
}

#[test]
fn inside_click_off_rows_does_not_close_the_panel_and_an_outside_click_does() {
    let mut state = state();
    open_panel(&mut state, populated());
    let layout = layout(&state);
    // The blank row between the list and the footer.
    let blank = layout.footer_row.unwrap().y - 1;

    click(&mut state, layout.inner.x + 1, blank);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::TodoPanel(_))
    ));
    click(&mut state, layout.outer.right(), layout.outer.y + 1);
    assert!(state.overlay.is_none());
}

#[test]
fn a_near_miss_on_the_footer_row_is_inert() {
    let mut state = state();
    open_panel(&mut state, populated());
    let layout = layout(&state);
    let (add, _) = layout.buttons[0];

    click(&mut state, add.right(), add.y);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::TodoPanel(_))
    ));
}

#[test]
fn clicking_a_row_opens_the_editor_and_the_add_button_composes() {
    let mut state = state();
    open_panel(&mut state, populated());
    let layout = layout(&state);

    click(&mut state, layout.list.x + 4, layout.list.y + 1);
    assert_eq!(edit(&state).todo_id, Some(2));

    let mut state = super::todo_overlays::state();
    open_panel(&mut state, populated());
    let layout = super::todo_overlays::layout(&state);
    let (add, _) = layout.buttons[0];
    click(&mut state, add.x, add.y);
    assert_eq!(edit(&state).todo_id, None);
}

#[test]
fn the_footer_drops_buttons_by_rank_and_keeps_add_and_close() {
    use super::super::todo_panel::{todo_panel_layout, ClientTodoPanelOverlay, TodoPanelButton};
    let mut panel = ClientTodoPanelOverlay::new("pane_1".into(), "boot-1".into());
    panel.todos = vec![todo(1, "a", TodoPriority::Normal, false)];
    let buttons = |panel: &ClientTodoPanelOverlay, screen_width: u16| {
        todo_panel_layout(
            panel,
            Rect::new(0, 0, screen_width, 20),
            Rect::new(0, 0, screen_width, 20),
        )
        .unwrap()
        .buttons
        .into_iter()
        .map(|(_, button)| button)
        .collect::<Vec<_>>()
    };

    // The narrowest panel holds only the two that never drop.
    assert_eq!(
        buttons(&panel, 80),
        [TodoPanelButton::Add, TodoPanelButton::Close]
    );
    panel.todos[0].text = "x".repeat(50);
    assert_eq!(
        buttons(&panel, 80),
        [
            TodoPanelButton::Add,
            TodoPanelButton::Toggle,
            TodoPanelButton::ClearDone,
            TodoPanelButton::Close
        ]
    );
}

// -- editor -------------------------------------------------------------------

#[test]
fn enter_inserts_a_newline_and_ctrl_s_saves_then_returns_to_the_panel() {
    let mut state = state();
    open_panel(&mut state, populated());
    state.handle_input_bytes(b"a");
    assert_eq!(edit(&state).todo_id, None);
    state.handle_input_bytes(b"one\rtwo");

    let (id, method) = only(&state.handle_input_bytes(b"\x13"));
    assert!(matches!(&method, Method::TodoAdd(params)
        if params.text == "one\ntwo" && params.priority == Some(TodoPriority::Normal)));
    // The editor closes only once the save is answered.
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::TodoEdit(_))
    ));
    let (_, actions) = state.handle_endpoint_result(
        "boot-1",
        &id,
        Ok(ResponseResult::Todo {
            todo: todo(4, "one\ntwo", TodoPriority::Normal, false),
        }),
    );

    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::TodoPanel(_))
    ));
    assert!(matches!(actions.as_slice(),
        [ClientShellAction::Endpoint { request, .. }]
            if matches!(request.method, Method::TodoList(_))));
}

#[test]
fn a_save_the_store_rejects_keeps_the_editor_open_with_the_typed_text() {
    let mut state = state();
    open_panel(&mut state, populated());
    state.handle_input_bytes(b"a");
    state.handle_input_bytes(b"too many");
    let (id, _) = only(&state.handle_input_bytes(b"\x13"));

    state.handle_endpoint_result(
        "boot-1",
        &id,
        Err(ClientShellEndpointError {
            code: Some("todo_limit_reached".into()),
            message: "pane already has the maximum of 50 todos".into(),
        }),
    );

    assert_eq!(edit(&state).text.text(), "too many");
    let notice = state.visible_endpoint_notice.as_ref().expect("notice");
    assert_eq!(notice.title, "todo save failed");
    assert!(notice.body.contains("maximum"));
    // And it can be saved again.
    assert!(matches!(
        only(&state.handle_input_bytes(b"\x13")).1,
        Method::TodoAdd(_)
    ));
}

#[test]
fn an_empty_buffer_never_saves() {
    let mut state = state();
    open_panel(&mut state, populated());
    state.handle_input_bytes(b"\r");
    state.handle_input_bytes(b"\x15");

    assert!(methods(&state.handle_input_bytes(b"\x13")).is_empty());
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::TodoEdit(_))
    ));
}

#[test]
fn tab_cycles_priority_and_cancel_discards_the_buffer() {
    let mut state = state();
    open_panel(&mut state, populated());
    state.handle_input_bytes(b"j\r");
    assert_eq!(edit(&state).priority, TodoPriority::Normal);
    state.handle_input_bytes(b"\t");
    assert_eq!(edit(&state).priority, TodoPriority::High);

    let cancelled = state.handle_input_bytes(b"\x1b");

    assert!(methods(&cancelled)
        .iter()
        .all(|(_, method)| matches!(method, Method::TodoList(_))));
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::TodoPanel(_))
    ));
}

#[test]
fn editing_saves_text_done_and_priority_and_ctrl_t_is_inert_while_composing() {
    let mut state = state();
    open_panel(&mut state, populated());
    state.handle_input_bytes(b"\r");
    assert_eq!(
        edit(&state).text.text(),
        "ship the overlay kit before the todo board lands"
    );
    state.handle_input_bytes(b"\x14");
    assert!(edit(&state).done);

    let (_, method) = only(&state.handle_input_bytes(b"\x13"));
    assert!(matches!(&method, Method::TodoUpdate(params)
        if params.id == 1
            && params.done == Some(true)
            && params.priority == Some(TodoPriority::High)
            && !params.clear_link
            && params.link_pane_id.is_none()));

    let mut state = super::todo_overlays::state();
    bind(&mut state, KeybindAction::AddPaneTodo);
    state.handle_input_bytes(b"\x14");
    assert!(!edit(&state).done);
}

#[test]
fn the_add_action_composes_on_the_focused_pane_without_the_panel() {
    let mut state = state();
    bind(&mut state, KeybindAction::AddPaneTodo);
    assert!(edit(&state).suspended.is_none());
    state.handle_input_bytes(b"note");
    let (id, method) = only(&state.handle_input_bytes(b"\x13"));
    assert!(matches!(&method, Method::TodoAdd(params) if params.pane_id == "pane_1"));

    state.handle_endpoint_result(
        "boot-1",
        &id,
        Ok(ResponseResult::Todo {
            todo: todo(1, "note", TodoPriority::Normal, false),
        }),
    );

    assert!(state.overlay.is_none());
}

#[test]
fn ctrl_g_saves_then_follows_a_live_link_and_is_inert_without_one() {
    let mut linked = todo(1, "deploy", TodoPriority::High, false);
    linked.link_pane_id = Some("w2:p3".into());
    linked.link_label = Some("infra".into());
    linked.link_alive = true;
    let mut state = state();
    open_panel(&mut state, vec![linked]);
    state.handle_input_bytes(b"\r!");
    let text = screen(&mut state).join("\n");
    assert!(text.contains("w2:p3 · infra"), "{text}");
    assert!(text.contains("^g go"), "{text}");

    let (id, method) = only(&state.handle_input_bytes(b"\x07"));
    assert!(
        matches!(&method, Method::TodoUpdate(params) if params.text.as_deref() == Some("deploy!"))
    );
    let (_, actions) = state.handle_endpoint_result("boot-1", &id, Ok(ResponseResult::Ok {}));
    assert!(state.overlay.is_none());
    assert!(matches!(actions.as_slice(),
        [ClientShellAction::Endpoint { request, .. }]
            if matches!(&request.method, Method::PaneFocus(target) if target.pane_id == "w2:p3")));

    let mut state = super::todo_overlays::state();
    open_panel(&mut state, populated());
    state.handle_input_bytes(b"\r");
    assert!(methods(&state.handle_input_bytes(b"\x07")).is_empty());
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::TodoEdit(_))
    ));
}

#[test]
fn the_editor_title_names_the_todo_and_the_rows_line_up() {
    let mut state = state();
    open_panel(&mut state, populated());
    state.handle_input_bytes(b"jj\r");
    let rows = screen(&mut state);
    let text = rows.join("\n");

    assert!(text.contains("edit todo/note #3"), "{text}");
    assert!(text.contains(" priority  ⇥    normal"), "{text}");
    assert!(text.contains(" link      ^l   none"), "{text}");
    assert!(text.contains(" done      ^t   yes"), "{text}");
    assert!(text.contains(" ^s save "), "{text}");
    assert!(text.contains(" esc cancel "), "{text}");
    let title_x = rows
        .iter()
        .find_map(|row| row.find("edit todo/note"))
        .unwrap();
    let label_x = rows.iter().find_map(|row| row.find("priority")).unwrap();
    assert_eq!(title_x, label_x);

    let mut state = super::todo_overlays::state();
    bind(&mut state, KeybindAction::AddPaneTodo);
    let text = screen(&mut state).join("\n");
    assert!(text.contains("new todo/note"), "{text}");
    assert!(!text.contains(" done "), "{text}");
}

#[test]
fn clicking_the_text_places_the_cursor_instead_of_cancelling() {
    let mut state = state();
    bind(&mut state, KeybindAction::AddPaneTodo);
    state.handle_input_bytes(b"hello\rworld wide");
    state.compose(W, H).unwrap();
    let rows = state
        .hits
        .todo_edit
        .as_ref()
        .and_then(|layout| layout.rows.clone())
        .unwrap();

    click(&mut state, rows.text_area.x + 2, rows.text_area.y);
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::TodoEdit(_))
    ));
    assert_eq!(
        (
            edit(&state).text.cursor_line(),
            edit(&state).text.cursor_column()
        ),
        (0, 2)
    );
    click(&mut state, rows.text_area.right() - 1, rows.text_area.y + 1);
    assert_eq!(
        (
            edit(&state).text.cursor_line(),
            edit(&state).text.cursor_column()
        ),
        (1, 10)
    );
    // The priority row cycles; anywhere off the controls cancels.
    click(&mut state, rows.priority.x + 3, rows.priority.y);
    assert_eq!(edit(&state).priority, TodoPriority::High);
    click(&mut state, 0, 0);
    assert!(state.overlay.is_none());
}

#[test]
fn pasting_into_a_todo_keeps_newlines_drops_other_controls_and_stops_at_the_limit() {
    let mut state = state();
    bind(&mut state, KeybindAction::AddPaneTodo);
    state.handle_raw_events(vec![crate::raw_input::RawInputEvent::Paste(
        "first\nse\x1bcond\ttab".into(),
    )]);
    assert_eq!(edit(&state).text.text(), "first\nsecondtab");

    let mut state = super::todo_overlays::state();
    bind(&mut state, KeybindAction::AddPaneTodo);
    state.handle_raw_events(vec![crate::raw_input::RawInputEvent::Paste(
        "x".repeat(520),
    )]);
    assert_eq!(edit(&state).text.text().chars().count(), 500);
}

#[test]
fn keybind_help_lists_the_todo_actions_and_chords() {
    let (live, _) = Config::default().live_keybinds_with_diagnostics().unwrap();
    let groups = crate::input::keybind_help_groups(&live.keybinds, &live.prefix);
    let find = |group: &str, label: &str| {
        groups
            .iter()
            .find(|(name, _)| *name == group)
            .and_then(|(_, entries)| entries.iter().find(|(_, l)| l == label))
            .map(|(key, _)| key.clone())
    };

    assert_eq!(
        find("panes", "pane todos").as_deref(),
        Some("prefix+ctrl+t")
    );
    assert_eq!(find("panes", "add pane todo").as_deref(), Some("unset"));
    assert_eq!(find("pane todos", "add todo").as_deref(), Some("a"));
    assert_eq!(
        find("todo edit modal", "save todo").as_deref(),
        Some("ctrl+s / alt+enter")
    );
    assert_eq!(
        find("todo edit modal", "toggle done").as_deref(),
        Some("ctrl+t")
    );
}

#[test]
fn the_wheel_scrolls_what_is_under_the_pointer_and_the_editor_stays() {
    let wheel = |state: &mut ClientShellState| {
        state.handle_raw_events(vec![crate::raw_input::RawInputEvent::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 1,
            row: 1,
            modifiers: KeyModifiers::empty(),
        })])
    };
    let mut bare = state();
    let without = wheel(&mut bare);

    let mut state = state();
    bind(&mut state, KeybindAction::AddPaneTodo);
    state.compose(W, H).unwrap();
    let with = wheel(&mut state);
    edit(&state);
    assert_eq!(
        format!("{:?}", with.actions),
        format!("{:?}", without.actions),
        "the wheel does what it does with no editor open"
    );
}
