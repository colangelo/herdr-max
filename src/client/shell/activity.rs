use super::*;
use crate::api::schema::AgentStatus;

impl ClientShellState {
    pub(crate) fn tick_activity(&mut self, now: std::time::Instant) -> bool {
        let local_working = self.hits.agents.iter().any(|(_, id)| {
            self.snapshot.as_deref().is_some_and(|snapshot| {
                snapshot
                    .agents
                    .iter()
                    .any(|agent| agent.pane_id == *id && agent.agent_status == AgentStatus::Working)
            })
        });
        let remote_working = self.hits.endpoint_agents.iter().any(|(_, endpoint, id)| {
            self.endpoints
                .iter()
                .find(|e| &e.endpoint_id == endpoint && e.status == ClientEndpointStatus::Online)
                .and_then(|e| e.snapshot.as_deref())
                .is_some_and(|snapshot| {
                    snapshot.agents.iter().any(|agent| {
                        agent.pane_id == *id && agent.agent_status == AgentStatus::Working
                    })
                })
        });
        let working = self.config.status_spinner == crate::config::StatusSpinnerConfig::On
            && (local_working || remote_working);
        if !working {
            self.activity_deadline = None;
            return false;
        }
        let Some(deadline) = self.activity_deadline else {
            self.activity_deadline = Some(now + self.config.status_spinner_interval);
            return false;
        };
        if now < deadline {
            return false;
        }
        self.config.spinner_frame = self.config.spinner_frame.wrapping_add(1);
        self.activity_deadline = Some(now + self.config.status_spinner_interval);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spinner_never_ticks_without_visible_rows_or_when_disabled() {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        let now = std::time::Instant::now();
        assert!(!state.tick_activity(now));
        assert!(state.activity_deadline.is_none());
        state.config.status_spinner = crate::config::StatusSpinnerConfig::Off;
        assert!(!state.tick_activity(now + std::time::Duration::from_secs(1)));
        assert_eq!(state.config.spinner_frame, 0);
    }
    #[test]
    fn one_visible_working_row_arms_one_shared_tick_and_hiding_it_disarms() {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        let mut snapshot = super::super::tests::snapshot();
        let agent:crate::protocol::ClientShellAgent=serde_json::from_value(serde_json::json!({
            "pane_id":"pane_1","workspace_id":"ws_1","tab_id":"tab_1","name":null,"display_agent":null,"agent":null,"title":null,
            "terminal_title":null,"terminal_title_stripped":null,"agent_status":"working","state_change_seq":0,"state_labels":[],"tokens":[],"focused":true
        })).unwrap();
        snapshot.agents.push(agent);
        state.set_snapshot(Box::new(snapshot));
        state
            .hits
            .agents
            .push((Rect::new(0, 0, 10, 1), "pane_1".into()));
        let now = std::time::Instant::now();
        assert!(!state.tick_activity(now));
        assert!(state.tick_activity(now + state.config.status_spinner_interval));
        assert_eq!(state.config.spinner_frame, 1);
        state.hits.agents.clear();
        assert!(!state.tick_activity(now + std::time::Duration::from_secs(1)));
        assert!(state.activity_deadline.is_none());
    }
}
