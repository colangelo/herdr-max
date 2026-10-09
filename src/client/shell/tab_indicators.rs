use super::render::display_width;
use super::*;
use crate::config::NotificationCenterPositionConfig;
use crate::ui::{todo_priority_color, TodoDisplaySummary};

fn label(glyph: char, count: usize) -> String {
    match count {
        0 => format!(" {glyph} "),
        1..=99 => format!(" {glyph} {count} "),
        _ => format!(" {glyph} 99+ "),
    }
}
fn notification(snapshot: &ClientShellSnapshot) -> Option<usize> {
    Some(
        snapshot
            .resource_facts
            .as_ref()?
            .notifications
            .as_ref()?
            .unread,
    )
}
fn todos(snapshot: &ClientShellSnapshot) -> Option<TodoDisplaySummary> {
    let mut summary = TodoDisplaySummary::default();
    for fact in snapshot
        .resource_facts
        .as_ref()?
        .pane_todos
        .as_ref()?
        .values()
    {
        let item = TodoDisplaySummary::from_fact(fact);
        summary.open = summary.open.saturating_add(item.open);
        summary.total = summary.total.saturating_add(item.total);
        if item.open > 0 {
            summary.priority = summary.priority.max(item.priority);
        }
    }
    Some(summary)
}
fn paint_notification(buffer: &mut Buffer, rect: Rect, unread: usize, p: &Palette, floating: bool) {
    let style = if unread > 0 {
        Style::default().fg(p.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(if floating { p.overlay0 } else { p.overlay1 })
    };
    buffer.set_stringn(
        rect.x,
        rect.y,
        label('и', unread),
        usize::from(rect.width),
        style,
    );
}

/// Reserve independent trailing hit areas before tabs and plugin status badges.
/// The fork gives the outermost notification indicator first claim on narrow bars.
pub(super) fn paint_bar(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) -> Rect {
    let mut remaining = area;
    if area.is_empty() {
        return remaining;
    }
    if config.notification_center_position == NotificationCenterPositionConfig::TopRight {
        if let Some(unread) = notification(snapshot) {
            let width = display_width(&label('и', unread)).min(remaining.width);
            let rect = Rect::new(remaining.right() - width, remaining.y, width, 1);
            paint_notification(buffer, rect, unread, &config.palette, false);
            hits.notification_indicator = rect;
            remaining.width -= width;
        }
    }
    if let Some(summary) = todos(snapshot) {
        let width = display_width(&label('τ', summary.open)).min(remaining.width);
        let rect = Rect::new(remaining.right() - width, remaining.y, width, 1);
        if width > 0 {
            let style = if summary.open > 0 {
                Style::default()
                    .fg(todo_priority_color(
                        summary.priority,
                        &config.palette,
                        config.todo_color,
                    ))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(config.palette.overlay1)
            };
            buffer.set_stringn(
                rect.x,
                rect.y,
                label('τ', summary.open),
                usize::from(rect.width),
                style,
            );
            hits.todo_board = rect;
            remaining.width -= width;
        }
    }
    remaining
}
pub(super) fn paint_floating(
    buffer: &mut Buffer,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) -> Option<Rect> {
    if config.notification_center_position != NotificationCenterPositionConfig::BottomRight
        || buffer.area.is_empty()
    {
        return None;
    }
    let unread = notification(snapshot)?;
    let width = display_width(&label('и', unread)).min(buffer.area.width);
    let rect = Rect::new(
        buffer.area.right() - width,
        buffer.area.bottom() - 1,
        width,
        1,
    );
    paint_notification(buffer, rect, unread, &config.palette, true);
    hits.notification_indicator = rect;
    Some(rect)
}

impl ClientShellState {
    /// The indicators toggle their panels as the fork's did: the notification
    /// indicator opens and closes the notification center, the tab bar's todo
    /// count the todo board, a pane's todo mark that pane's todo panel. They answer before an open panel routes the
    /// click, so a second click on the same control closes it; a cell the todo
    /// panel covers belongs to the panel, not to a mark beneath it.
    pub(super) fn toggle_indicator_panel_at(
        &mut self,
        point: (u16, u16),
        outcome: &mut ClientShellInput,
    ) -> bool {
        let chrome_mode = matches!(
            self.mode,
            ClientShellMode::Terminal | ClientShellMode::Navigate | ClientShellMode::Resize
        );
        if chrome_mode
            && matches!(
                self.overlay,
                None | Some(ClientShellOverlay::NotificationCenter(_))
            )
            && super::contains(self.hits.notification_indicator, point)
        {
            self.toggle_notification_center(outcome);
            return true;
        }
        if chrome_mode
            && matches!(self.overlay, None | Some(ClientShellOverlay::TodoBoard(_)))
            && super::contains(self.hits.todo_board, point)
        {
            self.toggle_todo_board(outcome);
            return true;
        }
        let panel_covers = self
            .hits
            .todo_panel
            .as_ref()
            .is_some_and(|panel| super::contains(panel.outer, point));
        if !chrome_mode
            || panel_covers
            || !matches!(self.overlay, None | Some(ClientShellOverlay::TodoPanel(_)))
        {
            return false;
        }
        let Some(pane_id) = self
            .hits
            .pane_todos
            .iter()
            .find(|(rect, _)| super::contains(*rect, point))
            .map(|(_, pane_id)| pane_id.clone())
        else {
            return false;
        };
        self.toggle_todo_panel(&pane_id, outcome);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tab_controls_and_plugin_accent_badge_fit_before_indicators() {
        let mut snapshot = super::super::tests::snapshot();
        snapshot.resource_facts = Some(crate::protocol::ClientShellResourceFacts {
            notifications: Some(crate::protocol::ClientNotificationSummary {
                total: 2,
                unread: 2,
                ..Default::default()
            }),
            pane_todos: Some(Default::default()),
            ..Default::default()
        });
        snapshot.tab_bar_right = vec![crate::protocol::ClientShellTabStatusSegment {
            text: "PLUGIN".into(),
            accent: true,
        }];
        let config = ClientShellConfig::from_config(&Config::default());
        let area = Rect::new(0, 0, 70, 1);
        let mut buffer = Buffer::empty(area);
        let mut hits = ShellHitMap::default();
        let mut scroll = 0;
        let mut reveal = false;
        super::super::render::render_tab_bar(
            &mut buffer,
            area,
            &snapshot,
            &config,
            &mut scroll,
            &mut reveal,
            None,
            &mut hits,
        );
        let row: String = (0..70).map(|x| buffer[(x, 0)].symbol()).collect();
        assert!(row.ends_with(" τ  и 2 "));
        let plugin = row.find("PLUGIN").unwrap() as u16;
        assert_eq!(buffer[(plugin, 0)].bg, config.palette.accent);
        assert!(hits.tabs.iter().all(|(rect, _)| rect.right() <= plugin));
        assert!(hits.new_tab.right() <= plugin);
    }

    #[test]
    fn indicators_preserve_fork_grammar_counts_order_and_text_accent() {
        assert_eq!(label('и', 0), " и ");
        assert_eq!(label('и', 150), " и 99+ ");
        assert_eq!(label('τ', 5), " τ 5 ");
        let mut snapshot = super::super::tests::snapshot();
        snapshot.resource_facts = Some(crate::protocol::ClientShellResourceFacts {
            notifications: Some(crate::protocol::ClientNotificationSummary {
                total: 200,
                unread: 150,
                ..Default::default()
            }),
            pane_todos: Some(
                [(
                    "pane_1".into(),
                    crate::protocol::ClientPaneTodoSummary {
                        total: 5,
                        open: 5,
                        highest_priority: Some("high".into()),
                        ..Default::default()
                    },
                )]
                .into(),
            ),
            ..Default::default()
        });
        let mut config = ClientShellConfig::from_config(&Config::default());
        let mut hits = ShellHitMap::default();
        let area = Rect::new(0, 0, 40, 1);
        let mut buffer = Buffer::empty(area);
        let content = paint_bar(&mut buffer, area, &snapshot, &config, &mut hits);
        assert_eq!(hits.notification_indicator.right(), area.right());
        assert_eq!(hits.todo_board.right(), hits.notification_indicator.x);
        assert_eq!(content.right(), hits.todo_board.x);
        assert_eq!(
            buffer[(hits.notification_indicator.x + 1, 0)].fg,
            config.palette.accent
        );
        assert_eq!(
            buffer[(hits.notification_indicator.x + 1, 0)].bg,
            ratatui::style::Color::Reset
        );
        assert_eq!(buffer[(hits.todo_board.x + 1, 0)].fg, config.palette.red);
        config.notification_center_position = NotificationCenterPositionConfig::BottomRight;
        let area = Rect::new(4, 2, 10, 3);
        let mut buffer = Buffer::empty(area);
        let rect = paint_floating(&mut buffer, &snapshot, &config, &mut hits).unwrap();
        assert_eq!(rect.right(), area.right());
        assert_eq!(rect.bottom(), area.bottom());
        snapshot.resource_facts = None;
        assert!(paint_floating(&mut buffer, &snapshot, &config, &mut hits).is_none());
    }
}
