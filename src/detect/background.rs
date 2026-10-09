//! Background work a Claude pane reports on screen, counted for the sidebar's
//! background mark (fork issue 172).

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
        assert_eq!(count(&with_footer("  ⏸ manual mode on · 1 shell · ← 1 agent")), 1);
        assert_eq!(count(&with_footer("  ⏸ manual mode on · 2 shells · ← 1 agent")), 2);
        assert_eq!(count(&with_footer("  ⏵⏵ bypass permissions on · 2 shells")), 2);
    }

    #[test]
    fn a_comma_list_sums_every_kind() {
        assert_eq!(
            count(&with_footer("  ⏸ manual mode on · 3 shells, 1 monitor · ← 1 agent")),
            4
        );
        assert_eq!(count(&with_footer("  ⏸ manual mode on · 2 monitors · ← 1 agent")), 2);
    }

    #[test]
    fn the_standing_agent_entry_and_a_bare_footer_count_nothing() {
        assert_eq!(count(&with_footer("  ⏸ manual mode on · ← 1 agent")), 0);
        assert_eq!(count(&with_footer("  ⏸ manual mode on")), 0);
        assert_eq!(count("❯\n"), 0);
    }

    #[test]
    fn a_longer_word_is_not_a_shell() {
        assert_eq!(count(&with_footer("  ⏵⏵ bypass permissions on · 2 shellsx")), 0);
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
        assert_eq!(count("✻ Sautéed for 49s · done 10:38 PM · 2 shells still running\n❯\n"), 0);
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
