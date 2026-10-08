//! A cursor-bearing text buffer for modal composition.
//!
//! Pure data: it renders nothing and owns no keymap. Callers translate keys
//! into calls on it, which keeps the project's state/render split intact and
//! lets a modal adopt it without inheriting another modal's key policy.
//!
//! The cursor is a byte offset into the buffer, moved only over `char`
//! boundaries. Grapheme clusters are not a unit here: a combining sequence
//! moves one `char` at a time, matching how the rest of Herdr's text handling
//! behaves and avoiding a new dependency.
//!
//! Motions and kills that have a line flavour — home, end, kill-to-end,
//! kill-to-start — are scoped to the cursor's line, which is readline's
//! behaviour and is indistinguishable from whole-buffer scoping on the
//! single-line text that is the common case.

use super::text::display_width;

/// Which way a motion or a kill runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Direction {
    Backward,
    Forward,
}

/// How a word motion or a word kill groups characters. One definition, shared
/// by the rename input's word delete and this field's motions, so "a word"
/// means the same thing in every text input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WordClass {
    Word,
    Separator,
}

pub(crate) fn word_class(ch: char) -> WordClass {
    if ch.is_alphanumeric() || ch == '_' {
        WordClass::Word
    } else {
        WordClass::Separator
    }
}

/// How many edits undo can walk back. Bounded so a long composing session
/// cannot grow without limit; the oldest entry is dropped when it is reached.
const UNDO_DEPTH: usize = 32;

/// Tracks whether the previous edit can absorb the next one into the same undo
/// entry. Only a run of typed characters coalesces: one undo per keystroke
/// would exhaust `UNDO_DEPTH` in a sentence and a half.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LastEdit {
    Insert,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TextField {
    text: String,
    /// Byte offset into `text`, always on a `char` boundary.
    cursor: usize,
    /// One-slot kill ring, deliberately not the system clipboard: system paste
    /// stays on bracketed paste.
    kill: String,
    undo: Vec<(String, usize)>,
    /// The store's limit, enforced as text is composed so the field cannot
    /// build a value the server will reject.
    max_chars: usize,
    last_edit: Option<LastEdit>,
}

impl TextField {
    pub(crate) fn new(max_chars: usize) -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            kill: String::new(),
            undo: Vec::new(),
            max_chars,
            last_edit: None,
        }
    }

    /// Seed the field from stored text, cursor at the end — where a reader who
    /// just opened the modal expects to continue typing.
    pub(crate) fn from_text(text: &str, max_chars: usize) -> Self {
        let text: String = text.chars().take(max_chars).collect();
        let cursor = text.len();
        Self {
            text,
            cursor,
            kill: String::new(),
            undo: Vec::new(),
            max_chars,
            last_edit: None,
        }
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// The raw insertion point. Rendering and hit-testing want
    /// [`Self::cursor_line`] and [`Self::cursor_column`] instead; this is the
    /// byte offset the tests pin the motions against.
    #[cfg(test)]
    pub(crate) fn cursor(&self) -> usize {
        self.cursor
    }

    pub(crate) fn char_count(&self) -> usize {
        self.text.chars().count()
    }

    /// Zero-based index of the line the cursor sits on.
    pub(crate) fn cursor_line(&self) -> usize {
        self.text[..self.cursor].matches('\n').count()
    }

    /// Display columns from the start of the cursor's line to the cursor.
    pub(crate) fn cursor_column(&self) -> usize {
        display_width(&self.text[self.line_start()..self.cursor])
    }

    pub(crate) fn lines(&self) -> impl Iterator<Item = &str> {
        self.text.split('\n')
    }

    pub(crate) fn line_count(&self) -> usize {
        self.text.matches('\n').count() + 1
    }

    // -- motions ---------------------------------------------------------

    pub(crate) fn move_char(&mut self, direction: Direction) {
        self.last_edit = None;
        match direction {
            Direction::Backward => {
                if let Some(offset) = self.prev_boundary(self.cursor) {
                    self.cursor = offset;
                }
            }
            Direction::Forward => {
                if let Some(offset) = self.next_boundary(self.cursor) {
                    self.cursor = offset;
                }
            }
        }
    }

    /// Skip whitespace, then the run of like-classed characters beyond it —
    /// the same shape as the word delete, so moving over a word and deleting
    /// it cover the same span.
    pub(crate) fn move_word(&mut self, direction: Direction) {
        self.last_edit = None;
        self.cursor = self.word_boundary(direction);
    }

    /// Move to the neighbouring line, keeping the same offset within it where
    /// the line is long enough. Only meaningful once the field holds a
    /// newline, but harmless before that: with one line there is nowhere to go.
    pub(crate) fn move_line(&mut self, direction: Direction) {
        self.last_edit = None;
        let column = self.text[self.line_start()..self.cursor].chars().count();
        let target_start = match direction {
            Direction::Backward => {
                let start = self.line_start();
                if start == 0 {
                    return;
                }
                self.text[..start - 1]
                    .rfind('\n')
                    .map(|idx| idx + 1)
                    .unwrap_or(0)
            }
            Direction::Forward => {
                let end = self.line_end();
                if end == self.text.len() {
                    return;
                }
                end + 1
            }
        };
        let target_end = self.text[target_start..]
            .find('\n')
            .map(|idx| target_start + idx)
            .unwrap_or(self.text.len());
        self.cursor = self.text[target_start..target_end]
            .char_indices()
            .nth(column)
            .map(|(offset, _)| target_start + offset)
            .unwrap_or(target_end);
    }

    /// Put the cursor at a display column of a line, clamping to what is
    /// actually there. This is what a click in the rendered field means.
    pub(crate) fn place_cursor(&mut self, line: usize, column: usize) {
        self.last_edit = None;
        let line = line.min(self.line_count() - 1);
        let start = self
            .text
            .match_indices('\n')
            .nth(line.wrapping_sub(1))
            .map(|(idx, _)| idx + 1)
            .filter(|_| line > 0)
            .unwrap_or(0);
        let end = self.text[start..]
            .find('\n')
            .map(|idx| start + idx)
            .unwrap_or(self.text.len());
        let mut offset = start;
        let mut width = 0usize;
        for ch in self.text[start..end].chars() {
            if width >= column {
                break;
            }
            width += display_width(&ch.to_string());
            offset += ch.len_utf8();
        }
        self.cursor = offset;
    }

    pub(crate) fn move_home(&mut self) {
        self.last_edit = None;
        self.cursor = self.line_start();
    }

    pub(crate) fn move_end(&mut self) {
        self.last_edit = None;
        self.cursor = self.line_end();
    }

    // -- edits -----------------------------------------------------------

    /// Insert one typed character. Refused whole when the buffer is already at
    /// the store's limit, so a full field simply stops accepting input rather
    /// than composing a todo the server will reject.
    /// Empty the field, cursor included. The kill ring survives: a clear is
    /// not a kill, so it does not clobber what is waiting to be yanked.
    pub(crate) fn clear(&mut self) {
        if self.text.is_empty() {
            return;
        }
        self.push_undo();
        self.text.clear();
        self.cursor = 0;
        self.last_edit = None;
    }

    pub(crate) fn insert_char(&mut self, ch: char) -> bool {
        if self.char_count() >= self.max_chars {
            return false;
        }
        // A newline ends the run: undoing a multi-line entry a line at a time
        // is more useful than undoing the whole paragraph.
        let coalesce = ch != '\n' && self.last_edit == Some(LastEdit::Insert);
        if !coalesce {
            self.push_undo();
        }
        self.text.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
        self.last_edit = (ch != '\n').then_some(LastEdit::Insert);
        true
    }

    /// Insert a run of text at the cursor — the paste path. Takes what fits
    /// rather than refusing outright: dropping an entire paste because its
    /// tail overruns the limit loses more than it protects.
    pub(crate) fn insert_str(&mut self, text: &str) -> bool {
        let room = self.max_chars.saturating_sub(self.char_count());
        let insert: String = text.chars().take(room).collect();
        if insert.is_empty() {
            return false;
        }
        self.push_undo();
        self.text.insert_str(self.cursor, &insert);
        self.cursor += insert.len();
        self.last_edit = None;
        true
    }

    pub(crate) fn delete_backward(&mut self) -> bool {
        let Some(start) = self.prev_boundary(self.cursor) else {
            return false;
        };
        self.push_undo();
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
        self.last_edit = None;
        true
    }

    pub(crate) fn delete_forward(&mut self) -> bool {
        let Some(end) = self.next_boundary(self.cursor) else {
            return false;
        };
        self.push_undo();
        self.text.replace_range(self.cursor..end, "");
        self.last_edit = None;
        true
    }

    /// Kill to the end of the cursor's line. At the end of a line that has a
    /// line below it, kills the newline instead — readline's behaviour, and
    /// the only way to rejoin two lines.
    pub(crate) fn kill_to_end(&mut self) -> bool {
        let end = self.line_end();
        if end == self.cursor {
            return self.kill_range(self.cursor, self.next_boundary(self.cursor).unwrap_or(end));
        }
        self.kill_range(self.cursor, end)
    }

    pub(crate) fn kill_to_start(&mut self) -> bool {
        self.kill_range(self.line_start(), self.cursor)
    }

    pub(crate) fn kill_word_backward(&mut self) -> bool {
        let start = self.word_boundary(Direction::Backward);
        self.kill_range(start, self.cursor)
    }

    /// Put the last kill back at the cursor. Herdr's own ring, not the system
    /// clipboard.
    pub(crate) fn yank(&mut self) -> bool {
        if self.kill.is_empty() {
            return false;
        }
        let kill = std::mem::take(&mut self.kill);
        let inserted = self.insert_str(&kill);
        self.kill = kill;
        inserted
    }

    pub(crate) fn undo(&mut self) -> bool {
        let Some((text, cursor)) = self.undo.pop() else {
            return false;
        };
        self.text = text;
        self.cursor = cursor.min(self.text.len());
        self.last_edit = None;
        true
    }

    // -- internals -------------------------------------------------------

    fn push_undo(&mut self) {
        if self.undo.len() == UNDO_DEPTH {
            self.undo.remove(0);
        }
        self.undo.push((self.text.clone(), self.cursor));
    }

    fn kill_range(&mut self, start: usize, end: usize) -> bool {
        if start >= end {
            // Nothing to take. Leaving the ring alone means a stray kill at
            // the end of a line cannot silently discard what was yanked from.
            return false;
        }
        self.push_undo();
        self.kill = self.text[start..end].to_string();
        self.text.replace_range(start..end, "");
        self.cursor = start;
        self.last_edit = None;
        true
    }

    fn prev_boundary(&self, offset: usize) -> Option<usize> {
        self.text[..offset]
            .chars()
            .next_back()
            .map(|ch| offset - ch.len_utf8())
    }

    fn next_boundary(&self, offset: usize) -> Option<usize> {
        self.text[offset..]
            .chars()
            .next()
            .map(|ch| offset + ch.len_utf8())
    }

    fn line_start(&self) -> usize {
        self.text[..self.cursor]
            .rfind('\n')
            .map(|idx| idx + 1)
            .unwrap_or(0)
    }

    fn line_end(&self) -> usize {
        self.text[self.cursor..]
            .find('\n')
            .map(|idx| self.cursor + idx)
            .unwrap_or(self.text.len())
    }

    /// Where a word motion or a word kill in `direction` lands: past any
    /// whitespace, then past the run of like-classed characters beyond it.
    fn word_boundary(&self, direction: Direction) -> usize {
        let mut offset = self.cursor;
        match direction {
            Direction::Backward => {
                while let Some(prev) = self.prev_boundary(offset) {
                    if !self.text[prev..].starts_with(char::is_whitespace) {
                        break;
                    }
                    offset = prev;
                }
                let Some(class) = self
                    .prev_boundary(offset)
                    .and_then(|prev| self.text[prev..].chars().next())
                    .map(word_class)
                else {
                    return offset;
                };
                while let Some(prev) = self.prev_boundary(offset) {
                    let Some(ch) = self.text[prev..].chars().next() else {
                        break;
                    };
                    if ch.is_whitespace() || word_class(ch) != class {
                        break;
                    }
                    offset = prev;
                }
                offset
            }
            Direction::Forward => {
                while let Some(ch) = self.text[offset..].chars().next() {
                    if !ch.is_whitespace() {
                        break;
                    }
                    offset += ch.len_utf8();
                }
                let Some(class) = self.text[offset..].chars().next().map(word_class) else {
                    return offset;
                };
                while let Some(ch) = self.text[offset..].chars().next() {
                    if ch.is_whitespace() || word_class(ch) != class {
                        break;
                    }
                    offset += ch.len_utf8();
                }
                offset
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAP: usize = 500;

    fn field(text: &str) -> TextField {
        TextField::from_text(text, CAP)
    }

    /// Move the cursor to just before the nth character of the whole buffer,
    /// counting in `char`s so the multi-byte cases below read the same as the
    /// ASCII ones. Deliberately not `move_home`, which is line-scoped.
    fn seek(field: &mut TextField, chars_from_start: usize) {
        while field.cursor() > 0 {
            field.move_char(Direction::Backward);
        }
        for _ in 0..chars_from_start {
            field.move_char(Direction::Forward);
        }
    }

    #[test]
    fn a_new_field_opens_with_the_cursor_at_the_end() {
        let field = field("héllo");
        assert_eq!(field.cursor(), field.text().len());
        assert_eq!(field.cursor_column(), 5);
    }

    #[test]
    fn typing_inserts_at_the_cursor_rather_than_appending() {
        let mut field = field("world");
        field.move_home();
        for ch in "hello ".chars() {
            assert!(field.insert_char(ch));
        }
        assert_eq!(field.text(), "hello world");
        assert_eq!(field.cursor(), 6);
    }

    #[test]
    fn character_motion_steps_over_multi_byte_characters() {
        // Three chars of three, four, and two bytes: a byte-stepping cursor
        // would land inside one of them and panic on the next slice.
        let mut field = field("日🙂é");
        field.move_home();
        assert_eq!(field.cursor(), 0);
        field.move_char(Direction::Forward);
        assert_eq!(field.cursor(), 3);
        field.move_char(Direction::Forward);
        assert_eq!(field.cursor(), 7);
        field.move_char(Direction::Forward);
        assert_eq!(field.cursor(), 9);
        // Already at the end: the motion is a no-op, not an overrun.
        field.move_char(Direction::Forward);
        assert_eq!(field.cursor(), 9);
        field.move_char(Direction::Backward);
        assert_eq!(field.cursor(), 7);
    }

    #[test]
    fn word_motion_runs_over_multi_byte_words() {
        let mut field = field("café 日本語 done");
        field.move_home();
        field.move_word(Direction::Forward);
        assert_eq!(&field.text()[..field.cursor()], "café");
        field.move_word(Direction::Forward);
        assert_eq!(&field.text()[..field.cursor()], "café 日本語");
        field.move_word(Direction::Backward);
        assert_eq!(&field.text()[..field.cursor()], "café ");
    }

    #[test]
    fn home_and_end_are_scoped_to_the_cursors_line() {
        let mut field = field("first\nsecond");
        field.move_home();
        assert_eq!(field.cursor(), 6, "the cursor started on the second line");
        field.move_end();
        assert_eq!(field.cursor(), field.text().len());
        // Back onto the first line, where home/end must not cross the newline.
        seek(&mut field, 2);
        field.move_home();
        assert_eq!(field.cursor(), 0);
        field.move_end();
        assert_eq!(field.cursor(), 5);
    }

    #[test]
    fn line_motion_keeps_its_column_and_stops_at_the_ends() {
        let mut field = field("first\nsh\nthird");
        seek(&mut field, 3);
        field.move_line(Direction::Forward);
        assert_eq!(field.cursor_line(), 1);
        assert_eq!(field.cursor(), 8, "the short line clamps to its end");
        field.move_line(Direction::Forward);
        assert_eq!(field.cursor_line(), 2);
        assert_eq!(field.cursor_column(), 2, "column resumes from where it was");
        field.move_line(Direction::Forward);
        assert_eq!(field.cursor_line(), 2, "no line below to move to");
        field.move_line(Direction::Backward);
        field.move_line(Direction::Backward);
        assert_eq!(field.cursor_line(), 0);
        field.move_line(Direction::Backward);
        assert_eq!(field.cursor_line(), 0, "no line above either");
    }

    #[test]
    fn delete_forward_and_backward_act_relative_to_the_cursor() {
        let mut field = field("日🙂é");
        seek(&mut field, 1);
        assert!(field.delete_forward());
        assert_eq!(field.text(), "日é");
        assert_eq!(field.cursor(), 3, "delete-forward leaves the cursor put");
        assert!(field.delete_backward());
        assert_eq!(field.text(), "é");
        assert_eq!(field.cursor(), 0);
        assert!(!field.delete_backward(), "nothing before the cursor");
    }

    #[test]
    fn kill_to_start_empties_a_single_line_field_and_yank_puts_it_back() {
        let mut field = field("réstore me");
        assert!(field.kill_to_start());
        assert!(field.text().is_empty());
        assert!(field.yank());
        assert_eq!(field.text(), "réstore me");
    }

    #[test]
    fn yank_lands_at_the_cursor_not_at_the_end() {
        let mut field = field("alpha beta");
        assert!(field.kill_word_backward());
        assert_eq!(field.text(), "alpha ");
        field.move_home();
        assert!(field.yank());
        assert_eq!(field.text(), "betaalpha ");
        assert_eq!(field.cursor(), 4, "the cursor follows the yanked text");
    }

    #[test]
    fn kill_to_end_stops_at_the_line_and_then_joins_it() {
        let mut field = field("first\nsecond");
        seek(&mut field, 2);
        assert!(field.kill_to_end());
        assert_eq!(field.text(), "fi\nsecond");
        // At the end of the line now, so the next kill takes the newline.
        assert!(field.kill_to_end());
        assert_eq!(field.text(), "fisecond");
        // The kill ring holds one entry: the newline replaced "rst".
        field.move_end();
        assert!(field.yank());
        assert_eq!(field.text(), "fisecond\n");
    }

    #[test]
    fn a_kill_that_takes_nothing_leaves_the_ring_alone() {
        let mut field = field("keep me");
        assert!(field.kill_to_start());
        assert!(!field.kill_to_start(), "already at the line start");
        assert!(!field.kill_to_end(), "nothing after the cursor either");
        assert!(field.yank());
        assert_eq!(field.text(), "keep me", "the earlier kill survived");
    }

    #[test]
    fn kill_word_backward_takes_whitespace_then_the_like_classed_run() {
        let mut field = field("path/to/file   ");
        assert!(field.kill_word_backward());
        assert_eq!(field.text(), "path/to/", "the separators are their own run");
        assert!(field.kill_word_backward());
        assert_eq!(field.text(), "path/to");
    }

    #[test]
    fn undo_restores_the_text_before_a_kill() {
        let mut field = field("undo the kill");
        assert!(field.kill_to_start());
        assert!(field.text().is_empty());
        assert!(field.undo());
        assert_eq!(field.text(), "undo the kill");
        assert_eq!(field.cursor(), field.text().len());
    }

    #[test]
    fn undo_takes_a_run_of_typing_back_in_one_step() {
        let mut field = TextField::new(CAP);
        for ch in "hello".chars() {
            assert!(field.insert_char(ch));
        }
        assert!(field.undo());
        assert!(
            field.text().is_empty(),
            "an uninterrupted run of typing is one undo entry"
        );
        // A motion breaks the run, so the two halves undo separately.
        for ch in "ab".chars() {
            field.insert_char(ch);
        }
        field.move_home();
        for ch in "cd".chars() {
            field.insert_char(ch);
        }
        assert_eq!(field.text(), "cdab");
        assert!(field.undo());
        assert_eq!(field.text(), "ab");
        assert!(field.undo());
        assert!(field.text().is_empty());
        assert!(!field.undo(), "nothing left to undo");
    }

    #[test]
    fn the_undo_stack_is_bounded() {
        let mut field = TextField::new(CAP);
        // Each kill is its own entry, so this pushes well past the cap.
        for idx in 0..UNDO_DEPTH * 2 {
            field.insert_str(&format!("{idx} "));
            field.kill_word_backward();
        }
        assert_eq!(field.undo.len(), UNDO_DEPTH);
    }

    #[test]
    fn the_limit_refuses_a_typed_character_and_leaves_the_buffer_intact() {
        let mut field = TextField::from_text("日本語", 3);
        assert_eq!(field.char_count(), 3);
        assert!(!field.insert_char('x'));
        assert_eq!(field.text(), "日本語");
        assert!(!field.undo(), "a refused insert records no undo entry");

        // Room for one more: the character lands, the next does not.
        let mut field = TextField::from_text("日本", 3);
        assert!(field.insert_char('語'));
        assert!(!field.insert_char('x'));
        assert_eq!(field.text(), "日本語");
    }

    #[test]
    fn a_paste_takes_what_fits_rather_than_being_dropped_whole() {
        let mut field = TextField::from_text("ab", 5);
        assert!(field.insert_str("cdefgh"));
        assert_eq!(field.text(), "abcde");
        assert!(!field.insert_str("more"), "no room left at all");
        assert_eq!(field.text(), "abcde");
    }

    #[test]
    fn newlines_are_ordinary_text_to_the_field() {
        let mut field = TextField::new(CAP);
        for ch in "one".chars() {
            field.insert_char(ch);
        }
        assert!(field.insert_char('\n'));
        for ch in "two".chars() {
            field.insert_char(ch);
        }
        assert_eq!(field.text(), "one\ntwo");
        assert_eq!(field.line_count(), 2);
        assert_eq!(field.cursor_line(), 1);
        assert_eq!(field.cursor_column(), 3);
        assert_eq!(field.lines().collect::<Vec<_>>(), vec!["one", "two"]);
        // The limit counts newlines like any other character.
        assert_eq!(field.char_count(), 7);
    }
}
