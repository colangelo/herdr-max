//! The session todo board: every pane's todos in one place.
//!
//! A todo is written *in* a pane, because that is where the work is, but it is
//! read *across* panes, when deciding what to pick up next. The pane panel
//! answers "what about this pane?"; this answers "what is outstanding?", and it
//! is actionable where `herdr todo list --all` is not — activating a row goes
//! to the pane that owns the todo.
//!
//! Deliberately built from the same parts as the panel: the kit's `ButtonRow`
//! footer, its `ListCursor`, and the panel's own row renderer. The board is the
//! panel widened, not a second feature.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::overlay::{
    render_search_row, AnchoredPanelSpec, ButtonRow, ButtonSpec, SearchRow, VerticalAnchor,
    LIST_DIALOG_MAX_WIDTH,
};
use super::text::truncate_end;
use super::todo_panel::{pane_todo_row_hides_text, render_pane_todo_row};
use super::widgets::{
    heading_style, panel_contrast_fg, render_action_button, render_modal_header,
    render_panel_shell, sub_heading_style, FOOTER_ROWS, HEADER_ROWS,
};
use crate::app::state::{AppState, TodoBoardButton, TodoBoardItem};

/// Columns the board asks for before its own content or footer has a say.
///
/// Raised from 64 after dogfooding: a session whose longest row was 66 columns
/// got a 66-column box in the middle of a very wide terminal, truncating todos
/// at a width the screen had no need to impose.
const TODO_BOARD_MIN_WIDTH: u16 = 80;

/// Columns a group's todos are drawn in from its heading.
///
/// Applied by narrowing the rect a todo is drawn into rather than by teaching
/// [`render_pane_todo_row`] an indent: that renderer is shared with the pane
/// todo panel so a todo reads the same in both, and a board concern has no
/// business in it. Headings are deliberately not indented — a heading is the
/// thing being indented *from*.
pub(crate) const TODO_BOARD_TODO_INDENT: u16 = 2;

/// Rows the board's header block occupies: its title, its search row, and the
/// blank row under them. One more than [`HEADER_ROWS`] because this overlay's
/// header is two lines rather than one.
pub(crate) const TODO_BOARD_HEADER_ROWS: u16 = HEADER_ROWS + 1;

/// The board's footer, in the panel's language: the shortcut hint inside the
/// filled box, in render order.
///
/// `toggle` and `clear done` are absent when no pane holds a todo — there is
/// nothing to toggle or clear — and `go` is absent unless the selected todo's
/// link resolves, since there is nowhere to go otherwise. `open` and `close`
/// always survive: `open` is why the board exists over the CLI's flat listing,
/// and an overlay that could not be dismissed is the dead end a footer
/// prevents.
///
/// There is no `add`: adding is pane-scoped, and the board has no pane of its
/// own to add to.
pub(crate) fn todo_board_button_specs(
    has_todos: bool,
    has_live_link: bool,
) -> Vec<ButtonSpec<TodoBoardButton>> {
    let mut specs = Vec::with_capacity(5);
    specs.push(ButtonSpec {
        button: TodoBoardButton::Open,
        hint: Some("↵"),
        label: "open pane",
        drop_rank: None,
    });
    if has_todos {
        specs.push(ButtonSpec {
            button: TodoBoardButton::Toggle,
            hint: Some("spc"),
            label: "toggle",
            drop_rank: Some(0),
        });
    }
    if has_live_link {
        specs.push(ButtonSpec {
            button: TodoBoardButton::Go,
            hint: Some("g"),
            label: "go",
            drop_rank: Some(2),
        });
    }
    if has_todos {
        specs.push(ButtonSpec {
            button: TodoBoardButton::ClearDone,
            hint: Some("c"),
            label: "clear done",
            drop_rank: Some(1),
        });
    }
    specs.push(ButtonSpec {
        button: TodoBoardButton::Close,
        hint: Some("esc"),
        label: "close",
        drop_rank: None,
    });
    specs
}

/// Where the board sits. Centred rather than anchored: it belongs to the
/// session, not to a pane, so there is nothing to hang it off.
///
/// It grows to its content in both directions and lets the screen do the
/// clamping, because the board's whole job is showing a session's worth of
/// todos at once — a fixed box turned that into scrolling as soon as a few
/// panes had work. Width stops at the cap every centred list dialog shares.
///
/// `content_width` is what the caller measured its rows to need, and
/// `item_count` every row of the projection rather than the filtered ones, so
/// a query does not resize the board. The width is never less than the
/// footer's natural width, measured from the full set of boxes rather than
/// the ones the current selection happens to offer, so the box does not
/// resize as the selection moves either.
pub(crate) fn todo_board_spec(
    area: Rect,
    item_count: usize,
    content_width: u16,
) -> AnchoredPanelSpec {
    AnchoredPanelSpec {
        anchor: area,
        screen: area,
        content_width: content_width
            .max(ButtonRow::natural_width(&todo_board_button_specs(true, true)).saturating_add(2)),
        width_bounds: (TODO_BOARD_MIN_WIDTH, LIST_DIALOG_MAX_WIDTH),
        rows: item_count.min(usize::from(u16::MAX)) as u16,
        max_rows: u16::MAX,
        footer_rows: FOOTER_ROWS,
        detail_rows: 0,
        detail_placement: crate::ui::overlay::TODO_DETAIL_PLACEMENT,
        header_rows: TODO_BOARD_HEADER_ROWS,
        vertical: VerticalAnchor::Centered,
    }
}

/// [`todo_board_spec`] resolved with no detail box.
#[cfg(test)]
pub(crate) fn todo_board_geometry(
    area: Rect,
    item_count: usize,
    content_width: u16,
) -> Option<super::overlay::PanelGeometry> {
    todo_board_spec(area, item_count, content_width).resolve()
}

/// A group heading's text: the space, then the pane's label —
/// `infra · imap-jmap-mcp`.
///
/// No addressable identifier. It carried one, and it earned nothing: the
/// identifier is a creation counter in a base-32 alphabet, so `w5:pV` is the
/// 27th pane ever opened in that space rather than a position you can count to
/// — unreadable *and* unrelated to what is on screen. It also named the space
/// twice, since `w5` and the space's own name are the same fact. What a
/// heading is for is recognising the pane, which the space and the label do,
/// and `Enter` travels there without anyone having to read an address.
///
/// The todo link chip still carries its identifier, because a chip is a
/// destination you may want to address rather than a group you are reading.
pub(crate) fn todo_board_heading_text(space: &str, label: &str) -> String {
    match (space.is_empty(), label.is_empty()) {
        (false, false) => format!(" {space} · {label}"),
        (false, true) => format!(" {space}"),
        (true, false) => format!(" {label}"),
        (true, true) => String::new(),
    }
}

/// A group heading as styled spans: the space in the help pane's heading
/// style, ` · ` dim, the label in its sub-heading style, cut to `max_width`.
pub(crate) fn todo_board_heading_line(
    p: &crate::app::state::Palette,
    space: &str,
    label: &str,
    max_width: usize,
) -> Line<'static> {
    let text = truncate_end(&todo_board_heading_text(space, label), max_width);
    let space_len = if space.is_empty() {
        0
    } else {
        space.chars().count() + 1
    };
    let sep_len = if !space.is_empty() && !label.is_empty() {
        3
    } else {
        0
    };
    let mut chars = text.chars();
    let mut take = |n: usize| chars.by_ref().take(n).collect::<String>();
    let space_part = take(space_len);
    let sep_part = take(sep_len);
    let label_part: String = chars.collect();
    let mut spans = Vec::new();
    for (part, style) in [
        (space_part, heading_style(p)),
        (sep_part, Style::default().fg(p.overlay0)),
        (label_part, sub_heading_style(p)),
    ] {
        if !part.is_empty() {
            spans.push(Span::styled(part, style));
        }
    }
    Line::from(spans)
}

pub(super) fn render_todo_board(app: &AppState, frame: &mut Frame) {
    let Some(board) = app.todo_board() else {
        return;
    };
    let area = frame.area();
    // The same resolved geometry the mouse hit-tests against, so what is drawn
    // and what is clickable cannot diverge.
    let Some(geometry) = app.todo_board_geometry() else {
        return;
    };
    let p = &app.palette;
    super::dim_background(frame, area);
    if render_panel_shell(frame, geometry.outer, p.accent, p.panel_bg).is_none() {
        return;
    }
    let header = geometry.header;
    if header.height >= 2 {
        // `todos/notes`, not `todos`: what a pane records is as often a note
        // to self as a task, and a title naming only one of them says the
        // other does not belong here. Only the title changes — the CLI, the
        // config key, the socket method and the protocol are addresses, and
        // renaming those costs a protocol bump and a permanent divergence from
        // upstream's `todo` naming.
        render_modal_header(frame, geometry.header_row(0), "todos/notes", p);
        let count = board
            .all_items
            .iter()
            .filter(|item| matches!(item, TodoBoardItem::Todo { .. }))
            .count();
        render_search_row(
            frame,
            geometry.header_row(1),
            &board.search,
            SearchRow {
                placeholder: "search todos",
                count: format!("{count} {}", if count == 1 { "todo" } else { "todos" }),
                chip: None,
            },
            p,
        );
    }

    // The empty state still falls through to the footer: opening the board on
    // a quiet session must say so rather than look like a broken keybinding.
    if board.items.is_empty() {
        let message = if board.all_items.is_empty() {
            " nothing outstanding"
        } else {
            " no match"
        };
        frame.render_widget(
            Paragraph::new(message).style(Style::default().fg(p.overlay0)),
            Rect::new(
                geometry.list.x,
                geometry.list.y,
                geometry.list.width,
                geometry.list.height.min(1),
            ),
        );
    }

    let (start, visible) = board.list.window(geometry.list, board.items.len());
    for (row, item) in board.items.iter().skip(start).take(visible).enumerate() {
        let row_rect = Rect::new(
            geometry.list.x,
            geometry.list.y + row as u16,
            geometry.list.width,
            1,
        );
        match item {
            // The separation between groups is the row itself; there is
            // nothing to draw into it.
            TodoBoardItem::GroupGap => {}
            TodoBoardItem::PaneHeading { space, label } => {
                // The help pane's heading colours (fork issue 142): the space
                // like a group name, the pane's label like a key chord.
                frame.render_widget(
                    Paragraph::new(todo_board_heading_line(
                        p,
                        space,
                        label,
                        row_rect.width as usize,
                    )),
                    row_rect,
                );
            }
            TodoBoardItem::Todo { pane_id, todo_id } => {
                let Some(todo) = app.pane_todo_ref(*pane_id, *todo_id) else {
                    continue;
                };
                render_pane_todo_row(
                    frame,
                    app,
                    todo_board_todo_rect(row_rect),
                    todo,
                    start + row == board.list.selected,
                );
            }
        }
    }

    if let Some(detail) = geometry.detail {
        let row = todo_board_todo_rect(Rect::new(detail.x, detail.y, detail.width, 1));
        let text = board
            .selected_todo()
            .and_then(|(pane_id, todo_id)| app.pane_todo_ref(pane_id, todo_id))
            .filter(|todo| pane_todo_row_hides_text(app, row, todo))
            .map(|todo| todo.text.as_str());
        crate::ui::overlay::render_detail_box(frame, detail, text, p);
    }

    if let Some(buttons) = app.todo_board_buttons() {
        let hovered = board.hovered_button;
        for placed in buttons.placed() {
            let style = if hovered == Some(placed.button) {
                Style::default()
                    .fg(panel_contrast_fg(p))
                    .bg(p.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(p.text)
                    .bg(p.surface0)
                    .add_modifier(Modifier::BOLD)
            };
            render_action_button(frame, placed.rect, placed.hint, placed.label, style);
        }
    }
}

/// A board row narrowed to where its todo is drawn: in from its heading by
/// [`TODO_BOARD_TODO_INDENT`].
pub(crate) fn todo_board_todo_rect(row_rect: Rect) -> Rect {
    Rect::new(
        row_rect.x.saturating_add(TODO_BOARD_TODO_INDENT),
        row_rect.y,
        row_rect.width.saturating_sub(TODO_BOARD_TODO_INDENT),
        row_rect.height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    use crate::terminal::todo::{TodoPriority, TodoUpdate};
    use crate::ui::test_support;

    /// A one-pane app whose pane carries `todos`, laid out for the snapshot
    /// frame with the board left closed — the background an open board is
    /// diffed against.
    fn app_with_todos(todos: &[(&str, bool, TodoPriority)]) -> AppState {
        let mut app = test_support::app_with_one_pane("board");
        let pane_id = app.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let terminal = app
            .terminals
            .get_mut(&terminal_id)
            .expect("test terminal should exist");
        for (text, done, priority) in todos {
            let todo = terminal
                .add_todo(text, *priority, None, 100)
                .expect("todo should be added");
            if *done {
                terminal
                    .update_todo(
                        todo.id,
                        TodoUpdate {
                            done: Some(true),
                            ..TodoUpdate::default()
                        },
                        200,
                    )
                    .expect("todo should be updated");
            }
        }
        test_support::layout(&mut app);
        app
    }

    fn snapshot(todos: &[(&str, bool, TodoPriority)]) -> test_support::OverlaySnapshot {
        let base = app_with_todos(todos);
        let mut open = app_with_todos(todos);
        open.open_todo_board(&crate::terminal::TerminalRuntimeRegistry::new());
        test_support::layout(&mut open);
        test_support::overlay_snapshot(&base, &open)
    }

    #[test]
    fn snapshot_populated() {
        snapshot(&[
            ("rerun the deploy", false, TodoPriority::High),
            ("check the 403", false, TodoPriority::Normal),
            ("archive the change", true, TodoPriority::Low),
        ])
        .assert(
            Rect::new(0, 7, 80, 11),
            &[
                "┌──────────────────────────────────────────────────────────────────────────────┐",
                "│ todos/notes                                                                  │",
                "│ / search todos                                                       3 todos │",
                "│                                                                              │",
                "│ board · pane 1                                                               │",
                "│   ▲ rerun the deploy                                                       #1│",
                "│   ● check the 403                                                          #2│",
                "│   ✓ archive the change                                                     #3│",
                "│                                                                              │",
                "│            ↵ open pane    spc toggle    c clear done    esc close            │",
                "└──────────────────────────────────────────────────────────────────────────────┘",
            ],
        );
    }

    /// Two panes with work, which is the case the board exists for and the
    /// only one that can show a gap. The blank row between the groups is the
    /// separation the single-group snapshot has nothing to demonstrate.
    fn app_with_two_groups() -> AppState {
        let mut app = test_support::app_with_one_pane("board");
        let first = app.workspaces[0].tabs[0].root_pane;
        let second = app.workspaces[0].test_split(ratatui::layout::Direction::Horizontal);
        app.ensure_test_terminals();
        for (pane_id, text) in [(first, "rerun the deploy"), (second, "check the 403")] {
            let terminal_id = app.workspaces[0].tabs[0].panes[&pane_id]
                .attached_terminal_id
                .clone();
            app.terminals
                .get_mut(&terminal_id)
                .expect("test terminal should exist")
                .add_todo(text, TodoPriority::Normal, None, 100)
                .expect("todo should be added");
        }
        test_support::layout(&mut app);
        app
    }

    /// Fork issue 142 (ac picked it): title, box, search, list. The box sits
    /// right under the title, the search row under the box, and the list
    /// right under the search row's blank.
    #[test]
    fn the_detail_box_sits_under_the_title_above_the_search() {
        let mut open = app_with_todos(&[
            ("the plan\nstep one\nstep two", false, TodoPriority::High),
            ("one line", false, TodoPriority::Normal),
        ]);
        open.open_todo_board(&crate::terminal::TerminalRuntimeRegistry::new());
        test_support::layout(&mut open);
        let geometry = open.todo_board_geometry().expect("board");
        let detail = geometry.detail.expect("the box is there");
        let title = geometry.header_row(0);
        let search = geometry.header_row(1);

        assert_eq!(detail.y, title.y + 1, "the box is right under the title");
        assert_eq!(search.y, detail.y + detail.height, "search under the box");
        assert_eq!(geometry.list.y, search.y + 2, "a blank row, then the list");
        let buffer = test_support::draw_sized(
            &open,
            test_support::SNAPSHOT_WIDTH,
            test_support::SNAPSHOT_HEIGHT,
        );
        let rows = test_support::rect_rows(&buffer, geometry.outer).join("\n");
        let at = |needle: &str| {
            rows.find(needle)
                .unwrap_or_else(|| panic!("{needle}: {rows}"))
        };
        assert!(at("todos/notes") < at("step two"));
        assert!(at("step two") < at("search todos"));
        assert!(at("search todos") < at("the plan ⏎"));
    }

    /// The group header takes the help pane's two styles, from the one helper
    /// both use.
    #[test]
    fn the_group_header_uses_the_help_panes_styles() {
        let app = app_with_todos(&[("one", false, TodoPriority::Normal)]);
        let p = &app.palette;
        let line = todo_board_heading_line(p, "infra", "infra-helper", 80);
        let styles: Vec<_> = line.spans.iter().map(|span| span.style).collect();
        assert_eq!(
            styles,
            vec![
                crate::ui::widgets::heading_style(p),
                Style::default().fg(p.overlay0),
                crate::ui::widgets::sub_heading_style(p),
            ]
        );
        assert_eq!(line.to_string(), " infra · infra-helper");
        assert_eq!(styles[0].fg, Some(p.accent));
        assert_eq!(styles[2].fg, Some(p.mauve));

        // And the help pane itself draws with the same two.
        let help = crate::ui::keybind_help::keybind_help_lines(&app);
        let group = &help[0].1.spans[0].style;
        assert_eq!(*group, crate::ui::widgets::heading_style(p));
        let chord = help
            .iter()
            .find_map(|(_, line)| (line.spans.len() == 2).then(|| line.spans[0].style))
            .expect("a key chord");
        assert_eq!(chord, crate::ui::widgets::sub_heading_style(p));
    }

    /// A heading cut short keeps the split between its parts.
    #[test]
    fn a_cut_group_header_keeps_its_styles() {
        let app = app_with_todos(&[]);
        let line = todo_board_heading_line(&app.palette, "infra", "infra-helper", 12);
        assert_eq!(line.to_string().chars().count(), 12);
        assert_eq!(line.spans[0].content, " infra");
        assert_eq!(line.spans[0].style.fg, Some(app.palette.accent));
        assert_eq!(
            line.spans.last().expect("a span").style.fg,
            Some(app.palette.mauve)
        );
    }

    #[test]
    fn snapshot_two_groups_are_separated_by_a_blank_row() {
        let base = app_with_two_groups();
        let mut open = app_with_two_groups();
        open.open_todo_board(&crate::terminal::TerminalRuntimeRegistry::new());
        test_support::layout(&mut open);
        test_support::overlay_snapshot(&base, &open).assert(
            Rect::new(0, 6, 80, 12),
            &[
                "┌──────────────────────────────────────────────────────────────────────────────┐",
                "│ todos/notes                                                                  │",
                "│ / search todos                                                       2 todos │",
                "│                                                                              │",
                "│ board · pane 1                                                               │",
                "│   ● rerun the deploy                                                       #1│",
                "│                                                                              │",
                "│ board · pane 2                                                               │",
                "│   ● check the 403                                                          #1│",
                "│                                                                              │",
                "│            ↵ open pane    spc toggle    c clear done    esc close            │",
                "└──────────────────────────────────────────────────────────────────────────────┘",
            ],
        );
    }

    /// `/` and a word: the other group and its gap are gone, the heading of
    /// the match stays, and the box keeps its size.
    #[test]
    fn snapshot_board_searching() {
        let base = app_with_two_groups();
        let mut open = app_with_two_groups();
        open.open_todo_board(&crate::terminal::TerminalRuntimeRegistry::new());
        if let Some(board) = open.todo_board_mut() {
            board.search.focus();
            board.search.query.insert_str("403");
        }
        open.refilter_todo_board();
        test_support::layout(&mut open);
        test_support::overlay_snapshot(&base, &open).assert(
            Rect::new(0, 6, 80, 12),
            &[
                "┌──────────────────────────────────────────────────────────────────────────────┐",
                "│ todos/notes                                                                  │",
                "│ / 403                                                                2 todos │",
                "│                                                                              │",
                "│ board · pane 2                                                               │",
                "│   ● check the 403                                                          #1│",
                "│                                                                              │",
                "│                                                                              │",
                "│                                                                              │",
                "│                                                                              │",
                "│            ↵ open pane    spc toggle    c clear done    esc close            │",
                "└──────────────────────────────────────────────────────────────────────────────┘",
            ],
        );
    }

    /// The board shows the selected todo's full text in the same kit box as
    /// the pane panel, whenever a row hides text.
    #[test]
    fn the_board_shows_the_detail_box_for_a_todo_that_hides_text() {
        let mut open = app_with_todos(&[
            ("the plan\nstep one\nstep two", false, TodoPriority::High),
            ("one line", false, TodoPriority::Normal),
        ]);
        open.open_todo_board(&crate::terminal::TerminalRuntimeRegistry::new());
        test_support::layout(&mut open);
        let geometry = open.todo_board_geometry().expect("board");
        let detail = geometry
            .detail
            .expect("a row hides text, so the box is there");
        let buffer = test_support::draw_sized(
            &open,
            test_support::SNAPSHOT_WIDTH,
            test_support::SNAPSHOT_HEIGHT,
        );
        let inside = Rect::new(
            detail.x + 1,
            detail.y + 1,
            detail.width - 2,
            detail.height - 2,
        );
        let rows = test_support::rect_rows(&buffer, inside).join(" ");
        assert!(rows.contains("step two"), "{rows}");
    }

    #[test]
    fn snapshot_empty() {
        snapshot(&[]).assert(
            Rect::new(0, 8, 80, 8),
            &[
                "┌──────────────────────────────────────────────────────────────────────────────┐",
                "│ todos/notes                                                                  │",
                "│ / search todos                                                       0 todos │",
                "│                                                                              │",
                "│ nothing outstanding                                                          │",
                "│                                                                              │",
                "│                           ↵ open pane    esc close                           │",
                "└──────────────────────────────────────────────────────────────────────────────┘",
            ],
        );
    }

    /// The board is the panel widened, so a todo drawn on it is the same text
    /// the panel draws — same glyph, same first line, same done styling. The
    /// board indents it under its heading, which is why the comparison starts
    /// at the indent: what moves is the rect, never the row renderer.
    #[test]
    fn a_todo_row_reads_exactly_as_it_does_on_the_pane_panel() {
        let todos = &[("check the 403", false, TodoPriority::Normal)];

        let mut board = app_with_todos(todos);
        board.open_todo_board(&crate::terminal::TerminalRuntimeRegistry::new());
        test_support::layout(&mut board);
        let board_buffer = test_support::draw_sized(
            &board,
            test_support::SNAPSHOT_WIDTH,
            test_support::SNAPSHOT_HEIGHT,
        );
        let (list, _) = board.todo_board_list_window().expect("the board is open");
        // Row 0 is the pane heading; row 1 is the todo.
        let todo_row = Rect::new(list.x, list.y + 1, list.width, 1);
        let board_row = test_support::row_text(
            &board_buffer,
            Rect::new(
                todo_row.x + TODO_BOARD_TODO_INDENT,
                todo_row.y,
                todo_row.width - TODO_BOARD_TODO_INDENT,
                1,
            ),
        );
        assert_eq!(
            test_support::row_text(
                &board_buffer,
                Rect::new(todo_row.x, todo_row.y, TODO_BOARD_TODO_INDENT, 1),
            ),
            " ".repeat(TODO_BOARD_TODO_INDENT as usize),
            "the indent is blank, so the todo sits under its heading"
        );

        let mut panel = app_with_todos(todos);
        let pane_id = panel.workspaces[0].tabs[0].root_pane;
        panel.open_pane_todos(pane_id);
        test_support::layout(&mut panel);
        let panel_buffer = test_support::draw_sized(
            &panel,
            test_support::SNAPSHOT_WIDTH,
            test_support::SNAPSHOT_HEIGHT,
        );
        let (panel_list, _) = panel
            .pane_todo_panel_list_window()
            .expect("the panel is open");
        let panel_row = test_support::row_text(
            &panel_buffer,
            Rect::new(panel_list.x, panel_list.y, panel_list.width, 1),
        );

        // Each surface right-aligns the row's `#id` at its own width, so the
        // comparison is of everything before it: glyph, text and chip.
        let strip_id = |row: &str| row.trim_end().trim_end_matches("#1").trim_end().to_string();
        assert!(
            board_row.trim_end().ends_with("#1"),
            "board row: {board_row:?}"
        );
        assert!(
            panel_row.trim_end().ends_with("#1"),
            "panel row: {panel_row:?}"
        );
        assert_eq!(strip_id(&board_row), strip_id(&panel_row));
    }

    /// The kit's footer convention: a blank row between the last entry and the
    /// buttons, rather than buttons drawn flush against it.
    #[test]
    fn the_footer_keeps_its_blank_row_above_the_buttons() {
        let geometry = todo_board_geometry(
            Rect::new(
                0,
                0,
                test_support::SNAPSHOT_WIDTH,
                test_support::SNAPSHOT_HEIGHT,
            ),
            4,
            TODO_BOARD_MIN_WIDTH,
        )
        .expect("geometry resolves");
        assert_eq!(
            geometry.list.y + geometry.list.height,
            geometry.footer_row.expect("footer").y - 1
        );
        assert_eq!(
            geometry.header.y + TODO_BOARD_HEADER_ROWS,
            geometry.list.y,
            "a blank row sits between the search row and the first entry"
        );
    }

    /// The board is never narrower than the controls it means to show — the
    /// bug the notification center's footer had.
    #[test]
    fn the_board_is_at_least_as_wide_as_its_own_footer() {
        let geometry = todo_board_geometry(
            Rect::new(
                0,
                0,
                test_support::SNAPSHOT_WIDTH,
                test_support::SNAPSHOT_HEIGHT,
            ),
            4,
            TODO_BOARD_MIN_WIDTH,
        )
        .expect("geometry resolves");
        let natural = ButtonRow::natural_width(&todo_board_button_specs(true, true));
        assert!(
            geometry.inner.width >= natural,
            "inner {} should fit the footer's {natural}",
            geometry.inner.width
        );
    }

    /// The board is the one view of a whole session's todos, so it grows to
    /// its content rather than scrolling a fixed box as soon as a few panes
    /// have work.
    #[test]
    fn the_board_grows_for_its_content_in_both_directions() {
        let screen = Rect::new(0, 0, 200, 60);

        // The floor is whichever is wider: the minimum, or the footer's own
        // natural width — the board is never too narrow for its controls.
        let floor = TODO_BOARD_MIN_WIDTH
            .max(ButtonRow::natural_width(&todo_board_button_specs(true, true)) + 2);
        let small = todo_board_geometry(screen, 4, TODO_BOARD_MIN_WIDTH).expect("resolves");
        assert_eq!(small.outer.width, floor);
        // Pinned, not merely derived from the constant: a floor of 64 gave a
        // session whose longest row was 66 columns a 66-column box in the
        // middle of a very wide terminal, which is the complaint this width
        // answers. Lowering it again should fail here and read why.
        assert!(
            small.outer.width >= 80,
            "the floor is what makes the board readable, got {}",
            small.outer.width
        );

        // A long heading or todo widens it.
        let wide = todo_board_geometry(screen, 4, 90).expect("resolves");
        assert_eq!(wide.outer.width, 90);
        assert!(
            wide.outer.width > floor,
            "content widened it past the floor"
        );

        // Many panes' todos make it taller — the old fixed ceiling turned a
        // busy session into scrolling.
        let tall = todo_board_geometry(screen, 40, TODO_BOARD_MIN_WIDTH).expect("resolves");
        assert!(
            tall.list.height >= 40,
            "40 items should not scroll on a 60-row screen, got {}",
            tall.list.height
        );
    }

    /// It stops growing before it swallows the terminal, in both directions,
    /// at the width every centred list dialog shares.
    #[test]
    fn the_board_stops_growing_at_its_cap_and_at_the_screen() {
        let screen = Rect::new(0, 0, 200, 60);
        let capped = todo_board_geometry(screen, 4, 400).expect("resolves");
        assert_eq!(capped.outer.width, LIST_DIALOG_MAX_WIDTH);
        assert_eq!(capped.outer.x, (200 - LIST_DIALOG_MAX_WIDTH) / 2, "centred");

        // A short screen clamps rather than overflowing it.
        let short = todo_board_geometry(Rect::new(0, 0, 80, 14), 40, TODO_BOARD_MIN_WIDTH)
            .expect("resolves");
        assert!(short.outer.height <= 14, "got {}", short.outer.height);
        let narrow = todo_board_geometry(Rect::new(0, 0, 40, 30), 4, 400).expect("resolves");
        assert!(narrow.outer.width <= 40, "got {}", narrow.outer.width);
    }

    /// An empty board is still a panel rather than collapsing to its chrome.
    #[test]
    fn an_empty_board_keeps_a_minimum_height() {
        let geometry = todo_board_geometry(Rect::new(0, 0, 200, 60), 0, TODO_BOARD_MIN_WIDTH)
            .expect("resolves");
        // One row for the empty state between the header and the footer.
        assert_eq!(
            geometry.outer.height,
            2 + TODO_BOARD_HEADER_ROWS + 1 + crate::ui::FOOTER_ROWS
        );
    }

    #[test]
    fn a_heading_reads_space_then_pane() {
        assert_eq!(
            todo_board_heading_text("infra", "imap-jmap-mcp"),
            " infra · imap-jmap-mcp"
        );
        // A pane with no label of its own still names its space.
        assert_eq!(todo_board_heading_text("infra", ""), " infra");
        assert_eq!(todo_board_heading_text("", "claude"), " claude");
    }
}
