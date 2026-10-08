//! One definition of a list's detail box.
//!
//! A list row is one line, so a row whose text does not fit hides some of it.
//! The detail box is where that text goes: an inner bordered box between the
//! list and the footer, holding the selected row's full text wrapped to the
//! panel. The pane todo panel and the todo board both use it, so the rule for
//! when it shows and the way it is drawn cannot differ between them.
//!
//! Whether a row hides text is the dialog's to say — only it knows how wide
//! its row's text is allowed to be. What the box needs and how it looks is
//! said here.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::ui::widgets::render_panel_shell;

/// Text rows a detail box draws at most. Capped so one long note cannot crowd
/// out the list it belongs to; the rest is still in `herdr todo list` and in
/// the editor.
pub(crate) const DETAIL_MAX_TEXT_ROWS: u16 = 6;

/// Where the todo detail box sits, in the pane todos panel and on the session
/// board alike (fork issue 142): `BelowTitle` is title, box, search, list;
/// `AboveList` is title, search, box, list. One line to change.
pub(crate) const TODO_DETAIL_PLACEMENT: crate::ui::overlay::DetailPlacement =
    crate::ui::overlay::DetailPlacement::BelowTitle;

/// What the box says for a selection that hides nothing, so a box kept open
/// for the other rows reads as intentional rather than as a blank.
const NOTHING_HIDDEN: &str = "full text in the row";

/// Columns the box wraps text to, in a detail rect `width` wide: the box's two
/// borders and a column of padding inside each.
pub(crate) fn detail_text_width(width: u16) -> usize {
    usize::from(width.saturating_sub(4))
}

/// Rows a box needs to show `text` in a detail rect `width` wide: its two
/// borders and the wrapped text, capped at [`DETAIL_MAX_TEXT_ROWS`].
pub(crate) fn detail_box_rows(text: &str, width: u16) -> u16 {
    let text_width = detail_text_width(width);
    if text_width == 0 {
        return 0;
    }
    let wrapped = crate::ui::text_wrap::wrap_layout(text, text_width).len() as u16;
    2 + wrapped.clamp(1, DETAIL_MAX_TEXT_ROWS)
}

/// Draw the detail box into `area`: the selected row's full text, or — for a
/// row that hides nothing — a dim note that its text is already on screen.
pub(crate) fn render_detail_box(
    frame: &mut Frame,
    area: Rect,
    text: Option<&str>,
    p: &crate::app::state::Palette,
) {
    let Some(inner) = render_panel_shell(frame, area, p.overlay0, p.panel_bg) else {
        return;
    };
    let text_area = Rect::new(
        inner.x + 1,
        inner.y,
        inner.width.saturating_sub(2),
        inner.height,
    );
    if text_area.width == 0 || text_area.height == 0 {
        return;
    }
    let Some(text) = text else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                NOTHING_HIDDEN,
                Style::default()
                    .fg(p.overlay0)
                    .add_modifier(Modifier::ITALIC),
            ))),
            Rect::new(text_area.x, text_area.y, text_area.width, 1),
        );
        return;
    };
    // `WrappedRow` offsets are into their own logical line, not the whole
    // text, so the line has to be picked before the slice.
    let lines: Vec<&str> = text.split('\n').collect();
    let rows = crate::ui::text_wrap::wrap_layout(text, text_area.width as usize);
    for (row, wrapped) in rows.iter().take(text_area.height as usize).enumerate() {
        let Some(line) = lines.get(wrapped.line) else {
            continue;
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                line[wrapped.start..wrapped.end].to_string(),
                Style::default().fg(p.subtext0),
            ))),
            Rect::new(text_area.x, text_area.y + row as u16, text_area.width, 1),
        );
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, Terminal};

    use super::*;
    use crate::app::state::Palette;

    /// Fork issue 140: the frame was `surface_dim`, the panel colour itself,
    /// so the box had no visible border. Both surfaces (the pane todos panel
    /// and the session board) draw it through this function.
    #[test]
    fn the_box_frame_is_visible_against_the_panel_in_every_palette() {
        let palettes = [
            ("catppuccin", Palette::catppuccin()),
            ("catppuccin_latte", Palette::catppuccin_latte()),
            ("terminal", Palette::terminal()),
            ("tokyo_night", Palette::tokyo_night()),
            ("dracula", Palette::dracula()),
            ("nord", Palette::nord()),
            ("gruvbox", Palette::gruvbox()),
            ("one_light", Palette::one_light()),
            ("solarized", Palette::solarized()),
            ("vesper", Palette::vesper()),
        ];
        for (name, p) in &palettes {
            let mut terminal = Terminal::new(TestBackend::new(20, 5)).unwrap();
            terminal
                .draw(|frame| render_detail_box(frame, Rect::new(0, 0, 20, 5), Some("hi"), p))
                .unwrap();
            let corner = &terminal.backend().buffer()[(0, 0)];
            assert_eq!(corner.fg, p.overlay0, "{name}");
            assert_ne!(corner.fg, p.panel_bg, "{name}: frame differs from panel");
        }
    }

    #[test]
    fn a_box_is_its_borders_and_its_wrapped_text_capped() {
        assert_eq!(detail_box_rows("one", 40), 3);
        assert_eq!(detail_box_rows("one\ntwo\nthree", 40), 5);
        let long = "word ".repeat(200);
        assert_eq!(detail_box_rows(&long, 40), 2 + DETAIL_MAX_TEXT_ROWS);
        assert_eq!(detail_box_rows("one", 4), 0, "no room for a column of text");
    }
}
