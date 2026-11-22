//! The server side of `herdr.seen` sharing (openspec/changes/core-phase4,
//! D8 and D11): facts from the label thread go through the reducer, and its
//! writes go back to core. Unix only, like the core backend.

use std::collections::HashSet;

use super::core_labels::{CoreAddress, CoreFact, CoreLabels, LabelWrite};
use super::App;
use crate::layout::PaneId;
use crate::terminal::TerminalId;

impl App {
    /// `[experimental] core_label_sync`, at startup and on reload.
    pub(crate) fn set_core_label_sync(&mut self, enabled: bool) {
        if self.core_labels.is_some() == enabled {
            return;
        }
        self.core_labels = enabled.then(CoreLabels::default);
        #[cfg(not(test))]
        crate::pane::set_core_label_sync(enabled, self.event_tx.clone());
    }

    pub(crate) fn core_label_sync(&self) -> bool {
        self.core_labels.is_some()
    }

    pub(super) fn handle_core_labels(&mut self, fact: CoreFact) {
        let Some(labels) = self.core_labels.as_mut() else {
            return;
        };
        let writes = labels.apply(fact);
        self.send_label_writes(writes);
    }

    /// The core panes some person is looking at now (D7).
    pub(crate) fn look_core_labels(&mut self, on_screen: HashSet<CoreAddress>) {
        let Some(labels) = self.core_labels.as_mut() else {
            return;
        };
        let writes = labels.look(on_screen);
        self.send_label_writes(writes);
    }

    /// `pane.mark_seen` marked these panes read: raise their labels (D8).
    pub(super) fn core_labels_marked_seen(&mut self, panes: &[(usize, PaneId)]) {
        if self.core_labels.is_none() {
            return;
        }
        let addresses: Vec<_> = panes
            .iter()
            .filter_map(|&(ws_idx, pane_id)| {
                let pane = self.state.workspaces.get(ws_idx)?.pane_state(pane_id)?;
                self.primary_core_address(&pane.attached_terminal_id)
            })
            .collect();
        let Some(labels) = self.core_labels.as_mut() else {
            return;
        };
        let writes: Vec<_> = addresses
            .iter()
            .filter_map(|address| labels.herdr_marked_seen(address))
            .collect();
        self.send_label_writes(writes);
    }

    /// The panes `mark_panes_seen(scope)` would mark read.
    pub(super) fn unread_panes(&self, scope: Option<usize>) -> Vec<(usize, PaneId)> {
        self.state
            .workspaces
            .iter()
            .enumerate()
            .filter(|(ws_idx, _)| scope.is_none_or(|scope| scope == *ws_idx))
            .flat_map(|(ws_idx, ws)| {
                ws.tabs
                    .iter()
                    .flat_map(|tab| tab.panes.iter())
                    .filter(|(_, pane)| !pane.seen)
                    .map(move |(pane_id, _)| (ws_idx, *pane_id))
            })
            .collect()
    }

    /// The terminal's address in this server's primary core session; `None`
    /// for a PTY pane or one in another core session.
    pub(crate) fn primary_core_address(&self, terminal_id: &TerminalId) -> Option<CoreAddress> {
        let link = self.terminal_runtimes.get(terminal_id)?.core_terminal()?;
        (link.session == crate::pane::core_session_name()).then_some(link.address)
    }

    fn send_label_writes(&self, writes: Vec<LabelWrite>) {
        for write in writes {
            (self.core_label_write)(write);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pane::Observed;

    thread_local! {
        static WRITTEN: std::cell::RefCell<Vec<LabelWrite>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    fn record(write: LabelWrite) {
        WRITTEN.with(|written| written.borrow_mut().push(write));
    }

    fn written() -> Vec<LabelWrite> {
        WRITTEN.with(|written| std::mem::take(&mut *written.borrow_mut()))
    }

    fn app() -> App {
        let mut app = App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            tokio::sync::mpsc::unbounded_channel().1,
            crate::api::EventHub::default(),
        );
        app.core_label_write = record;
        app
    }

    #[test]
    fn off_by_default_and_facts_are_ignored_while_off() {
        let mut app = app();
        assert!(!app.core_label_sync());
        app.handle_core_labels(CoreFact::Snapshot(vec![(
            "t1".into(),
            Observed {
                turn: 2,
                seen: None,
            },
        )]));
        app.look_core_labels(["t1".to_string()].into_iter().collect());
        assert!(written().is_empty());
    }

    #[test]
    fn on_the_reducer_writes_through_the_label_thread() {
        let mut app = app();
        app.set_core_label_sync(true);
        app.handle_core_labels(CoreFact::Snapshot(vec![(
            "t1".into(),
            Observed {
                turn: 2,
                seen: Some(1),
            },
        )]));
        app.look_core_labels(["t1".to_string()].into_iter().collect());
        assert_eq!(
            written(),
            vec![LabelWrite::Seen {
                address: "t1".into(),
                turn: 2
            }]
        );
        app.set_core_label_sync(false);
        assert!(!app.core_label_sync());
    }
}
