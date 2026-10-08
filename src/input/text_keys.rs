//! One editing keymap for every overlay text input.
//!
//! The buffer lives in [`TextField`] and the keymap lives here, so this is only
//! ever a lookup table. Every input that used to carry its own append-only
//! editing — the rename modals, worktree create, the navigator's search box,
//! the keybind-help search box — reaches the same motions, kills and word
//! boundaries through it.
//!
//! The chords are readline's, with one substitution forced by how terminals
//! report keys: undo is offered on `ctrl+_`, `ctrl+-`, and `ctrl+/` at once.
//! Herdr's own parser turns the legacy `0x1F` byte into `ctrl+-`
//! (`src/input/parse.rs`), so that arm is what makes undo reachable without the
//! enhanced keyboard protocol; the other two are how the same physical chords
//! arrive once a terminal reports them individually.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::ui::text_field::{Direction as Motion, TextField};

/// Whether the field is one line or many.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    /// Enter and the vertical arrows belong to the overlay: Enter commits and
    /// the arrows move a list or a scroll.
    SingleLine,
    /// Enter inserts a newline and the vertical arrows move between lines.
    Multiline,
}

/// Apply the shared editing set. Returns whether the key was one of it, so a
/// caller can fall through to its own bindings.
///
/// Callers put their own arms first: a search box's `ctrl+u` is this field's
/// kill-to-start, and an overlay that wants a letter keeps it.
pub(crate) fn apply_text_key(text: &mut TextField, key: KeyEvent, shape: Shape) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let multiline = shape == Shape::Multiline;
    match key.code {
        // -- motions --
        KeyCode::Char('a') if ctrl => text.move_home(),
        KeyCode::Char('e') if ctrl => text.move_end(),
        KeyCode::Char('b') if ctrl => text.move_char(Motion::Backward),
        KeyCode::Char('f') if ctrl => text.move_char(Motion::Forward),
        KeyCode::Char('b') if alt => text.move_word(Motion::Backward),
        KeyCode::Char('f') if alt => text.move_word(Motion::Forward),
        KeyCode::Left if ctrl || alt => text.move_word(Motion::Backward),
        KeyCode::Right if ctrl || alt => text.move_word(Motion::Forward),
        KeyCode::Left => text.move_char(Motion::Backward),
        KeyCode::Right => text.move_char(Motion::Forward),
        KeyCode::Up if multiline => text.move_line(Motion::Backward),
        KeyCode::Down if multiline => text.move_line(Motion::Forward),
        KeyCode::Home => text.move_home(),
        KeyCode::End => text.move_end(),

        // -- kills and deletes --
        KeyCode::Char('k') if ctrl => {
            text.kill_to_end();
        }
        KeyCode::Char('u') if ctrl => {
            text.kill_to_start();
        }
        KeyCode::Char('w') if ctrl => {
            text.kill_word_backward();
        }
        KeyCode::Char('y') if ctrl => {
            text.yank();
        }
        // Undo, on every chord a terminal might deliver 0x1F or its
        // enhanced-protocol equivalent under.
        KeyCode::Char('_' | '-' | '/') if ctrl => {
            text.undo();
        }
        KeyCode::Char('d') if ctrl => {
            text.delete_forward();
        }
        KeyCode::Delete => {
            text.delete_forward();
        }
        // cmd+backspace is "delete to the start of the line" on macOS, which
        // is exactly kill-to-start — and on the single-line text that is the
        // common case, exactly the whole-buffer clear it used to be.
        KeyCode::Backspace if key.modifiers.contains(KeyModifiers::SUPER) => {
            text.kill_to_start();
        }
        KeyCode::Backspace if ctrl || alt => {
            text.kill_word_backward();
        }
        // `ctrl+h` is readline's backward-delete-char, and is what many
        // terminals send for Backspace itself.
        KeyCode::Char('h') if ctrl => {
            text.delete_backward();
        }
        KeyCode::Backspace => {
            text.delete_backward();
        }

        // -- insertion --
        KeyCode::Enter if multiline => {
            text.insert_char('\n');
        }
        KeyCode::Char(c) if key.modifiers.difference(KeyModifiers::SHIFT).is_empty() => {
            text.insert_char(c);
        }
        _ => return false,
    }
    true
}
