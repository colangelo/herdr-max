use crate::{app::state::Palette, terminal::todo::TodoPriority};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};

#[derive(Clone, Copy, Default)]
pub(crate) struct TodoDisplaySummary {
    pub total: usize,
    pub open: usize,
    pub priority: Option<TodoPriority>,
}
impl TodoDisplaySummary {
    pub(crate) fn from_terminal(terminal: &crate::terminal::TerminalState) -> Self {
        let mut result = Self {
            total: terminal.todos().len(),
            ..Self::default()
        };
        for todo in terminal.todos().iter().filter(|todo| !todo.done) {
            result.open += 1;
            result.priority = Some(
                result
                    .priority
                    .map_or(todo.priority, |old| old.max(todo.priority)),
            );
        }
        result
    }
    pub(crate) fn from_fact(fact: &crate::protocol::ClientPaneTodoSummary) -> Self {
        Self {
            total: fact.total,
            open: fact.open,
            priority: match fact.highest_priority.as_deref() {
                Some("high") => Some(TodoPriority::High),
                Some("normal") => Some(TodoPriority::Normal),
                Some("low") => Some(TodoPriority::Low),
                _ => None,
            },
        }
    }
}
pub(crate) struct PaneTodoIndicator {
    pub rect: Rect,
    pub label: String,
    pub color: Color,
    pub outstanding: bool,
}
pub(crate) fn todo_priority_color(
    priority: Option<TodoPriority>,
    palette: &Palette,
    override_color: Option<Color>,
) -> Color {
    override_color.unwrap_or_else(|| match priority {
        Some(TodoPriority::High) => palette.red,
        Some(TodoPriority::Normal) => palette.yellow,
        Some(TodoPriority::Low) => palette.blue,
        None => palette.overlay0,
    })
}
pub(crate) fn pane_todo_indicator_for_rect(
    rect: Rect,
    top_border: bool,
    summary: TodoDisplaySummary,
    enabled: bool,
    palette: &Palette,
    override_color: Option<Color>,
) -> Option<PaneTodoIndicator> {
    if !enabled || !top_border {
        return None;
    }
    let label = if summary.open > 0 {
        format!(" ▾ τ {} ", summary.open)
    } else {
        " ▾ ".to_owned()
    };
    let width = u16::try_from(super::text::display_width(&label)).ok()?;
    if rect.width < width.saturating_add(2) || rect.height == 0 {
        return None;
    }
    let color = if summary.open > 0 {
        todo_priority_color(summary.priority, palette, override_color)
    } else if summary.total > 0 {
        palette.overlay1
    } else {
        palette.overlay0
    };
    Some(PaneTodoIndicator {
        rect: Rect::new(rect.right() - 1 - width, rect.y, width, 1),
        label,
        color,
        outstanding: summary.open > 0,
    })
}
pub(crate) fn paint_pane_todo_indicator(buffer: &mut Buffer, indicator: &PaneTodoIndicator) {
    let rect = indicator.rect.intersection(buffer.area);
    if rect != indicator.rect {
        return;
    }
    let style = Style::default().fg(indicator.color);
    buffer.set_stringn(
        rect.x,
        rect.y,
        &indicator.label,
        usize::from(rect.width),
        style,
    );
    if indicator.outstanding {
        for x in rect.x + 3..rect.right() {
            buffer[(x, rect.y)].set_style(style.add_modifier(Modifier::BOLD));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_bordered_pane_keeps_empty_done_and_outstanding_affordances_distinct() {
        let p = Palette::catppuccin();
        let pane = Rect::new(5, 3, 30, 8);
        let empty =
            pane_todo_indicator_for_rect(pane, true, TodoDisplaySummary::default(), true, &p, None)
                .unwrap();
        let done = pane_todo_indicator_for_rect(
            pane,
            true,
            TodoDisplaySummary {
                total: 2,
                ..Default::default()
            },
            true,
            &p,
            None,
        )
        .unwrap();
        let open = pane_todo_indicator_for_rect(
            pane,
            true,
            TodoDisplaySummary {
                total: 3,
                open: 2,
                priority: Some(TodoPriority::High),
            },
            true,
            &p,
            None,
        )
        .unwrap();
        assert_eq!(empty.label, " ▾ ");
        assert_eq!(done.label, " ▾ ");
        assert_ne!(empty.color, done.color);
        assert_eq!(open.label, " ▾ τ 2 ");
        assert_eq!(open.color, p.red);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 40, 15));
        paint_pane_todo_indicator(&mut buffer, &open);
        assert!(!buffer[(open.rect.x + 1, open.rect.y)]
            .modifier
            .contains(Modifier::BOLD));
        assert!(buffer[(open.rect.x + 3, open.rect.y)]
            .modifier
            .contains(Modifier::BOLD));
        assert_eq!(open.rect.right(), pane.right() - 1);
    }
    #[test]
    fn disabled_borderless_and_too_narrow_panes_offer_no_hit_target() {
        let p = Palette::catppuccin();
        let s = TodoDisplaySummary::default();
        assert!(
            pane_todo_indicator_for_rect(Rect::new(0, 0, 4, 6), true, s, true, &p, None).is_none()
        );
        assert!(
            pane_todo_indicator_for_rect(Rect::new(0, 0, 30, 6), false, s, true, &p, None)
                .is_none()
        );
        assert!(
            pane_todo_indicator_for_rect(Rect::new(0, 0, 30, 6), true, s, false, &p, None)
                .is_none()
        );
    }
}
