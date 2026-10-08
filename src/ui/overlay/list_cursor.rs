//! One cursor for every overlay list.
//!
//! `ListCursor` and `ListCursor` were the same struct with a
//! different field name, and several overlays kept a bare `selected: usize`
//! next to a hand-rolled "keep the selection visible" rule. This is that, once,
//! with the windowing the overlays used to re-derive.
//!
//! It deliberately does not know about `crate::ui::list_motion`: bubble motion
//! is a display-order transform over a settled list, it composes with a cursor
//! rather than living in one, and folding it in would drag animation state into
//! a type the modals also use.

use ratatui::layout::Rect;

/// The selection and scroll of one overlay list.
///
/// `scroll` is mutated where the selection moves — never during render — so
/// what is drawn and what a click maps to are read from the same value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct ListCursor {
    pub selected: usize,
    /// First visible index.
    pub scroll: usize,
}

impl ListCursor {
    pub(crate) fn new(selected: usize) -> Self {
        Self {
            selected,
            scroll: 0,
        }
    }

    /// Move the selection, clamping at both ends rather than wrapping.
    pub(crate) fn move_by(&mut self, delta: isize, len: usize) {
        if len == 0 {
            self.selected = 0;
            return;
        }
        self.selected = self.selected.saturating_add_signed(delta).min(len - 1);
    }

    pub(crate) fn select(&mut self, idx: usize) {
        self.selected = idx;
    }

    /// Follow the pointer. `None` — the pointer is off the list — leaves the
    /// selection where it was.
    pub(crate) fn hover(&mut self, idx: Option<usize>) {
        if let Some(idx) = idx {
            self.selected = idx;
        }
    }

    /// Scroll by the minimum needed to bring the selection into a `visible`-row
    /// window: reveal the nearest edge rather than recentering, and leave the
    /// window alone when the selection is already in it.
    pub(crate) fn reveal(&mut self, visible: usize, len: usize) {
        self.scroll = reveal_scroll(self.scroll, self.selected, visible, len);
    }

    /// Move the visible window without moving the selection, clamped at both
    /// ends. The wheel on a long scannable list scrolls the view; dragging the
    /// selection with it makes the list unreadable while you look for
    /// something in it.
    pub(crate) fn scroll_by(&mut self, delta: isize, visible: usize, len: usize) {
        let max = len.saturating_sub(visible);
        self.scroll = self.scroll.saturating_add_signed(delta).min(max);
    }

    /// Reveal `index`, and pull `context` into view as well when the two fit
    /// in the window together. See [`reveal_scroll_with_context`].
    pub(crate) fn reveal_with_context(&mut self, context: usize, visible: usize, len: usize) {
        self.scroll = reveal_scroll_with_context(self.scroll, context, self.selected, visible, len);
    }

    /// The first visible index and how many rows fit. Clamps a scroll left
    /// stale by a list that shrank underneath it.
    pub(crate) fn window(&self, list: Rect, len: usize) -> (usize, usize) {
        let visible = list.height as usize;
        (self.scroll.min(len.saturating_sub(visible)), visible)
    }

    /// The index drawn on this cell, the inverse of [`Self::window`] — so the
    /// row the mouse picks is the row the renderer drew there.
    pub(crate) fn row_at(&self, list: Rect, col: u16, row: u16, len: usize) -> Option<usize> {
        let within = col >= list.x
            && col < list.x.saturating_add(list.width)
            && row >= list.y
            && row < list.y.saturating_add(list.height);
        if len == 0 || !within {
            return None;
        }
        let (start, _) = self.window(list, len);
        let idx = start + (row - list.y) as usize;
        (idx < len).then_some(idx)
    }
}

/// The smallest scroll offset that keeps `index` inside a `visible`-row window.
///
/// Free-standing because the navigator scrolls in display-line space while its
/// selection is a row index, so it needs the arithmetic without the cursor.
pub(crate) fn reveal_scroll(scroll: usize, index: usize, visible: usize, len: usize) -> usize {
    if visible == 0 {
        return 0;
    }
    let scroll = if index < scroll {
        index
    } else if index >= scroll.saturating_add(visible) {
        index.saturating_add(1).saturating_sub(visible)
    } else {
        scroll
    };
    scroll.min(len.saturating_sub(visible))
}

/// [`reveal_scroll`], plus a second index that should be on screen when it can
/// be.
///
/// The selection always wins: a group taller than the window shows the
/// selection rather than the heading above it. Without this, a list whose
/// first row is never selectable — the board's first heading — can never be
/// scrolled back to, because the scroll floor is the selection's own index.
pub(crate) fn reveal_scroll_with_context(
    scroll: usize,
    context: usize,
    index: usize,
    visible: usize,
    len: usize,
) -> usize {
    let scroll = reveal_scroll(scroll, index, visible, len);
    if visible == 0 || context >= scroll {
        return scroll;
    }
    if index.saturating_sub(context) < visible {
        return context;
    }
    scroll
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: Rect = Rect {
        x: 4,
        y: 10,
        width: 20,
        height: 5,
    };

    /// Regression: the board's first row is a heading and its selection can
    /// only be a todo, so the scroll floor was the first todo's index and the
    /// heading above it could never be scrolled back to.
    #[test]
    fn context_pulls_an_unselectable_leading_row_back_into_view() {
        // Selection on item 1, heading at 0, five-row window: revealing 1
        // alone floors the scroll at 1 and strands the heading.
        assert_eq!(reveal_scroll(9, 1, 5, 20), 1);
        assert_eq!(reveal_scroll_with_context(9, 0, 1, 5, 20), 0);

        // The selection wins when the group cannot fit its heading too.
        assert_eq!(reveal_scroll_with_context(0, 0, 12, 5, 20), 8);

        // A context already on screen changes nothing.
        assert_eq!(reveal_scroll_with_context(6, 6, 8, 5, 20), 6);
    }

    #[test]
    fn scrolling_the_window_leaves_the_selection_alone() {
        let mut cursor = ListCursor::new(3);
        cursor.scroll_by(4, 5, 20);
        assert_eq!((cursor.scroll, cursor.selected), (4, 3));
        // Clamped at the far end, and never past it.
        cursor.scroll_by(100, 5, 20);
        assert_eq!((cursor.scroll, cursor.selected), (15, 3));
        // And at zero.
        cursor.scroll_by(-100, 5, 20);
        assert_eq!((cursor.scroll, cursor.selected), (0, 3));
    }

    #[test]
    fn movement_clamps_at_both_ends_rather_than_wrapping() {
        let mut cursor = ListCursor::new(0);
        cursor.move_by(-1, 8);
        assert_eq!(cursor.selected, 0);
        cursor.move_by(100, 8);
        assert_eq!(cursor.selected, 7);
        cursor.move_by(1, 8);
        assert_eq!(cursor.selected, 7);
    }

    #[test]
    fn an_empty_list_parks_the_selection_at_zero() {
        let mut cursor = ListCursor::new(6);
        cursor.move_by(1, 0);
        assert_eq!(cursor.selected, 0);
    }

    /// The rule the navigator, the notification center and the todo panel each
    /// used to implement: scroll the minimum needed, never recenter, and leave
    /// the window alone when the selection is already inside it.
    #[test]
    fn revealing_scrolls_by_the_minimum_and_never_recenters() {
        let mut cursor = ListCursor::new(0);
        cursor.reveal(5, 20);
        assert_eq!(cursor.scroll, 0);

        // Down past the bottom edge: one row of scroll, not a recentre.
        cursor.select(5);
        cursor.reveal(5, 20);
        assert_eq!(cursor.scroll, 1);

        // Already visible: the window does not move.
        cursor.select(3);
        cursor.reveal(5, 20);
        assert_eq!(cursor.scroll, 1);

        // Up past the top edge: the top edge is revealed, nothing more.
        cursor.select(0);
        cursor.reveal(5, 20);
        assert_eq!(cursor.scroll, 0);

        // A jump lands the selection on the nearest edge.
        cursor.select(19);
        cursor.reveal(5, 20);
        assert_eq!(cursor.scroll, 15);
    }

    #[test]
    fn a_window_with_no_rows_scrolls_nowhere() {
        let mut cursor = ListCursor::new(9);
        cursor.reveal(0, 20);
        assert_eq!(cursor.scroll, 0);
    }

    /// A list that shrank under a stale scroll still renders from a valid
    /// start rather than off the end.
    #[test]
    fn the_window_clamps_a_scroll_the_list_outgrew() {
        let cursor = ListCursor {
            selected: 0,
            scroll: 40,
        };
        assert_eq!(cursor.window(LIST, 8), (3, 5));
        assert_eq!(cursor.window(LIST, 2), (0, 5));
    }

    #[test]
    fn clicking_selects_the_index_that_was_drawn_there() {
        let cursor = ListCursor {
            selected: 7,
            scroll: 3,
        };
        let (start, visible) = cursor.window(LIST, 20);
        assert_eq!((start, visible), (3, 5));
        for offset in 0..visible {
            assert_eq!(
                cursor.row_at(LIST, LIST.x, LIST.y + offset as u16, 20),
                Some(start + offset),
                "row {offset} maps back to what render drew on it"
            );
        }
    }

    #[test]
    fn a_click_off_the_list_or_past_its_last_row_hits_nothing() {
        let cursor = ListCursor::new(0);
        assert_eq!(cursor.row_at(LIST, LIST.x - 1, LIST.y, 20), None);
        assert_eq!(cursor.row_at(LIST, LIST.x + LIST.width, LIST.y, 20), None);
        assert_eq!(cursor.row_at(LIST, LIST.x, LIST.y - 1, 20), None);
        assert_eq!(cursor.row_at(LIST, LIST.x, LIST.y + LIST.height, 20), None);
        // Fewer entries than rows: the blank rows below the last one are not
        // clickable.
        assert_eq!(cursor.row_at(LIST, LIST.x, LIST.y + 1, 2), Some(1));
        assert_eq!(cursor.row_at(LIST, LIST.x, LIST.y + 2, 2), None);
        assert_eq!(cursor.row_at(LIST, LIST.x, LIST.y, 0), None);
    }

    #[test]
    fn hovering_off_the_list_leaves_the_selection_alone() {
        let mut cursor = ListCursor::new(4);
        cursor.hover(None);
        assert_eq!(cursor.selected, 4);
        cursor.hover(Some(1));
        assert_eq!(cursor.selected, 1);
    }
}
