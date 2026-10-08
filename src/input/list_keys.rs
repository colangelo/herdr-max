//! One keymap for every list-bearing overlay.
//!
//! Which key does what *inside* an overlay stays that overlay's business; what
//! this owns is the set of chords that move a selection, so a new overlay does
//! not re-decide it. The navigator's list moved on `j`/`k` and — because those
//! arms carried no modifier guard — on `ctrl+j`/`ctrl+k` too, while its search
//! box moved only on the arrows and `ctrl+n`/`ctrl+p`. Nothing recorded which
//! set an overlay was supposed to offer.
//!
//! Overlays match their own keys first and fall through to [`list_chord`], so
//! an overlay that already binds a letter keeps it.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::ui::overlay::ListCursor;

/// An overlay's own letter key, with Shift read as capitalisation rather than
/// as a chord.
///
/// The letter actions in the todo panel, the todo board and the notification
/// center guard on their modifiers so the `ctrl+` forms fall through to
/// [`list_chord`] — `ctrl+d` is half a page down, not a second way to spell
/// "remove". That guard was written as "no modifiers at all", which also
/// swallowed Shift: `C` did nothing where `c` cleared the completed todos, and
/// nothing on screen explained why.
///
/// Shift on a letter is how the letter is typed; Ctrl and Alt are chords and
/// are left untouched so they still fall through. Only for overlays with no
/// text input — in one with a search box, `J` is text and must stay `J`.
pub(crate) fn overlay_letter_key(key: KeyEvent) -> KeyEvent {
    if key.modifiers != KeyModifiers::SHIFT {
        return key;
    }
    match key.code {
        KeyCode::Char(c) if c.is_ascii_uppercase() => KeyEvent {
            code: KeyCode::Char(c.to_ascii_lowercase()),
            modifiers: KeyModifiers::empty(),
            ..key
        },
        _ => key,
    }
}

/// A movement every list-bearing overlay accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ListChord {
    Prev,
    Next,
    HalfPageUp,
    HalfPageDown,
    First,
    Last,
}

/// Whether plain characters are this overlay's chords or its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlainChars {
    /// No text input has focus: `j` / `k` move.
    AreChords,
    /// A search box has focus: `j` / `k` are text, and only the modified
    /// chords move — so reaching for the search never demotes a picker to the
    /// arrow keys.
    AreText,
}

/// The movement this key asks for, if any.
///
/// First and last are `Home` / `End` rather than `g` / `G`: the letters are
/// already spoken for in some overlays (`g` follows a todo's link) and the
/// shared set must not depend on which overlay is asking.
pub(crate) fn list_chord(
    code: KeyCode,
    modifiers: KeyModifiers,
    plain: PlainChars,
) -> Option<ListChord> {
    let ctrl = modifiers == KeyModifiers::CONTROL;
    let bare = modifiers.is_empty();
    let chords = plain == PlainChars::AreChords;
    match code {
        KeyCode::Up if bare => Some(ListChord::Prev),
        KeyCode::Down if bare => Some(ListChord::Next),
        KeyCode::Char('k' | 'p') if ctrl => Some(ListChord::Prev),
        KeyCode::Char('j' | 'n') if ctrl => Some(ListChord::Next),
        KeyCode::Char('k') if bare && chords => Some(ListChord::Prev),
        KeyCode::Char('j') if bare && chords => Some(ListChord::Next),
        // Half a page is chord-mode only: in a focused text input `ctrl+u` is
        // readline's kill-to-start and `ctrl+d` its delete-forward, and a
        // search box that could not kill its line would be the worse trade.
        KeyCode::Char('u') if ctrl && chords => Some(ListChord::HalfPageUp),
        KeyCode::Char('d') if ctrl && chords => Some(ListChord::HalfPageDown),
        KeyCode::Home if bare => Some(ListChord::First),
        KeyCode::End if bare => Some(ListChord::Last),
        _ => None,
    }
}

impl ListChord {
    /// How many rows this chord moves, or where it jumps to, for a list of
    /// `len` rows shown `visible` at a time.
    pub(crate) fn target(self, selected: usize, visible: usize, len: usize) -> usize {
        let half = (visible / 2).max(1);
        let last = len.saturating_sub(1);
        match self {
            Self::Prev => selected.saturating_sub(1),
            Self::Next => selected.saturating_add(1).min(last),
            Self::HalfPageUp => selected.saturating_sub(half),
            Self::HalfPageDown => selected.saturating_add(half).min(last),
            Self::First => 0,
            Self::Last => last,
        }
    }

    /// Move a cursor and keep the new selection on screen.
    pub(crate) fn apply(self, cursor: &mut ListCursor, visible: usize, len: usize) {
        if len == 0 {
            cursor.select(0);
            return;
        }
        cursor.select(self.target(cursor.selected, visible, len));
        cursor.reveal(visible, len);
    }

    /// The signed step this chord asks for, for overlays that move their
    /// selection through their own mover rather than a [`ListCursor`].
    pub(crate) fn delta(self, selected: usize, visible: usize, len: usize) -> isize {
        let target = self.target(selected, visible, len) as isize;
        target - selected as isize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(code: KeyCode, modifiers: KeyModifiers) -> Option<ListChord> {
        list_chord(code, modifiers, PlainChars::AreChords)
    }

    fn in_text(code: KeyCode, modifiers: KeyModifiers) -> Option<ListChord> {
        list_chord(code, modifiers, PlainChars::AreText)
    }

    const NONE: KeyModifiers = KeyModifiers::empty();
    const CTRL: KeyModifiers = KeyModifiers::CONTROL;

    #[test]
    fn every_move_chord_is_accepted() {
        for (code, modifiers, expected) in [
            (KeyCode::Up, NONE, ListChord::Prev),
            (KeyCode::Down, NONE, ListChord::Next),
            (KeyCode::Char('k'), NONE, ListChord::Prev),
            (KeyCode::Char('j'), NONE, ListChord::Next),
            (KeyCode::Char('k'), CTRL, ListChord::Prev),
            (KeyCode::Char('j'), CTRL, ListChord::Next),
            (KeyCode::Char('p'), CTRL, ListChord::Prev),
            (KeyCode::Char('n'), CTRL, ListChord::Next),
            (KeyCode::Char('u'), CTRL, ListChord::HalfPageUp),
            (KeyCode::Char('d'), CTRL, ListChord::HalfPageDown),
            (KeyCode::Home, NONE, ListChord::First),
            (KeyCode::End, NONE, ListChord::Last),
        ] {
            assert_eq!(chord(code, modifiers), Some(expected), "{code:?}");
        }
    }

    /// No list requires the arrow keys, and a stray modifier on one does
    /// nothing rather than moving.
    #[test]
    fn a_modified_arrow_is_not_a_chord() {
        assert_eq!(chord(KeyCode::Up, KeyModifiers::ALT), None);
        assert_eq!(chord(KeyCode::Down, CTRL), None);
    }

    #[test]
    fn a_focused_search_box_keeps_the_modified_chords_and_types_the_plain_ones() {
        assert_eq!(in_text(KeyCode::Up, NONE), Some(ListChord::Prev));
        assert_eq!(in_text(KeyCode::Down, NONE), Some(ListChord::Next));
        assert_eq!(in_text(KeyCode::Char('j'), CTRL), Some(ListChord::Next));
        assert_eq!(in_text(KeyCode::Char('n'), CTRL), Some(ListChord::Next));
        assert_eq!(in_text(KeyCode::Char('k'), CTRL), Some(ListChord::Prev));
        assert_eq!(in_text(KeyCode::Char('p'), CTRL), Some(ListChord::Prev));

        // Plain `j` / `k` are text there, not movement.
        assert_eq!(in_text(KeyCode::Char('j'), NONE), None);
        assert_eq!(in_text(KeyCode::Char('k'), NONE), None);

        // And `ctrl+u` / `ctrl+d` belong to the text field.
        assert_eq!(in_text(KeyCode::Char('u'), CTRL), None);
        assert_eq!(in_text(KeyCode::Char('d'), CTRL), None);
    }

    #[test]
    fn a_chord_targets_the_row_it_says_it_does() {
        // 20 rows, 6 visible, selection on row 8.
        assert_eq!(ListChord::Prev.target(8, 6, 20), 7);
        assert_eq!(ListChord::Next.target(8, 6, 20), 9);
        assert_eq!(ListChord::HalfPageUp.target(8, 6, 20), 5);
        assert_eq!(ListChord::HalfPageDown.target(8, 6, 20), 11);
        assert_eq!(ListChord::First.target(8, 6, 20), 0);
        assert_eq!(ListChord::Last.target(8, 6, 20), 19);

        // Clamping, not wrapping, at both ends.
        assert_eq!(ListChord::Prev.target(0, 6, 20), 0);
        assert_eq!(ListChord::HalfPageUp.target(1, 6, 20), 0);
        assert_eq!(ListChord::Next.target(19, 6, 20), 19);
        assert_eq!(ListChord::HalfPageDown.target(18, 6, 20), 19);
    }

    /// A list with no rows on screen still moves one row at a time rather than
    /// standing still.
    #[test]
    fn half_a_page_is_at_least_one_row() {
        assert_eq!(ListChord::HalfPageDown.target(0, 0, 20), 1);
        assert_eq!(ListChord::HalfPageDown.target(0, 1, 20), 1);
    }

    #[test]
    fn applying_a_chord_moves_the_cursor_and_keeps_it_visible() {
        let mut cursor = ListCursor::new(0);
        ListChord::Last.apply(&mut cursor, 6, 20);
        assert_eq!(cursor.selected, 19);
        assert_eq!(cursor.scroll, 14);

        ListChord::First.apply(&mut cursor, 6, 20);
        assert_eq!(cursor.selected, 0);
        assert_eq!(cursor.scroll, 0);
    }

    #[test]
    fn an_empty_list_parks_at_zero() {
        let mut cursor = ListCursor::new(3);
        ListChord::Next.apply(&mut cursor, 6, 0);
        assert_eq!(cursor.selected, 0);
    }
}
