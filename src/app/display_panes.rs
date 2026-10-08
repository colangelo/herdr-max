//! Display panes: tmux's `prefix q`. For a few seconds every pane of the
//! current tab carries a label with its index, its public address, its name and
//! its size in characters; a digit focuses the pane it names and any other key
//! closes the labels.
//!
//! Presentation only. Everything shown is already known to the renderer: the
//! pane rects come from the view geometry, the address and the name from the
//! same helpers the navigator uses. Nothing here is a runtime fact.

use std::time::Instant;

use ratatui::layout::Rect;

use super::state::{AppState, Overlay};
use crate::layout::PaneId;

/// Panes past this index still get a label, but no number to press.
pub(crate) const DISPLAY_PANES_MAX_INDEX: usize = 9;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayPanesState {
    /// When the labels close on their own.
    pub(crate) deadline: Instant,
}

/// One pane's label, in layout reading order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DisplayPaneLabel {
    /// 1-based, and only for the first [`DISPLAY_PANES_MAX_INDEX`] panes.
    pub index: Option<usize>,
    pub pane_id: PaneId,
    /// The public address, `w5:p16`.
    pub address: String,
    /// What the navigator and the todo board call the pane.
    pub name: String,
    /// The pane's terminal: what its program sees as `tput cols`/`lines`.
    pub inner_rect: Rect,
    pub focused: bool,
}

impl AppState {
    /// Whether the size labels show because a pane is being resized: a split
    /// border is held, resize mode is on, or the last step was under
    /// `display_panes_duration` ago. A passive layer, never a mode.
    pub(crate) fn resize_labels_visible(&self) -> bool {
        matches!(
            self.drag.as_ref().map(|drag| &drag.target),
            Some(super::state::DragTarget::PaneSplit { .. })
        ) || self.resize_labels_window_visible()
            || self.mode == super::state::Mode::Resize
            || self.resize_labels_until.is_some()
    }

    /// The herdr window, a sidebar edge or a pane was resized (fork issues
    /// 138, 145, 150): the pane labels plus the window summary bar and the
    /// sidebar section sizes, for the linger time counted from this event.
    pub(crate) fn show_window_resize_labels(&mut self, now: Instant) {
        self.resize_labels_until = Some(now + self.display_panes_duration);
        self.resize_labels_window = true;
    }

    /// Whether the labels are the window-resize view: lingering after a window
    /// resize, a sidebar drag or a pane resize, or while a sidebar or pane
    /// edge is held or resize mode is on (fork issues 145, 150). A pane
    /// resize shows the same full view as `prefix+i`.
    pub(crate) fn resize_labels_window_visible(&self) -> bool {
        (self.resize_labels_window && self.resize_labels_until.is_some())
            || self.mode == super::state::Mode::Resize
            || matches!(
                self.drag.as_ref().map(|drag| &drag.target),
                Some(
                    super::state::DragTarget::PaneSplit { .. }
                        | super::state::DragTarget::SidebarDivider
                        | super::state::DragTarget::SidebarSectionDivider
                )
            )
    }

    /// Any other key or click: the labels go now.
    pub(crate) fn hide_resize_labels(&mut self) {
        self.resize_labels_until = None;
        self.resize_labels_window = false;
    }

    /// When the lingering labels go, for the loops' wake-up list.
    pub(crate) fn resize_labels_deadline(&self) -> Option<Instant> {
        self.resize_labels_until
    }

    /// Drop the lingering labels once their time is up; `true` when that
    /// changed what is drawn.
    pub(crate) fn expire_resize_labels(&mut self, now: Instant) -> bool {
        if self.resize_labels_until.is_some_and(|until| now >= until) {
            self.resize_labels_until = None;
            self.resize_labels_window = false;
            return true;
        }
        false
    }

    pub(crate) fn open_display_panes(&mut self, now: Instant) {
        self.open_overlay(Overlay::DisplayPanes(DisplayPanesState {
            deadline: now + self.display_panes_duration,
        }));
    }

    /// When the open labels close on their own, for the loops' wake-up list.
    pub(crate) fn display_panes_deadline(&self) -> Option<Instant> {
        self.display_panes().map(|state| state.deadline)
    }

    /// The labels to draw, one per visible pane of the current tab, in the
    /// layout's reading order. Built from the view geometry, so it is exactly
    /// the panes on screen: a zoomed tab labels only its zoomed pane.
    pub(crate) fn display_panes_labels(&self) -> Vec<DisplayPaneLabel> {
        let Some(ws_idx) = self.active else {
            return Vec::new();
        };
        let Some(ws) = self.workspaces.get(ws_idx) else {
            return Vec::new();
        };
        self.view
            .pane_infos
            .iter()
            .enumerate()
            .map(|(position, info)| DisplayPaneLabel {
                index: (position < DISPLAY_PANES_MAX_INDEX).then_some(position + 1),
                pane_id: info.id,
                address: ws
                    .public_pane_number(info.id)
                    .map(|number| crate::workspace::public_pane_id_for_number(&ws.id, number))
                    .unwrap_or_default(),
                name: self.pane_display_label(ws_idx, info.id),
                inner_rect: info.inner_rect,
                focused: info.is_focused,
            })
            .collect()
    }

    /// The pane a pressed digit names, if a label shows that number.
    pub(crate) fn display_panes_target(&self, index: usize) -> Option<(usize, PaneId)> {
        if index == 0 || index > DISPLAY_PANES_MAX_INDEX {
            return None;
        }
        let ws_idx = self.active?;
        let info = self.view.pane_infos.get(index - 1)?;
        Some((ws_idx, info.id))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ratatui::layout::Direction;

    use super::*;
    use crate::app::state::Mode;

    pub(crate) fn two_pane_state() -> (AppState, PaneId, PaneId) {
        let mut state = AppState::test_new();
        state.workspaces = vec![crate::workspace::Workspace::test_new("one")];
        state.active = Some(0);
        state.selected = 0;
        state.mode = Mode::Terminal;
        let left = state.workspaces[0].tabs[0].root_pane;
        let right = state.workspaces[0].test_split(Direction::Horizontal);
        state.ensure_test_terminals();
        crate::ui::test_support::layout(&mut state);
        (state, left, right)
    }

    #[test]
    fn opening_puts_the_app_in_display_panes_with_a_deadline() {
        let (mut state, _, _) = two_pane_state();
        let now = Instant::now();

        state.open_display_panes(now);

        assert_eq!(state.mode, Mode::DisplayPanes);
        assert_eq!(
            state.display_panes_deadline(),
            Some(now + state.display_panes_duration)
        );
        state.assert_invariants_for_test();
    }

    #[test]
    fn labels_number_panes_in_reading_order_with_address_name_and_size() {
        let (mut state, left, right) = two_pane_state();
        state.open_display_panes(Instant::now());

        let labels = state.display_panes_labels();

        assert_eq!(labels.len(), 2);
        let infos = &state.view.pane_infos;
        for (label, (pane_id, info)) in labels
            .iter()
            .zip([left, right].into_iter().zip(infos.iter()))
        {
            assert_eq!(label.pane_id, pane_id);
            assert_eq!(label.inner_rect, info.inner_rect);
            assert_eq!(label.name, state.pane_display_label(0, pane_id));
        }
        assert_eq!(labels[0].index, Some(1));
        assert_eq!(labels[1].index, Some(2));
        let ws = &state.workspaces[0];
        assert_eq!(
            labels[1].address,
            crate::workspace::public_pane_id_for_number(
                &ws.id,
                ws.public_pane_number(right).expect("numbered pane"),
            )
        );
        assert!(labels[1].focused, "the split leaves the new pane focused");
        assert!(!labels[0].focused);
    }

    #[test]
    fn a_digit_names_the_pane_with_that_label() {
        let (mut state, left, right) = two_pane_state();
        state.open_display_panes(Instant::now());

        assert_eq!(state.display_panes_target(1), Some((0, left)));
        assert_eq!(state.display_panes_target(2), Some((0, right)));
        assert_eq!(state.display_panes_target(3), None);
        assert_eq!(state.display_panes_target(0), None);
    }

    #[test]
    fn a_closed_overlay_has_no_deadline() {
        let (state, _, _) = two_pane_state();
        assert_eq!(state.display_panes_deadline(), None);
    }

    // Fork issue 122: the same labels while a pane is resized.
    mod resize_labels {
        use super::*;
        use crate::app::state::{DragState, DragTarget};

        fn split_drag() -> DragState {
            DragState {
                target: DragTarget::PaneSplit {
                    path: Vec::new(),
                    direction: Direction::Horizontal,
                    area: Rect::new(0, 0, 80, 24),
                    grab_offset: 0,
                },
            }
        }

        #[test]
        fn they_show_while_a_split_border_is_dragged() {
            let (mut state, _, _) = two_pane_state();
            assert!(!state.resize_labels_visible());

            state.drag = Some(split_drag());

            assert!(state.resize_labels_visible());
            assert!(state.resize_labels_window_visible(), "the full view");
            assert_eq!(state.mode, Mode::Terminal, "not a mode: input is untouched");
        }

        #[test]
        fn they_stay_a_second_after_the_last_resize_then_go() {
            let (mut state, _, _) = two_pane_state();
            let now = Instant::now();
            state.show_window_resize_labels(now);

            assert!(state.resize_labels_visible());
            assert_eq!(
                state.resize_labels_deadline(),
                Some(now + state.display_panes_duration)
            );
            assert!(!state.expire_resize_labels(
                now + state.display_panes_duration - Duration::from_millis(1)
            ));
            assert!(state.resize_labels_visible());
            assert!(state.expire_resize_labels(now + state.display_panes_duration));
            assert!(!state.resize_labels_visible());
            assert_eq!(state.resize_labels_deadline(), None);
        }

        #[test]
        fn one_setting_sets_how_long_both_label_views_stay() {
            let (mut state, _, _) = two_pane_state();
            state.display_panes_duration = Duration::from_millis(7000);
            let now = Instant::now();

            state.open_display_panes(now);
            assert_eq!(
                state.display_panes_deadline(),
                Some(now + Duration::from_millis(7000))
            );
            state.show_window_resize_labels(now);
            assert_eq!(
                state.resize_labels_deadline(),
                Some(now + Duration::from_millis(7000))
            );
            assert!(!state.expire_resize_labels(now + Duration::from_millis(6999)));
            assert!(state.expire_resize_labels(now + Duration::from_millis(7000)));
        }

        #[test]
        fn every_resize_event_rearms_from_that_event() {
            let (mut state, _, _) = two_pane_state();
            let first = Instant::now();
            let second = first + Duration::from_secs(2);
            state.show_window_resize_labels(first);
            state.show_window_resize_labels(second);
            let linger = state.display_panes_duration;
            assert!(!state.expire_resize_labels(first + linger));
            assert!(!state.expire_resize_labels(second + linger - Duration::from_millis(1)));
            assert!(state.expire_resize_labels(second + linger));
        }

        #[test]
        fn a_window_resize_arms_the_window_view_and_a_drag_step_keeps_it() {
            let (mut state, _, _) = two_pane_state();
            let now = Instant::now();
            state.show_window_resize_labels(now);
            assert!(
                state.resize_labels_window_visible(),
                "a pane resize shows the full view (fork issue 150)"
            );

            state.show_window_resize_labels(now + Duration::from_secs(1));
            assert!(state.resize_labels_window_visible());

            assert!(state
                .expire_resize_labels(now + Duration::from_secs(1) + state.display_panes_duration));
            assert!(!state.resize_labels_window_visible());
            assert!(
                !state.resize_labels_window_visible(),
                "expiry cleared the flag"
            );
        }

        #[test]
        fn resize_mode_shows_them_until_it_ends() {
            let (mut state, _, _) = two_pane_state();
            state.mode = Mode::Resize;
            assert!(state.resize_labels_visible());
            assert!(state.resize_labels_window_visible(), "the full view");
            state.mode = Mode::Terminal;
            assert!(!state.resize_labels_visible());
        }

        #[test]
        fn another_key_or_click_hides_them_at_once() {
            let (mut state, _, _) = two_pane_state();
            state.show_window_resize_labels(Instant::now());

            state.hide_resize_labels();

            assert!(!state.resize_labels_visible());
        }
    }
}
