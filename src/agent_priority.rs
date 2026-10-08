//! The two rankings over an agent's `(state, seen)` pair.
//!
//! They answer different questions and disagree on exactly one pair of
//! combinations. Keeping them here, named, is what stops them drifting apart:
//! this module replaces three byte-identical copies of the attention table,
//! one of which was quietly being used to choose a *displayed* state, and a
//! fourth table that had already worked out the display order independently.

use crate::detect::AgentState;

/// Ranking for *what a group of panes is*: the state an aggregating row shows.
///
/// `Working` outranks a pane that finished while unseen, so a workspace, tab,
/// or worktree space holding an actively working agent never renders as done.
/// `Blocked` still leads — a blocked agent cannot proceed at all.
pub(crate) fn display_priority(state: AgentState, seen: bool) -> u8 {
    match (state, seen) {
        (AgentState::Blocked, _) => 4,
        (AgentState::Working, _) => 3,
        (AgentState::Idle, false) => 2,
        (AgentState::Idle, true) => 1,
        (AgentState::Unknown, _) => 0,
    }
}

/// Ranking for *what wants the user next*: attention-ordered sorting and
/// notification decisions.
///
/// A pane that finished while unseen outranks a working one, because a
/// finished agent is waiting on the user and a working one is not. That is
/// deliberately the reverse of [`display_priority`], and surfacing finished
/// agents first is the point of the priority sorts — not a bug to fix.
pub(crate) fn attention_priority(state: AgentState, seen: bool) -> u8 {
    match (state, seen) {
        (AgentState::Blocked, _) => 4,
        (AgentState::Idle, false) => 3,
        (AgentState::Working, _) => 2,
        (AgentState::Idle, true) => 1,
        (AgentState::Unknown, _) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every distinct `(state, seen)` combination. `seen` only discriminates
    /// `Idle`, so the other three states appear once each.
    const COMBINATIONS: [(AgentState, bool); 5] = [
        (AgentState::Blocked, true),
        (AgentState::Working, true),
        (AgentState::Idle, false),
        (AgentState::Idle, true),
        (AgentState::Unknown, true),
    ];

    type Ranking = fn(AgentState, bool) -> u8;

    fn ranks(priority: Ranking) -> Vec<u8> {
        COMBINATIONS
            .iter()
            .map(|(state, seen)| priority(*state, *seen))
            .collect()
    }

    #[test]
    fn display_priority_ranks_working_above_done() {
        // blocked, working, done, idle, unknown
        assert_eq!(ranks(display_priority), vec![4, 3, 2, 1, 0]);
    }

    #[test]
    fn attention_priority_ranks_done_above_working() {
        // blocked, working, done, idle, unknown — the existing table, exactly.
        assert_eq!(ranks(attention_priority), vec![4, 2, 3, 1, 0]);
    }

    #[test]
    fn both_rankings_are_total_orders_over_every_combination() {
        for priority in [display_priority as Ranking, attention_priority] {
            let mut seen_ranks = ranks(priority);
            seen_ranks.sort_unstable();
            seen_ranks.dedup();
            assert_eq!(
                seen_ranks,
                vec![0, 1, 2, 3, 4],
                "a ranking must map the five combinations onto 0..5 without ties"
            );
        }
    }

    #[test]
    fn rankings_agree_except_on_working_versus_done() {
        for (state, seen) in [
            (AgentState::Blocked, true),
            (AgentState::Idle, true),
            (AgentState::Unknown, true),
        ] {
            assert_eq!(
                display_priority(state, seen),
                attention_priority(state, seen),
                "{state:?} (seen={seen}) must rank the same in both"
            );
        }

        assert!(
            display_priority(AgentState::Working, true) > display_priority(AgentState::Idle, false)
        );
        assert!(
            attention_priority(AgentState::Idle, false)
                > attention_priority(AgentState::Working, true)
        );
    }

    #[test]
    fn seen_only_discriminates_idle() {
        for state in [
            AgentState::Blocked,
            AgentState::Working,
            AgentState::Unknown,
        ] {
            assert_eq!(
                display_priority(state, true),
                display_priority(state, false)
            );
            assert_eq!(
                attention_priority(state, true),
                attention_priority(state, false)
            );
        }
    }
}
