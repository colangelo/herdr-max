use super::*;
use crate::config::{SidebarActiveBorderConfig as Border, SidebarStyleConfig};
use ratatui::style::Color;

pub(super) fn header_style(config: &ClientShellConfig) -> Style {
    let style = Style::default().fg(config.palette.overlay0);
    if config.sidebar_style == SidebarStyleConfig::Editorial {
        style.add_modifier(Modifier::DIM)
    } else {
        style.add_modifier(Modifier::BOLD)
    }
}

pub(super) fn number_label(symbol: Option<char>, prefix: &str) -> String {
    symbol
        .map(|symbol| format!("{prefix}{symbol}"))
        .unwrap_or_default()
}

pub(super) fn number_reserve(label: &str, border: Border) -> u16 {
    if label.is_empty() {
        0
    } else {
        super::render::display_width(label).saturating_add(1 + u16::from(border == Border::Right))
    }
}

pub(super) fn draw_number(
    buffer: &mut Buffer,
    rect: Rect,
    y: u16,
    label: &str,
    color: Color,
    border: Border,
) {
    let width = super::render::display_width(label);
    if label.is_empty() || y >= rect.bottom() || rect.width < width.saturating_add(4) {
        return;
    }
    let x = rect
        .right()
        .saturating_sub(width + u16::from(border == Border::Right));
    buffer.set_stringn(x, y, label, usize::from(width), Style::default().fg(color));
}

pub(super) fn draw_active_border(
    buffer: &mut Buffer,
    rect: Rect,
    active: bool,
    config: &ClientShellConfig,
    row_gap: u16,
) {
    if !active || rect.is_empty() {
        return;
    }
    let vertical = match config.sidebar_border_style {
        crate::config::PaneBorderActiveStyleConfig::Light => "│",
        crate::config::PaneBorderActiveStyleConfig::Heavy => "┃",
        crate::config::PaneBorderActiveStyleConfig::Double => "║",
    };
    let horizontal = match config.sidebar_border_style {
        crate::config::PaneBorderActiveStyleConfig::Light => "─",
        crate::config::PaneBorderActiveStyleConfig::Heavy => "━",
        crate::config::PaneBorderActiveStyleConfig::Double => "═",
    };
    let style = Style::default().fg(config.sidebar_border_color.unwrap_or(config.palette.accent));
    match config.sidebar_active_border {
        Border::Left | Border::Right => {
            let x = if config.sidebar_active_border == Border::Left {
                rect.x
            } else {
                rect.right().saturating_sub(1)
            };
            for y in rect.y..rect.bottom() {
                buffer[(x, y)].set_symbol(vertical).set_style(style);
            }
        }
        Border::Above | Border::Below | Border::Both => {
            if row_gap == 0 {
                return;
            }
            // Gap lines only: a neighboring entry or header must never be covered.
            for y in [
                rect.y
                    .checked_sub(1)
                    .filter(|_| config.sidebar_active_border != Border::Below),
                (rect.bottom() < buffer.area.bottom())
                    .then_some(rect.bottom())
                    .filter(|_| config.sidebar_active_border != Border::Above),
            ]
            .into_iter()
            .flatten()
            {
                if y < buffer.area.y {
                    continue;
                }
                if (rect.x..rect.right()).all(|x| buffer[(x, y)].symbol() == " ") {
                    for x in rect.x..rect.right() {
                        buffer[(x, y)].set_symbol(horizontal).set_style(style);
                    }
                }
            }
        }
        Border::Off => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editorial_number_prefix_and_reserve_match_the_fork() {
        assert_eq!(number_label(Some('a'), "₽"), "₽a");
        assert_eq!(number_label(None, "₽"), "");
        assert_eq!(number_reserve("₽a", Border::Off), 3);
        assert_eq!(number_reserve("₽a", Border::Right), 4);
        let area = Rect::new(0, 0, 14, 2);
        let mut buffer = Buffer::empty(area);
        buffer.set_style(area, Style::default().bg(Color::Rgb(1, 2, 3)));
        draw_number(&mut buffer, area, 0, "₽a", Color::Red, Border::Right);
        assert_eq!(buffer[(11, 0)].symbol(), "₽");
        assert_eq!(buffer[(11, 0)].bg, Color::Rgb(1, 2, 3));
    }
    #[test]
    fn editorial_name_truncates_before_jump_without_breaking_active_band() {
        let mut config = Config::default();
        config.ui.sidebar_style = SidebarStyleConfig::Editorial;
        config.ui.show_workspace_numbers = true;
        config.ui.workspace_number_prefix = "₽".into();
        let config = ClientShellConfig::from_config(&config);
        let area = Rect::new(0, 0, 16, 2);
        let entry = WorkspaceEntry {
            visible_index: 9,
            group_collapsed: None,
            index: 0,
            indented: false,
            last_child: false,
        };
        let rows = vec![vec![crate::ui::ResolvedToken {
            kind: crate::ui::ResolvedTokenKind::Workspace("界 very long workspace name".into()),
            style: Default::default(),
        }]];
        let mut buffer = Buffer::empty(area);
        super::super::sidebar::render_workspace_rows(
            &mut buffer,
            area,
            crate::api::schema::AgentStatus::Working,
            &config,
            &entry,
            rows,
            true,
            false,
            false,
            false,
            &config.palette,
        );
        assert_eq!(buffer[(14, 0)].symbol(), "₽");
        assert_eq!(buffer[(15, 0)].symbol(), "a");
        assert!((0..16).all(|x| buffer[(x, 0)].bg == config.palette.active_row_bg));
    }
}
