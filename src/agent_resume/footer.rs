//! The live model and effort of a Claude pane, read from the status line Claude
//! Code draws at the bottom of the pane (`| Sonnet 5.5 | ⚡medium |`), for a
//! restore command whose hook record is older than what the pane shows (fork
//! issue 144). Read from the detection text (the bottom of the buffer), never
//! from the scrollable viewport.

/// Footer model names and the ids `claude --model` takes; the same table the
/// idle-maintenance footer parser uses. An unknown name keeps the record.
const MODELS: &[(&str, &str)] = &[
    ("Opus 5.5", "claude-opus-5-5"),
    ("Opus 5", "claude-opus-5"),
    ("Sonnet 5.5", "claude-sonnet-5-5"),
    ("Sonnet 5", "claude-sonnet-5"),
    ("Haiku 4.5", "claude-haiku-4-5-20251001"),
    ("Fable 5.1", "claude-fable-5-1"),
];
const EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
/// Rows from the bottom that can hold the status line.
const FOOTER_ROWS: usize = 8;

/// The `(model id, effort)` the footer shows, or `None` when no footer can be
/// read or its model is not one we can map.
pub fn claude_footer_profile(detection_text: &str) -> Option<(String, String)> {
    detection_text
        .lines()
        .rev()
        .filter(|line| !line.trim().is_empty())
        .take(FOOTER_ROWS)
        .find_map(profile_of_line)
}

fn profile_of_line(line: &str) -> Option<(String, String)> {
    let bolt = line.find('⚡')?;
    let effort: String = line[bolt + '⚡'.len_utf8()..]
        .chars()
        .take_while(char::is_ascii_lowercase)
        .collect();
    if !EFFORTS.contains(&effort.as_str()) {
        return None;
    }
    let before = line[..bolt].trim_end().strip_suffix('|')?.trim_end();
    let name = before.rsplit('|').next()?;
    // A marker (the proxy's `⇄`) may precede the name.
    let name = name
        .trim()
        .trim_start_matches(|c: char| !c.is_alphanumeric());
    let (_, id) = MODELS.iter().find(|(known, _)| *known == name)?;
    Some(((*id).to_string(), effort))
}

/// `argv` with the value after `--model` and `--effort` set to the footer's,
/// added when the command has none.
pub fn with_footer_profile(argv: &[String], model: &str, effort: &str) -> Vec<String> {
    let mut argv = argv.to_vec();
    for (flag, value) in [("--model", model), ("--effort", effort)] {
        match argv.iter().position(|arg| arg == flag) {
            Some(at) if at + 1 < argv.len() => argv[at + 1] = value.to_string(),
            _ => {
                argv.push(flag.to_string());
                argv.push(value.to_string());
            }
        }
    }
    argv
}

/// A Claude resume command with the model and effort its footer shows now, the
/// record's kept when no footer can be read or its model is unknown.
pub fn argv_with_live_footer(agent: &str, argv: &[String], detection_text: &str) -> Vec<String> {
    match claude_footer_profile(detection_text) {
        Some((model, effort)) if agent == "claude" => with_footer_profile(argv, &model, &effort),
        _ => argv.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_footer_gives_a_model_id_and_an_effort() {
        let screen = "answer\n─────\n❯\n─────\n  ~/dev/herdr | Sonnet 5.5 | ⚡medium | ctx 12%\n";
        assert_eq!(
            claude_footer_profile(screen),
            Some(("claude-sonnet-5-5".into(), "medium".into()))
        );
        let proxy = "  x | ⇄ Opus 5.5 | ⚡xhigh |\n";
        assert_eq!(
            claude_footer_profile(proxy),
            Some(("claude-opus-5-5".into(), "xhigh".into()))
        );
        assert_eq!(
            claude_footer_profile("| Fable 5.1 | ⚡max"),
            Some(("claude-fable-5-1".into(), "max".into()))
        );
    }

    #[test]
    fn no_footer_or_an_unknown_model_or_effort_gives_nothing() {
        assert_eq!(claude_footer_profile(""), None);
        assert_eq!(claude_footer_profile("just output\n❯ \n"), None);
        assert_eq!(claude_footer_profile("| Mystery 9 | ⚡medium |"), None);
        assert_eq!(claude_footer_profile("| Sonnet 5.5 | ⚡turbo |"), None);
        // An old footer line far above the bottom rows is not the live one.
        let mut text = String::from("| Sonnet 5.5 | ⚡low |\n");
        text.push_str(&"output\n".repeat(FOOTER_ROWS));
        assert_eq!(claude_footer_profile(&text), None);
    }

    /// Fork issue 144: the live case, a hook record of fable/xhigh under a
    /// footer that says Sonnet 5.5 medium.
    #[test]
    fn a_stale_record_gives_way_to_a_readable_footer_and_stays_without_one() {
        let record: Vec<String> = [
            "claude",
            "--resume",
            "s",
            "--model",
            "claude-fable-5-1",
            "--effort",
            "xhigh",
        ]
        .map(String::from)
        .to_vec();
        let live = "output\n  ~/x | Sonnet 5.5 | ⚡medium |\n";
        assert_eq!(
            argv_with_live_footer("claude", &record, live),
            [
                "claude",
                "--resume",
                "s",
                "--model",
                "claude-sonnet-5-5",
                "--effort",
                "medium"
            ]
        );
        assert_eq!(
            argv_with_live_footer("claude", &record, "no footer here\n"),
            record
        );
        assert_eq!(
            argv_with_live_footer("claude", &record, "| Unknown 1 | ⚡medium |"),
            record,
            "an unknown model name keeps the record"
        );
        assert_eq!(
            argv_with_live_footer("codex", &record, live),
            record,
            "only Claude's command is rewritten"
        );
    }

    #[test]
    fn the_footer_values_replace_the_records_and_are_added_when_missing() {
        let record: Vec<String> = [
            "claude",
            "--resume",
            "s",
            "--model",
            "claude-fable-5-1",
            "--effort",
            "xhigh",
            "--permission-mode",
            "plan",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            with_footer_profile(&record, "claude-sonnet-5-5", "medium"),
            [
                "claude",
                "--resume",
                "s",
                "--model",
                "claude-sonnet-5-5",
                "--effort",
                "medium",
                "--permission-mode",
                "plan"
            ]
        );
        let bare: Vec<String> = ["claude", "--resume", "s"].map(String::from).to_vec();
        assert_eq!(
            with_footer_profile(&bare, "claude-sonnet-5-5", "low"),
            [
                "claude",
                "--resume",
                "s",
                "--model",
                "claude-sonnet-5-5",
                "--effort",
                "low"
            ]
        );
    }
}
