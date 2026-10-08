//! Agent hints in the loops (fork issue 157): the wake-up deadline and the
//! tick that drops a hint whose time is up. The hint itself lives on
//! `TerminalState`.

use std::time::Instant;

use super::state::AppState;
use crate::layout::PaneId;

impl AppState {
    /// When the earliest live hint next needs the loop, for the wake-up list.
    pub(crate) fn agent_hint_deadline(&self) -> Option<Instant> {
        self.terminals
            .values()
            .filter_map(crate::terminal::TerminalState::agent_hint_deadline)
            .min()
    }

    /// The panes whose hint is due to be dropped at `now`.
    pub(crate) fn due_agent_hint_panes(&self, now: Instant) -> Vec<PaneId> {
        if self
            .agent_hint_deadline()
            .is_none_or(|deadline| deadline > now)
        {
            return Vec::new();
        }
        self.workspaces
            .iter()
            .flat_map(|ws| ws.tabs.iter())
            .flat_map(|tab| tab.panes.iter())
            .filter(|(_, pane)| {
                self.terminals
                    .get(&pane.attached_terminal_id)
                    .and_then(crate::terminal::TerminalState::agent_hint_deadline)
                    .is_some_and(|deadline| deadline <= now)
            })
            .map(|(pane_id, _)| *pane_id)
            .collect()
    }
}
