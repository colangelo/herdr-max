use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

use super::widgets::panel_contrast_fg;
use crate::{
    app::state::{CopyFeedback, Palette},
    config::ToastClipboardPosition,
};

pub(crate) fn copy_feedback_rect(
    area: Rect,
    feedback: &CopyFeedback,
    offset_rows: u16,
    position: ToastClipboardPosition,
) -> Rect {
    if area.is_empty() {
        return Rect::default();
    }

    let content_width = feedback.message.len() as u16 + 4;
    let width = content_width.min(area.width);
    let height = 3u16.min(area.height);
    let x = match position {
        ToastClipboardPosition::TopLeft | ToastClipboardPosition::BottomLeft => area.x,
        ToastClipboardPosition::TopCenter
        | ToastClipboardPosition::BottomCenter
        | ToastClipboardPosition::Pane => area.x + area.width.saturating_sub(width) / 2,
        ToastClipboardPosition::TopRight | ToastClipboardPosition::BottomRight => {
            area.x + area.width.saturating_sub(width)
        }
    };
    let y = match position {
        ToastClipboardPosition::TopLeft
        | ToastClipboardPosition::TopCenter
        | ToastClipboardPosition::TopRight => area.y + offset_rows.min(area.height),
        ToastClipboardPosition::BottomLeft
        | ToastClipboardPosition::BottomCenter
        | ToastClipboardPosition::BottomRight
        | ToastClipboardPosition::Pane => area.y + area.height.saturating_sub(height + offset_rows),
    };
    Rect::new(x, y, width, height)
}

pub(crate) fn copy_feedback_rect_in_pane(pane: Rect, feedback: &CopyFeedback) -> Option<Rect> {
    let width = u16::try_from(super::text::display_width(&feedback.message))
        .ok()?
        .checked_add(4)?;
    let height = 3;
    (width > 0 && width <= pane.width && height <= pane.height).then(|| {
        Rect::new(
            pane.x + (pane.width - width) / 2,
            pane.y + (pane.height - height) / 2,
            width,
            height,
        )
    })
}

pub(crate) fn render_copy_feedback_buffer(
    buffer: &mut Buffer,
    area: Rect,
    feedback: &CopyFeedback,
    offset_rows: u16,
    position: ToastClipboardPosition,
    palette: &Palette,
) -> Rect {
    render_copy_feedback_buffer_for_source(
        buffer,
        area,
        None,
        feedback,
        offset_rows,
        position,
        palette,
    )
}

pub(crate) fn render_copy_feedback_buffer_for_source(
    buffer: &mut Buffer,
    area: Rect,
    source: Option<Rect>,
    feedback: &CopyFeedback,
    offset_rows: u16,
    position: ToastClipboardPosition,
    palette: &Palette,
) -> Rect {
    let feedback_area = if position == ToastClipboardPosition::Pane {
        source.and_then(|pane| copy_feedback_rect_in_pane(pane, feedback))
    } else {
        None
    }
    .unwrap_or_else(|| copy_feedback_rect(area, feedback, offset_rows, position));
    if feedback_area.is_empty() {
        return feedback_area;
    }

    Clear.render(feedback_area, buffer);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.green))
        .style(Style::default().bg(palette.panel_bg));
    let inner = block.inner(feedback_area);
    block.render(feedback_area, buffer);

    if inner.height == 0 {
        return feedback_area;
    }

    let text = Line::from(vec![
        Span::styled("●", Style::default().fg(palette.green).bg(palette.panel_bg)),
        Span::raw(" "),
        Span::styled(
            &feedback.message,
            Style::default()
                .fg(palette.text)
                .bg(palette.panel_bg)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    Paragraph::new(text).render(inner, buffer);
    feedback_area
}

pub(crate) fn render_config_diagnostic_buffer(
    buffer: &mut Buffer,
    area: Rect,
    message: &str,
    palette: &Palette,
    mut covered: impl FnMut(Rect),
) -> u16 {
    let style = Style::default()
        .fg(panel_contrast_fg(palette))
        .bg(palette.yellow)
        .add_modifier(Modifier::BOLD);
    let mut rendered_rows = 0u16;

    for (row, line) in message
        .lines()
        .filter(|line| !line.trim().is_empty())
        .take(area.height as usize)
        .enumerate()
    {
        let text = format!(" {line} ");
        let width = (text.len() as u16).min(area.width);
        let diagnostic_area = Rect::new(
            area.x + area.width.saturating_sub(width),
            area.y + row as u16,
            width,
            1,
        );

        covered(diagnostic_area);
        Clear.render(diagnostic_area, buffer);
        Paragraph::new(Span::styled(text, style)).render(diagnostic_area, buffer);
        rendered_rows = rendered_rows.saturating_add(1);
    }

    rendered_rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_feedback_rect_uses_configured_position() {
        let area = Rect::new(10, 20, 100, 40);
        let feedback = CopyFeedback {
            message: "copied to clipboard".to_owned(),
            source_pane: None,
        };

        let top = copy_feedback_rect(area, &feedback, 0, ToastClipboardPosition::TopCenter);
        assert_eq!(top.y, area.y);
        assert_eq!(top.x, area.x + area.width.saturating_sub(top.width) / 2);

        let bottom = copy_feedback_rect(area, &feedback, 0, ToastClipboardPosition::BottomCenter);
        assert_eq!(bottom.bottom(), area.bottom());
        assert_eq!(
            bottom.x,
            area.x + area.width.saturating_sub(bottom.width) / 2
        );
    }
    #[test]
    fn feedback_centers_in_a_pane_that_holds_it() {
        let pane = Rect::new(40, 5, 60, 20);
        let feedback = CopyFeedback {
            message: "copied to clipboard".into(),
            source_pane: None,
        };
        let copy = copy_feedback_rect_in_pane(pane, &feedback).expect("fits");
        assert_eq!((copy.width, copy.height), (23, 3));
        assert_eq!((copy.x, copy.y), (40 + (60 - 23) / 2, 5 + (20 - 3) / 2));
    }
    #[test]
    fn feedback_does_not_center_in_a_pane_too_small_for_it() {
        let feedback = CopyFeedback {
            message: "copied to clipboard".into(),
            source_pane: None,
        };
        assert_eq!(
            copy_feedback_rect_in_pane(Rect::new(0, 0, 22, 10), &feedback),
            None
        );
        assert_eq!(
            copy_feedback_rect_in_pane(Rect::new(0, 0, 40, 2), &feedback),
            None
        );
    }
    #[test]
    fn pane_position_falls_back_to_bottom_center() {
        let area = Rect::new(0, 0, 80, 24);
        let feedback = CopyFeedback {
            message: "copied to clipboard".into(),
            source_pane: None,
        };
        assert_eq!(
            copy_feedback_rect(area, &feedback, 0, ToastClipboardPosition::Pane),
            copy_feedback_rect(area, &feedback, 0, ToastClipboardPosition::BottomCenter)
        );
    }
}
