//! Soft word-wrap layout for editable text.
//!
//! Pure geometry beside [`super::text_field::TextField`]: given text and a
//! width, produce the wrapped visual rows, and map the caret's logical
//! position onto them and a click back off them. One layout serves the
//! renderer, the caret math and the mouse hit-test, so the caret can never
//! sit where the renderer did not put the character.
//!
//! Wrapping is presentation only — nothing here touches the stored text, and
//! no soft break introduces a character into it.

use super::text::char_display_width;

/// One visual row of the wrapped text: a byte slice of one logical line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WrappedRow {
    /// Index of the source logical line (text split on `\n`).
    pub line: usize,
    /// Byte range within that line.
    pub start: usize,
    pub end: usize,
    /// Display column of `start` within the logical line, for caret mapping.
    pub start_col: usize,
    /// Display width of the slice.
    pub width: usize,
}

/// Wrap `text` to `width` display columns.
///
/// Breaks at the last space that fits; the space is consumed by the break (it
/// stays in the stored text but is not rendered, so rows do not end in a
/// ragged gutter). A word wider than the whole width hard-breaks at the
/// width. An explicit newline always ends a row, and the empty text still
/// yields one empty row so the caret has somewhere to be.
pub(crate) fn wrap_layout(text: &str, width: usize) -> Vec<WrappedRow> {
    let width = width.max(1);
    let mut rows = Vec::new();
    for (line_idx, line) in text.split('\n').enumerate() {
        let mut start = 0usize;
        let mut start_col = 0usize;
        let mut row_width = 0usize;
        // Last space in the current row a break may consume: its byte range
        // and the display column it starts at.
        let mut break_at: Option<(usize, usize, usize)> = None;
        for (byte, ch) in line.char_indices() {
            let ch_width = char_display_width(ch);
            if row_width + ch_width > width && row_width > 0 {
                let (end, next_start, consumed) = match break_at {
                    Some((space_start, space_end, space_col)) => {
                        (space_start, space_end, space_col - start_col)
                    }
                    None => (byte, byte, row_width),
                };
                rows.push(WrappedRow {
                    line: line_idx,
                    start,
                    end,
                    start_col,
                    width: consumed,
                });
                // Columns consumed so far: the row itself, plus the space a
                // soft break swallowed.
                start_col += match break_at {
                    Some((_, _, space_col)) => space_col - start_col + 1,
                    None => row_width,
                };
                start = next_start;
                row_width = line[start..byte]
                    .chars()
                    .map(char_display_width)
                    .sum::<usize>();
                break_at = None;
            }
            if ch == ' ' {
                break_at = Some((byte, byte + 1, start_col + row_width));
            }
            row_width += ch_width;
        }
        rows.push(WrappedRow {
            line: line_idx,
            start,
            end: line.len(),
            start_col,
            width: row_width,
        });
    }
    rows
}

/// The visual (row, column) the caret at logical (`line`, `column`) sits on.
///
/// A caret past a consumed break space belongs to the next row's start; a
/// caret at the very end of a row sits on the blank cell after its text.
pub(crate) fn caret_visual_position(
    rows: &[WrappedRow],
    line: usize,
    column: usize,
) -> (usize, usize) {
    let mut result = (0, 0);
    for (idx, row) in rows.iter().enumerate() {
        if row.line < line || (row.line == line && row.start_col <= column) {
            result = (idx, (column.saturating_sub(row.start_col)).min(row.width));
        }
    }
    result
}

/// The logical (line, column) for a click on visual (`row`, `column`),
/// clamped into the nearest real position.
pub(crate) fn visual_to_logical(rows: &[WrappedRow], row: usize, column: usize) -> (usize, usize) {
    let Some(row) = rows.get(row).or_else(|| rows.last()) else {
        return (0, 0);
    };
    (row.line, row.start_col + column.min(row.width))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slices<'a>(text: &'a str, rows: &[WrappedRow]) -> Vec<&'a str> {
        let lines: Vec<&str> = text.split('\n').collect();
        rows.iter()
            .map(|row| &lines[row.line][row.start..row.end])
            .collect()
    }

    #[test]
    fn breaks_at_the_last_space_that_fits_and_consumes_it() {
        let text = "rerun the deploy and check";
        let rows = wrap_layout(text, 12);
        assert_eq!(slices(text, &rows), ["rerun the", "deploy and", "check"]);
        // The consumed spaces exist in the text but start the next row past
        // them: columns 0..9, 10..20, 21..26.
        assert_eq!(
            rows.iter().map(|r| r.start_col).collect::<Vec<_>>(),
            [0, 10, 21]
        );
    }

    #[test]
    fn a_word_wider_than_the_block_hard_breaks() {
        let text = "xxxxxxxxxxxx";
        let rows = wrap_layout(text, 5);
        assert_eq!(slices(text, &rows), ["xxxxx", "xxxxx", "xx"]);
    }

    #[test]
    fn explicit_newlines_always_end_a_row() {
        let text = "one\ntwo three\n\nfour";
        let rows = wrap_layout(text, 40);
        assert_eq!(slices(text, &rows), ["one", "two three", "", "four"]);
        assert_eq!(
            rows.iter().map(|r| r.line).collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
    }

    #[test]
    fn empty_text_yields_one_empty_row() {
        let rows = wrap_layout("", 10);
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].start, rows[0].end, rows[0].width), (0, 0, 0));
    }

    #[test]
    fn wide_glyphs_wrap_by_display_width() {
        // Each CJK glyph is two columns, so four fit in eight.
        let text = "今日の予定を確認する";
        let rows = wrap_layout(text, 8);
        assert!(rows.iter().all(|r| r.width <= 8));
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn caret_maps_through_the_wrap_and_back() {
        let text = "rerun the deploy and check";
        let rows = wrap_layout(text, 12);
        // Caret at logical column 12 ("d" of deploy, column 10 starts row 1).
        assert_eq!(caret_visual_position(&rows, 0, 12), (1, 2));
        assert_eq!(visual_to_logical(&rows, 1, 2), (0, 12));
        // Caret on the consumed break space renders at its row's end.
        assert_eq!(caret_visual_position(&rows, 0, 9), (0, 9));
        // Caret at the very start and very end.
        assert_eq!(caret_visual_position(&rows, 0, 0), (0, 0));
        assert_eq!(caret_visual_position(&rows, 0, 26), (2, 5));
    }

    #[test]
    fn clicks_clamp_into_the_nearest_real_position() {
        let text = "one\ntwo";
        let rows = wrap_layout(text, 10);
        // Past the end of a row, and past the last row.
        assert_eq!(visual_to_logical(&rows, 0, 99), (0, 3));
        assert_eq!(visual_to_logical(&rows, 99, 1), (1, 1));
        assert_eq!(visual_to_logical(&[], 0, 0), (0, 0));
    }
}
