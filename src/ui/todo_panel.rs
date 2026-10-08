use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::overlay::{ButtonRow, ButtonSpec};
use super::text::{display_width, display_width_u16, truncate_end};
use super::widgets::{panel_contrast_fg, render_action_button, render_panel_shell};
use crate::app::state::{AppState, PaneTodoPanelButton};
use crate::terminal::todo::{PaneTodo, TodoPriority};

/// The footer row, in the notification center's language: the shortcut hint
/// inside the filled box, in render order.
///
/// `toggle` and `clear done` are absent on a pane with no todos — there is
/// nothing to toggle or clear — and `go` is absent unless the selected todo's
/// link resolves, since there is nowhere to go otherwise. When the row is too
/// narrow they drop in that order, `go` last: following a link is the one
/// action whose key nothing else on screen advertises. `add` and `close`
/// always survive: an empty panel that could not add is the dead end this
/// footer exists to prevent.
pub(crate) fn pane_todo_panel_button_rects(
    footer_row: Rect,
    has_todos: bool,
    has_live_link: bool,
) -> Option<ButtonRow<PaneTodoPanelButton>> {
    let mut specs = Vec::with_capacity(5);
    specs.push(ButtonSpec {
        button: PaneTodoPanelButton::Add,
        hint: Some("a"),
        label: "add",
        drop_rank: None,
    });
    if has_todos {
        specs.push(ButtonSpec {
            button: PaneTodoPanelButton::Toggle,
            hint: Some("spc"),
            label: "toggle",
            drop_rank: Some(0),
        });
    }
    if has_live_link {
        specs.push(ButtonSpec {
            button: PaneTodoPanelButton::Go,
            hint: Some("g"),
            label: "go",
            drop_rank: Some(2),
        });
    }
    if has_todos {
        specs.push(ButtonSpec {
            button: PaneTodoPanelButton::ClearDone,
            hint: Some("c"),
            label: "clear done",
            drop_rank: Some(1),
        });
    }
    specs.push(ButtonSpec {
        button: PaneTodoPanelButton::Close,
        hint: Some("esc"),
        label: "close",
        drop_rank: None,
    });
    ButtonRow::layout(footer_row, &specs)
}

/// The link chip at a row's right edge, for a todo that carries a link. One
/// definition for the renderer and the mouse hit-test, so clicking the chip and
/// seeing the chip cannot drift apart.
///
/// A live link leads with the target's public identifier and follows it with
/// the captured label — `→ w2:pC · claude` — because the identifier is the part
/// you can act on and the label is the part you recognise. A dead link has no
/// identifier to lead with, so it keeps its label alone.
/// The row's trailing identity, as rendered: `#<id>`. The `#` is display
/// only — the CLI accepts the bare number — and marks the cells as an
/// identity rather than a count, beside indicators that really are counts.
pub(crate) fn pane_todo_row_id_text(todo_id: u64) -> String {
    format!("#{todo_id}")
}

/// The part of a row the link chip may occupy: everything left of the id and
/// the space before it. One definition for the renderer and both mouse
/// hit-tests, so the chip cannot be drawn in one place and clicked in
/// another.
pub(crate) fn pane_todo_row_chip_area(row: Rect, todo_id: u64) -> Rect {
    let reserved = display_width_u16(&pane_todo_row_id_text(todo_id)).saturating_add(1);
    Rect::new(row.x, row.y, row.width.saturating_sub(reserved), row.height)
}

pub(crate) fn pane_todo_link_chip(
    row: Rect,
    public_id: Option<&str>,
    label: &str,
) -> Option<(Rect, String)> {
    if (label.is_empty() && public_id.is_none()) || row.width < 16 {
        return None;
    }
    // The chip takes what it needs so long as it leaves the todo's own text a
    // readable minimum, past the three-cell state glyph. The panel sizes
    // itself from `pane_todo_link_chip_text`, so on an untruncated chip this
    // budget is not the binding constraint.
    let budget = (row.width as usize).saturating_sub(3 + CHIP_MIN_TEXT_COLUMNS);
    // Four cells of frame: the leading space, the arrow and its space, and the
    // trailing space.
    let content = budget.saturating_sub(4);
    // The identifier is short and takes its width off the top; the label is
    // what gives when the row is narrow.
    let label = match public_id {
        Some(id) => truncate_end(label, content.saturating_sub(display_width(id) + 3)),
        None => truncate_end(label, content),
    };
    let text = pane_todo_link_chip_text(public_id, &label);
    let width = display_width_u16(&text);
    if width == 0 || width >= row.width {
        return None;
    }
    Some((Rect::new(row.x + row.width - width, row.y, width, 1), text))
}

/// Columns the todo's own text keeps whatever its link is called.
const CHIP_MIN_TEXT_COLUMNS: usize = 8;

/// The chip's text before any truncation. One definition, so the panel sizes
/// itself for exactly what the chip is going to draw.
pub(crate) fn pane_todo_link_chip_text(public_id: Option<&str>, label: &str) -> String {
    match public_id {
        Some(id) if label.is_empty() => format!(" → {id} "),
        Some(id) => format!(" → {id} · {label} "),
        None => format!(" → {label} "),
    }
}

/// What a todo shows on its single panel row. A todo may hold more than one
/// line; the panel lists one row each, so the rest is signalled rather than
/// shown.
pub(crate) fn pane_todo_row_text(text: &str, budget: usize) -> String {
    let Some((first, _)) = text.split_once('\n') else {
        return truncate_end(text, budget);
    };
    let marker = " ⏎";
    let first = truncate_end(first, budget.saturating_sub(display_width(marker)));
    format!("{first}{marker}")
}

/// Columns a todo's text gets on a row `row_rect` wide: what is left after its
/// state glyph, its link chip, its `#id` and the space before it. One
/// definition for [`render_pane_todo_row`], which cuts the text to it, and
/// for [`pane_todo_row_hides_text`], which asks whether it did.
pub(crate) fn pane_todo_row_text_budget(app: &AppState, row_rect: Rect, todo: &PaneTodo) -> usize {
    let id_width = display_width(&pane_todo_row_id_text(todo.id));
    let chip_width = todo
        .link
        .as_ref()
        .and_then(|link| {
            pane_todo_link_chip(
                pane_todo_row_chip_area(row_rect, todo.id),
                app.pane_todo_link_public_id(todo).as_deref(),
                &link.label,
            )
        })
        .map(|(rect, _)| usize::from(rect.width))
        .unwrap_or(0);
    usize::from(row_rect.width).saturating_sub(3 + chip_width + id_width + 1)
}

/// Whether a todo's row hides some of its text: a second line, or a first
/// line wider than the row gives it. What decides the detail box — the text,
/// never which space or pane the panel belongs to.
pub(crate) fn pane_todo_row_hides_text(app: &AppState, row_rect: Rect, todo: &PaneTodo) -> bool {
    todo.text.contains('\n')
        || display_width(&todo.text) > pane_todo_row_text_budget(app, row_rect, todo)
}

/// Rows of detail box the todos on a list need so that any of them can be
/// selected without the panel changing height: the largest need among them,
/// or none when no row hides text. `row_width` is how wide a todo's row is,
/// `list_width` how wide the box is.
pub(crate) fn pane_todo_detail_rows<'a>(
    app: &AppState,
    todos: impl IntoIterator<Item = &'a PaneTodo>,
    row_width: u16,
    list_width: u16,
) -> u16 {
    let most = 2 + crate::ui::overlay::DETAIL_MAX_TEXT_ROWS;
    let row = Rect::new(0, 0, row_width, 1);
    let mut rows = 0;
    for todo in todos {
        if pane_todo_row_hides_text(app, row, todo) {
            rows = rows.max(crate::ui::overlay::detail_box_rows(&todo.text, list_width));
            if rows >= most {
                break;
            }
        }
    }
    rows
}

/// Three-cell state block, mirroring the notification center's dot column.
fn todo_glyph(todo: &PaneTodo) -> &'static str {
    if todo.done {
        return " ✓ ";
    }
    match todo.priority {
        TodoPriority::High => " ▲ ",
        TodoPriority::Normal => " ● ",
        TodoPriority::Low => " ▼ ",
    }
}

/// Draw one todo on one row: the state glyph, the todo's first line, and its
/// link chip.
///
/// Shared with the todo board, which is the whole reason it is a function: the
/// board is "the panel widened", so a todo that looked different there would
/// read as a second feature rather than a second view of the same one.
pub(crate) fn render_pane_todo_row(
    frame: &mut Frame,
    app: &AppState,
    row_rect: Rect,
    todo: &PaneTodo,
    is_selected: bool,
) {
    let p = &app.palette;
    let public_id = app.pane_todo_link_public_id(todo);
    let id_text = pane_todo_row_id_text(todo.id);
    let id_width = display_width(&id_text);
    let chip = todo.link.as_ref().and_then(|link| {
        pane_todo_link_chip(
            pane_todo_row_chip_area(row_rect, todo.id),
            public_id.as_deref(),
            &link.label,
        )
    });

    let (glyph_style, text_style, row_style) = if is_selected {
        // The band alone marks selection; the glyph keeps signalling priority
        // and done state so a selected row stays legible.
        let base = Style::default().fg(panel_contrast_fg(p)).bg(p.accent);
        (base, base, base)
    } else if todo.done {
        (
            Style::default().fg(p.overlay0),
            Style::default()
                .fg(p.overlay0)
                .add_modifier(Modifier::CROSSED_OUT),
            Style::default(),
        )
    } else {
        (
            Style::default().fg(app.pane_todo_indicator_color(Some(todo.priority))),
            Style::default().fg(p.text),
            Style::default(),
        )
    };

    let text_budget = pane_todo_row_text_budget(app, row_rect, todo);
    let text = pane_todo_row_text(&todo.text, text_budget);
    let pad = text_budget.saturating_sub(display_width(&text));
    let line = Line::from(vec![
        Span::styled(todo_glyph(todo), glyph_style),
        Span::styled(text, text_style),
        Span::styled(" ".repeat(pad), row_style),
    ]);
    frame.render_widget(Paragraph::new(line).style(row_style), row_rect);

    // The identity claims the true right edge, chip beside it, so ids line up
    // in one scannable column whether or not a row carries a link.
    if (id_width as u16) < row_rect.width {
        let id_rect = Rect::new(
            row_rect.x + row_rect.width - id_width as u16,
            row_rect.y,
            id_width as u16,
            1,
        );
        let id_style = if is_selected {
            Style::default().fg(panel_contrast_fg(p)).bg(p.accent)
        } else {
            Style::default().fg(p.overlay0)
        };
        frame.render_widget(Paragraph::new(id_text).style(id_style), id_rect);
    }

    if let Some((chip_rect, chip_text)) = chip {
        // A dead link keeps its captured label but reads as inert.
        let chip_style = if is_selected {
            Style::default().fg(panel_contrast_fg(p)).bg(p.accent)
        } else if app.pane_todo_link_target(todo).is_some() {
            Style::default().fg(p.blue)
        } else {
            Style::default().fg(p.overlay0).add_modifier(Modifier::DIM)
        };
        frame.render_widget(Paragraph::new(chip_text).style(chip_style), chip_rect);
    }
}

pub(super) fn render_pane_todo_panel(app: &AppState, frame: &mut Frame) {
    let Some(rect) = app.pane_todo_panel_rect() else {
        return;
    };
    let p = &app.palette;
    let Some(inner) = render_panel_shell(frame, rect, p.accent, p.panel_bg) else {
        return;
    };
    let Some(panel) = app.pane_todos() else {
        return;
    };
    let todos = app.pane_todos_in_display_order(panel.pane_id);

    // The empty state still falls through to the footer below: an empty pane
    // is the one you most want to add a todo to, so its panel must not be a
    // dead end.
    if todos.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " no todos",
                Style::default().fg(p.overlay0),
            ))),
            Rect::new(inner.x, inner.y, inner.width, inner.height.min(1)),
        );
    }

    if let Some((list, start)) = app.pane_todo_panel_list_window() {
        for (row, todo) in todos
            .iter()
            .skip(start)
            .take(list.height as usize)
            .enumerate()
        {
            let row_rect = Rect::new(list.x, list.y + row as u16, list.width, 1);
            render_pane_todo_row(
                frame,
                app,
                row_rect,
                todo,
                start + row == panel.list.selected,
            );
        }
    }

    if let Some(detail) = app.pane_todo_panel_detail_rect() {
        let row = Rect::new(detail.x, detail.y, detail.width, 1);
        let text = todos
            .get(panel.list.selected)
            .filter(|todo| pane_todo_row_hides_text(app, row, todo))
            .map(|todo| todo.text.as_str());
        crate::ui::overlay::render_detail_box(frame, detail, text, p);
    }

    if let Some(buttons) = app.pane_todo_panel_buttons() {
        let hovered = panel.hovered_button;
        let style_for = |button: PaneTodoPanelButton| {
            if hovered == Some(button) {
                Style::default()
                    .fg(panel_contrast_fg(p))
                    .bg(p.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(p.text)
                    .bg(p.surface0)
                    .add_modifier(Modifier::BOLD)
            }
        };
        for placed in buttons.placed() {
            render_action_button(
                frame,
                placed.rect,
                placed.hint,
                placed.label,
                style_for(placed.button),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, layout::Rect, Terminal};

    use crate::app::state::AppState;
    use crate::terminal::todo::{TodoLink, TodoPriority, TodoUpdate};
    use crate::ui::test_support::{self, row_text};
    use crate::workspace::Workspace;

    /// A one-pane app carrying `todos`, laid out at `width`x`height`, with the
    /// panel left closed — the background an open panel is diffed against.
    fn app_with_todos(
        todos: &[(&str, bool, TodoPriority)],
        width: u16,
        height: u16,
    ) -> (AppState, crate::layout::PaneId) {
        let mut app = test_support::app_with_one_pane("todos");
        test_support::layout_sized(&mut app, width, height);
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
        test_support::layout_sized(&mut app, width, height);
        (app, pane_id)
    }

    fn snapshot(
        todos: &[(&str, bool, TodoPriority)],
        width: u16,
        height: u16,
    ) -> test_support::OverlaySnapshot {
        let (base, _) = app_with_todos(todos, width, height);
        let (mut open, pane_id) = app_with_todos(todos, width, height);
        open.open_pane_todos(pane_id);
        test_support::layout_sized(&mut open, width, height);
        test_support::overlay_snapshot_sized(&base, &open, width, height)
    }

    /// A todo carrying several points is readable in the panel: its row still
    /// shows one line and keeps its place in the list, and the lines it cannot
    /// show render under the list for the selection.
    #[test]
    fn the_selected_multi_line_todo_renders_under_the_list() {
        let multi = "your call: 3 open threads\n#68 the arrow-key queue needs one repro elsewhere\n#67 multi-point todos are parked";
        let (mut app, pane_id) = app_with_todos(
            &[
                (multi, false, TodoPriority::High),
                ("left undone: 7 flaky tests", false, TodoPriority::Normal),
            ],
            test_support::SNAPSHOT_WIDTH,
            test_support::SNAPSHOT_HEIGHT,
        );
        app.open_pane_todos(pane_id);
        test_support::layout_sized(
            &mut app,
            test_support::SNAPSHOT_WIDTH,
            test_support::SNAPSHOT_HEIGHT,
        );

        // Selection starts on the multi-line todo, so a detail block exists.
        let detail = app
            .pane_todo_panel_detail_rect()
            .expect("a multi-line selection shows its rest");
        let buffer = test_support::draw_sized(
            &app,
            test_support::SNAPSHOT_WIDTH,
            test_support::SNAPSHOT_HEIGHT,
        );
        // Inside the box's border, joined and whitespace-normalised: the point
        // is that the text is readable, not where the wrap happens to fall.
        let inside = Rect::new(
            detail.x + 1,
            detail.y + 1,
            detail.width - 2,
            detail.height - 2,
        );
        let rendered = test_support::rect_rows(&buffer, inside)
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            rendered.contains("#68 the arrow-key queue needs one repro elsewhere"),
            "the second line is readable: {rendered}"
        );
        assert!(
            rendered.contains("multi-point todos are parked"),
            "the third line is readable: {rendered}"
        );

        // The list still maps one row to one todo — the reason the detail
        // block exists rather than wrapping rows in place.
        let list = app
            .pane_todo_panel_list_window()
            .expect("an open panel has a list")
            .0;
        assert_eq!(list.height as usize, 2, "one row per todo, still");
        assert!(
            detail.y + detail.height <= list.y,
            "the detail sits above the list, not inside it"
        );
    }

    /// The detail box's text, joined and whitespace-normalised: the point is
    /// what is readable, not where the wrap happens to fall.
    fn detail_text(app: &AppState) -> Option<String> {
        let detail = app.pane_todo_panel_detail_rect()?;
        let buffer = test_support::draw_sized(
            app,
            test_support::SNAPSHOT_WIDTH,
            test_support::SNAPSHOT_HEIGHT,
        );
        let inside = Rect::new(
            detail.x + 1,
            detail.y + 1,
            detail.width - 2,
            detail.height - 2,
        );
        Some(
            test_support::rect_rows(&buffer, inside)
                .join(" ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
        )
    }

    fn add_todos(
        app: &mut AppState,
        pane_id: crate::layout::PaneId,
        todos: &[(&str, TodoPriority)],
    ) {
        let terminal_id = app
            .workspaces
            .iter()
            .find_map(|ws| ws.pane_state(pane_id))
            .expect("the pane exists")
            .attached_terminal_id
            .clone();
        let terminal = app
            .terminals
            .get_mut(&terminal_id)
            .expect("test terminal should exist");
        for (text, priority) in todos {
            terminal
                .add_todo(text, *priority, None, 100)
                .expect("todo should be added");
        }
    }

    /// #106: a single line cut to fit its row hides text just as a second
    /// line does, so it opens the detail too.
    #[test]
    fn detail_shows_for_a_cut_one_line_todo() {
        let long = "one line far wider than any panel row could hold, so the row cuts its end off";
        let (mut app, pane_id) = app_with_todos(
            &[(long, false, TodoPriority::High)],
            test_support::SNAPSHOT_WIDTH,
            test_support::SNAPSHOT_HEIGHT,
        );
        app.open_pane_todos(pane_id);
        test_support::layout_sized(
            &mut app,
            test_support::SNAPSHOT_WIDTH,
            test_support::SNAPSHOT_HEIGHT,
        );

        let text = detail_text(&app).expect("a cut line opens the detail");

        assert!(text.contains("the row cuts its end off"), "{text}");
    }

    /// #106: the detail used to show only while the *selected* todo had a
    /// second line, and the selection opens on the first todo in display
    /// order — so a space whose top todo was one line showed none. The same
    /// todos now show the same box in any space.
    #[test]
    fn detail_is_the_same_in_any_space() {
        let todos = [
            ("short and urgent", TodoPriority::High),
            ("the plan\nstep one\nstep two", TodoPriority::Normal),
        ];
        let mut app = test_support::app_with_one_pane("herdr");
        app.workspaces
            .push(crate::workspace::Workspace::test_new("master"));
        app.ensure_test_terminals();
        let herdr_pane = app.workspaces[0].tabs[0].root_pane;
        let master_pane = app.workspaces[1].tabs[0].root_pane;
        add_todos(&mut app, herdr_pane, &todos);
        add_todos(&mut app, master_pane, &todos);

        let mut seen = Vec::new();
        for (ws_idx, pane_id) in [(0, herdr_pane), (1, master_pane)] {
            app.close_pane_todos();
            app.active = Some(ws_idx);
            app.mode = crate::app::state::Mode::Terminal;
            test_support::layout(&mut app);
            app.open_pane_todos(pane_id);
            test_support::layout(&mut app);
            let detail = app
                .pane_todo_panel_detail_rect()
                .expect("a todo in the panel hides text, so the box is there");
            seen.push((detail.height, detail_text(&app)));
        }

        assert_eq!(seen[0], seen[1], "the same box in both spaces");
    }

    #[test]
    fn panel_height_is_steady_as_the_selection_moves() {
        let (mut app, pane_id) = app_with_todos(
            &[
                ("the plan\nstep one\nstep two", false, TodoPriority::High),
                ("one line", false, TodoPriority::Normal),
            ],
            test_support::SNAPSHOT_WIDTH,
            test_support::SNAPSHOT_HEIGHT,
        );
        app.open_pane_todos(pane_id);
        test_support::layout(&mut app);
        let on_multi = app.pane_todo_panel_rect().expect("panel");
        assert!(detail_text(&app).is_some_and(|text| text.contains("step two")));

        app.pane_todos_move_selection(1);
        test_support::layout(&mut app);

        assert_eq!(
            app.pane_todo_panel_rect().expect("panel"),
            on_multi,
            "the panel does not jump"
        );
        let text = detail_text(&app).expect("the box stays");
        assert!(!text.contains("one line"), "no text of the todo: {text}");
        assert!(text.contains("full text in the row"), "{text}");
    }

    /// The detail in its own inner box, above the list (fork issue 142).
    #[test]
    fn snapshot_detail_box() {
        snapshot(
            &[
                ("the plan\nstep one\nstep two", false, TodoPriority::High),
                ("one line", false, TodoPriority::Normal),
            ],
            test_support::SNAPSHOT_WIDTH,
            test_support::SNAPSHOT_HEIGHT,
        )
        .assert(
            Rect::new(50, 2, 30, 11),
            &[
                "┌────────────────────────────┐",
                "│┌──────────────────────────┐│",
                "││ the plan                 ││",
                "││ step one                 ││",
                "││ step two                 ││",
                "│└──────────────────────────┘│",
                "│ ▲ the plan ⏎             #1│",
                "│ ● one line               #2│",
                "│                            │",
                "│     a add    esc close     │",
                "└────────────────────────────┘",
            ],
        );
    }

    /// Wide enough for the whole footer: every box the panel can offer is on
    /// the row.
    #[test]
    fn snapshot_populated() {
        snapshot(
            &[
                (
                    "ship the overlay kit before the todo board lands",
                    false,
                    TodoPriority::High,
                ),
                (
                    "write the rendered-layout tests first",
                    false,
                    TodoPriority::Normal,
                ),
                ("read the plan", true, TodoPriority::Normal),
            ],
            test_support::SNAPSHOT_WIDTH,
            test_support::SNAPSHOT_HEIGHT,
        )
        .assert(
            Rect::new(23, 2, 57, 7),
            &[
                "┌───────────────────────────────────────────────────────┐",
                "│ ▲ ship the overlay kit before the todo board lands  #1│",
                "│ ● write the rendered-layout tests first             #2│",
                "│ ✓ read the plan                                     #3│",
                "│                                                       │",
                "│   a add    spc toggle    c clear done    esc close    │",
                "└───────────────────────────────────────────────────────┘",
            ],
        );
    }

    /// A pane with nothing on it still opens a panel offering `add`: sizing the
    /// footer away is what made a quiet pane a dead end.
    #[test]
    fn snapshot_empty() {
        snapshot(
            &[],
            test_support::SNAPSHOT_WIDTH,
            test_support::SNAPSHOT_HEIGHT,
        )
        .assert(
            Rect::new(50, 2, 30, 5),
            &[
                "┌────────────────────────────┐",
                "│ no todos                   │",
                "│                            │",
                "│     a add    esc close     │",
                "└────────────────────────────┘",
            ],
        );
    }

    /// Too narrow for the whole footer: the row drops boxes from the least
    /// essential up, and `add` and `close` survive.
    #[test]
    fn snapshot_narrow() {
        snapshot(
            &[
                ("ship the kit", false, TodoPriority::High),
                ("read the plan", true, TodoPriority::Normal),
            ],
            34,
            test_support::SNAPSHOT_HEIGHT,
        )
        .assert(
            Rect::new(4, 3, 30, 6),
            &[
                "┌────────────────────────────┐",
                "│ ▲ ship the kit           #1│",
                "│ ✓ read the plan          #2│",
                "│                            │",
                "│     a add    esc close     │",
                "└────────────────────────────┘",
            ],
        );
    }

    /// A workspace with one pane, the panel open on it, and the frame geometry
    /// the notification center tests use.
    fn app_with_open_panel(todos: &[(&str, bool, TodoPriority)]) -> AppState {
        let mut app = AppState::test_new();
        app.workspaces = vec![Workspace::test_new("todos")];
        app.active = Some(0);
        app.ensure_test_terminals();
        app.view.tab_bar_rect = Rect::new(0, 0, 80, 1);
        app.view.terminal_area = Rect::new(0, 1, 80, 24);

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

        app.open_pane_todos(pane_id);
        app
    }

    fn draw(app: &AppState) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(80, 25)).unwrap();
        terminal
            .draw(|frame| render_pane_todo_panel(app, frame))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn rows_render_in_presentation_order() {
        let app = app_with_open_panel(&[
            ("normal one", false, TodoPriority::Normal),
            ("high one", false, TodoPriority::High),
            ("finished", true, TodoPriority::High),
        ]);
        let (list, _) = app
            .pane_todo_panel_list_window()
            .expect("panel list window should exist");
        let buffer = draw(&app);

        assert!(row_text(&buffer, Rect::new(list.x, list.y, list.width, 1)).contains("high one"));
        assert!(
            row_text(&buffer, Rect::new(list.x, list.y + 1, list.width, 1)).contains("normal one")
        );
        assert!(
            row_text(&buffer, Rect::new(list.x, list.y + 2, list.width, 1)).contains("finished"),
            "done todos sink to the bottom"
        );
    }

    #[test]
    fn done_rows_are_dimmed_and_struck() {
        // Two todos on purpose: `open_pane_todos` starts with `selected: 0`,
        // and a selected row is painted by the selection branch (accent band),
        // never the done branch. Done todos sink, so row 1 is the done one and
        // row 0 keeps the cursor.
        let app = app_with_open_panel(&[
            ("still open", false, TodoPriority::Normal),
            ("finished", true, TodoPriority::Normal),
        ]);
        let (list, _) = app
            .pane_todo_panel_list_window()
            .expect("panel list window should exist");
        let buffer = draw(&app);

        // Column 3 is the first text cell after the three-cell state block.
        let cell = &buffer[(list.x + 3, list.y + 1)];
        assert_eq!(cell.style().fg, Some(app.palette.overlay0));
        assert!(cell
            .style()
            .add_modifier
            .contains(ratatui::style::Modifier::CROSSED_OUT));
    }

    #[test]
    fn a_dead_link_chip_renders_dimmed_and_a_live_one_does_not() {
        // The live/dead distinction only exists on an unselected row: a
        // selected row's chip takes the accent band like the rest of the row.
        // The decoy is High priority so it sorts to row 0 and holds the
        // starting selection, while `todos()` keeps insertion order, so the
        // linked todo is still `todos()[0]` and lands on row 1.
        let mut app = app_with_open_panel(&[
            ("go look", false, TodoPriority::Normal),
            ("decoy", false, TodoPriority::High),
        ]);
        let pane_id = app.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let todo_id = app.terminals[&terminal_id].todos()[0].id;

        // A live link points at a pane that still exists.
        app.terminals
            .get_mut(&terminal_id)
            .expect("test terminal should exist")
            .update_todo(
                todo_id,
                TodoUpdate {
                    link: Some(Some(TodoLink {
                        pane: Some(pane_id),
                        label: "infra".into(),
                    })),
                    ..TodoUpdate::default()
                },
                300,
            )
            .expect("todo should be updated");
        let (list, _) = app
            .pane_todo_panel_list_window()
            .expect("panel list window should exist");
        // Row 1: the linked todo, which is not the selected row. The chip's
        // area stops short of the row's trailing `#id`.
        let row = Rect::new(list.x, list.y + 1, list.width, 1);
        let public_id = app.pane_todo_link_public_id(&app.terminals[&terminal_id].todos()[0]);
        let (chip, _) = pane_todo_link_chip(
            super::pane_todo_row_chip_area(row, todo_id),
            public_id.as_deref(),
            "infra",
        )
        .expect("chip should fit");
        let buffer = draw(&app);
        assert!(row_text(&buffer, chip).contains('→'));
        assert_eq!(
            buffer[(chip.x + 1, chip.y)].style().fg,
            Some(app.palette.blue)
        );

        // A dead link keeps its label and reads as inert.
        app.terminals
            .get_mut(&terminal_id)
            .expect("test terminal should exist")
            .update_todo(
                todo_id,
                TodoUpdate {
                    link: Some(Some(TodoLink {
                        pane: None,
                        label: "infra".into(),
                    })),
                    ..TodoUpdate::default()
                },
                400,
            )
            .expect("todo should be updated");
        // Recomputed rather than reused: a dead chip drops the identifier, so
        // it is shorter and the right-aligned rect moves with it.
        let (chip, _) = pane_todo_link_chip(row, None, "infra").expect("chip should fit");
        let buffer = draw(&app);
        assert!(row_text(&buffer, chip).contains("infra"));
        assert_eq!(
            buffer[(chip.x + 1, chip.y)].style().fg,
            Some(app.palette.overlay0),
            "a dead link is dimmed"
        );
    }

    /// Spec: "the link is presented with that pane's public identifier first
    /// and its captured label after it", and "a moved target is addressed by
    /// where it is now". The identifier is the part you can act on — it is
    /// what `herdr pane` and a sibling agent's prompt take — so it leads.
    #[test]
    fn a_live_link_chip_leads_with_the_public_id_and_a_dead_one_shows_its_label_alone() {
        let mut app = app_with_open_panel(&[
            ("go look", false, TodoPriority::Normal),
            ("decoy", false, TodoPriority::High),
        ]);
        let pane_id = app.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let todo_id = app.terminals[&terminal_id].todos()[0].id;
        let link = |app: &mut AppState, target: Option<crate::layout::PaneId>, at: u64| {
            app.terminals
                .get_mut(&terminal_id)
                .expect("test terminal should exist")
                .update_todo(
                    todo_id,
                    TodoUpdate {
                        link: Some(Some(TodoLink {
                            pane: target,
                            label: "infra".into(),
                        })),
                        ..TodoUpdate::default()
                    },
                    at,
                )
                .expect("todo should be updated");
        };

        link(&mut app, Some(pane_id), 300);
        let public_id = app
            .session_public_pane_id(pane_id)
            .expect("a live pane has a public id");
        let (list, _) = app
            .pane_todo_panel_list_window()
            .expect("panel list window should exist");
        // Row 1: the linked todo, which is not the selected row. The chip's
        // area stops short of the row's trailing `#id`.
        let row = Rect::new(list.x, list.y + 1, list.width, 1);
        let chip_area = super::pane_todo_row_chip_area(row, todo_id);
        let (chip, text) =
            pane_todo_link_chip(chip_area, Some(&public_id), "infra").expect("chip should fit");
        assert_eq!(text, format!(" → {public_id} · infra "));

        // The drawn cells are the cells the hit-test is handed, so what looks
        // clickable is clickable.
        let buffer = draw(&app);
        assert_eq!(row_text(&buffer, chip), text);

        link(&mut app, None, 400);
        let (dead_chip, dead_text) =
            pane_todo_link_chip(chip_area, None, "infra").expect("chip should fit");
        assert_eq!(dead_text, " → infra ", "no identifier to lead with");
        let buffer = draw(&app);
        assert_eq!(row_text(&buffer, dead_chip), dead_text);
    }

    /// Spec: "it occupies a single row showing the first line with a marker".
    /// The panel sizes itself from `todos.len()`, so a todo that grew a second
    /// line must not grow a second row under it.
    #[test]
    fn a_multi_line_todo_occupies_one_row_showing_its_first_line() {
        let app = app_with_open_panel(&[
            ("first line\nsecond line", false, TodoPriority::High),
            ("plain", false, TodoPriority::Normal),
        ]);
        let (list, _) = app
            .pane_todo_panel_list_window()
            .expect("panel list window should exist");
        let buffer = draw(&app);

        let row = row_text(&buffer, Rect::new(list.x, list.y, list.width, 1));
        assert!(row.contains("first line"));
        assert!(row.contains('⏎'), "the marker says more follows: {row}");
        assert!(!row.contains("second line"));
        assert!(
            row_text(&buffer, Rect::new(list.x, list.y + 1, list.width, 1)).contains("plain"),
            "the next todo still starts on the very next row"
        );
    }

    #[test]
    fn the_row_text_marker_fits_inside_the_budget() {
        assert_eq!(pane_todo_row_text("one line", 20), "one line");
        assert_eq!(pane_todo_row_text("one\ntwo", 20), "one ⏎");
        // The marker is reserved out of the budget rather than overrunning it.
        let squeezed = pane_todo_row_text("a long first line\nmore", 10);
        assert!(display_width(&squeezed) <= 10, "{squeezed}");
        assert!(squeezed.ends_with('⏎'));
    }

    /// Spec: "an empty panel can still add". The empty state used to render no
    /// footer at all, which — now that every pane carries an indicator to open
    /// it with — would make clicking a quiet pane a dead end.
    #[test]
    fn an_empty_pane_shows_the_empty_state_and_can_still_add() {
        let app = app_with_open_panel(&[]);
        let buffer = draw(&app);
        let rect = app.pane_todo_panel_rect().expect("panel rect should exist");

        assert!(row_text(
            &buffer,
            Rect::new(rect.x + 1, rect.y + 1, rect.width - 2, 1)
        )
        .contains("no todos"));

        let buttons = app
            .pane_todo_panel_buttons()
            .expect("an empty panel still offers a footer");
        assert!(
            buttons.rect(PaneTodoPanelButton::Toggle).is_none()
                && buttons.rect(PaneTodoPanelButton::ClearDone).is_none(),
            "nothing to toggle or clear on a pane with no todos"
        );

        let footer = row_text(&buffer, Rect::new(rect.x, buttons.row_y(), rect.width, 1));
        assert!(footer.contains("a add"), "the way out of the empty state");
        assert!(footer.contains("esc close"));
    }

    /// Following a link worked from the first release of the panel — `g`, and
    /// a click on the chip — but the footer never said so, which made a chip
    /// that reads like a label the only visible route.
    #[test]
    fn the_footer_offers_go_only_while_the_selected_todo_has_a_live_link() {
        // Wide text so the panel is roomy enough to hold every box.
        let wide = "x".repeat(60);
        let mut app = app_with_open_panel(&[(wide.as_str(), false, TodoPriority::Normal)]);
        assert!(
            app.pane_todo_panel_buttons()
                .expect("footer buttons")
                .rect(PaneTodoPanelButton::Go)
                .is_none(),
            "an unlinked todo has nowhere to go"
        );

        let pane_id = app.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let todo_id = app.terminals[&terminal_id].todos()[0].id;
        let link = |app: &mut AppState, target: Option<crate::layout::PaneId>, at: u64| {
            app.terminals
                .get_mut(&terminal_id)
                .expect("test terminal should exist")
                .update_todo(
                    todo_id,
                    TodoUpdate {
                        link: Some(Some(TodoLink {
                            pane: target,
                            label: "infra".into(),
                        })),
                        ..TodoUpdate::default()
                    },
                    at,
                )
                .expect("todo should be updated");
        };

        link(&mut app, Some(pane_id), 300);
        let buttons = app.pane_todo_panel_buttons().expect("footer buttons");
        let go = buttons
            .rect(PaneTodoPanelButton::Go)
            .expect("a live link offers the go button");
        assert_eq!(
            buttons.button_at(go.x, go.y),
            Some(PaneTodoPanelButton::Go),
            "the drawn box is the box the mouse hits"
        );
        let rect = app.pane_todo_panel_rect().expect("panel rect should exist");
        let buffer = draw(&app);
        assert!(
            row_text(&buffer, Rect::new(rect.x, buttons.row_y(), rect.width, 1)).contains("g go"),
            "the footer advertises the key that was already there"
        );

        // A dead link is inert, so the button goes away rather than lying.
        link(&mut app, None, 400);
        assert!(app
            .pane_todo_panel_buttons()
            .expect("footer buttons")
            .rect(PaneTodoPanelButton::Go)
            .is_none());
    }

    #[test]
    fn the_footer_sits_below_the_list_in_the_settings_button_language() {
        let app = app_with_open_panel(&[("only one", false, TodoPriority::Normal)]);
        let (list, _) = app
            .pane_todo_panel_list_window()
            .expect("panel list window should exist");
        let buttons = app
            .pane_todo_panel_buttons()
            .expect("footer buttons should exist");
        let rect = app.pane_todo_panel_rect().expect("panel rect should exist");

        // One blank row separates the last todo from the buttons — the panel
        // convention, so nothing sits flush against the footer.
        assert_eq!(
            list.y + list.height + 1,
            buttons.row_y(),
            "one blank row between the list and the buttons"
        );
        assert_eq!(buttons.row_y(), rect.y + rect.height - 2);

        // A short todo pins the panel to its 30-cell minimum, and 28 inner
        // cells hold only two boxes: all four need 7 + 12 + 14 + 11 plus three
        // 2-cell gaps = 50, dropping `toggle` still needs 36. So `add` and
        // `close` are what is left. `add` outranks `clear done` here because
        // it is this panel's primary action and the one a mouse user has no
        // other route to — clearing done work keeps its `c` key.
        assert!(buttons.rect(PaneTodoPanelButton::Toggle).is_none());
        assert!(buttons.rect(PaneTodoPanelButton::ClearDone).is_none());

        let buffer = draw(&app);
        let footer = row_text(&buffer, Rect::new(rect.x, buttons.row_y(), rect.width, 1));
        assert!(footer.contains("a add"));
        assert!(footer.contains("esc close"));

        // The separator row really is blank — geometry alone would not catch a
        // stray draw into it.
        let separator = row_text(
            &buffer,
            Rect::new(list.x, list.y + list.height, list.width, 1),
        );
        assert!(
            separator.trim().is_empty(),
            "the row above the buttons should be blank, got {separator:?}"
        );
    }

    #[test]
    fn a_wide_panel_shows_all_four_footer_buttons() {
        // 50 cells of text push the panel to 56 (50 + borders + glyph block +
        // trailing space), whose 54 inner cells clear the 50 all four boxes
        // need.
        let wide = "x".repeat(50);
        let app = app_with_open_panel(&[(wide.as_str(), false, TodoPriority::Normal)]);
        let rect = app.pane_todo_panel_rect().expect("panel rect should exist");
        let buttons = app
            .pane_todo_panel_buttons()
            .expect("footer buttons should exist");

        assert!(buttons.rect(PaneTodoPanelButton::Toggle).is_some());
        assert!(buttons.rect(PaneTodoPanelButton::ClearDone).is_some());

        let buffer = draw(&app);
        let footer = row_text(&buffer, Rect::new(rect.x, buttons.row_y(), rect.width, 1));
        assert!(footer.contains("a add"));
        assert!(footer.contains("spc toggle"));
        assert!(footer.contains("c clear done"));
        assert!(footer.contains("esc close"));
    }

    #[test]
    fn the_panel_hangs_from_the_pane_it_belongs_to() {
        let mut app = app_with_open_panel(&[("only one", false, TodoPriority::Normal)]);
        let pane_id = app.workspaces[0].tabs[0].root_pane;
        app.view.pane_infos = vec![crate::layout::PaneInfo {
            id: pane_id,
            rect: Rect::new(20, 4, 40, 10),
            inner_rect: Rect::new(21, 5, 38, 8),
            scrollbar_rect: None,
            borders: ratatui::widgets::Borders::ALL,
            is_focused: true,
        }];

        let rect = app.pane_todo_panel_rect().expect("panel rect should exist");
        assert_eq!(rect.x + rect.width, 60, "right-aligned with the pane");
        assert_eq!(rect.y, 5, "hangs off the pane's top border");
    }

    #[test]
    fn selection_clamps_to_the_list_and_survives_an_empty_pane() {
        let mut app = app_with_open_panel(&[
            ("first", false, TodoPriority::Normal),
            ("second", false, TodoPriority::Normal),
        ]);

        app.pane_todos_move_selection(5);
        assert_eq!(
            app.pane_todos().expect("panel state").list.selected,
            1,
            "selection stops at the last row"
        );
        app.pane_todos_move_selection(-9);
        assert_eq!(app.pane_todos().expect("panel state").list.selected, 0);

        let empty = app_with_open_panel(&[]);
        assert!(empty.selected_pane_todo().is_none());
    }
}
