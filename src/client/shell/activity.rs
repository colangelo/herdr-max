use super::*;
use crate::api::schema::AgentStatus;

/// Whether the pane is listed with background items in `snapshot`'s facts.
fn has_background(snapshot: &ClientShellSnapshot, pane_id: &str) -> bool {
    super::state_presentation::Background::from_snapshot(snapshot, pane_id).count > 0
}

impl ClientShellState {
    /// Steps the shared working spinner. The client loop calls this on every
    /// wake, so an armed tick that is not yet due returns before scanning the
    /// visible rows; a row that disappears disarms the tick at its deadline.
    pub(crate) fn tick_activity(&mut self, now: std::time::Instant) -> bool {
        if self.config.status_spinner != crate::config::StatusSpinnerConfig::On {
            self.activity_deadline = None;
            return false;
        }
        if self
            .activity_deadline
            .is_some_and(|deadline| now < deadline)
        {
            return false;
        }
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
        // The braille mark moves on the same tick, on idle and done rows too.
        let braille = self.config.state_presentation.background_mark()
            == crate::config::BackgroundMarkConfig::Braille;
        let local_background = braille
            && self.hits.agents.iter().any(|(_, id)| {
                self.snapshot
                    .as_deref()
                    .is_some_and(|snapshot| has_background(snapshot, id))
            });
        let remote_background = braille
            && self.hits.endpoint_agents.iter().any(|(_, endpoint, id)| {
                self.endpoints
                    .iter()
                    .find(|e| {
                        &e.endpoint_id == endpoint && e.status == ClientEndpointStatus::Online
                    })
                    .and_then(|e| e.snapshot.as_deref())
                    .is_some_and(|snapshot| has_background(snapshot, id))
            });
        if !(local_working || remote_working || local_background || remote_background) {
            self.activity_deadline = None;
            return false;
        }
        if self.activity_deadline.is_none() {
            self.activity_deadline = Some(now + self.config.status_spinner_interval);
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
    fn state_with_visible_working_row(config: &Config) -> ClientShellState {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(config));
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
        state
    }
    #[test]
    fn one_visible_working_row_arms_one_shared_tick_and_hiding_it_disarms() {
        let mut state = state_with_visible_working_row(&Config::default());
        let now = std::time::Instant::now();
        assert!(!state.tick_activity(now));
        assert!(state.tick_activity(now + state.config.status_spinner_interval));
        assert_eq!(state.config.spinner_frame, 1);
        state.hits.agents.clear();
        assert!(!state.tick_activity(now + std::time::Duration::from_secs(1)));
        assert!(state.activity_deadline.is_none());
    }
    #[test]
    fn an_armed_tick_waits_for_its_deadline_and_turning_the_spinner_off_disarms_it() {
        let mut state = state_with_visible_working_row(&Config::default());
        let interval = state.config.status_spinner_interval;
        let now = std::time::Instant::now();
        assert!(!state.tick_activity(now));
        // Early wakes keep the armed deadline, even once the row is gone; the
        // row check runs when the tick is due.
        state.hits.agents.clear();
        assert!(!state.tick_activity(now + interval / 2));
        assert_eq!(state.activity_deadline, Some(now + interval));
        assert!(!state.tick_activity(now + interval));
        assert!(state.activity_deadline.is_none());
        assert_eq!(state.config.spinner_frame, 0);

        let mut state = state_with_visible_working_row(&Config::default());
        assert!(!state.tick_activity(now));
        state.config.status_spinner = crate::config::StatusSpinnerConfig::Off;
        assert!(!state.tick_activity(now + interval / 2));
        assert!(state.activity_deadline.is_none());
        assert_eq!(
            state.timer_delay(now),
            std::time::Duration::from_millis(100)
        );
    }
    #[test]
    fn spinner_frame_wraps_and_the_interval_is_clamped_from_config() {
        let mut state = state_with_visible_working_row(&Config::default());
        state.config.spinner_frame = u8::MAX;
        let now = std::time::Instant::now();
        assert!(!state.tick_activity(now));
        assert!(state.tick_activity(now + state.config.status_spinner_interval));
        assert_eq!(state.config.spinner_frame, 0);

        let interval = |ms| {
            let mut config = Config::default();
            config.ui.status_spinner_ms = ms;
            ClientShellConfig::from_config(&config).status_spinner_interval
        };
        assert_eq!(
            interval(1),
            std::time::Duration::from_millis(crate::config::MIN_STATUS_SPINNER_MS)
        );
        assert_eq!(
            interval(10_000),
            std::time::Duration::from_millis(crate::config::MAX_STATUS_SPINNER_MS)
        );
        assert_eq!(interval(333), std::time::Duration::from_millis(333));
    }
    #[test]
    fn an_idle_row_with_background_work_ticks_only_in_braille_mode() {
        let idle: crate::protocol::ClientShellAgent = serde_json::from_value(serde_json::json!({
            "pane_id":"pane_1","workspace_id":"ws_1","tab_id":"tab_1","name":null,"display_agent":null,"agent":null,"title":null,
            "terminal_title":null,"terminal_title_stripped":null,"agent_status":"idle","state_change_seq":0,"state_labels":[],"tokens":[],"focused":true
        })).unwrap();
        for (mark, ticks) in [
            (crate::config::BackgroundMarkConfig::Frames, false),
            (crate::config::BackgroundMarkConfig::Braille, true),
        ] {
            let mut config = Config::default();
            config.ui.background_mark = mark;
            let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
            let mut snapshot = super::super::tests::snapshot();
            snapshot.agents.push(idle.clone());
            snapshot.resource_facts = Some(crate::protocol::ClientShellResourceFacts {
                background_count: Some(std::collections::BTreeMap::from([(
                    "pane_1".to_owned(),
                    2,
                )])),
                ..Default::default()
            });
            state.set_snapshot(Box::new(snapshot));
            state
                .hits
                .agents
                .push((Rect::new(0, 0, 10, 1), "pane_1".into()));
            let now = std::time::Instant::now();
            assert!(!state.tick_activity(now));
            assert_eq!(
                state.tick_activity(now + state.config.status_spinner_interval),
                ticks,
                "{mark:?}"
            );
        }
    }
}
