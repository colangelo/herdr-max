//! One search for every overlay list that can be searched.
//!
//! The navigator grew a search box — a query field, a focus flag, word
//! matching, a key routing between the list chords and the editing set, and a
//! row with a caret — and the move picker and the todo board had none. Rather
//! than a copy per dialog, this is that search once, and a dialog only decides
//! what text each of its rows is matched against.
//!
//! The same key map in every dialog:
//!
//! | Key | List focus | Search focus |
//! |---|---|---|
//! | `/` | focus the search | types `/` |
//! | typing | the dialog's own commands | edits the query |
//! | arrows, `ctrl+j/k/n/p`, Home, End | move | move |
//! | Enter | the dialog's accept | accept the selected row |
//! | Esc | clear a set query, then close | leave the search, keep the query |
//!
//! List focus stays the dialog's: its own letters are its commands there. The
//! kit only answers for the search's side, plus the one list-focus rule every
//! dialog shares, Esc clearing a query before it closes anything
//! ([`ListSearch::clear_on_escape`]).

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::app::list_keys::{list_chord, ListChord, PlainChars};
use crate::app::text_keys::{apply_text_key, Shape};
use crate::ui::text::display_width;
use crate::ui::text_field::TextField;

/// A list's search: the query and whether it has the keyboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListSearch {
    pub query: TextField,
    pub focused: bool,
}

impl Default for ListSearch {
    fn default() -> Self {
        Self {
            query: crate::app::state::search_query_field(),
            focused: false,
        }
    }
}

/// What a key did while the search had focus, for the dialog to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchKey {
    /// The editing set took it: the dialog refilters and selects the first
    /// match.
    Edited,
    /// Esc: the search let go of the keyboard and kept its query.
    Left,
    /// Enter: the dialog accepts its selected row.
    Accept,
    /// A shared list chord: the selection moves as it would from the list.
    Chord(ListChord),
    /// Nothing the search does.
    Ignored,
}

impl ListSearch {
    /// The query as matched: surrounding blanks do not count.
    pub(crate) fn text(&self) -> &str {
        self.query.text().trim()
    }

    /// Whether a query is set, which is what makes a list filtered.
    pub(crate) fn is_active(&self) -> bool {
        !self.text().is_empty()
    }

    /// Every whitespace-separated word of the query appears in `text`,
    /// ignoring case. An empty query matches everything.
    pub(crate) fn matches(&self, text: &str) -> bool {
        text_matches_query(self.text(), text)
    }

    /// `/` from the list.
    pub(crate) fn focus(&mut self) {
        self.focused = true;
    }

    /// Esc from the list: a set query is cleared first, and only a list with
    /// no query is closed. Returns whether Esc was spent here.
    pub(crate) fn clear_on_escape(&mut self) -> bool {
        if !self.is_active() {
            return false;
        }
        self.query.clear();
        true
    }

    /// Route a key while the search has focus: the shared chords with plain
    /// characters as text, so `j` / `k` are typed rather than moving, then the
    /// shared editing set on the query.
    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> SearchKey {
        match key.code {
            KeyCode::Esc => {
                self.focused = false;
                SearchKey::Left
            }
            KeyCode::Enter => SearchKey::Accept,
            _ => {
                if let Some(chord) = list_chord(key.code, key.modifiers, PlainChars::AreText) {
                    return SearchKey::Chord(chord);
                }
                if apply_text_key(&mut self.query, key, Shape::SingleLine) {
                    SearchKey::Edited
                } else {
                    SearchKey::Ignored
                }
            }
        }
    }

    /// Pasted text, which only a focused search takes.
    pub(crate) fn insert_str(&mut self, text: &str) -> bool {
        self.focused && self.query.insert_str(text)
    }
}

/// Every whitespace-separated word of `query` appears in `text`, ignoring
/// case. The rule [`ListSearch::matches`] applies, for rows matched against a
/// query that is not held in a `ListSearch`.
pub(crate) fn text_matches_query(query: &str, text: &str) -> bool {
    let haystack = text.to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|needle| haystack.contains(needle))
}

/// What the search row says besides its query.
pub(crate) struct SearchRow<'a> {
    /// Shown dim while the query is empty; it names the dialog's job, since a
    /// search row has no title bar of its own.
    pub placeholder: &'a str,
    /// The count at the row's end, `"12 panes"`.
    pub count: String,
    /// Drawn in place of the query, for a dialog whose list is filtered by
    /// something other than text (the navigator's state chips). No caret then.
    pub chip: Option<Vec<Span<'static>>>,
}

/// Draw a search row: `/`, the query or its placeholder, then the count.
pub(crate) fn render_search_row(
    frame: &mut Frame,
    area: Rect,
    search: &ListSearch,
    row: SearchRow<'_>,
    p: &crate::app::state::Palette,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let focus_style = if search.focused {
        Style::default().fg(p.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(p.overlay0)
    };
    let mut spans = vec![Span::styled(" / ", focus_style)];
    let has_chip = row.chip.is_some();
    match row.chip {
        Some(chip) => spans.extend(chip),
        None if !search.is_active() => spans.push(Span::styled(
            row.placeholder.to_string(),
            Style::default().fg(p.overlay0),
        )),
        None => spans.push(Span::styled(
            search.text().to_string(),
            Style::default().fg(p.text),
        )),
    }
    // The count sits at the row's end, one column in. A query long enough to
    // reach it wins, and the count gives way rather than being cut in half.
    let used: usize = spans.iter().map(|span| display_width(&span.content)).sum();
    let count_width = display_width(&row.count);
    let room = usize::from(area.width);
    if used + 1 + count_width < room {
        spans.push(Span::raw(" ".repeat(room - used - count_width - 1)));
        spans.push(Span::styled(row.count, Style::default().fg(p.overlay0)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
    if search.focused && !has_chip {
        set_search_caret(frame, area, &search.query);
    }
}

/// Put the terminal cursor in a search row's query, past its `" / "`.
pub(crate) fn set_search_caret(frame: &mut Frame, row: Rect, field: &TextField) {
    if row.width == 0 {
        return;
    }
    let caret_x = row
        .x
        .saturating_add(3)
        .saturating_add(field.cursor_column().min(u16::MAX as usize) as u16)
        .min(row.right().saturating_sub(1));
    frame.set_cursor_position((caret_x, row.y));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn search_for(query: &str) -> ListSearch {
        let mut search = ListSearch::default();
        search.query.insert_str(query);
        search
    }

    #[test]
    fn list_search_matches_every_word_ignoring_case() {
        assert!(search_for("").matches("anything"));
        assert!(search_for("   ").matches("anything"));
        assert!(search_for("ctx").matches("CTX logs"));
        assert!(search_for("Logs CTX").matches("ctx · 2 · logs"));
        assert!(!search_for("ctx build").matches("ctx · 2 · logs"));
        assert!(search_for("#12").matches("rerun the deploy #12 infra · claude"));
        assert!(search_for("ctx").is_active());
        assert!(!search_for("  ").is_active());
    }

    #[test]
    fn list_search_keys_route_chords_edits_and_esc() {
        let mut search = ListSearch::default();
        search.focus();

        // Plain letters are text while the search has focus, `j` included.
        assert_eq!(
            search.handle_key(key(KeyCode::Char('j'))),
            SearchKey::Edited
        );
        assert_eq!(
            search.handle_key(key(KeyCode::Char('/'))),
            SearchKey::Edited
        );
        assert_eq!(search.query.text(), "j/");

        // The modified chords and the arrows still move the list.
        assert_eq!(
            search.handle_key(ctrl('j')),
            SearchKey::Chord(ListChord::Next)
        );
        assert_eq!(
            search.handle_key(key(KeyCode::Up)),
            SearchKey::Chord(ListChord::Prev)
        );
        // `ctrl+u` is the field's kill-to-start, not half a page.
        assert_eq!(search.handle_key(ctrl('u')), SearchKey::Edited);
        assert_eq!(search.query.text(), "");

        assert_eq!(search.handle_key(key(KeyCode::Enter)), SearchKey::Accept);
        assert_eq!(
            search.handle_key(key(KeyCode::F(5))),
            SearchKey::Ignored,
            "a key nothing claims"
        );

        // Esc lets go of the keyboard and keeps the query.
        search.query.insert_str("ctx");
        assert_eq!(search.handle_key(key(KeyCode::Esc)), SearchKey::Left);
        assert!(!search.focused);
        assert_eq!(search.text(), "ctx");

        // From the list, Esc clears a set query first, then has nothing to do.
        assert!(search.clear_on_escape());
        assert!(!search.is_active());
        assert!(!search.clear_on_escape());
    }

    #[test]
    fn only_a_focused_search_takes_a_paste() {
        let mut search = ListSearch::default();
        assert!(!search.insert_str("ctx"));
        search.focus();
        assert!(search.insert_str("ctx"));
        assert_eq!(search.text(), "ctx");
    }
}
