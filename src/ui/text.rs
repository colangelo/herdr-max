use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub(crate) fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

pub(crate) fn truncate_end(text: &str, max_width: usize) -> String {
    if display_width(text) <= max_width {
        return text.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    if max_width == 1 {
        return "…".to_string();
    }

    let prefix = take_prefix_width(text, max_width.saturating_sub(1));
    format!("{prefix}…")
}

pub(crate) fn truncate_start(text: &str, max_width: usize) -> String {
    if display_width(text) <= max_width {
        return text.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    if max_width == 1 {
        return "…".to_string();
    }

    let suffix = take_suffix_width(text, max_width.saturating_sub(1));
    format!("…{suffix}")
}

fn take_suffix_width(text: &str, max_width: usize) -> String {
    let mut output = Vec::new();
    let mut width = 0usize;
    for ch in text.chars().rev() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_width > max_width {
            break;
        }
        output.push(ch);
        width += ch_width;
    }
    output.into_iter().rev().collect()
}

fn take_prefix_width(text: &str, max_width: usize) -> String {
    let mut output = String::new();
    let mut width = 0usize;
    for ch in text.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_width > max_width {
            break;
        }
        output.push(ch);
        width += ch_width;
    }
    output
}

pub(crate) fn middle_elide(text: &str, max_width: usize) -> String {
    if display_width(text) <= max_width {
        return text.to_string();
    }
    if max_width <= 1 {
        return "…".to_string();
    }

    let content_width = max_width.saturating_sub(1);
    let left_width = content_width / 2;
    let right_width = content_width.saturating_sub(left_width);
    let prefix = take_prefix_width(text, left_width);
    let suffix = take_suffix_width(text, right_width);
    format!("{prefix}…{suffix}")
}

/// Split `text` into pieces of at most `max_bytes` bytes without cutting a
/// UTF-8 character. A character wider than `max_bytes` becomes a piece of its
/// own, so every piece is non-empty and valid UTF-8. Empty text yields one
/// empty piece.
pub(crate) fn split_utf8_chunks(text: &str, max_bytes: usize) -> Vec<&str> {
    let max_bytes = max_bytes.max(1);
    let mut pieces = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + max_bytes).min(text.len());
        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            end = start + text[start..].chars().next().map_or(1, char::len_utf8);
        }
        pieces.push(&text[start..end]);
        start = end;
    }
    if pieces.is_empty() {
        pieces.push("");
    }
    pieces
}

// Restored fork items (v0.9.3 sync): re-home next to their kin later.
pub(crate) fn relative_time_label(now_unix: u64, then_unix: u64) -> String {
    let seconds = now_unix.saturating_sub(then_unix);
    if seconds < 10 {
        "now".to_string()
    } else if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 60 * 60 {
        format!("{}m", seconds / 60)
    } else if seconds < 24 * 60 * 60 {
        format!("{}h", seconds / (60 * 60))
    } else {
        format!("{}d", seconds / (24 * 60 * 60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_end_uses_display_width() {
        let text = truncate_end("提交 herdr 的反馈", 16);

        assert_eq!(text, "提交 herdr 的反…");
        assert!(display_width(&text) <= 16);
    }
    #[test]
    fn truncate_start_keeps_the_display_width_aware_suffix() {
        assert_eq!(truncate_start("asks L:20 A:0", 4), "…A:0");
        assert_eq!(truncate_start("界界末尾", 5), "…末尾");
        assert_eq!(truncate_start("界", 0), "");
        assert_eq!(truncate_start("界", 1), "…");
        assert_eq!(truncate_start("界", 2), "界");
    }

    #[test]
    fn split_utf8_chunks_respects_byte_limit_and_round_trips() {
        let text = "a".repeat(1000);
        let pieces = split_utf8_chunks(&text, 300);
        assert_eq!(
            pieces.iter().map(|piece| piece.len()).collect::<Vec<_>>(),
            vec![300, 300, 300, 100]
        );
        assert_eq!(pieces.concat(), text);
    }

    #[test]
    fn split_utf8_chunks_never_cuts_a_multibyte_character() {
        // Emoji are 4 bytes (one is a ZWJ sequence of several scalars), CJK 3,
        // accented Latin 2; limits chosen to land mid-character repeatedly.
        let text = "héllo 🦀 世界 👩‍💻 日本語テキスト ✓ ".repeat(40);
        for max_bytes in [1, 2, 3, 4, 5, 7, 10, 64, 300] {
            let pieces = split_utf8_chunks(&text, max_bytes);
            assert_eq!(pieces.concat(), text, "limit {max_bytes}");
            for piece in &pieces {
                assert!(!piece.is_empty(), "limit {max_bytes}");
                let single_char = piece.chars().count() == 1;
                assert!(
                    piece.len() <= max_bytes || single_char,
                    "limit {max_bytes}: piece {piece:?} is {} bytes",
                    piece.len()
                );
            }
        }
    }

    #[test]
    fn split_utf8_chunks_keeps_short_and_empty_text_whole() {
        assert_eq!(split_utf8_chunks("", 300), vec![""]);
        assert_eq!(split_utf8_chunks("世界", 300), vec!["世界"]);
        assert_eq!(split_utf8_chunks("世界", 6), vec!["世界"]);
        assert_eq!(split_utf8_chunks("世界", 5), vec!["世", "界"]);
    }

    #[test]
    fn split_utf8_chunks_gives_a_wide_character_its_own_piece() {
        let text = "héllo wörld ".repeat(50);
        let pieces = split_utf8_chunks(&text, 7);
        assert!(pieces
            .iter()
            .all(|piece| !piece.is_empty() && piece.len() <= 7));
        assert_eq!(pieces.concat(), text);
        assert_eq!(
            split_utf8_chunks("日本語", 2),
            ["日", "本", "語"],
            "a character wider than the limit is a piece of its own, and the last one leaves no empty tail"
        );
        assert_eq!(split_utf8_chunks("", 300), [""]);
    }
}
