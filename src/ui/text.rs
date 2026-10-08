use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub(crate) fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// Columns one character occupies. Zero-width and unassigned characters count
/// as nothing, matching how [`display_width`] measures a whole string.
pub(crate) fn char_display_width(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0)
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

/// Like [`truncate_end`], but cuts the beginning: `…text`, so the end of the
/// text stays visible.
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

/// Compact "time ago" label for notification timestamps: "now", "45s",
/// "12m", "3h", "9d". Saturates safely when the clock moves backwards.
/// `1 pane`, `3 panes`.
pub(crate) fn pane_count(count: usize) -> String {
    if count == 1 {
        "1 pane".to_string()
    } else {
        format!("{count} panes")
    }
}

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
    fn truncate_start_keeps_the_end_and_uses_display_width() {
        assert_eq!(truncate_start("asks L:20 A:0", 13), "asks L:20 A:0");
        assert_eq!(truncate_start("asks L:20 A:0", 9), "…L:20 A:0");
        assert_eq!(truncate_start("asks L:20 A:0", 4), "…A:0");
        assert_eq!(truncate_start("asks L:20 A:0", 1), "…");
        assert_eq!(truncate_start("asks L:20 A:0", 0), "");
        // Measured in cells, not characters.
        let text = truncate_start("提交 herdr 的反馈", 8);
        assert_eq!(text, "… 的反馈");
        assert!(display_width(&text) <= 8);
    }

    #[test]
    fn relative_time_labels_scale_with_age() {
        assert_eq!(relative_time_label(100, 95), "now");
        assert_eq!(relative_time_label(100, 55), "45s");
        assert_eq!(relative_time_label(1000, 100), "15m");
        assert_eq!(relative_time_label(10_000, 100), "2h");
        assert_eq!(relative_time_label(1_000_000, 100), "11d");
        // Clock moved backwards: saturates to "now" instead of underflowing.
        assert_eq!(relative_time_label(50, 100), "now");
    }
}
