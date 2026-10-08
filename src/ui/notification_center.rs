use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::overlay::{ButtonRow, ButtonSpec};
use super::text::{display_width, relative_time_label, truncate_end};
use super::widgets::{panel_contrast_fg, render_action_button, render_panel_shell};
use crate::app::state::{NotificationCenterButton, ToastKind};
use crate::app::AppState;

/// The footer row, in the settings-panel language: the shortcut hint inside
/// the filled box, in render order.
///
/// The panel sizes itself to fit all three (see
/// [`notification_center_footer_width`]), so `mark read` is dropped only when
/// the screen itself is too narrow for the row.
fn notification_center_button_specs() -> [ButtonSpec<NotificationCenterButton>; 3] {
    [
        ButtonSpec {
            button: NotificationCenterButton::MarkRead,
            hint: Some("r"),
            label: "mark read",
            drop_rank: Some(0),
        },
        ButtonSpec {
            button: NotificationCenterButton::Clear,
            hint: Some("c"),
            label: "clear all",
            drop_rank: None,
        },
        ButtonSpec {
            button: NotificationCenterButton::Close,
            hint: Some("esc"),
            label: "close",
            drop_rank: None,
        },
    ]
}

/// Inner columns the whole footer wants. The panel measures this alongside its
/// rows, because a notification centre that cannot show "mark read" hides its
/// primary action *and* the `r` key that is the only other way to reach it.
pub(crate) fn notification_center_footer_width() -> u16 {
    ButtonRow::natural_width(&notification_center_button_specs())
}

pub(crate) fn notification_center_button_rects(
    footer_row: Rect,
) -> Option<ButtonRow<NotificationCenterButton>> {
    ButtonRow::layout(footer_row, &notification_center_button_specs())
}

/// Hit area for the floating indicator used when
/// `ui.notification_center_position = "bottom-right"`: the last row of the
/// frame, right-aligned, mirroring where the dropdown opens.
pub(super) fn floating_notification_indicator_rect(area: Rect, width: u16) -> Rect {
    if area.width == 0 || area.height == 0 {
        return Rect::default();
    }
    let width = width.min(area.width);
    Rect::new(
        area.x + area.width - width,
        area.y + area.height - 1,
        width,
        1,
    )
}

/// Draw the bottom-right floating indicator. A no-op for the default
/// top-right position, where the indicator lives in the tab bar instead.
pub(super) fn render_floating_notification_indicator(app: &AppState, frame: &mut Frame) {
    if app.notification_center_position
        != crate::config::NotificationCenterPositionConfig::BottomRight
    {
        return;
    }
    let rect = app.view.notification_hit_area;
    if rect.width == 0 {
        return;
    }
    let p = &app.palette;
    let unread = app.notification_log.unread_count();
    // Same quiet grammar as the sidebar's « collapse toggle: a bare glyph
    // with no background, dim at rest and accent + bold while unread.
    let style = if unread > 0 {
        Style::default().fg(p.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(p.overlay0)
    };
    frame.render_widget(
        Paragraph::new(super::tabs::notification_indicator_label(unread)).style(style),
        rect,
    );
}

pub(super) fn render_notification_center(app: &AppState, frame: &mut Frame) {
    let Some(rect) = app.notification_center_rect() else {
        return;
    };
    let p = &app.palette;
    let Some(inner) = render_panel_shell(frame, rect, p.accent, p.panel_bg) else {
        return;
    };

    if app.notification_log.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " no notifications",
                Style::default().fg(p.overlay0),
            ))),
            Rect::new(inner.x, inner.y, inner.width, inner.height.min(1)),
        );
        return;
    }

    let Some((list, start)) = app.notification_center_list_window() else {
        return;
    };
    let selected = app
        .notification_center()
        .map(|center| center.list.selected)
        .unwrap_or(0);
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);

    for (row, entry) in app
        .notification_log
        .entries_newest_first()
        .skip(start)
        .take(list.height as usize)
        .enumerate()
    {
        let idx = start + row;
        let row_rect = Rect::new(list.x, list.y + row as u16, list.width, 1);
        let is_selected = idx == selected;

        let age = relative_time_label(now_unix, entry.posted_at_unix);
        let age_width = display_width(&age);
        let text_budget = (list.width as usize).saturating_sub(3 + age_width + 1);
        let title = truncate_end(&entry.title, text_budget);
        let context_budget = text_budget.saturating_sub(display_width(&title));
        let context = if entry.context.is_empty() || context_budget < 6 {
            String::new()
        } else {
            truncate_end(&format!(" · {}", entry.context), context_budget)
        };
        let pad_width = (list.width as usize)
            .saturating_sub(3 + display_width(&title) + display_width(&context) + age_width + 1);

        let (dot_style, title_style, dim_style, row_style) = if is_selected {
            let fg = panel_contrast_fg(p);
            let selected_base = Style::default().fg(fg).bg(p.accent);
            // The band alone marks selection; dot and bold keep signalling
            // read state so a selected row stays distinguishable.
            let selected_title = if entry.read {
                selected_base
            } else {
                selected_base.add_modifier(Modifier::BOLD)
            };
            (selected_base, selected_title, selected_base, selected_base)
        } else if !entry.read {
            // Unread: kind-colored dot, bold title.
            let dot_color = match entry.kind {
                ToastKind::NeedsAttention => p.red,
                ToastKind::Finished => p.blue,
                ToastKind::UpdateInstalled => p.accent,
            };
            (
                Style::default().fg(dot_color),
                Style::default().fg(p.text).add_modifier(Modifier::BOLD),
                Style::default().fg(p.overlay0),
                Style::default(),
            )
        } else {
            // Read: blank dot column, dim regular-weight title.
            (
                Style::default(),
                Style::default().fg(p.overlay0),
                Style::default().fg(p.overlay0),
                Style::default(),
            )
        };
        let dot = if !entry.read { " ● " } else { "   " };

        let line = Line::from(vec![
            Span::styled(dot, dot_style),
            Span::styled(title, title_style),
            Span::styled(context, dim_style),
            Span::styled(" ".repeat(pad_width), row_style),
            Span::styled(age, dim_style),
            Span::styled(" ", row_style),
        ]);
        frame.render_widget(Paragraph::new(line).style(row_style), row_rect);
    }

    if let Some(buttons) = app.notification_center_buttons() {
        let hovered = app
            .notification_center()
            .and_then(|center| center.hovered_button);
        // Same button language as the settings action buttons: filled boxes
        // with the shortcut hint inside, secondary (surface) at rest and
        // accent on hover.
        let style_for = |button: NotificationCenterButton| {
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
    use crate::app::state::{AppState, ToastKind, ToastNotification};
    use crate::ui::test_support;
    use ratatui::layout::Rect;
    use ratatui::{backend::TestBackend, Terminal};

    fn toast(title: &str) -> ToastNotification {
        ToastNotification {
            kind: ToastKind::Finished,
            title: title.to_string(),
            context: String::new(),
            position: None,
            target: None,
            anchor_pane: None,
        }
    }

    /// A quiet one-pane app carrying `titles` in the notification log, laid out
    /// at the snapshot size.
    fn app_with_notifications(
        position: crate::config::NotificationCenterPositionConfig,
        titles: &[&str],
    ) -> AppState {
        let mut app = test_support::app_with_one_pane("notify");
        app.notification_center_position = position;
        for title in titles {
            app.post_notification(toast(title));
        }
        test_support::layout(&mut app);
        app
    }

    fn opened(
        position: crate::config::NotificationCenterPositionConfig,
        titles: &[&str],
    ) -> AppState {
        let mut app = app_with_notifications(position, titles);
        app.open_notification_center();
        test_support::layout(&mut app);
        app
    }

    /// Wide enough for all three boxes; the narrow cases above drop
    /// `mark read` first.
    #[test]
    fn snapshot_top_right_wide() {
        let position = crate::config::NotificationCenterPositionConfig::TopRight;
        let titles = ["the build finished on the release runner"];
        test_support::overlay_snapshot(
            &app_with_notifications(position, &titles),
            &opened(position, &titles),
        )
        .assert(
            Rect::new(30, 1, 50, 5),
            &[
                "┌────────────────────────────────────────────────┐",
                "│ ● the build finished on the release runner now │",
                "│                                                │",
                "│    r mark read    c clear all    esc close     │",
                "└────────────────────────────────────────────────┘",
            ],
        );
    }

    #[test]
    fn snapshot_bottom_right_populated() {
        let position = crate::config::NotificationCenterPositionConfig::BottomRight;
        test_support::overlay_snapshot(
            &app_with_notifications(position, &["build finished", "tests failed"]),
            &opened(position, &["build finished", "tests failed"]),
        )
        .assert(
            Rect::new(37, 18, 43, 6),
            &[
                "┌─────────────────────────────────────────┐",
                "│ ● tests failed                      now │",
                "│ ● build finished                    now │",
                "│                                         │",
                "│ r mark read    c clear all    esc close │",
                "└─────────────────────────────────────────┘",
            ],
        );
    }

    /// An empty log still opens a panel: the footer goes away, but the box does
    /// not, so the toggle is never a dead end.
    #[test]
    fn snapshot_empty() {
        let position = crate::config::NotificationCenterPositionConfig::TopRight;
        test_support::overlay_snapshot(
            &app_with_notifications(position, &[]),
            &opened(position, &[]),
        )
        .assert(
            Rect::new(50, 1, 30, 3),
            &[
                "┌────────────────────────────┐",
                "│ no notifications           │",
                "└────────────────────────────┘",
            ],
        );
    }

    #[test]
    fn snapshot_top_right_populated() {
        let position = crate::config::NotificationCenterPositionConfig::TopRight;
        test_support::overlay_snapshot(
            &app_with_notifications(position, &["build finished", "tests failed"]),
            &opened(position, &["build finished", "tests failed"]),
        )
        .assert(
            Rect::new(37, 1, 43, 6),
            &[
                "┌─────────────────────────────────────────┐",
                "│ ● tests failed                      now │",
                "│ ● build finished                    now │",
                "│                                         │",
                "│ r mark read    c clear all    esc close │",
                "└─────────────────────────────────────────┘",
            ],
        );
    }

    #[test]
    fn floating_indicator_rect_hugs_the_bottom_right_corner() {
        let area = Rect::new(0, 0, 80, 25);
        let rect = floating_notification_indicator_rect(area, 5);
        assert_eq!(rect, Rect::new(75, 24, 5, 1));

        assert_eq!(
            floating_notification_indicator_rect(Rect::default(), 5),
            Rect::default()
        );
    }

    #[test]
    fn floating_indicator_renders_bare_accent_glyph_at_bottom_right() {
        let mut app = AppState::test_new();
        app.notification_center_position =
            crate::config::NotificationCenterPositionConfig::BottomRight;
        app.post_notification(toast("one"));
        app.post_notification(toast("two"));
        let area = Rect::new(0, 0, 80, 25);
        app.view.notification_hit_area = floating_notification_indicator_rect(
            area,
            super::super::tabs::notification_indicator_width(app.notification_log.unread_count()),
        );

        let backend = TestBackend::new(80, 25);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_floating_notification_indicator(&app, frame))
            .unwrap();
        let buffer = terminal.backend().buffer();

        let rect = app.view.notification_hit_area;
        assert_eq!(rect.y, 24, "indicator sits on the frame's last row");
        assert_eq!(rect.x + rect.width, 80, "right-aligned to the frame edge");
        let row: String = (rect.x..rect.x + rect.width)
            .map(|x| buffer[(x, rect.y)].symbol())
            .collect();
        assert!(row.contains("и 2"), "indicator row: {row:?}");
        let mid = &buffer[(rect.x + 1, rect.y)];
        assert_eq!(
            mid.style().fg,
            Some(app.palette.accent),
            "unread glyph uses the accent color"
        );
        assert_ne!(
            mid.style().bg,
            Some(app.palette.accent),
            "no filled pill: the glyph stays bare like the sidebar toggle"
        );
    }

    #[test]
    fn footer_renders_settings_style_buttons_above_the_bottom_border() {
        let mut app = AppState::test_new();
        app.view.terminal_area = Rect::new(0, 1, 80, 24);
        app.view.tab_bar_rect = Rect::new(0, 0, 80, 1);
        // A long title widens the panel enough for all three footer buttons.
        app.post_notification(toast("claude finished a very long-running task"));
        app.post_notification(toast("two"));
        app.open_notification_center();

        let panel = app.notification_center_rect().expect("panel rect");
        let buttons = app.notification_center_buttons().expect("footer buttons");

        let backend = TestBackend::new(80, 25);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_notification_center(&app, frame))
            .unwrap();
        let buffer = terminal.backend().buffer();

        // The buttons sit on the row directly above the panel's bottom border.
        assert_eq!(buttons.row_y(), panel.y + panel.height - 2);

        // And the row between the last entry and the buttons renders blank.
        // Geometry alone would not catch a stray draw into the separator, so
        // this reads the buffer rather than the rects.
        let (list, _) = app.notification_center_list_window().expect("list window");
        let separator_y = list.y + list.height;
        assert_eq!(
            separator_y + 1,
            buttons.row_y(),
            "separator precedes buttons"
        );
        let separator: String = (list.x..list.x + list.width)
            .map(|x| buffer[(x, separator_y)].symbol())
            .collect();
        assert!(
            separator.trim().is_empty(),
            "the row above the buttons should be blank, got {separator:?}"
        );

        // Each carries the settings-style filled background (surface0 at
        // rest) with the shortcut hint inside the box.
        for (placed, text) in
            buttons
                .placed()
                .iter()
                .zip(["r mark read", "c clear all", "esc close"])
        {
            let rect = placed.rect;
            let mid = &buffer[(rect.x + rect.width / 2, rect.y)];
            assert_eq!(mid.style().bg, Some(app.palette.surface0));
            let row: String = (rect.x..rect.x + rect.width)
                .map(|x| buffer[(x, rect.y)].symbol())
                .collect();
            assert!(row.contains(text), "button row {text:?}: {row:?}");
        }
    }

    #[test]
    fn unread_rows_show_dot_and_bold_while_read_rows_dim() {
        let mut app = AppState::test_new();
        app.view.terminal_area = Rect::new(0, 1, 80, 24);
        app.view.tab_bar_rect = Rect::new(0, 0, 80, 1);
        app.post_notification(toast("older"));
        app.post_notification(toast("newer"));
        let older_id = app
            .notification_log
            .entries_newest_first()
            .last()
            .expect("older entry")
            .id;
        app.notification_log.mark_read(older_id);
        app.open_notification_center();
        // Move selection off both rows under test? The panel always has a
        // selection; keep it on row 0 and inspect row 1 (read) plus row 0's
        // selected styling separately via the unselected unread row below.
        app.notification_center_move_selection(1);
        // Selection now sits on the read row (row 1); row 0 is the plain
        // unread rendering.
        let (list, _start) = app
            .notification_center_list_window()
            .expect("list window present");

        let backend = TestBackend::new(80, 25);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_notification_center(&app, frame))
            .unwrap();
        let buffer = terminal.backend().buffer();

        // Unread row (unselected): kind-colored dot and a bold title.
        let dot = &buffer[(list.x + 1, list.y)];
        assert_eq!(dot.symbol(), "●");
        assert_eq!(dot.style().fg, Some(app.palette.blue));
        let title_cell = &buffer[(list.x + 3, list.y)];
        assert!(title_cell
            .style()
            .add_modifier
            .contains(ratatui::style::Modifier::BOLD));
        assert_eq!(title_cell.style().fg, Some(app.palette.text));

        // The read row is selected here: the band marks the selection, but
        // the dot stays absent and the title stays regular-weight so read
        // state remains visible on the selected row.
        let selected_read_dot = &buffer[(list.x + 1, list.y + 1)];
        assert_eq!(
            selected_read_dot.symbol(),
            " ",
            "selected read rows still drop the dot"
        );
        assert_eq!(selected_read_dot.style().bg, Some(app.palette.accent));
        let selected_read_title = &buffer[(list.x + 3, list.y + 1)];
        assert!(
            !selected_read_title
                .style()
                .add_modifier
                .contains(ratatui::style::Modifier::BOLD),
            "selected read rows stay regular-weight"
        );

        // With the selection moved back to the unread row, the read row keeps
        // its blank-dot/dim rendering and the selected unread row keeps its
        // dot and bold on the band.
        app.notification_center_move_selection(-1);
        terminal
            .draw(|frame| render_notification_center(&app, frame))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let selected_unread_dot = &buffer[(list.x + 1, list.y)];
        assert_eq!(
            selected_unread_dot.symbol(),
            "●",
            "selected unread rows keep the dot"
        );
        let read_dot = &buffer[(list.x + 1, list.y + 1)];
        assert_eq!(read_dot.symbol(), " ", "read rows drop the dot");
        let read_title = &buffer[(list.x + 3, list.y + 1)];
        assert_eq!(
            read_title.style().fg,
            Some(app.palette.overlay0),
            "read titles dim to the muted gray"
        );
        assert!(!read_title
            .style()
            .add_modifier
            .contains(ratatui::style::Modifier::BOLD));
    }
}
