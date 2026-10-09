//! Background work a Claude pane reports on screen, counted for the sidebar's
//! background mark (fork issue 172).
//!
//! The count is a presentation fact: it never decides the pane's status.
//! Claude lists what it keeps running in the footer under the prompt box
//! (`⏸ manual mode on · 3 shells, 1 monitor · ← 1 agent`) and, while a turn
//! waits on them, in a summary line (`✳ Waiting for 3 background agents to
//! finish`, `✻ Brewed for 4s · 2 MCP tasks still running`).

use super::Agent;

/// How many non-empty screen lines, from the bottom, are searched.
const TAIL_LINES: usize = 12;

/// Segment words in the footer that are background work. The standing
/// `← N agent` entry is the session's own agent, not work it launched.
const FOOTER_KINDS: [&str; 6] = ["shell", "monitor", "workflow", "task", "mcp", "teammate"];

/// Glyphs Claude puts before its activity summary lines.
const SUMMARY_GLYPHS: [char; 6] = ['*', '·', '✢', '✳', '✶', '✻'];

/// The number of background items shown on a Claude pane's screen, 0 for any
/// other agent. Saturates at 255.
pub fn background_count(agent: Option<Agent>, content: &str) -> u8 {
    if agent != Some(Agent::Claude) {
        return 0;
    }
    let total = content
        .lines()
        .rev()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(TAIL_LINES)
        .map(|line| {
            footer_total(line)
                .max(waiting_total(line))
                .max(mcp_total(line))
        })
        .max()
        .unwrap_or(0);
    u8::try_from(total).unwrap_or(u8::MAX)
}

fn leading_number(text: &str) -> Option<u32> {
    text.split_whitespace().next()?.parse().ok()
}

/// `⏸ manual mode on · 3 shells, 1 monitor · ← 1 agent` -> 4.
fn footer_total(line: &str) -> u32 {
    if !(line.starts_with('⏸') || line.starts_with('⏵')) {
        return 0;
    }
    let mut total = 0u32;
    for segment in line.split('·').skip(1) {
        for part in segment.split(',') {
            let part = part.trim();
            if part.starts_with('←') {
                continue;
            }
            let mut words = part.split_whitespace();
            let (Some(count), Some(kind)) = (words.next(), words.next()) else {
                continue;
            };
            let Ok(count) = count.parse::<u32>() else {
                continue;
            };
            let kind = kind.trim_end_matches('s').to_ascii_lowercase();
            if FOOTER_KINDS.contains(&kind.as_str()) {
                total = total.saturating_add(count);
            }
        }
    }
    total
}

/// `✳ Waiting for 3 background agents to finish` -> 3.
fn waiting_total(line: &str) -> u32 {
    let Some(rest) = line
        .strip_prefix(SUMMARY_GLYPHS)
        .map(str::trim_start)
        .and_then(|rest| rest.strip_prefix("Waiting for "))
    else {
        return 0;
    };
    match leading_number(rest) {
        Some(count) if rest.contains("background agent") => count,
        _ => 0,
    }
}

/// `✻ Brewed for 4s · 2 MCP tasks still running` -> 2.
fn mcp_total(line: &str) -> u32 {
    if !line.starts_with(SUMMARY_GLYPHS) {
        return 0;
    }
    line.split('·')
        .map(str::trim)
        .find(|segment| segment.contains("MCP task") && segment.ends_with("still running"))
        .and_then(leading_number)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::Agent;

    fn count(screen: &str) -> u8 {
        background_count(Some(Agent::Claude), screen)
    }

    fn with_footer(footer: &str) -> String {
        format!("────────\n❯\n────────\n{footer}\n")
    }

    // The footers below are the ones captured from Claude Code 2.1.295 in
    // openspec/changes/background-work-mark/captures.

    #[test]
    fn one_and_two_shells_count_one_and_two() {
        assert_eq!(
            count(&with_footer("  ⏸ manual mode on · 1 shell · ← 1 agent")),
            1
        );
        assert_eq!(
            count(&with_footer("  ⏸ manual mode on · 2 shells · ← 1 agent")),
            2
        );
        assert_eq!(
            count(&with_footer("  ⏵⏵ bypass permissions on · 2 shells")),
            2
        );
    }

    #[test]
    fn a_comma_list_sums_every_kind() {
        assert_eq!(
            count(&with_footer(
                "  ⏸ manual mode on · 3 shells, 1 monitor · ← 1 agent"
            )),
            4
        );
        assert_eq!(
            count(&with_footer("  ⏸ manual mode on · 2 monitors · ← 1 agent")),
            2
        );
    }

    #[test]
    fn the_standing_agent_entry_and_a_bare_footer_count_nothing() {
        assert_eq!(count(&with_footer("  ⏸ manual mode on · ← 1 agent")), 0);
        assert_eq!(count(&with_footer("  ⏸ manual mode on")), 0);
        assert_eq!(count("❯\n"), 0);
    }

    #[test]
    fn a_longer_word_is_not_a_shell() {
        assert_eq!(
            count(&with_footer("  ⏵⏵ bypass permissions on · 2 shellsx")),
            0
        );
    }

    #[test]
    fn the_workflow_and_subagent_rows_under_the_footer_do_not_double_count() {
        let screen = "────────\n❯\n────────\n  ⏸ manual mode on · 2 monitors · ← 1 agent\n\n  ⏺ main\n  ◯ general-purpose  Sleep 90 then answer ok (A)   6s · ↓ 79.1k tokens\n  ◯ general-purpose  Sleep 90 then answer ok (B)   5s · ↓ 78.9k tokens\n";
        assert_eq!(count(screen), 2);
    }

    #[test]
    fn waiting_for_background_agents_and_mcp_tasks_count_their_number() {
        assert_eq!(
            count("✳ Waiting for 3 background agents to finish\n────────\n❯\n────────\n"),
            3
        );
        assert_eq!(count("✻ Brewed for 4s · 2 MCP tasks still running\n❯\n"), 2);
    }

    #[test]
    fn text_in_the_conversation_is_not_a_footer() {
        assert_eq!(count("the footer said ⏸ manual mode on · 3 shells\n❯\n"), 0);
        assert_eq!(
            count("✻ Sautéed for 49s · done 10:38 PM · 2 shells still running\n❯\n"),
            0
        );
    }

    #[test]
    fn the_count_saturates_and_other_agents_have_none() {
        assert_eq!(count(&with_footer("  ⏸ manual mode on · 999 shells")), 255);
        assert_eq!(
            background_count(Some(Agent::Codex), &with_footer("  ⏸ x · 2 shells")),
            0
        );
        assert_eq!(background_count(None, &with_footer("  ⏸ x · 2 shells")), 0);
    }
}
