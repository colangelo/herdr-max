#[cfg(unix)]
use serde::{Deserialize, Serialize};

/// Long-lived pane runtime transferred during server replacement.
///
/// Handoff preserves server-owned session state such as PTYs, processes, agent
/// identity, and durable plugin/session metadata. It intentionally does not
/// preserve transient coordination such as in-flight requests, waits,
/// subscriptions, client sockets, or pane-to-pane messages; clients reconnect
/// and retry those operations after replacement.
#[cfg(unix)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct HandoffRuntimeState {
    pub pane_id: u32,
    pub child_pid: u32,
    pub rows: u16,
    pub cols: u16,
    pub cell_width_px: u32,
    pub cell_height_px: u32,
    #[serde(default)]
    pub keyboard_protocol_flags: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyboard_protocol_ansi: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_state: Option<crate::pane::InputState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_history_ansi: Option<String>,
    /// Canonical label of the agent detected in the pane before the handoff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Live agent state before the handoff. Only carried for non-idle states;
    /// absent means the receiving server seeds `Idle`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_state: Option<String>,
    /// The hook-reported agent status before the handoff, so the receiving
    /// server keeps it until the next report. Named apart from `agent_state`,
    /// which older fork servers write as a label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_agent_state: Option<crate::terminal::state::HandoffAgentState>,
    /// The pane finished and nobody has looked at it yet ("done"). Written
    /// only when true, so older servers read the manifest unchanged (fork
    /// issue 128).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unseen: bool,
}

#[cfg(unix)]
impl HandoffRuntimeState {
    pub fn with_pane_id(mut self, pane_id: crate::layout::PaneId) -> Self {
        self.pane_id = pane_id.raw();
        self
    }

    /// Pre-restart agent identity and state to seed detection with after import.
    /// `None` when the manifest predates the field or carries no known agent.
    pub fn agent_seed(&self) -> Option<(crate::detect::Agent, crate::detect::AgentState)> {
        let agent = crate::detect::parse_agent_label(self.agent.as_deref()?)?;
        let state = self
            .agent_state
            .as_deref()
            .and_then(parse_handoff_agent_state)
            .unwrap_or(crate::detect::AgentState::Idle);
        Some((agent, state))
    }
}

/// Manifest label for a live agent state; `None` for states that should not be
/// carried across the handoff (Idle is the default seed, Unknown means no agent).
#[cfg(unix)]
pub(crate) fn handoff_agent_state_label(state: crate::detect::AgentState) -> Option<&'static str> {
    match state {
        crate::detect::AgentState::Working => Some("working"),
        crate::detect::AgentState::Blocked => Some("blocked"),
        crate::detect::AgentState::Idle | crate::detect::AgentState::Unknown => None,
    }
}

#[cfg(unix)]
fn parse_handoff_agent_state(label: &str) -> Option<crate::detect::AgentState> {
    match label {
        "working" => Some(crate::detect::AgentState::Working),
        "blocked" => Some(crate::detect::AgentState::Blocked),
        "idle" => Some(crate::detect::AgentState::Idle),
        _ => None,
    }
}

#[derive(Debug)]
pub(crate) struct ImportedHandoffRuntime {
    #[cfg(unix)]
    pub master_fd: std::os::fd::RawFd,
    #[cfg(unix)]
    pub state: HandoffRuntimeState,
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::detect::{Agent, AgentState};

    fn base_state() -> HandoffRuntimeState {
        HandoffRuntimeState {
            pane_id: 7,
            child_pid: 4242,
            rows: 40,
            cols: 120,
            cell_width_px: 10,
            cell_height_px: 20,
            keyboard_protocol_flags: 0,
            keyboard_protocol_ansi: None,
            input_state: None,
            terminal_title: None,
            initial_history_ansi: None,
            agent: None,
            agent_state: None,
            hook_agent_state: None,
            unseen: false,
        }
    }

    #[test]
    fn manifest_roundtrips_agent_state() {
        let mut state = base_state();
        state.agent = Some("claude".into());
        state.agent_state = Some("working".into());
        let encoded = serde_json::to_string(&state).expect("serialize");
        let decoded: HandoffRuntimeState = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(decoded.agent.as_deref(), Some("claude"));
        assert_eq!(decoded.agent_state.as_deref(), Some("working"));
        assert_eq!(
            decoded.agent_seed(),
            Some((Agent::Claude, AgentState::Working))
        );
    }

    #[test]
    fn manifest_without_agent_fields_deserializes_with_no_seed() {
        // A manifest produced by an older server has no agent fields; the
        // receiving server must fall back to the pre-change Idle seeding.
        let encoded = serde_json::json!({
            "pane_id": 7,
            "child_pid": 4242,
            "rows": 40,
            "cols": 120,
            "cell_width_px": 10,
            "cell_height_px": 20,
        })
        .to_string();
        let decoded: HandoffRuntimeState = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(decoded.agent, None);
        assert_eq!(decoded.agent_state, None);
        assert_eq!(decoded.agent_seed(), None);
    }

    #[test]
    fn idle_and_unknown_states_are_not_carried() {
        assert_eq!(
            handoff_agent_state_label(AgentState::Working),
            Some("working")
        );
        assert_eq!(
            handoff_agent_state_label(AgentState::Blocked),
            Some("blocked")
        );
        assert_eq!(handoff_agent_state_label(AgentState::Idle), None);
        assert_eq!(handoff_agent_state_label(AgentState::Unknown), None);
    }

    #[test]
    fn agent_seed_defaults_missing_or_unrecognized_state_to_idle() {
        let mut state = base_state();
        state.agent = Some("codex".into());
        assert_eq!(state.agent_seed(), Some((Agent::Codex, AgentState::Idle)));
        state.agent_state = Some("mystery-state".into());
        assert_eq!(state.agent_seed(), Some((Agent::Codex, AgentState::Idle)));
        state.agent = Some("not-an-agent".into());
        assert_eq!(state.agent_seed(), None);
    }
}
