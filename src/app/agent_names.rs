//! Agent names: the explicit herdr name, else a name-like terminal title
//! (fork issue 130). Worked out when an API call or a notification asks,
//! from the live title, so nothing is stored and the fallback follows the
//! title (a Claude `/rename`).

use std::collections::{HashMap, HashSet};

use super::state::AppState;
use crate::terminal::TerminalId;

/// Where an agent's name comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentNameSource {
    /// Set with `agent rename` (or a launch name); never replaced.
    Explicit,
    /// The agent's terminal title, standing in while it has no explicit name.
    Title,
}

impl AppState {
    /// The title fallback name of every agent that has one: agents with no
    /// explicit name whose stripped terminal title is name-like, is not
    /// another agent's explicit name or its own agent kind, and is not shared
    /// with another agent (then none of them gets it). One pass over the
    /// terminals; API and notification paths only, never per frame.
    pub(crate) fn agent_title_names(&self) -> HashMap<TerminalId, String> {
        let explicit: HashSet<&str> = self
            .terminals
            .values()
            .filter_map(|terminal| terminal.agent_name.as_deref())
            .collect();
        let mut owners: HashMap<String, Option<&TerminalId>> = HashMap::new();
        for (terminal_id, terminal) in &self.terminals {
            if terminal.agent_name.is_some() || !terminal.is_agent_terminal() {
                continue;
            }
            let Some(title) = terminal.terminal_title_stripped() else {
                continue;
            };
            if !crate::terminal::title_is_agent_name(&title)
                || explicit.contains(title.as_str())
                || terminal
                    .effective_agent_label()
                    .is_some_and(|label| label.eq_ignore_ascii_case(&title))
            {
                continue;
            }
            owners
                .entry(title)
                .and_modify(|owner| *owner = None)
                .or_insert(Some(terminal_id));
        }
        owners
            .into_iter()
            .filter_map(|(name, owner)| owner.map(|terminal_id| (terminal_id.clone(), name)))
            .collect()
    }

    /// An agent's name and where it comes from: the explicit name, else its
    /// title fallback.
    pub(crate) fn agent_name(&self, terminal_id: &TerminalId) -> Option<(String, AgentNameSource)> {
        let terminal = self.terminals.get(terminal_id)?;
        if let Some(name) = &terminal.agent_name {
            return Some((name.clone(), AgentNameSource::Explicit));
        }
        self.agent_title_names()
            .remove(terminal_id)
            .map(|name| (name, AgentNameSource::Title))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::{Agent, AgentState};
    use crate::terminal::TerminalState;

    fn agent(state: &mut AppState, title: &str) -> TerminalId {
        let terminal_id = TerminalId::alloc();
        let mut terminal = TerminalState::new(terminal_id.clone(), "/tmp".into());
        terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);
        terminal.set_terminal_title(Some(title.to_string()));
        state.terminals.insert(terminal_id.clone(), terminal);
        terminal_id
    }

    #[test]
    fn an_unnamed_agent_is_named_from_a_name_like_title() {
        let mut state = AppState::test_new();
        let named = agent(&mut state, "\u{2733} jev-astra");
        let generic = agent(&mut state, "\u{2733} Claude Code");
        let sentence = agent(&mut state, "fix the flaky test");

        assert_eq!(
            state.agent_name(&named),
            Some(("jev-astra".to_string(), AgentNameSource::Title))
        );
        assert_eq!(state.agent_name(&generic), None);
        assert_eq!(state.agent_name(&sentence), None);
    }

    #[test]
    fn an_explicit_name_wins_and_the_fallback_follows_the_title() {
        let mut state = AppState::test_new();
        let terminal_id = agent(&mut state, "alpha");
        state
            .terminals
            .get_mut(&terminal_id)
            .unwrap()
            .terminal_title = Some("beta".into());
        assert_eq!(
            state.agent_name(&terminal_id),
            Some(("beta".to_string(), AgentNameSource::Title))
        );

        state.terminals.get_mut(&terminal_id).unwrap().agent_name = Some("reviewer".into());
        assert_eq!(
            state.agent_name(&terminal_id),
            Some(("reviewer".to_string(), AgentNameSource::Explicit))
        );
    }

    #[test]
    fn shared_or_taken_fallbacks_are_dropped() {
        let mut state = AppState::test_new();
        let first = agent(&mut state, "worker");
        let second = agent(&mut state, "worker");
        let taken = agent(&mut state, "reviewer");
        let owner = agent(&mut state, "whatever");
        state.terminals.get_mut(&owner).unwrap().agent_name = Some("reviewer".into());

        let names = state.agent_title_names();
        assert!(!names.contains_key(&first));
        assert!(!names.contains_key(&second));
        assert!(
            !names.contains_key(&taken),
            "an explicit name elsewhere wins"
        );
    }

    #[test]
    fn a_shell_pane_has_no_fallback() {
        let mut state = AppState::test_new();
        let terminal_id = TerminalId::alloc();
        let mut terminal = TerminalState::new(terminal_id.clone(), "/tmp".into());
        terminal.set_terminal_title(Some("build-box".into()));
        state.terminals.insert(terminal_id.clone(), terminal);

        assert_eq!(state.agent_name(&terminal_id), None);
    }
}
