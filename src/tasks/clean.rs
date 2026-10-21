//! Text from the buckets other tools fill: forge titles and ask rows can be
//! written by anyone who can open an issue, so it is cleaned before the app
//! keeps or shows it.

/// `text` without control characters (ESC and the C1 range included) and
/// without the marks that reorder text, cut to `max` characters. The second
/// value is whether it was cut. A newline or a tab becomes a space.
pub(crate) fn clean(text: &str, max: usize) -> (String, bool) {
    let mut out = String::with_capacity(text.len().min(max * 4));
    let mut count = 0;
    for c in text.chars() {
        let c = match c {
            '\n' | '\r' | '\t' => ' ',
            c if c.is_control() => continue,
            '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}' => continue,
            c => c,
        };
        if count == max {
            return (out, true);
        }
        out.push(c);
        count += 1;
    }
    (out, false)
}

pub(crate) fn clean_opt(text: &mut Option<String>, max: usize) {
    if let Some(value) = text.as_mut() {
        *value = clean(value, max).0;
    }
}

#[cfg(test)]
mod tests {
    use super::clean;

    #[test]
    fn control_characters_and_reordering_marks_are_removed() {
        assert_eq!(
            clean("a\u{1b}[31mred\u{7}\u{202e}b\nc", 100),
            ("a[31mredb c".to_owned(), false)
        );
    }

    #[test]
    fn long_text_is_cut_on_a_character_boundary() {
        assert_eq!(clean("caff\u{e8}!", 5), ("caff\u{e8}".to_owned(), true));
        assert_eq!(clean("caff\u{e8}", 5), ("caff\u{e8}".to_owned(), false));
    }
}
