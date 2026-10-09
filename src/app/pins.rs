//! Pinning spaces and agents to the top of the sidebar (fork issue 148).
//!
//! A pin is a number: pinned rows are listed first by ascending `pin_order`, so
//! the first pinned is first and the next second, under every sort. Pinning
//! takes one more than the largest number in use among the same kind of row,
//! which needs no stored counter and continues where a restored session left
//! off. State only: the order itself is applied where the lists are built.

use super::state::AppState;
use crate::layout::PaneId;

impl AppState {
    /// Pin the space at `ws_idx` below the spaces pinned before it. A space
    /// that is already pinned keeps its place. `true` when something changed.
    pub(crate) fn pin_workspace(&mut self, ws_idx: usize) -> bool {
        let next = self
            .workspaces
            .iter()
            .filter_map(|ws| ws.pin_order)
            .max()
            .map_or(1, |max| max + 1);
        let Some(ws) = self.workspaces.get_mut(ws_idx) else {
            return false;
        };
        if ws.pin_order.is_some() {
            return false;
        }
        ws.pin_order = Some(next);
        self.mark_session_dirty();
        true
    }

    /// Unpin the space at `ws_idx`; it goes back where the sort puts it.
    pub(crate) fn unpin_workspace(&mut self, ws_idx: usize) -> bool {
        let Some(ws) = self.workspaces.get_mut(ws_idx) else {
            return false;
        };
        if ws.pin_order.take().is_none() {
            return false;
        }
        self.mark_session_dirty();
        true
    }

    /// The terminal behind a pane, mutably. The pin lives on the terminal, so
    /// it follows the agent when the pane moves between tabs and spaces.
    fn pane_terminal_mut(
        &mut self,
        pane_id: PaneId,
    ) -> Option<&mut crate::terminal::TerminalState> {
        let terminal_id = self
            .workspaces
            .iter()
            .find_map(|ws| ws.pane_state(pane_id))?
            .attached_terminal_id
            .clone();
        self.terminals.get_mut(&terminal_id)
    }

    /// Pin the agent in `pane_id` below the agents pinned before it. Already
    /// pinned keeps its place. `true` when something changed.
    pub(crate) fn pin_agent(&mut self, pane_id: PaneId) -> bool {
        let next = self
            .terminals
            .values()
            .filter_map(|terminal| terminal.pin_order)
            .max()
            .map_or(1, |max| max + 1);
        let Some(terminal) = self.pane_terminal_mut(pane_id) else {
            return false;
        };
        if terminal.pin_order.is_some() {
            return false;
        }
        terminal.pin_order = Some(next);
        self.mark_session_dirty();
        true
    }

    /// Unpin the agent in `pane_id`; it goes back where the sort puts it.
    pub(crate) fn unpin_agent(&mut self, pane_id: PaneId) -> bool {
        let Some(terminal) = self.pane_terminal_mut(pane_id) else {
            return false;
        };
        if terminal.pin_order.take().is_none() {
            return false;
        }
        self.mark_session_dirty();
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::app::state::AppState;
    use crate::workspace::Workspace;

    fn three_spaces() -> AppState {
        let mut state = AppState::test_new();
        state.workspaces = ["a", "b", "c"]
            .into_iter()
            .map(Workspace::test_new)
            .collect();
        state
    }

    #[test]
    fn pins_take_the_next_number_and_keep_their_place() {
        let mut state = three_spaces();
        assert!(state.pin_workspace(2));
        assert!(state.pin_workspace(0));
        assert_eq!(state.workspaces[2].pin_order, Some(1));
        assert_eq!(state.workspaces[0].pin_order, Some(2));
        assert!(!state.pin_workspace(2), "pinning again changes nothing");
        assert_eq!(state.workspaces[2].pin_order, Some(1));
        assert_eq!(state.workspaces[1].pin_order, None);
    }

    #[test]
    fn unpin_clears_and_a_later_pin_goes_last() {
        let mut state = three_spaces();
        state.pin_workspace(0);
        state.pin_workspace(1);
        assert!(state.unpin_workspace(0));
        assert!(!state.unpin_workspace(0), "already unpinned");
        assert!(state.pin_workspace(0));
        assert_eq!(state.workspaces[1].pin_order, Some(2));
        assert_eq!(state.workspaces[0].pin_order, Some(3), "re-pinned last");
    }

    #[test]
    fn pinned_spaces_hold_the_state_invariants_even_on_adversarial_state() {
        let mut state = AppState::test_with_adversarial_identity_state();
        state.workspaces.push(Workspace::test_new("extra"));
        state.ensure_test_terminals();
        state.pin_workspace(1);
        state.pin_workspace(0);
        state.assert_invariants_for_test();
        state.unpin_workspace(1);
        state.pin_workspace(1);
        state.assert_invariants_for_test();
    }
}
