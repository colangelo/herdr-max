//! Synchronized input for a tab (fork issue 141): the app-level switches over
//! `Tab::sync`. The state and its set arithmetic live on `Tab`; this is the
//! part that knows which workspace and tab are on screen.

use std::time::Instant;

use super::state::AppState;
use crate::layout::PaneId;

impl AppState {
    /// Turn sync on or off for the tab on screen. `true` when it is on now,
    /// `false` when it went off or there is no tab.
    pub(crate) fn toggle_sync_panes(&mut self) -> bool {
        let Some(tab) = self
            .active
            .and_then(|ws_idx| self.workspaces.get_mut(ws_idx))
            .and_then(crate::workspace::Workspace::active_tab_mut)
        else {
            return false;
        };
        tab.set_sync(!tab.is_syncing());
        tab.is_syncing()
    }

    /// Set sync for one tab. `false` when the tab does not exist.
    pub(crate) fn set_tab_sync(&mut self, ws_idx: usize, tab_idx: usize, on: bool) -> bool {
        let Some(tab) = self
            .workspaces
            .get_mut(ws_idx)
            .and_then(|ws| ws.tabs.get_mut(tab_idx))
        else {
            return false;
        };
        tab.set_sync(on);
        true
    }

    /// Start a group of the focused pane and `other` in one tab (fork issue
    /// 155), replacing whatever sync state the tab had.
    pub(crate) fn start_sync_pair(&mut self, ws_idx: usize, tab_idx: usize, other: PaneId) {
        let Some(tab) = self
            .workspaces
            .get_mut(ws_idx)
            .and_then(|ws| ws.tabs.get_mut(tab_idx))
        else {
            return;
        };
        let focused = tab.layout.focused();
        tab.start_sync_pair(focused, other);
    }

    /// "Sync input" in a pane menu (fork issue 155). On a pane other than the
    /// focused one (`source_pane_id` names the focused pane the menu was
    /// opened from) it starts a group of just those two; on the focused pane
    /// it is the whole-tab switch, like the key.
    pub(crate) fn sync_input_from_menu(
        &mut self,
        ws_idx: usize,
        tab_idx: usize,
        pane_id: PaneId,
        source_pane_id: Option<PaneId>,
    ) {
        match source_pane_id {
            Some(source) if source != pane_id => self.start_sync_pair(ws_idx, tab_idx, pane_id),
            _ => {
                self.toggle_sync_panes();
            }
        }
    }

    /// When the earliest ending group's grace is up, for the loops' wake-up
    /// list.
    pub(crate) fn sync_deadline(&self) -> Option<Instant> {
        self.workspaces
            .iter()
            .flat_map(|ws| ws.tabs.iter())
            .filter_map(crate::workspace::Tab::sync_deadline)
            .min()
    }

    /// The loop's tick: end sync mode where the grace is up. `true` when that
    /// changed what is drawn.
    pub(crate) fn expire_sync(&mut self, now: Instant) -> bool {
        let mut changed = false;
        for tab in self.workspaces.iter_mut().flat_map(|ws| ws.tabs.iter_mut()) {
            changed |= tab.expire_sync(now);
        }
        changed
    }

    /// Take a pane out of its tab's synced set or put it back. `None` when its
    /// tab does not sync.
    pub(crate) fn toggle_pane_sync(&mut self, ws_idx: usize, pane_id: PaneId) -> Option<bool> {
        self.toggle_pane_sync_at(ws_idx, pane_id, Instant::now())
    }

    /// [`Self::toggle_pane_sync`] with the clock given, for the grace.
    pub(crate) fn toggle_pane_sync_at(
        &mut self,
        ws_idx: usize,
        pane_id: PaneId,
        now: Instant,
    ) -> Option<bool> {
        let tab_idx = self
            .workspaces
            .get(ws_idx)?
            .find_tab_index_for_pane(pane_id)?;
        self.workspaces
            .get_mut(ws_idx)?
            .tabs
            .get_mut(tab_idx)?
            .toggle_pane_sync(pane_id, now)
    }

    /// The panes that also get what is typed into the focused pane of the
    /// workspace on screen. Empty unless its tab syncs and the focused pane
    /// is in the set.
    #[cfg(test)]
    pub(crate) fn sync_peer_panes(&self, ws_idx: usize) -> Vec<PaneId> {
        let Some(ws) = self.workspaces.get(ws_idx) else {
            return Vec::new();
        };
        let (Some(focused), Some(tab)) = (ws.focused_pane_id(), ws.active_tab()) else {
            return Vec::new();
        };
        tab.sync_peers(focused)
    }
}

#[cfg(test)]
mod tests {
    use crate::app::state::AppState;
    use crate::layout::PaneId;
    use crate::workspace::Workspace;
    use ratatui::layout::Direction;

    /// One workspace, one tab, three panes (a, b, c), `a` focused.
    fn three_panes() -> (AppState, [PaneId; 3]) {
        let mut state = AppState::test_new();
        let mut ws = Workspace::test_new("one");
        let a = ws.tabs[0].root_pane;
        let b = ws.test_split(Direction::Horizontal);
        let c = ws.test_split(Direction::Vertical);
        ws.tabs[0].layout.focus_pane(a);
        state.workspaces = vec![ws];
        state.active = Some(0);
        state.selected = 0;
        state.ensure_test_terminals();
        (state, [a, b, c])
    }

    #[test]
    fn sync_is_off_by_default_and_toggles_per_tab() {
        let (mut state, _) = three_panes();
        assert!(!state.workspaces[0].tabs[0].is_syncing());
        assert!(state.sync_peer_panes(0).is_empty());

        assert!(state.toggle_sync_panes());
        assert!(state.workspaces[0].tabs[0].is_syncing());
        assert!(!state.toggle_sync_panes());
        assert!(!state.workspaces[0].tabs[0].is_syncing());
    }

    #[test]
    fn turning_it_on_puts_every_pane_in_and_the_peers_exclude_the_focused_one() {
        let (mut state, [a, b, c]) = three_panes();
        state.toggle_sync_panes();
        let tab = &state.workspaces[0].tabs[0];
        assert!([a, b, c].iter().all(|pane| tab.pane_synced(*pane)));
        let peers = state.sync_peer_panes(0);
        assert_eq!(peers.len(), 2);
        assert!(peers.contains(&b) && peers.contains(&c) && !peers.contains(&a));
    }

    #[test]
    fn a_pane_can_be_taken_out_and_put_back() {
        let (mut state, [_, b, c]) = three_panes();
        state.toggle_sync_panes();

        assert_eq!(state.toggle_pane_sync(0, b), Some(false));
        assert_eq!(state.sync_peer_panes(0), vec![c]);
        assert_eq!(state.toggle_pane_sync(0, b), Some(true));
        assert_eq!(state.sync_peer_panes(0).len(), 2);
    }

    #[test]
    fn an_excluded_focused_pane_is_typed_into_alone() {
        let (mut state, [a, _, _]) = three_panes();
        state.toggle_sync_panes();
        state.toggle_pane_sync(0, a);
        assert!(state.sync_peer_panes(0).is_empty());
    }

    #[test]
    fn toggling_a_pane_does_nothing_while_the_tab_does_not_sync() {
        let (mut state, [_, b, _]) = three_panes();
        assert_eq!(state.toggle_pane_sync(0, b), None);
        assert!(state.sync_peer_panes(0).is_empty());
    }

    #[test]
    fn a_pane_made_while_sync_is_on_joins_and_a_closed_one_leaves() {
        let (mut state, [a, b, _]) = three_panes();
        state.toggle_sync_panes();
        let d = state.workspaces[0].test_split(Direction::Horizontal);
        state.workspaces[0].tabs[0].layout.focus_pane(a);
        assert!(
            state.workspaces[0].tabs[0].pane_synced(d),
            "a new pane joins"
        );

        state.toggle_pane_sync(0, b);
        state.workspaces[0].tabs[0].panes.remove(&b);
        assert!(
            !state.workspaces[0].tabs[0].pane_synced(b),
            "a closed pane is out"
        );
    }

    #[test]
    fn turning_it_on_again_forgets_earlier_exclusions() {
        let (mut state, [_, b, _]) = three_panes();
        state.toggle_sync_panes();
        state.toggle_pane_sync(0, b);
        state.toggle_sync_panes();
        state.toggle_sync_panes();
        assert!(state.workspaces[0].tabs[0].pane_synced(b));
    }

    // ---- fork issue 155: explicit members, a pair, a grace

    #[test]
    fn the_menu_on_another_pane_starts_a_pair_and_the_rest_stay_out() {
        let (mut state, [a, b, c]) = three_panes();
        state.sync_input_from_menu(0, 0, b, Some(a));
        let tab = &state.workspaces[0].tabs[0];
        assert!(tab.pane_synced(a) && tab.pane_synced(b) && !tab.pane_synced(c));
        assert_eq!(state.sync_peer_panes(0), vec![b]);
    }

    #[test]
    fn the_menu_on_the_focused_pane_and_the_key_sync_the_whole_tab() {
        let (mut state, [a, b, c]) = three_panes();
        state.sync_input_from_menu(0, 0, a, None);
        let tab = &state.workspaces[0].tabs[0];
        assert!([a, b, c].iter().all(|pane| tab.pane_synced(*pane)));
        assert!(tab.sync.as_ref().unwrap().whole_tab);

        let (mut state, [a, b, c]) = three_panes();
        state.toggle_sync_panes();
        let tab = &state.workspaces[0].tabs[0];
        assert!([a, b, c].iter().all(|pane| tab.pane_synced(*pane)));
    }

    #[test]
    fn a_new_pane_joins_a_whole_tab_group_and_never_a_pair() {
        let (mut state, [a, b, _]) = three_panes();
        state.toggle_sync_panes();
        let d = state.workspaces[0].test_split(Direction::Horizontal);
        assert!(state.workspaces[0].tabs[0].pane_synced(d));

        let (mut state, [a2, b2, _]) = three_panes();
        let _ = (a, b);
        state.start_sync_pair(0, 0, b2);
        let d = state.workspaces[0].test_split(Direction::Horizontal);
        let tab = &state.workspaces[0].tabs[0];
        assert!(!tab.pane_synced(d), "a pair stays two");
        assert!(tab.pane_synced(a2) && tab.pane_synced(b2));
    }

    #[test]
    fn right_clicking_members_out_and_others_in_edits_the_group() {
        let (mut state, [a, b, c]) = three_panes();
        state.start_sync_pair(0, 0, b);
        assert_eq!(state.toggle_pane_sync(0, c), Some(true), "c joins");
        assert_eq!(state.toggle_pane_sync(0, a), Some(false), "a leaves");
        let tab = &state.workspaces[0].tabs[0];
        assert!(!tab.pane_synced(a) && tab.pane_synced(b) && tab.pane_synced(c));
    }

    #[test]
    fn a_group_of_one_stays_in_sync_mode() {
        let (mut state, [a, b, _]) = three_panes();
        state.start_sync_pair(0, 0, b);
        state.toggle_pane_sync(0, a);
        let tab = &state.workspaces[0].tabs[0];
        assert!(tab.is_syncing() && !tab.sync_ending());
        assert!(tab.pane_synced(b));
        assert!(state.sync_deadline().is_none());
    }

    #[test]
    fn an_empty_group_ends_sync_after_the_grace() {
        use crate::workspace::SYNC_GRACE;
        use std::time::{Duration, Instant};
        let (mut state, [a, b, _]) = three_panes();
        let t0 = Instant::now();
        state.start_sync_pair(0, 0, b);
        state.toggle_pane_sync_at(0, a, t0);
        state.toggle_pane_sync_at(0, b, t0);

        let tab = &state.workspaces[0].tabs[0];
        assert!(tab.is_syncing() && tab.sync_ending(), "still in sync mode");
        assert!(
            state.sync_peer_panes(0).is_empty(),
            "nothing is typed anywhere"
        );
        assert_eq!(state.sync_deadline(), Some(t0 + SYNC_GRACE));

        assert!(!state.expire_sync(t0 + SYNC_GRACE - Duration::from_millis(1)));
        assert!(state.workspaces[0].tabs[0].is_syncing());
        assert!(state.expire_sync(t0 + SYNC_GRACE));
        assert!(!state.workspaces[0].tabs[0].is_syncing(), "back to normal");
        assert_eq!(state.sync_deadline(), None);
        assert_eq!(
            state.toggle_pane_sync(0, a),
            None,
            "so a click opens the menu"
        );
    }

    #[test]
    fn a_click_during_the_grace_puts_the_pane_back_and_cancels_the_countdown() {
        use std::time::{Duration, Instant};
        let (mut state, [a, b, c]) = three_panes();
        let t0 = Instant::now();
        state.start_sync_pair(0, 0, b);
        state.toggle_pane_sync_at(0, a, t0);
        state.toggle_pane_sync_at(0, b, t0);

        let t1 = t0 + Duration::from_secs(1);
        assert_eq!(state.toggle_pane_sync_at(0, c, t1), Some(true));
        let tab = &state.workspaces[0].tabs[0];
        assert!(!tab.sync_ending() && tab.pane_synced(c));
        assert_eq!(state.sync_deadline(), None);
        assert!(!state.expire_sync(t0 + Duration::from_secs(60)));
        assert!(state.workspaces[0].tabs[0].is_syncing());
    }

    #[test]
    fn a_group_whose_panes_all_closed_ends_through_the_grace() {
        use crate::workspace::SYNC_GRACE;
        use std::time::Instant;
        let (mut state, [a, b, _]) = three_panes();
        state.start_sync_pair(0, 0, b);
        state.workspaces[0].tabs[0].panes.remove(&a);
        state.workspaces[0].tabs[0].panes.remove(&b);
        let t0 = Instant::now();
        assert!(state.expire_sync(t0), "the tick starts the grace");
        assert!(state.workspaces[0].tabs[0].sync_ending());
        assert!(state.expire_sync(t0 + SYNC_GRACE));
        assert!(!state.workspaces[0].tabs[0].is_syncing());
    }

    #[test]
    fn another_tab_is_not_touched() {
        let (mut state, _) = three_panes();
        state.workspaces[0].test_add_tab(Some("two"));
        state.workspaces[0].switch_tab(0);
        state.toggle_sync_panes();
        assert!(state.workspaces[0].tabs[0].is_syncing());
        assert!(!state.workspaces[0].tabs[1].is_syncing());
    }
}
