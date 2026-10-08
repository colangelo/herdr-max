//! Keys, mouse and timeout for the display panes labels. See
//! [`crate::app::display_panes`] for what the labels show.

use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};

use super::modal::leave_modal;
use crate::app::{
    state::{AppState, OverlayKind},
    App,
};

impl App {
    /// A digit naming a shown pane focuses it; every key closes the labels and
    /// is consumed, as in tmux.
    pub(crate) fn handle_display_panes_key(&mut self, key: KeyEvent) {
        let target = match key.code {
            KeyCode::Char(digit) if key.modifiers.difference(KeyModifiers::SHIFT).is_empty() => {
                digit
                    .to_digit(10)
                    .and_then(|index| self.state.display_panes_target(index as usize))
            }
            _ => None,
        };
        close_display_panes(&mut self.state);
        if let Some((ws_idx, pane_id)) = target {
            self.focus_pane_internal_via_api(ws_idx, pane_id);
        }
    }

    /// Mouse input while the labels are up closes them on a press and is
    /// otherwise swallowed, so a click cannot land on a pane under a label.
    pub(super) fn handle_display_panes_mouse(&mut self, mouse: MouseEvent) -> bool {
        if matches!(mouse.kind, MouseEventKind::Down(_)) {
            close_display_panes(&mut self.state);
        }
        true
    }

    /// Close the labels once their deadline has passed. Called from both the
    /// local and the headless loop's timer pass; `true` when something changed.
    pub(crate) fn expire_display_panes(&mut self, now: Instant) -> bool {
        if self
            .state
            .display_panes_deadline()
            .is_none_or(|deadline| now < deadline)
        {
            return false;
        }
        close_display_panes(&mut self.state);
        true
    }
}

fn close_display_panes(state: &mut AppState) {
    if state.display_panes().is_none() {
        return;
    }
    state.close_overlay(OverlayKind::DisplayPanes);
    leave_modal(state);
}

#[cfg(test)]
mod tests {
    use crossterm::event::MouseButton;
    use ratatui::layout::Direction;

    use super::*;
    use crate::{
        app::state::Mode, config::Config, input::TerminalKey, layout::PaneId, workspace::Workspace,
    };

    fn app_with_two_panes() -> (App, PaneId, PaneId) {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &Config::default(),
            true,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.workspaces = vec![Workspace::test_new("one")];
        let left = app.state.workspaces[0].tabs[0].root_pane;
        let right = app.state.workspaces[0].test_split(Direction::Horizontal);
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        app.state.update_available = None;
        app.state.latest_release_notes_available = false;
        crate::ui::test_support::layout(&mut app.state);
        (app, left, right)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    fn focused(app: &App) -> Option<PaneId> {
        app.state.workspaces[0].focused_pane_id()
    }

    #[test]
    fn prefix_i_opens_the_labels() {
        let (mut app, _, _) = app_with_two_panes();
        app.state.mode = Mode::Prefix;

        app.handle_prefix_key(TerminalKey::new(KeyCode::Char('i'), KeyModifiers::empty()));

        assert_eq!(app.state.mode, Mode::DisplayPanes);
        assert!(app.state.display_panes().is_some());
        app.state.assert_invariants_for_test();
    }

    #[test]
    fn a_digit_focuses_the_pane_it_names_and_closes() {
        let (mut app, left, right) = app_with_two_panes();
        assert_eq!(focused(&app), Some(right));
        app.state.open_display_panes(Instant::now());

        app.handle_display_panes_key(key(KeyCode::Char('1')));

        assert_eq!(focused(&app), Some(left));
        assert_eq!(app.state.mode, Mode::Terminal);
        assert!(app.state.display_panes().is_none());
        app.state.assert_invariants_for_test();
    }

    #[test]
    fn any_other_key_closes_without_moving_focus() {
        let (mut app, _, right) = app_with_two_panes();
        app.state.open_display_panes(Instant::now());

        app.handle_display_panes_key(key(KeyCode::Char('x')));

        assert_eq!(focused(&app), Some(right));
        assert_eq!(app.state.mode, Mode::Terminal);
        assert!(app.state.display_panes().is_none());
        app.state.assert_invariants_for_test();
    }

    #[test]
    fn a_digit_past_the_last_pane_just_closes() {
        let (mut app, _, right) = app_with_two_panes();
        app.state.open_display_panes(Instant::now());

        app.handle_display_panes_key(key(KeyCode::Char('9')));

        assert_eq!(focused(&app), Some(right));
        assert_eq!(app.state.mode, Mode::Terminal);
    }

    #[test]
    fn the_labels_close_at_their_deadline_and_not_before() {
        let (mut app, _, _) = app_with_two_panes();
        let opened = Instant::now();
        app.state.open_display_panes(opened);

        let deadline = opened + app.state.display_panes_duration;
        assert!(
            app.next_loop_deadline(opened, false)
                .is_some_and(|wake| wake <= deadline),
            "the loop wakes up in time to close the labels"
        );
        assert!(!app.expire_display_panes(deadline - std::time::Duration::from_millis(1)));
        assert_eq!(app.state.mode, Mode::DisplayPanes);

        assert!(app.expire_display_panes(deadline));
        assert_eq!(app.state.mode, Mode::Terminal);
        assert!(app.state.display_panes().is_none());
        assert!(!app.expire_display_panes(deadline), "nothing left to close");
    }

    #[test]
    fn expiry_leaves_another_overlay_alone() {
        let (mut app, _, _) = app_with_two_panes();
        app.state.open_display_panes(Instant::now());
        app.state.open_notification_center();

        assert!(!app.expire_display_panes(Instant::now() + app.state.display_panes_duration));
        assert_eq!(
            app.state.overlay.as_ref().map(|open| open.kind()),
            Some(OverlayKind::NotificationCenter)
        );
    }

    #[test]
    fn a_mouse_press_closes_the_labels_and_goes_no_further() {
        let (mut app, left, right) = app_with_two_panes();
        app.state.open_display_panes(Instant::now());
        let left_rect = app.state.view.pane_infos[0].inner_rect;

        app.handle_mouse(crate::app::input::mouse(
            MouseEventKind::Down(MouseButton::Left),
            left_rect.x + 1,
            left_rect.y + 1,
        ));

        assert_eq!(app.state.mode, Mode::Terminal);
        assert!(app.state.display_panes().is_none());
        assert_eq!(
            focused(&app),
            Some(right),
            "the press did not reach {left:?}"
        );
    }
}
