//! herdr's agent status mirrored into herdr-core as OSC 7501, so clients of
//! core (Herdr Mobile's Inbox) see it (openspec/changes/core-status-mirror,
//! D1). One way only: herdr never reads core's status (security review M3).

use crate::api::schema::AgentStatus;
use crate::pane::{CoreStatusApp, CoreStatusRecord, CoreStatusWord};

use super::api_helpers::pane_agent_status;
use super::App;

/// The record for an agent status; no agent or an unknown status clears it.
fn core_status_record(status: AgentStatus, agent: Option<&str>) -> CoreStatusRecord {
    let word = match (status, agent) {
        (_, None) | (AgentStatus::Unknown, _) => return CoreStatusRecord::Clear,
        (AgentStatus::Idle, _) => CoreStatusWord::Idle,
        (AgentStatus::Working, _) => CoreStatusWord::Working,
        (AgentStatus::Blocked, _) => CoreStatusWord::Blocked,
        (AgentStatus::Done, _) => CoreStatusWord::Done,
    };
    CoreStatusRecord::Report {
        word,
        app: agent.and_then(CoreStatusApp::parse),
    }
}

impl App {
    /// `[experimental] core_status_mirror`, at startup and on reload. Turning
    /// it on mirrors every pane's current status; turning it off clears them.
    pub(crate) fn set_core_status_mirror(&mut self, enabled: bool) {
        if self.core_status_mirror != enabled {
            self.core_status_mirror = enabled;
            self.sync_core_status_mirror();
        }
    }

    /// Hands every pane its current record (or `clear` when the mirror is
    /// off). PTY panes ignore it; core panes skip what they already wrote.
    pub(crate) fn sync_core_status_mirror(&self) {
        let records: Vec<_> = (0..self.state.workspaces.len())
            .flat_map(|ws_idx| self.core_status_records(ws_idx))
            .collect();
        for (ws_idx, pane_id, record) in records {
            self.send_core_status(ws_idx, pane_id, record);
        }
    }

    /// Once per server loop pass: when what a person sees or has read may
    /// have changed, every pane offers its record again, so a `done` pane read
    /// in herdr turns `idle` in core without an agent state change
    /// (openspec/changes/core-phase4, D6). Repeats of the record a pane last
    /// wrote cost one comparison.
    /// Returns whether the view was dirty, for the label sync's look
    /// (core-phase4 D7).
    pub(crate) fn sync_core_view(&mut self) -> bool {
        let dirty = std::mem::take(&mut self.state.core_view_dirty);
        if dirty && self.core_status_mirror {
            self.sync_core_status_mirror();
        }
        dirty
    }

    /// After a pane's agent state changed.
    pub(super) fn mirror_pane_core_status(&self, ws_idx: usize, pane_id: crate::layout::PaneId) {
        if !self.core_status_mirror {
            return;
        }
        if let Some(record) = self.pane_core_status_record(ws_idx, pane_id) {
            self.send_core_status(ws_idx, pane_id, record);
        }
    }

    fn core_status_records(
        &self,
        ws_idx: usize,
    ) -> Vec<(usize, crate::layout::PaneId, CoreStatusRecord)> {
        let Some(ws) = self.state.workspaces.get(ws_idx) else {
            return Vec::new();
        };
        ws.tabs
            .iter()
            .flat_map(|tab| tab.panes.keys().copied())
            .filter_map(|pane_id| {
                let record = if self.core_status_mirror {
                    self.pane_core_status_record(ws_idx, pane_id)?
                } else {
                    CoreStatusRecord::Clear
                };
                Some((ws_idx, pane_id, record))
            })
            .collect()
    }

    fn pane_core_status_record(
        &self,
        ws_idx: usize,
        pane_id: crate::layout::PaneId,
    ) -> Option<CoreStatusRecord> {
        let pane = self.state.workspaces.get(ws_idx)?.pane_state(pane_id)?;
        let terminal = self.state.terminals.get(&pane.attached_terminal_id)?;
        Some(core_status_record(
            pane_agent_status(terminal.state, pane.seen),
            terminal.effective_agent_label(),
        ))
    }

    fn send_core_status(
        &self,
        ws_idx: usize,
        pane_id: crate::layout::PaneId,
        record: CoreStatusRecord,
    ) {
        if let Some(runtime) =
            self.state
                .runtime_for_pane_in_workspace(&self.terminal_runtimes, ws_idx, pane_id)
        {
            runtime.mirror_core_status(record);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(word: CoreStatusWord, app: &str) -> CoreStatusRecord {
        CoreStatusRecord::Report {
            word,
            app: CoreStatusApp::parse(app),
        }
    }

    #[test]
    fn agent_statuses_map_to_cores_words() {
        let claude = Some("claude");
        assert_eq!(
            core_status_record(AgentStatus::Working, claude),
            report(CoreStatusWord::Working, "claude")
        );
        assert_eq!(
            core_status_record(AgentStatus::Blocked, claude),
            report(CoreStatusWord::Blocked, "claude")
        );
        assert_eq!(
            core_status_record(AgentStatus::Done, claude),
            report(CoreStatusWord::Done, "claude")
        );
        assert_eq!(
            core_status_record(AgentStatus::Idle, claude),
            report(CoreStatusWord::Idle, "claude")
        );
        assert_eq!(
            core_status_record(AgentStatus::Unknown, claude),
            CoreStatusRecord::Clear
        );
        for status in [AgentStatus::Working, AgentStatus::Idle, AgentStatus::Done] {
            assert_eq!(
                core_status_record(status, None),
                CoreStatusRecord::Clear,
                "no agent (released or never detected): {status:?}"
            );
        }
        assert_eq!(
            core_status_record(AgentStatus::Working, Some("bad:label")),
            CoreStatusRecord::Report {
                word: CoreStatusWord::Working,
                app: None
            }
        );
    }

    fn app_with_agent_pane() -> (App, crate::layout::PaneId) {
        let mut app = App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            tokio::sync::mpsc::unbounded_channel().1,
            crate::api::EventHub::default(),
        );
        let ws = crate::workspace::Workspace::test_new("test");
        let pane_id = ws.tabs[0].root_pane;
        let terminal_id = ws.tabs[0].panes[&pane_id].attached_terminal_id.clone();
        let mut terminal = crate::terminal::TerminalState::new(terminal_id.clone(), "/tmp".into());
        terminal.set_detected_state(
            Some(crate::detect::Agent::Claude),
            crate::detect::AgentState::Working,
        );
        app.state.terminals.insert(terminal_id, terminal);
        app.state.workspaces.push(ws);
        app.state.active = Some(0);
        (app, pane_id)
    }

    // core-phase4 verifier 4 (unit): reading a done pane asks for a new
    // pass, and the pass hands the pane `idle`.
    #[test]
    fn reading_a_done_pane_offers_idle_on_the_next_pass() {
        let (mut app, pane_id) = app_with_agent_pane();
        app.set_core_status_mirror(true);
        let terminal_id = app.state.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&terminal_id)
            .unwrap()
            .set_detected_state(
                Some(crate::detect::Agent::Claude),
                crate::detect::AgentState::Idle,
            );
        app.state.workspaces[0].tabs[0]
            .panes
            .get_mut(&pane_id)
            .unwrap()
            .seen = false;
        app.state.core_view_dirty = false;
        assert_eq!(
            app.core_status_records(0),
            vec![(0, pane_id, report(CoreStatusWord::Done, "claude"))]
        );

        assert!(app.state.mark_active_tab_seen());
        assert!(app.state.core_view_dirty, "reading asks for a pass");
        app.sync_core_view();
        assert!(!app.state.core_view_dirty, "the pass takes the flag");
        assert_eq!(
            app.core_status_records(0),
            vec![(0, pane_id, report(CoreStatusWord::Idle, "claude"))]
        );
    }

    #[test]
    fn the_mirror_hands_out_current_status_when_on_and_clears_when_off() {
        let (mut app, pane_id) = app_with_agent_pane();
        assert!(!app.core_status_mirror, "off by default");
        assert_eq!(
            app.core_status_records(0),
            vec![(0, pane_id, CoreStatusRecord::Clear)],
            "off: a sync clears"
        );
        app.set_core_status_mirror(true);
        assert_eq!(
            app.core_status_records(0),
            vec![(0, pane_id, report(CoreStatusWord::Working, "claude"))]
        );
        assert!(app.core_status_records(7).is_empty(), "no such workspace");
    }
}
