const CLAUDE_ACTIVITY_GLYPHS: &str = "·✢✳✶✻✽◐◓◑◒";

pub(crate) fn stripped_terminal_title(title: &str) -> Option<String> {
    let title = crate::platform::terminal_title_for_presentation(title).trim();
    if title.is_empty() {
        return None;
    }

    let mut chars = title.char_indices();
    let (_, first) = chars.next()?;
    let after_first = &title[first.len_utf8()..];
    let recognized =
        matches!(first, '\u{2800}'..='\u{28ff}') || CLAUDE_ACTIVITY_GLYPHS.contains(first);
    let stripped = if recognized
        && (after_first.is_empty() || after_first.chars().next().is_some_and(char::is_whitespace))
    {
        after_first.trim()
    } else {
        title
    };

    (!stripped.is_empty()).then(|| stripped.to_string())
}

/// Whether a stripped terminal title can stand in as an agent's name (fork
/// issue 130): one word of 1-64 characters from `[A-Za-z0-9._-]`, starting
/// with a letter or digit, that is not an agent kind or a shell name. Strict
/// on purpose: "Claude Code", a task sentence, a path or a prompt is no name,
/// and a wrong name is worse than none because tools act on it.
pub(crate) fn title_is_agent_name(title: &str) -> bool {
    title.len() <= 64
        && title
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphanumeric())
        && title
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && crate::detect::identify_agent(title).is_none()
        && !crate::platform::is_pane_shell_process_name(title)
}

#[cfg(test)]
mod tests {
    use super::stripped_terminal_title;

    #[test]
    fn only_name_like_titles_can_be_agent_names() {
        use super::title_is_agent_name;
        for name in [
            "jev-astra",
            "direction-builder",
            "bmlab-work-1",
            "m5_nexus.sbom",
            "A1",
        ] {
            assert!(title_is_agent_name(name), "{name}");
        }
        let too_long = "a".repeat(65);
        for not_a_name in [
            "",
            "Claude Code",
            "fix the flaky test",
            "claude",
            "Codex",
            "zsh",
            "~/src/herdr",
            "user@host",
            "-dash-first",
            "\u{2733}",
            too_long.as_str(),
        ] {
            assert!(!title_is_agent_name(not_a_name), "{not_a_name:?}");
        }
    }

    #[test]
    fn strips_one_recognized_leading_activity_glyph() {
        for title in [
            "⠋ task",
            "✳ task",
            "  ⠙   task  ",
            "✢ task",
            "✻ task",
            "◐ task",
            "◓ task",
            "◑ task",
            "◒ task",
        ] {
            assert_eq!(stripped_terminal_title(title).as_deref(), Some("task"));
        }
        assert_eq!(
            stripped_terminal_title("⠋ ⠙ task").as_deref(),
            Some("⠙ task")
        );
    }

    #[test]
    fn preserves_unrecognized_or_unbounded_symbols() {
        for (title, expected) in [
            ("★task", "★task"),
            ("★ production", "★ production"),
            ("✨ task", "✨ task"),
            ("☼ status", "☼ status"),
            ("@ task", "@ task"),
            ("task ⠋ detail", "task ⠋ detail"),
            ("[prod] task", "[prod] task"),
        ] {
            assert_eq!(stripped_terminal_title(title).as_deref(), Some(expected));
        }
    }

    #[test]
    fn preserves_unicode_text_and_elides_empty_results() {
        assert_eq!(
            stripped_terminal_title(" ⠋ 修复🙂标题 ").as_deref(),
            Some("修复🙂标题")
        );
        assert_eq!(stripped_terminal_title("  "), None);
        assert_eq!(stripped_terminal_title("⠋   "), None);
    }

    #[cfg(windows)]
    #[test]
    fn strips_one_windows_elevation_decoration_before_activity_glyph() {
        assert_eq!(
            stripped_terminal_title("Administrator:   ⠋ task").as_deref(),
            Some("task")
        );
        assert_eq!(
            stripped_terminal_title("Administrator: Administrator: task").as_deref(),
            Some("Administrator: task")
        );
        assert_eq!(stripped_terminal_title("Administrator: "), None);
    }
}
