//! Text from another machine, made safe to print on this terminal.
//!
//! A far herdr, its server or the programs it runs can put terminal
//! controls in what they say: an OSC 52 that sets the clipboard, an OSC 0
//! that retitles the window, a CSI that moves the cursor over earlier
//! lines. A human-readable line built from far text shows each control as
//! a visible escape instead (`\x1b`, `\x7f`, `\u{9b}`), so it reaches the
//! terminal as plain characters. JSON output needs none of this: serde
//! escapes every control already.

use std::borrow::Cow;

/// `text` for one line: every C0 control but tab (newline and carriage
/// return included), DEL, every C1 control, and the bidirectional and line
/// separator marks that reorder or split what is shown, escaped visibly.
pub(crate) fn visible_line(text: &str) -> Cow<'_, str> {
    let hidden = |ch: char| ch != '\t' && is_hidden(ch);
    if !text.chars().any(hidden) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + 8);
    for ch in text.chars() {
        if !hidden(ch) {
            out.push(ch);
        } else if (ch as u32) < 0x80 {
            out.push_str(&format!("\\x{:02x}", ch as u32));
        } else {
            out.push_str(&format!("\\u{{{:x}}}", ch as u32));
        }
    }
    Cow::Owned(out)
}

/// C0, DEL and C1 (`char::is_control`), and the marks that reorder or
/// break a line on screen: the bidirectional embeddings, overrides and
/// isolates, and the line and paragraph separators.
fn is_hidden(ch: char) -> bool {
    ch.is_control()
        || matches!(ch, '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// security review 2026-11-23 M1: OSC 52, OSC 0 and CSI from a far side
    /// print as visible text, with no ESC, BEL or ST left.
    #[test]
    fn terminal_controls_print_as_visible_escapes() {
        let osc52 = "\x1b]52;c;aGVsbG8=\x07";
        let osc0 = "\x1b]0;pwned\x1b\\";
        let csi = "\x1b[2K\x1b[1A\r";
        let c1 = "\u{9b}31m\u{9d}0;t\u{9c}";
        let far = format!("a{osc52}b{osc0}c{csi}d{c1}\x7f\n\te");
        let line = visible_line(&far);
        assert_eq!(
            line,
            "a\\x1b]52;c;aGVsbG8=\\x07b\\x1b]0;pwned\\x1b\\c\\x1b[2K\\x1b[1A\\x0dd\\u{9b}31m\\u{9d}0;t\\u{9c}\\x7f\\x0a\te"
        );
        assert!(!line.chars().any(|ch| ch.is_control() && ch != '\t'));
        assert_eq!(visible_line("\u{202e}txt.exe"), "\\u{202e}txt.exe");
    }

    #[test]
    fn plain_text_is_borrowed_unchanged() {
        for plain in ["", "herdr 0.9.3 · ok", "tab\there", "é ü 漢字"] {
            assert!(matches!(visible_line(plain), Cow::Borrowed(text) if text == plain));
        }
    }
}
