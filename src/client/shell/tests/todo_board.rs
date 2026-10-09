//! The session todo board, re-expressed from the fork's oracles
//! (src/ui/todo_board.rs, src/app/actions.rs, src/app/input/modal.rs,
//! src/app/input/mouse.rs at sync-snapshot/2026-10-08) over snapshots and
//! emitted endpoint requests: the board lists every pane's todos with a
//! `todo.list` that names no pane, and acts on each todo's owner pane.

use super::super::todo_board::{
    heading_text, ClientTodoBoardOverlay, TodoBoardButton, TodoBoardItem, TodoBoardLayout,
};
use super::super::todo_edit::{ClientTodoEditOverlay, SuspendedTodoSurface};
use super::*;
use crate::api::schema::{AgentStatus, Method, ResponseResult, TodoInfo};
use crate::input::KeybindAction;
use crate::protocol::{ClientShellPane, ClientShellTab};
use crate::terminal::todo::TodoPriority;

const W: u16 = 80;
const H: u16 = 25;

/// Spaces with one tab each, each tab a list of pane names (`""` for an
/// unnamed pane, which reads `pane N`). Pane ids are `w{space}:p{n}` with `n`
/// counting across the session from 1; the first pane is focused.
fn session(spaces: &[(&str, &[&str])]) -> ClientShellSnapshot {
    let mut snapshot = super::snapshot();
    snapshot.workspaces.clear();
    snapshot.tabs.clear();
    snapshot.panes.clear();
    let mut pane_number = 0;
    for (w, (label, panes)) in spaces.iter().enumerate() {
        let workspace_id = format!("w{w}");
        let tab_id = format!("{workspace_id}:t1");
        let mut workspace = super::snapshot().workspaces.remove(0);
        workspace.workspace_id = workspace_id.clone();
        workspace.number = w + 1;
        workspace.label = (*label).to_owned();
        workspace.focused = w == 0;
        workspace.active_tab_id = tab_id.clone();
        snapshot.workspaces.push(workspace);
        snapshot.tabs.push(ClientShellTab {
            tab_id: tab_id.clone(),
            workspace_id: workspace_id.clone(),
            number: 1,
            label: "1".into(),
            custom_label: false,
            zoomed: false,
            focused: w == 0,
            agent_status: AgentStatus::Idle,
        });
        for name in panes.iter() {
            pane_number += 1;
            let pane_id = format!("{workspace_id}:p{pane_number}");
            if pane_number == 1 {
                snapshot.focused_workspace_id = Some(workspace_id.clone());
                snapshot.focused_tab_id = Some(tab_id.clone());
                snapshot.focused_pane_id = Some(pane_id.clone());
            }
            snapshot.panes.push(ClientShellPane {
                pane_id,
                workspace_id: workspace_id.clone(),
                tab_id: tab_id.clone(),
                label: (!name.is_empty()).then(|| (*name).to_owned()),
                cwd: None,
                foreground_cwd: None,
                focused: pane_number == 1,
                right_click_passthrough: false,
            });
        }
    }
    snapshot
}

/// `board` holds `pane 1` and `pane 2`, `infra` holds `deploy-bot`.
fn sample_session() -> ClientShellSnapshot {
    session(&[("board", &["", ""]), ("infra", &["deploy-bot"])])
}

fn todo(pane: &str, id: u64, text: &str, priority: TodoPriority, done: bool) -> TodoInfo {
    TodoInfo {
        pane_id: pane.into(),
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

fn open_todo(pane: &str, id: u64, text: &str) -> TodoInfo {
    todo(pane, id, text, TodoPriority::Normal, false)
}

fn linked(mut todo: TodoInfo, target: &str, label: &str, alive: bool) -> TodoInfo {
    todo.link_pane_id = alive.then(|| target.to_owned());
    todo.link_label = Some(label.into());
    todo.link_alive = alive;
    todo
}

/// Items: heading, 3 todos, gap, heading, 1 todo, gap, heading, 2 todos.
fn sample_todos() -> Vec<TodoInfo> {
    vec![
        todo("w0:p1", 1, "rerun the deploy", TodoPriority::High, false),
        open_todo("w0:p1", 2, "check the 403"),
        todo("w0:p1", 3, "read the plan", TodoPriority::Normal, true),
        open_todo("w0:p2", 1, "rotate the key"),
        todo("w1:p3", 1, "tidy the dashboards", TodoPriority::Low, false),
        todo("w1:p3", 2, "ship v2", TodoPriority::Low, true),
    ]
}

fn state_with(snapshot: ClientShellSnapshot) -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface());
    state.compose(W, H).unwrap();
    state
}

fn methods(outcome: &ClientShellInput) -> Vec<(String, Method)> {
    requests(&outcome.actions)
}

fn requests(actions: &[ClientShellAction]) -> Vec<(String, Method)> {
    actions
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

fn is_board_list(method: &Method) -> bool {
    matches!(method, Method::TodoList(params) if params.pane_id.is_none())
}

fn bind(state: &mut ClientShellState, action: KeybindAction) -> ClientShellInput {
    let mut outcome = ClientShellInput::default();
    state.record_binding(crate::input::KeybindMatch::Action(action), &mut outcome);
    outcome
}

/// Open the board and answer its list with `todos`.
fn open_with(state: &mut ClientShellState, todos: Vec<TodoInfo>) {
    let (id, method) = only(&bind(state, KeybindAction::OpenTodoBoard));
    assert!(is_board_list(&method), "{method:?}");
    state.handle_endpoint_result("boot-1", &id, Ok(ResponseResult::TodoList { todos }));
    state.compose(W, H).unwrap();
}

fn sample() -> ClientShellState {
    let mut state = state_with(sample_session());
    open_with(&mut state, sample_todos());
    state
}

/// Ask for the list the way a tick or a change does, and answer it.
fn refetch(state: &mut ClientShellState, todos: Vec<TodoInfo>) {
    let mut outcome = ClientShellInput::default();
    state.request_todo_board_list(&mut outcome);
    let (id, method) = only(&outcome);
    assert!(is_board_list(&method), "{method:?}");
    state.handle_endpoint_result("boot-1", &id, Ok(ResponseResult::TodoList { todos }));
    state.compose(W, H).unwrap();
}

fn board(state: &ClientShellState) -> &ClientTodoBoardOverlay {
    match state.overlay.as_ref() {
        Some(ClientShellOverlay::TodoBoard(board)) => board,
        other => panic!("expected the todo board, got {other:?}"),
    }
}

fn edit(state: &ClientShellState) -> &ClientTodoEditOverlay {
    match state.overlay.as_ref() {
        Some(ClientShellOverlay::TodoEdit(edit)) => edit,
        other => panic!("expected the todo editor, got {other:?}"),
    }
}

fn layout(state: &ClientShellState) -> TodoBoardLayout {
    state.hits.todo_board_layout.clone().expect("board layout")
}

fn screen(state: &mut ClientShellState) -> Vec<String> {
    screen_at(state, W, H)
}

fn screen_at(state: &mut ClientShellState, width: u16, height: u16) -> Vec<String> {
    let frame = state.compose(width, height).expect("frame");
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

/// The board box: columns `x..x + width` of rows `y..y + height`.
fn boxed(state: &mut ClientShellState) -> Vec<String> {
    let outer = layout(state).outer;
    screen(state)
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

/// What each list row is: a heading as `# space · pane`, a todo as its text,
/// a gap as an empty string.
fn kinds(board: &ClientTodoBoardOverlay) -> Vec<String> {
    board
        .items
        .iter()
        .map(|item| match item {
            TodoBoardItem::PaneHeading { space, label } => {
                format!("#{}", heading_text(space, label))
            }
            TodoBoardItem::Todo(index) => board.todos[*index].text.clone(),
            TodoBoardItem::GroupGap => String::new(),
        })
        .collect()
}

/// The selected todo's pane and id.
fn selected(state: &ClientShellState) -> (String, u64) {
    let todo = board(state).selected_todo().expect("a selected todo");
    (todo.pane_id.clone(), todo.id)
}

fn mouse(state: &mut ClientShellState, kind: MouseEventKind, x: u16, y: u16) -> ClientShellInput {
    state.handle_raw_events(vec![crate::raw_input::RawInputEvent::Mouse(MouseEvent {
        kind,
        column: x,
        row: y,
        modifiers: KeyModifiers::empty(),
    })])
}

fn click(state: &mut ClientShellState, x: u16, y: u16) -> ClientShellInput {
    mouse(state, MouseEventKind::Down(MouseButton::Left), x, y)
}

/// Click the list row showing item `index`.
fn click_item(state: &mut ClientShellState, index: usize) -> ClientShellInput {
    let layout = layout(state);
    let y = layout.list.y + (index - layout.start) as u16;
    click(state, layout.list.x + 6, y)
}

fn live_link_session() -> ClientShellState {
    let mut state = state_with(sample_session());
    let mut todos = sample_todos();
    todos[0] = linked(todos[0].clone(), "w1:p3", "deploy-bot", true);
    open_with(&mut state, todos);
    state
}

// -- expected box rows --------------------------------------------------------

/// A box row: `inner` padded to the 78 cells between the borders.
fn row(inner: &str) -> String {
    format!("│{inner:<78}│")
}

/// The title row: `todos/notes`, the count one cell in from the right edge (fork issue 174).
fn title_row(count: &str) -> String {
    row(&format!(
        "{:<width$}{count} ",
        " todos/notes",
        width = 78 - 1 - count.len()
    ))
}

/// A rule: `─` inset one column on both sides.
fn rule_row() -> String {
    row(&format!(" {} ", "─".repeat(76)))
}

/// A todo row: indented two cells, the glyph, the text, the id at the right.
fn todo_row(glyph: &str, text: &str, id: u64) -> String {
    let tag = format!("#{id}");
    row(&format!(
        "  {glyph}{text:<pad$}{tag}",
        pad = 76 - 3 - tag.len()
    ))
}

fn border(left: &str, right: &str) -> String {
    format!("{left}{}{right}", "─".repeat(78))
}

/// The footer, centred in the 78 cells between the borders.
fn footer_row(buttons: &[&str]) -> String {
    let joined = buttons.join("  ");
    let pad = (78 - joined.chars().count()) / 2;
    row(&format!("{}{joined}", " ".repeat(pad)))
}

const OPEN: &str = " ↵ open pane ";
const TOGGLE: &str = " spc toggle ";
const CLEAR: &str = " c clear done ";
const CLOSE: &str = " esc close ";

fn one_group() -> (ClientShellSnapshot, Vec<TodoInfo>) {
    (
        session(&[("board", &[""])]),
        vec![
            todo("w0:p1", 1, "rerun the deploy", TodoPriority::High, false),
            open_todo("w0:p1", 2, "check the 403 on login"),
            todo("w0:p1", 3, "read the plan", TodoPriority::Normal, true),
        ],
    )
}

fn two_groups() -> (ClientShellSnapshot, Vec<TodoInfo>) {
    (
        session(&[("board", &["", ""])]),
        vec![
            todo("w0:p1", 1, "rerun the deploy", TodoPriority::High, false),
            open_todo("w0:p2", 1, "check the 403 on login"),
        ],
    )
}

// -- snapshots ----------------------------------------------------------------

#[test]
fn snapshot_populated() {
    let (snapshot, todos) = one_group();
    let mut state = state_with(snapshot);
    open_with(&mut state, todos);

    assert_eq!(layout(&state).outer, Rect::new(0, 7, 80, 11));
    assert_eq!(
        boxed(&mut state),
        vec![
            border("┌", "┐"),
            title_row("3 todos"),
            row(" / search todos"),
            rule_row(),
            row(" board · pane 1"),
            todo_row(" ▲ ", "rerun the deploy", 1),
            todo_row(" ● ", "check the 403 on login", 2),
            todo_row(" ✓ ", "read the plan", 3),
            row(""),
            footer_row(&[OPEN, TOGGLE, CLEAR, CLOSE]),
            border("└", "┘"),
        ]
    );
}

#[test]
fn snapshot_two_groups_are_separated_by_a_blank_row() {
    let (snapshot, todos) = two_groups();
    let mut state = state_with(snapshot);
    open_with(&mut state, todos);

    assert_eq!(layout(&state).outer, Rect::new(0, 6, 80, 12));
    assert_eq!(
        boxed(&mut state),
        vec![
            border("┌", "┐"),
            title_row("2 todos"),
            row(" / search todos"),
            rule_row(),
            row(" board · pane 1"),
            todo_row(" ▲ ", "rerun the deploy", 1),
            row(""),
            row(" board · pane 2"),
            todo_row(" ● ", "check the 403 on login", 1),
            row(""),
            footer_row(&[OPEN, TOGGLE, CLEAR, CLOSE]),
            border("└", "┘"),
        ]
    );
}

#[test]
fn snapshot_board_searching_keeps_the_box_and_the_count() {
    let (snapshot, todos) = two_groups();
    let mut state = state_with(snapshot);
    open_with(&mut state, todos);
    state.handle_input_bytes(b"/403");

    assert_eq!(layout(&state).outer, Rect::new(0, 6, 80, 12));
    assert_eq!(
        boxed(&mut state),
        vec![
            border("┌", "┐"),
            title_row("2 todos"),
            row(" / 403"),
            rule_row(),
            row(" board · pane 2"),
            todo_row(" ● ", "check the 403 on login", 1),
            row(""),
            row(""),
            row(""),
            row(""),
            footer_row(&[OPEN, TOGGLE, CLEAR, CLOSE]),
            border("└", "┘"),
        ]
    );
}

#[test]
fn snapshot_empty() {
    let mut state = state_with(session(&[("board", &[""])]));
    open_with(&mut state, Vec::new());

    assert_eq!(layout(&state).outer, Rect::new(0, 8, 80, 8));
    assert_eq!(
        boxed(&mut state),
        vec![
            border("┌", "┐"),
            title_row("0 todos"),
            row(" / search todos"),
            rule_row(),
            row(" nothing outstanding"),
            row(""),
            footer_row(&[OPEN, CLOSE]),
            border("└", "┘"),
        ]
    );
}

#[test]
fn a_query_that_matches_nothing_says_so_and_keeps_the_footer() {
    let mut state = sample();
    state.handle_input_bytes(b"/zzz");
    let rows = boxed(&mut state);

    assert_eq!(rows[4], row(" no match"));
    assert!(board(&state).items.is_empty());
    // With nothing to act on, the footer offers only what never drops.
    assert_eq!(rows[rows.len() - 2], footer_row(&[OPEN, CLOSE]));
}

#[test]
fn nothing_is_drawn_until_the_first_list_lands() {
    let mut state = state_with(sample_session());
    bind(&mut state, KeybindAction::OpenTodoBoard);
    let text = screen(&mut state).join("\n");

    assert!(!text.contains("todos/notes"), "{text}");
    assert!(!text.contains("nothing outstanding"), "{text}");
    assert!(state.hits.todo_board_layout.is_none());
}

// -- look ---------------------------------------------------------------------

#[test]
fn a_heading_reads_space_then_pane() {
    assert_eq!(
        heading_text("infra", "imap-jmap-mcp"),
        " infra · imap-jmap-mcp"
    );
    assert_eq!(heading_text("infra", ""), " infra");
    assert_eq!(heading_text("", "imap-jmap-mcp"), " imap-jmap-mcp");
    assert_eq!(heading_text("", ""), "");
}

#[test]
fn the_group_header_uses_the_help_panes_styles() {
    let (snapshot, todos) = one_group();
    let mut state = state_with(snapshot);
    open_with(&mut state, todos);
    let frame = state.compose(W, H).unwrap();
    let p = &state.config.palette;
    let color = crate::protocol::color_to_u32;
    let bold = crate::protocol::modifier_to_u16(Modifier::BOLD);
    let cell = |x: u16, y: u16| &frame.cells[usize::from(y) * usize::from(W) + usize::from(x)];
    // Outer (0, 7): the heading is the fifth row, text from the first inner
    // column: " board" (accent, bold), " · " (dim), "pane 1" (mauve, bold).
    let y = 11;
    assert_eq!(cell(2, y).symbol, "b");
    assert_eq!(cell(2, y).fg, color(p.accent));
    assert_ne!(cell(2, y).modifier & bold, 0);
    assert_eq!(cell(8, y).symbol, "·");
    assert_eq!(cell(8, y).fg, color(p.overlay0));
    assert_eq!(cell(8, y).modifier & bold, 0);
    assert_eq!(cell(10, y).symbol, "p");
    assert_eq!(cell(10, y).fg, color(p.mauve));
    assert_ne!(cell(10, y).modifier & bold, 0);
}

#[test]
fn a_cut_heading_keeps_its_styles() {
    let label = "x".repeat(200);
    let mut state = state_with(session(&[("infra", &[label.as_str()])]));
    let (id, _) = only(&bind(&mut state, KeybindAction::OpenTodoBoard));
    state.handle_endpoint_result(
        "boot-1",
        &id,
        Ok(ResponseResult::TodoList {
            todos: vec![open_todo("w0:p1", 1, "one")],
        }),
    );
    let frame = state.compose(130, 25).unwrap();
    let layout = layout(&state);
    let p = &state.config.palette;
    let color = crate::protocol::color_to_u32;
    let cell = |x: u16| &frame.cells[usize::from(layout.list.y) * 130 + usize::from(x)];

    // The board stops at its cap, so the heading is cut: the space name keeps
    // the accent colour and the cut's `…` takes the label's mauve.
    assert_eq!(layout.outer.width, 120);
    assert_eq!(cell(layout.list.x + 1).symbol, "i");
    assert_eq!(cell(layout.list.x + 1).fg, color(p.accent));
    let last = layout.list.right() - 1;
    assert_eq!(cell(last).symbol, "…");
    assert_eq!(cell(last).fg, color(p.mauve));
}

#[test]
fn the_selected_row_is_a_band_inside_the_indent() {
    let (snapshot, todos) = one_group();
    let mut state = state_with(snapshot);
    open_with(&mut state, todos);
    let frame = state.compose(W, H).unwrap();
    let p = &state.config.palette;
    let color = crate::protocol::color_to_u32;
    let cell = |x: u16, y: u16| &frame.cells[usize::from(y) * usize::from(W) + usize::from(x)];
    // The first todo is selected: row y = 12. The two indent cells are left
    // alone, the band starts after them.
    assert_ne!(cell(1, 12).bg, color(p.accent));
    assert_ne!(cell(2, 12).bg, color(p.accent));
    assert_eq!(cell(3, 12).bg, color(p.accent));
    assert_eq!(cell(77, 12).bg, color(p.accent));
    // The next row is not.
    assert_ne!(cell(3, 13).bg, color(p.accent));
}

#[test]
fn the_board_dims_what_is_behind_it() {
    let mut state = sample();
    let frame = state.compose(W, H).unwrap();
    let dim = crate::protocol::modifier_to_u16(Modifier::DIM);

    assert_ne!(frame.cells[0].modifier & dim, 0);
}

#[test]
fn a_todo_row_reads_as_it_does_on_the_pane_panel() {
    let mut state = state_with(sample_session());
    let todos = vec![linked(
        todo(
            "w0:p1",
            7,
            "deploy the release candidate",
            TodoPriority::High,
            false,
        ),
        "w1:p3",
        "deploy-bot",
        true,
    )];
    open_with(&mut state, todos);
    let rows = screen(&mut state);
    let text = rows.join("\n");

    assert!(text.contains(" ▲ deploy the release candidate"), "{text}");
    assert!(text.contains(" → w1:p3 · deploy-bot "), "{text}");
    assert!(text.contains("g go"), "{text}");
    let todo_row = rows
        .iter()
        .find(|row| row.contains("deploy the release"))
        .unwrap();
    assert!(todo_row.contains("#7│"), "{todo_row}");
}

#[test]
fn a_multi_line_todo_shows_its_text_in_a_preview_under_the_list() {
    let mut state = state_with(session(&[("board", &[""])]));
    open_with(
        &mut state,
        vec![
            todo(
                "w0:p1",
                1,
                "the plan\nsecond line\nthird line",
                TodoPriority::High,
                false,
            ),
            open_todo("w0:p1", 2, "one line"),
        ],
    );
    let layout = layout(&state);
    let detail = layout.detail.expect("preview");
    let title = layout.header_row(0);
    let search = layout.search_row();
    let rows = boxed(&mut state);
    let text = rows.join("\n");

    // Fork issue 174: title, search, rule, list; the preview sits under the list after a rule,
    // with no frame of its own.
    assert_eq!(search.y, title.y + 1);
    assert_eq!(layout.list.y, search.y + 2);
    assert_eq!(detail.y, layout.list.bottom());
    assert_eq!(layout.footer_row.unwrap().y, detail.bottom() + 1);
    let preview = usize::from(detail.y - layout.outer.y);
    assert_eq!(rows[preview], rule_row());
    assert_eq!(rows[preview + 1], row(" the plan"));
    assert_eq!(rows[preview + 2], row(" second line"));
    assert_eq!(rows[preview + 3], row(" third line"));
    assert!(text.contains("the plan ⏎"), "{text}");
    assert_eq!(text.matches('┌').count(), 1, "no inner frame: {text}");
}

#[test]
fn the_preview_keeps_its_height_and_names_an_empty_detail_as_the_selection_moves() {
    let mut state = state_with(session(&[("board", &[""])]));
    open_with(
        &mut state,
        vec![
            todo(
                "w0:p1",
                1,
                "the plan\nsecond line",
                TodoPriority::High,
                false,
            ),
            open_todo("w0:p1", 2, "one line"),
        ],
    );
    let before = layout(&state).outer;
    state.handle_input_bytes(b"j");
    let text = screen(&mut state).join("\n");

    assert_eq!(layout(&state).outer, before);
    assert!(text.contains("full text in the row"), "{text}");
    assert!(!text.contains("second line"), "{text}");
}

#[test]
fn the_footer_keeps_a_blank_row_above_the_buttons() {
    let mut state = sample();
    let layout = layout(&state);

    assert_eq!(layout.footer_row.unwrap().y, layout.list.bottom() + 1);
    assert_eq!(layout.header.y + 3, layout.list.y);
    let blank = usize::from(layout.footer_row.unwrap().y - layout.outer.y) - 1;
    assert_eq!(boxed(&mut state)[blank], row(""));
}

// -- geometry -----------------------------------------------------------------

/// Open the board over a single pane on a `width` x `height` screen and give
/// back where it sits.
fn open_on(width: u16, height: u16, todos: Vec<TodoInfo>) -> TodoBoardLayout {
    let mut state = state_with(session(&[("board", &[""])]));
    let (id, method) = only(&bind(&mut state, KeybindAction::OpenTodoBoard));
    assert!(is_board_list(&method));
    state.handle_endpoint_result("boot-1", &id, Ok(ResponseResult::TodoList { todos }));
    state.compose(width, height).unwrap();
    layout(&state)
}

#[test]
fn the_board_grows_for_its_content_in_both_directions() {
    // A narrow session still gets the 80-column floor.
    assert_eq!(layout(&sample()).outer.width, 80);

    // A row wanting 90 columns: indent 2 + glyph 3 + 79 + "#1" + 1, plus the
    // two borders and a trailing space.
    let wide = open_on(120, 30, vec![open_todo("w0:p1", 1, &"x".repeat(79))]);
    assert_eq!(wide.outer.width, 90);

    // Forty rows on a tall screen: the list shows all of them.
    let todos: Vec<TodoInfo> = (1..=39)
        .map(|id| open_todo("w0:p1", id, "a todo"))
        .collect();
    assert!(open_on(200, 60, todos).list.height >= 40);
}

#[test]
fn the_board_stops_growing_at_its_cap_and_at_the_screen() {
    // The cap, centred.
    let capped = open_on(200, 30, vec![open_todo("w0:p1", 1, &"x".repeat(200))]);
    assert_eq!((capped.outer.width, capped.outer.x), (120, 40));
    // A short screen: the height clamps to it.
    let many: Vec<TodoInfo> = (1..=40)
        .map(|id| open_todo("w0:p1", id, "a todo"))
        .collect();
    assert!(open_on(80, 14, many.clone()).outer.height <= 14);
    // A narrow one: so does the width.
    assert!(open_on(40, 25, many).outer.width <= 40);
}

#[test]
fn an_empty_board_keeps_a_minimum_height() {
    let mut state = state_with(session(&[("board", &[""])]));
    open_with(&mut state, Vec::new());

    assert_eq!(layout(&state).outer.height, 8);
}

// -- grouping and selection ---------------------------------------------------

#[test]
fn the_board_groups_todos_by_pane_in_the_order_the_server_lists_them() {
    let state = sample();

    assert_eq!(
        kinds(board(&state)),
        [
            "# board · pane 1",
            "rerun the deploy",
            "check the 403",
            "read the plan",
            "",
            "# board · pane 2",
            "rotate the key",
            "",
            "# infra · deploy-bot",
            "tidy the dashboards",
            "ship v2",
        ]
    );
    // The opening selection steps over the heading.
    assert_eq!(board(&state).selected, 1);
}

#[test]
fn a_single_group_carries_no_blank_row() {
    let (snapshot, todos) = one_group();
    let mut state = state_with(snapshot);
    open_with(&mut state, todos);

    assert!(!kinds(board(&state)).iter().any(String::is_empty));
}

#[test]
fn a_pane_missing_from_the_snapshot_keeps_its_todos_under_a_fallback_name() {
    let mut state = state_with(sample_session());
    open_with(
        &mut state,
        vec![
            open_todo("w0:p1", 1, "known"),
            open_todo("w9:p9", 1, "orphan"),
        ],
    );

    assert_eq!(
        kinds(board(&state)),
        ["# board · pane 1", "known", "", "# pane 9", "orphan"]
    );
}

#[test]
fn selection_steps_over_headings_and_gaps_in_both_directions() {
    let mut state = state_with(session(&[("one", &["", ""])]));
    open_with(
        &mut state,
        vec![
            open_todo("w0:p1", 1, "a"),
            open_todo("w0:p1", 2, "b"),
            open_todo("w0:p2", 1, "c"),
        ],
    );
    assert_eq!(board(&state).selected, 1);

    state.handle_input_bytes(b"j");
    assert_eq!(board(&state).selected, 2);
    // One step crosses the gap and the next heading.
    state.handle_input_bytes(b"j");
    assert_eq!(board(&state).selected, 5);
    state.handle_input_bytes(b"k");
    assert_eq!(board(&state).selected, 2);
    state.handle_input_bytes(b"\x1b[F");
    assert_eq!(board(&state).selected, 5);
    state.handle_input_bytes(b"\x1b[H");
    assert_eq!(board(&state).selected, 1);
    // Movement clamps at both ends.
    state.handle_input_bytes(b"k");
    assert_eq!(board(&state).selected, 1);
}

#[test]
fn ctrl_d_moves_half_a_page_and_never_removes() {
    let mut state = sample();
    let moved = state.handle_input_bytes(b"\x04");

    assert!(methods(&moved).is_empty());
    // Eleven rows, half a page is five: from 1 to 6.
    assert_eq!(board(&state).selected, 6);
    state.handle_input_bytes(b"\x15");
    assert_eq!(board(&state).selected, 1);
}

#[test]
fn the_first_heading_comes_back_after_scrolling_to_the_bottom_and_back() {
    let todos: Vec<TodoInfo> = (1..=30)
        .map(|id| open_todo("w0:p1", id, "a todo"))
        .collect();
    let mut state = state_with(session(&[("board", &[""])]));
    open_with(&mut state, todos);
    assert_eq!(board(&state).scroll, 0);

    state.handle_input_bytes(b"\x1b[F");
    assert!(board(&state).scroll > 0);
    assert_eq!(board(&state).selected, 30);
    state.handle_input_bytes(b"\x1b[H");
    assert_eq!(board(&state).scroll, 0, "the heading above the first todo");
    assert_eq!(board(&state).selected, 1);
}

// -- search -------------------------------------------------------------------

#[test]
fn slash_filters_todos_and_keeps_their_heading() {
    let mut state = sample();
    state.handle_input_bytes(b"/403");

    assert_eq!(kinds(board(&state)), ["# board · pane 1", "check the 403"]);
    assert_eq!(selected(&state), ("w0:p1".into(), 2));
    // Enter travels to the match's owner.
    let (_, method) = only(&state.handle_input_bytes(b"\r"));
    assert!(matches!(method, Method::PaneFocus(ref target) if target.pane_id == "w0:p1"));
}

#[test]
fn a_matching_heading_keeps_all_of_its_todos_and_words_combine() {
    let mut state = sample();
    state.handle_input_bytes(b"/infra");
    assert_eq!(
        kinds(board(&state)),
        ["# infra · deploy-bot", "tidy the dashboards", "ship v2"]
    );

    state.handle_input_bytes(b"\x15");
    state.handle_input_bytes(b"deploy tidy");
    assert_eq!(
        kinds(board(&state)),
        ["# infra · deploy-bot", "tidy the dashboards"]
    );
}

#[test]
fn a_query_matches_the_todo_id_too() {
    let mut state = sample();
    state.handle_input_bytes(b"/#3");

    assert_eq!(kinds(board(&state)), ["# board · pane 1", "read the plan"]);
}

#[test]
fn board_letters_type_while_searching() {
    let mut state = sample();
    state.handle_input_bytes(b"/d");

    assert_eq!(board(&state).search.text(), "d");
    assert!(board(&state).search_focused);
    // No removal fired from the typed letter, and j / k are text too.
    let typed = state.handle_input_bytes(b"jkc");
    assert!(methods(&typed).is_empty());
    assert_eq!(board(&state).search.text(), "djkc");
}

#[test]
fn esc_leaves_the_search_then_clears_the_query_then_closes() {
    let mut state = sample();
    state.handle_input_bytes(b"/403");

    state.handle_input_bytes(b"\x1b");
    assert!(!board(&state).search_focused);
    assert_eq!(board(&state).search.text(), "403");

    state.handle_input_bytes(b"\x1b");
    assert_eq!(board(&state).search.text(), "");
    assert_eq!(board(&state).items.len(), 11);

    state.handle_input_bytes(b"\x1b");
    assert!(state.overlay.is_none());
}

#[test]
fn the_count_stays_at_the_whole_board_while_a_query_narrows_it() {
    let mut state = sample();
    state.handle_input_bytes(b"/403");
    let rows = boxed(&mut state);

    // The count is on the title row (fork issue 174).
    assert!(rows[1].ends_with("6 todos │"), "{}", rows[1]);
}

#[test]
fn pasted_text_goes_to_the_search_only_while_it_has_focus() {
    let paste = |state: &mut ClientShellState, text: &str| {
        state.handle_raw_events(vec![crate::raw_input::RawInputEvent::Paste(text.into())])
    };
    let mut state = sample();
    paste(&mut state, "403");
    assert_eq!(board(&state).search.text(), "");

    state.handle_input_bytes(b"/");
    paste(&mut state, "check\n403");
    assert_eq!(board(&state).search.text(), "check403");
}

// -- actions ------------------------------------------------------------------

#[test]
fn enter_goes_to_the_owner_pane_across_a_space_boundary() {
    let mut state = sample();
    state.handle_input_bytes(b"\x1b[F");
    assert_eq!(selected(&state), ("w1:p3".into(), 2));

    let (_, method) = only(&state.handle_input_bytes(b"\r"));

    assert!(matches!(method, Method::PaneFocus(ref target) if target.pane_id == "w1:p3"));
    assert!(state.overlay.is_none());
}

#[test]
fn g_follows_the_linked_pane_not_the_owner_and_a_dead_link_is_inert() {
    let mut state = live_link_session();
    let text = screen(&mut state).join("\n");
    assert!(text.contains("g go"), "{text}");
    let (_, method) = only(&state.handle_input_bytes(b"g"));
    assert!(matches!(method, Method::PaneFocus(ref target) if target.pane_id == "w1:p3"));
    assert!(state.overlay.is_none());

    let mut state = state_with(sample_session());
    let mut todos = sample_todos();
    todos[0] = linked(todos[0].clone(), "w1:p3", "deploy-bot", false);
    open_with(&mut state, todos);
    let text = screen(&mut state).join("\n");
    assert!(text.contains(" → deploy-bot "), "{text}");
    assert!(!text.contains("g go"), "{text}");
    assert!(methods(&state.handle_input_bytes(b"g")).is_empty());
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::TodoBoard(_))
    ));
}

#[test]
fn space_toggles_the_owners_todo_and_the_list_is_fetched_again() {
    let mut state = sample();
    state.handle_input_bytes(b"\x1b[F");

    let (id, method) = only(&state.handle_input_bytes(b" "));
    assert!(matches!(&method, Method::TodoUpdate(params)
        if params.pane_id == "w1:p3"
            && params.id == 2
            && params.done == Some(false)
            && params.text.is_none()));
    let (_, actions) = state.handle_endpoint_result("boot-1", &id, Ok(ResponseResult::Ok {}));

    let [(_, list)] = requests(&actions).try_into().expect("one request");
    assert!(is_board_list(&list), "{list:?}");
}

#[test]
fn d_removes_the_selected_todo_from_its_owner_and_a_failed_change_fetches_nothing() {
    let mut state = sample();
    state.handle_input_bytes(b"\x1b[F");

    let (id, method) = only(&state.handle_input_bytes(b"d"));
    assert!(matches!(&method, Method::TodoRemove(params)
        if params.pane_id == "w1:p3" && params.id == 2));
    let (_, actions) = state.handle_endpoint_result(
        "boot-1",
        &id,
        Err(ClientShellEndpointError {
            code: Some("pane_not_found".into()),
            message: "gone".into(),
        }),
    );
    assert!(requests(&actions).is_empty());
}

#[test]
fn c_clears_done_todos_on_every_pane_the_board_shows() {
    let mut state = sample();
    let sent = methods(&state.handle_input_bytes(b"c"));

    let panes: Vec<String> = sent
        .iter()
        .map(|(_, method)| match method {
            Method::TodoClear(params) => {
                assert!(params.done_only);
                params.pane_id.clone()
            }
            other => panic!("expected todo.clear, got {other:?}"),
        })
        .collect();
    assert_eq!(panes, ["w0:p1", "w0:p2", "w1:p3"]);
}

#[test]
fn clearing_acts_only_on_the_panes_a_query_leaves() {
    let mut state = sample();
    state.handle_input_bytes(b"/infra");
    state.handle_input_bytes(b"\x1b");

    let sent = methods(&state.handle_input_bytes(b"c"));

    assert!(matches!(sent.as_slice(),
        [(_, Method::TodoClear(params))] if params.pane_id == "w1:p3" && params.done_only));
}

#[test]
fn a_shifted_letter_does_what_its_lowercase_form_does() {
    let mut state = sample();
    let (_, shifted) = only(&state.handle_input_bytes(b"D"));

    assert!(matches!(shifted, Method::TodoRemove(_)));
}

#[test]
fn esc_and_q_close_the_board() {
    for key in [&b"\x1b"[..], b"q"] {
        let mut state = sample();
        state.handle_input_bytes(key);
        assert!(state.overlay.is_none());
    }
}

#[test]
fn a_change_to_any_pane_fetches_the_list_and_a_pending_fetch_is_not_doubled() {
    let mut state = sample();
    let sent = methods(&state.handle_input_bytes(b"c"));
    let [(a, _), (b, _), (c, _)] = sent.as_slice() else {
        panic!("expected three clears, got {sent:?}");
    };

    let (_, first) = state.handle_endpoint_result("boot-1", a, Ok(ResponseResult::Ok {}));
    let [(list_id, list)] = requests(&first).try_into().expect("one list request");
    assert!(is_board_list(&list));
    // The next answers land while that list is on its way.
    let (_, second) = state.handle_endpoint_result("boot-1", b, Ok(ResponseResult::Ok {}));
    let (_, third) = state.handle_endpoint_result("boot-1", c, Ok(ResponseResult::Ok {}));
    assert!(requests(&second).is_empty() && requests(&third).is_empty());

    // When it returns the board asks once more, so the last clear shows.
    let (_, again) = state.handle_endpoint_result(
        "boot-1",
        &list_id,
        Ok(ResponseResult::TodoList {
            todos: sample_todos(),
        }),
    );
    let [(_, list)] = requests(&again).try_into().expect("one more list request");
    assert!(is_board_list(&list));
}

// -- refetch ------------------------------------------------------------------

#[test]
fn the_selection_survives_a_todo_removed_above_it() {
    let mut state = sample();
    state.handle_input_bytes(b"j");
    assert_eq!(selected(&state), ("w0:p1".into(), 2));

    let mut todos = sample_todos();
    todos.remove(0);
    refetch(&mut state, todos);

    assert_eq!(selected(&state), ("w0:p1".into(), 2));
    assert_eq!(board(&state).selected, 1);
    assert_eq!(board(&state).items.len(), 10);
}

#[test]
fn a_selection_whose_todo_is_gone_moves_to_the_nearest_one() {
    let mut state = sample();
    state.handle_input_bytes(b"j");
    let mut todos = sample_todos();
    todos.remove(1);
    refetch(&mut state, todos);

    // Index 1 is now the next todo in the same group.
    assert_eq!(selected(&state), ("w0:p1".into(), 3));
}

#[test]
fn removing_a_panes_last_todo_drops_its_heading() {
    let mut state = sample();
    let mut todos = sample_todos();
    todos.retain(|todo| todo.pane_id != "w0:p2");
    refetch(&mut state, todos);

    let kinds = kinds(board(&state));
    assert!(!kinds.contains(&"# board · pane 2".to_owned()), "{kinds:?}");
    assert_eq!(kinds.len(), 8);
}

#[test]
fn a_refetch_keeps_the_query_and_the_scroll() {
    let mut state = sample();
    state.handle_input_bytes(b"/the");
    let before = board(&state).scroll;
    refetch(&mut state, sample_todos());

    assert_eq!(board(&state).search.text(), "the");
    assert_eq!(board(&state).scroll, before);
    assert!(board(&state).items.len() < 11);
}

#[test]
fn a_new_todo_revision_in_the_snapshot_fetches_the_list_again() {
    let mut state = sample();
    let mut quiet = ClientShellInput::default();
    state.tick_todo_board(&mut quiet);
    assert!(
        methods(&quiet).is_empty(),
        "nothing changed, nothing to fetch"
    );

    let mut projected = sample_session();
    projected.revision = 2;
    projected.resource_facts = Some(crate::protocol::ClientShellResourceFacts {
        pane_todos: Some(
            [(
                "w1:p3".to_owned(),
                crate::protocol::ClientPaneTodoSummary {
                    total: 2,
                    open: 1,
                    highest_priority: Some("low".into()),
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
    state.tick_todo_board(&mut outcome);

    assert!(is_board_list(&only(&outcome).1));
    // And not again while that fetch is on its way.
    let mut busy = ClientShellInput::default();
    state.tick_todo_board(&mut busy);
    assert!(methods(&busy).is_empty());
}

// -- editor -------------------------------------------------------------------

#[test]
fn editing_from_the_board_opens_the_editor_on_the_owning_pane() {
    let mut state = sample();
    state.handle_input_bytes(b"\x1b[F");

    state.handle_input_bytes(b"e");

    assert_eq!(edit(&state).pane_id, "w1:p3");
    assert_eq!(edit(&state).todo_id, Some(2));
    assert_eq!(edit(&state).text.text(), "ship v2");
    assert!(matches!(
        edit(&state).suspended,
        Some(SuspendedTodoSurface::Board(_))
    ));
}

#[test]
fn cancelling_an_edit_opened_from_the_board_returns_to_the_board() {
    let mut state = sample();
    state.handle_input_bytes(b"\x1b[F");
    state.handle_input_bytes(b"e");

    let cancelled = state.handle_input_bytes(b"\x1b");

    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::TodoBoard(_))
    ));
    assert_eq!(selected(&state), ("w1:p3".into(), 2));
    assert_eq!(kinds(board(&state)).len(), 11);
    // The board fetches its list again, as the panel does.
    assert!(is_board_list(&only(&cancelled).1));
}

#[test]
fn saving_an_edit_opened_from_the_board_writes_to_the_owner_and_rebuilds_the_board() {
    let mut state = sample();
    state.handle_input_bytes(b"\x1b[F");
    state.handle_input_bytes(b"e");
    state.handle_input_bytes(b"!");

    let (id, method) = only(&state.handle_input_bytes(b"\x13"));
    assert!(matches!(&method, Method::TodoUpdate(params)
        if params.pane_id == "w1:p3"
            && params.id == 2
            && params.text.as_deref() == Some("ship v2!")));
    let (_, actions) = state.handle_endpoint_result(
        "boot-1",
        &id,
        Ok(ResponseResult::Todo {
            todo: todo("w1:p3", 2, "ship v2!", TodoPriority::Low, true),
        }),
    );

    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::TodoBoard(_))
    ));
    let [(list_id, list)] = requests(&actions).try_into().expect("one request");
    assert!(is_board_list(&list));
    let mut todos = sample_todos();
    todos[5].text = "ship v2!".into();
    state.handle_endpoint_result("boot-1", &list_id, Ok(ResponseResult::TodoList { todos }));
    assert_eq!(board(&state).selected_todo().unwrap().text, "ship v2!");
    assert_eq!(selected(&state), ("w1:p3".into(), 2));
}

#[test]
fn saving_and_following_from_the_board_leaves_no_overlay_behind() {
    let mut state = live_link_session();
    state.handle_input_bytes(b"e");

    let (id, _) = only(&state.handle_input_bytes(b"\x07"));
    let (_, actions) = state.handle_endpoint_result("boot-1", &id, Ok(ResponseResult::Ok {}));

    assert!(state.overlay.is_none());
    let [(_, follow)] = requests(&actions).try_into().expect("one request");
    assert!(matches!(follow, Method::PaneFocus(ref target) if target.pane_id == "w1:p3"));
}

// -- mouse --------------------------------------------------------------------

#[test]
fn the_first_click_selects_and_the_second_opens_the_owner() {
    let mut state = sample();
    let first = click_item(&mut state, 9);

    assert!(methods(&first).is_empty());
    assert_eq!(selected(&state), ("w1:p3".into(), 1));
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::TodoBoard(_))
    ));
    let (_, method) = only(&click_item(&mut state, 9));
    assert!(matches!(method, Method::PaneFocus(ref target) if target.pane_id == "w1:p3"));
    assert!(state.overlay.is_none());
}

#[test]
fn a_single_click_on_the_already_selected_first_row_acts() {
    let mut state = sample();
    let (_, method) = only(&click_item(&mut state, 1));

    assert!(matches!(method, Method::PaneFocus(ref target) if target.pane_id == "w0:p1"));
}

#[test]
fn clicking_a_heading_or_the_gap_between_groups_is_inert() {
    let mut state = sample();
    for index in [0, 4, 5, 7, 8] {
        let outcome = click_item(&mut state, index);
        assert!(methods(&outcome).is_empty(), "item {index}");
        assert_eq!(selected(&state), ("w0:p1".into(), 1), "item {index}");
        assert!(matches!(
            state.overlay,
            Some(ClientShellOverlay::TodoBoard(_))
        ));
    }
}

#[test]
fn a_chip_acts_on_the_first_click_wherever_the_selection_is() {
    let mut state = live_link_session();
    state.handle_input_bytes(b"jj");
    assert_eq!(selected(&state), ("w0:p1".into(), 3));
    let rows = screen(&mut state);
    let (y, line) = rows
        .iter()
        .enumerate()
        .find(|(_, line)| line.contains('→'))
        .expect("a chip");
    let x = line.chars().position(|c| c == '→').unwrap();

    let (_, method) = only(&click(&mut state, x as u16, y as u16));

    assert!(matches!(method, Method::PaneFocus(ref target) if target.pane_id == "w1:p3"));
    assert!(state.overlay.is_none());
}

#[test]
fn a_dead_chip_selects_its_row_and_goes_nowhere() {
    let mut state = state_with(sample_session());
    let mut todos = sample_todos();
    todos[1] = linked(todos[1].clone(), "w1:p3", "deploy-bot", false);
    open_with(&mut state, todos);
    let rows = screen(&mut state);
    let (y, line) = rows
        .iter()
        .enumerate()
        .find(|(_, line)| line.contains('→'))
        .expect("a chip");
    let x = line.chars().position(|c| c == '→').unwrap();

    let outcome = click(&mut state, x as u16, y as u16);

    assert!(methods(&outcome).is_empty());
    assert_eq!(selected(&state), ("w0:p1".into(), 2));
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::TodoBoard(_))
    ));
}

#[test]
fn the_pointer_and_the_wheel_leave_the_selection_alone() {
    let todos: Vec<TodoInfo> = (1..=30)
        .map(|id| open_todo("w0:p1", id, "a todo"))
        .collect();
    let mut state = state_with(session(&[("board", &[""])]));
    open_with(&mut state, todos);
    let layout = layout(&state);

    mouse(
        &mut state,
        MouseEventKind::Moved,
        layout.list.x + 4,
        layout.list.y + 5,
    );
    assert_eq!(board(&state).selected, 1, "the pointer does not drag it");
    for _ in 0..3 {
        mouse(
            &mut state,
            MouseEventKind::ScrollDown,
            layout.list.x + 4,
            layout.list.y,
        );
    }
    assert_eq!(board(&state).scroll, 3);
    assert_eq!(board(&state).selected, 1);
    mouse(
        &mut state,
        MouseEventKind::ScrollUp,
        layout.list.x + 4,
        layout.list.y,
    );
    assert_eq!(board(&state).scroll, 2);
}

#[test]
fn the_pointer_hovers_a_footer_button() {
    let mut state = sample();
    let layout = layout(&state);
    let (toggle, _) = layout
        .buttons
        .iter()
        .find(|(_, button)| *button == TodoBoardButton::Toggle)
        .copied()
        .unwrap();

    mouse(&mut state, MouseEventKind::Moved, toggle.x + 1, toggle.y);
    assert_eq!(board(&state).hovered_button, Some(TodoBoardButton::Toggle));
    mouse(
        &mut state,
        MouseEventKind::Moved,
        layout.list.x + 4,
        layout.list.y + 2,
    );
    assert_eq!(board(&state).hovered_button, None);
}

#[test]
fn the_footer_buttons_act_on_the_selection_and_close_dismisses() {
    let click_button = |state: &mut ClientShellState, which: TodoBoardButton| {
        let layout = layout(state);
        let (rect, _) = layout
            .buttons
            .iter()
            .find(|(_, button)| *button == which)
            .copied()
            .expect("button");
        click(state, rect.x + 1, rect.y)
    };

    let mut state = sample();
    let (_, method) = only(&click_button(&mut state, TodoBoardButton::Open));
    assert!(matches!(method, Method::PaneFocus(ref target) if target.pane_id == "w0:p1"));

    let mut state = sample();
    let (_, method) = only(&click_button(&mut state, TodoBoardButton::Toggle));
    assert!(matches!(method, Method::TodoUpdate(ref params) if params.id == 1));

    let mut state = sample();
    assert_eq!(
        methods(&click_button(&mut state, TodoBoardButton::ClearDone)).len(),
        3
    );

    let mut state = sample();
    click_button(&mut state, TodoBoardButton::Close);
    assert!(state.overlay.is_none());
}

#[test]
fn the_go_button_appears_only_for_a_live_link_and_follows_it() {
    let mut state = live_link_session();
    let layout = layout(&state);
    let (rect, _) = layout
        .buttons
        .iter()
        .find(|(_, button)| *button == TodoBoardButton::Go)
        .copied()
        .expect("go button");

    let (_, method) = only(&click(&mut state, rect.x + 1, rect.y));
    assert!(matches!(method, Method::PaneFocus(ref target) if target.pane_id == "w1:p3"));

    let state = sample();
    assert!(layout_has_no_go(&state));
}

fn layout_has_no_go(state: &ClientShellState) -> bool {
    !layout(state)
        .buttons
        .iter()
        .any(|(_, button)| *button == TodoBoardButton::Go)
}

#[test]
fn inside_clicks_off_the_rows_are_inert_and_the_search_row_takes_focus() {
    let mut state = sample();
    let layout = layout(&state);

    // The blank row under the search, the title and the footer's blank row.
    for y in [
        layout.search_row().y + 1,
        layout.header_row(0).y,
        layout.footer_row.unwrap().y - 1,
    ] {
        click(&mut state, layout.list.x + 3, y);
        assert!(matches!(
            state.overlay,
            Some(ClientShellOverlay::TodoBoard(_))
        ));
        assert!(!board(&state).search_focused);
    }
    // A near miss anywhere on the buttons' row is inert.
    click(&mut state, 0, layout.footer_row.unwrap().y);
    assert!(state.overlay.is_some());
    click(&mut state, layout.list.x + 3, layout.search_row().y);
    assert!(board(&state).search_focused);
}

#[test]
fn clicking_outside_the_board_dismisses_it() {
    let mut state = sample();

    click(&mut state, 0, 0);

    assert!(state.overlay.is_none());
}

#[test]
fn clicks_and_the_wheel_do_not_reach_the_panes_beneath() {
    let mut state = sample();
    let outcome = mouse(&mut state, MouseEventKind::ScrollDown, 1, 1);

    assert!(methods(&outcome).is_empty());
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::TodoBoard(_))
    ));
}

// -- keybindings --------------------------------------------------------------

#[test]
fn opening_the_board_asks_for_every_panes_todos() {
    let mut state = state_with(sample_session());
    let outcome = bind(&mut state, KeybindAction::OpenTodoBoard);

    assert!(is_board_list(&only(&outcome).1));
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::TodoBoard(_))
    ));
}

#[test]
fn keybind_help_lists_the_board_and_its_chords() {
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
        find("panes", "session todo board").as_deref(),
        Some("unset")
    );
    assert_eq!(
        find("todo board", "open owner pane").as_deref(),
        Some("enter / click twice")
    );
    assert_eq!(find("todo board", "edit todo").as_deref(), Some("e"));
    assert_eq!(find("todo board", "toggle done").as_deref(), Some("spc"));
    assert_eq!(find("todo board", "follow link").as_deref(), Some("g"));
    assert_eq!(find("todo board", "remove todo").as_deref(), Some("d"));
    assert_eq!(find("todo board", "clear done").as_deref(), Some("c"));
    assert_eq!(find("todo board", "search todos").as_deref(), Some("/"));
    assert_eq!(
        find("todo board", "close board").as_deref(),
        Some("esc / q")
    );
}

// -- the approved look (fork issue 174) -----------------------------------------

#[test]
fn the_board_title_row_carries_a_dim_count_and_the_rule_is_surface1() {
    let mut state = sample();
    let frame = state.compose(W, H).expect("frame");
    let layout = layout(&state);
    let title = layout.header_row(0);
    let cell = |x: u16, y: u16| &frame.cells[usize::from(y) * usize::from(W) + usize::from(x)];
    let p = &state.config.palette;
    // The count's last digit is two cells in from the title row's right edge.
    let count = cell(title.right() - 2, title.y);
    assert_eq!(count.symbol, "s", "the count ends the title row");
    assert_eq!(count.fg, crate::protocol::color_to_u32(p.overlay0));
    let rule = cell(title.x + 1, layout.search_row().y + 1);
    assert_eq!(rule.symbol, "─");
    assert_eq!(rule.fg, crate::protocol::color_to_u32(p.surface1));
}

#[test]
fn open_pane_is_always_the_accent_primary_and_the_rest_are_surface0() {
    let mut state = sample();
    let frame = state.compose(W, H).expect("frame");
    let layout = layout(&state);
    let p = &state.config.palette;
    let at =
        |rect: Rect| &frame.cells[usize::from(rect.y) * usize::from(W) + usize::from(rect.x + 1)];
    for (rect, button) in &layout.buttons {
        let cell = at(*rect);
        let expected = if *button == TodoBoardButton::Open {
            p.accent
        } else {
            p.surface0
        };
        assert_eq!(
            cell.bg,
            crate::protocol::color_to_u32(expected),
            "{button:?}"
        );
    }
}
