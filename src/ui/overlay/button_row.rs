//! One definition of an overlay's footer button row.
//!
//! Both panels used to hand-roll the same thing around
//! [`crate::ui::widgets::action_button_row_rects`]: decide which boxes fit at
//! this width, lay them out, hit-test a click against them, and treat a click
//! that lands on the row but on no box as inert. The two differed only in the
//! enum they returned.

use ratatui::layout::Rect;

use crate::ui::widgets::{action_button_width, centered_button_row};

/// Blank columns between two boxes.
const BUTTON_GAP: u16 = 2;

/// One box the overlay would like on its footer row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ButtonSpec<B> {
    pub button: B,
    /// Shortcut hint drawn inside the box.
    pub hint: Option<&'static str>,
    pub label: &'static str,
    /// Dropped in ascending order while the row is too narrow for everything.
    /// `None` is never dropped: the box that dismisses the overlay, and
    /// whatever the overlay would be a dead end without.
    pub drop_rank: Option<u8>,
}

/// A box that made it onto the row, and where it was drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlacedButton<B> {
    pub button: B,
    pub hint: Option<&'static str>,
    pub label: &'static str,
    pub rect: Rect,
}

/// What a click on the footer row landed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ButtonRowHit<B> {
    Button(B),
    /// The buttons' row, but no box: inert, so a near-miss on a button does not
    /// dismiss the overlay. Matched on the row rather than on the row's rect,
    /// because a click level with the buttons is a near-miss wherever on that
    /// line it lands — which is how both panels have always behaved.
    NearMiss,
}

/// A laid-out footer row: what fit, where each box was drawn, and what a click
/// on it hits. The renderer draws from this and the mouse layer hit-tests
/// against it, so what looks clickable is clickable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ButtonRow<B> {
    row: Rect,
    placed: Vec<PlacedButton<B>>,
}

impl<B: Copy> ButtonRow<B> {
    /// Lay `buttons` out centered on `row`, dropping the least essential box
    /// still standing until the row fits.
    ///
    /// When even the never-dropped boxes do not fit they are laid out anyway:
    /// a cramped row still tells the user what the overlay can do, and both
    /// panels behaved that way before this was shared.
    pub(crate) fn layout(row: Rect, buttons: &[ButtonSpec<B>]) -> Option<Self> {
        if row.width == 0 || row.height == 0 || buttons.is_empty() {
            return None;
        }

        let mut kept: Vec<&ButtonSpec<B>> = buttons.iter().collect();
        loop {
            if row_width(&kept) <= row.width {
                break;
            }
            let Some(next) = kept.iter().filter_map(|spec| spec.drop_rank).min() else {
                break;
            };
            kept.retain(|spec| spec.drop_rank != Some(next));
        }

        let widths: Vec<u16> = kept
            .iter()
            .map(|spec| action_button_width(spec.hint, spec.label))
            .collect();
        let placed = centered_button_row(row, &widths, BUTTON_GAP, 0)
            .into_iter()
            .zip(kept)
            .map(|(rect, spec)| PlacedButton {
                button: spec.button,
                hint: spec.hint,
                label: spec.label,
                rect,
            })
            .collect();
        Some(Self { row, placed })
    }

    /// The width this row wants with nothing dropped — what a panel must be
    /// at least as wide as if it means to show all of its own controls.
    pub(crate) fn natural_width(buttons: &[ButtonSpec<B>]) -> u16 {
        row_width(&buttons.iter().collect::<Vec<_>>())
    }

    /// The boxes that fit, in render order.
    pub(crate) fn placed(&self) -> &[PlacedButton<B>] {
        &self.placed
    }

    /// The row the boxes sit on. Production reads placement through
    /// [`Self::hit`] and [`Self::placed`]; this is for tests asserting where
    /// the row was actually drawn.
    #[cfg(test)]
    pub(crate) fn row_y(&self) -> u16 {
        self.row.y
    }

    /// The box under this cell, for hover.
    pub(crate) fn button_at(&self, col: u16, row: u16) -> Option<B> {
        self.placed
            .iter()
            .find(|placed| {
                row == placed.rect.y
                    && col >= placed.rect.x
                    && col < placed.rect.x.saturating_add(placed.rect.width)
            })
            .map(|placed| placed.button)
    }

    /// What a click lands on, near-misses included.
    pub(crate) fn hit(&self, col: u16, row: u16) -> Option<ButtonRowHit<B>> {
        if let Some(button) = self.button_at(col, row) {
            return Some(ButtonRowHit::Button(button));
        }
        (row == self.row.y).then_some(ButtonRowHit::NearMiss)
    }
}

impl<B: Copy + PartialEq> ButtonRow<B> {
    /// Where a button was drawn, or `None` when it was dropped. Production
    /// draws from [`Self::placed`]; this answers "did this box survive?" for
    /// tests.
    #[cfg(test)]
    pub(crate) fn rect(&self, button: B) -> Option<Rect> {
        self.placed
            .iter()
            .find(|placed| placed.button == button)
            .map(|placed| placed.rect)
    }
}

fn row_width<B>(buttons: &[&ButtonSpec<B>]) -> u16 {
    buttons
        .iter()
        .map(|spec| action_button_width(spec.hint, spec.label))
        .sum::<u16>()
        .saturating_add(BUTTON_GAP.saturating_mul(buttons.len().saturating_sub(1) as u16))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Button {
        Add,
        Toggle,
        ClearDone,
        Go,
        Close,
    }

    fn specs() -> Vec<ButtonSpec<Button>> {
        vec![
            ButtonSpec {
                button: Button::Add,
                hint: Some("a"),
                label: "add",
                drop_rank: None,
            },
            ButtonSpec {
                button: Button::Toggle,
                hint: Some("spc"),
                label: "toggle",
                drop_rank: Some(0),
            },
            ButtonSpec {
                button: Button::Go,
                hint: Some("g"),
                label: "go",
                drop_rank: Some(2),
            },
            ButtonSpec {
                button: Button::ClearDone,
                hint: Some("c"),
                label: "clear done",
                drop_rank: Some(1),
            },
            ButtonSpec {
                button: Button::Close,
                hint: Some("esc"),
                label: "close",
                drop_rank: None,
            },
        ]
    }

    fn row(width: u16) -> ButtonRow<Button> {
        ButtonRow::layout(Rect::new(0, 7, width, 1), &specs()).expect("row lays out")
    }

    fn buttons(row: &ButtonRow<Button>) -> Vec<Button> {
        row.placed().iter().map(|placed| placed.button).collect()
    }

    #[test]
    fn a_wide_row_keeps_every_box_in_spec_order() {
        assert_eq!(
            buttons(&row(80)),
            vec![
                Button::Add,
                Button::Toggle,
                Button::Go,
                Button::ClearDone,
                Button::Close
            ]
        );
    }

    #[test]
    fn a_narrow_row_drops_by_rank_and_keeps_the_boxes_that_never_drop() {
        // 52 columns hold everything; each step below drops the next rank.
        assert_eq!(buttons(&row(51)).len(), 4);
        assert!(!buttons(&row(51)).contains(&Button::Toggle));

        let dropped_two = buttons(&row(36));
        assert_eq!(dropped_two, vec![Button::Add, Button::Go, Button::Close]);

        let bare = buttons(&row(19));
        assert_eq!(bare, vec![Button::Add, Button::Close]);
    }

    /// Nothing fits, but the row still says what the overlay can do rather
    /// than going blank.
    #[test]
    fn a_row_too_narrow_for_anything_still_offers_what_never_drops() {
        assert_eq!(buttons(&row(4)), vec![Button::Add, Button::Close]);
    }

    #[test]
    fn the_box_that_is_drawn_is_the_box_the_mouse_hits() {
        let row = row(80);
        for placed in row.placed() {
            for col in placed.rect.x..placed.rect.x + placed.rect.width {
                assert_eq!(
                    row.hit(col, placed.rect.y),
                    Some(ButtonRowHit::Button(placed.button)),
                    "column {col} of {:?}",
                    placed.button
                );
            }
        }
    }

    #[test]
    fn a_click_on_the_row_but_on_no_box_is_a_near_miss() {
        let row = row(80);
        let add = row.rect(Button::Add).expect("add is never dropped");
        let gap = add.x + add.width;
        assert_eq!(row.button_at(gap, row.row_y()), None);
        assert_eq!(row.hit(gap, row.row_y()), Some(ButtonRowHit::NearMiss));
        // Level with the buttons is a near-miss wherever on that line it
        // lands, panel edges included.
        assert_eq!(row.hit(0, row.row_y()), Some(ButtonRowHit::NearMiss));
        assert_eq!(row.hit(gap, row.row_y() + 1), None);
    }

    #[test]
    fn a_row_with_no_room_at_all_lays_nothing_out() {
        assert!(ButtonRow::layout(Rect::new(0, 7, 0, 1), &specs()).is_none());
        assert!(ButtonRow::layout(Rect::new(0, 7, 40, 0), &specs()).is_none());
        assert!(ButtonRow::<Button>::layout(Rect::new(0, 7, 40, 1), &[]).is_none());
    }
}
