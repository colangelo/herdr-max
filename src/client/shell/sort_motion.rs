//! Bubble motion for the priority-sorted sidebar lists (`ui.sort_motion*`).
//!
//! Fork-only cosmetic: a re-sorted row holds its place for the settle delay,
//! then travels one slot per step instead of teleporting. Motion is client
//! state. Each endpoint boot has its own engine for the space list (keyed by
//! whole worktree units, so a group never splits) and for the agent panel;
//! the federated agent list has one engine keyed by boot and pane. Engines
//! advance only in [`ClientShellState::tick_sort_motion`]; every reader
//! (render, hit-testing, jump numbers, navigation) goes through the pure
//! projections here, so they agree on one order between ticks. A tick never
//! touches the endpoint: it only asks the client to recompose its chrome.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use super::aggregate_navigation::AggregateAgentRow;
use super::list_motion::{ListMotion, ListMotionEasing, ListMotionTiming};
use super::*;

#[derive(Debug, Default)]
pub(crate) struct SortMotion {
    /// `None` when `ui.sort_motion = "instant"`: lists re-sort at once.
    timing: Option<ListMotionTiming>,
    /// Space-list engines by endpoint boot id.
    workspaces: HashMap<String, ListMotion<String>>,
    /// Agent-panel engines by endpoint boot id.
    agents: HashMap<String, ListMotion<String>>,
    /// The federated agent list, keyed by (boot id, pane id).
    aggregate: ListMotion<(String, String)>,
    /// When the next step is due, for the client timer.
    pub(super) deadline: Option<Instant>,
}

fn timing_from_config(config: &Config) -> Option<ListMotionTiming> {
    (config.ui.sort_motion == crate::config::SortMotionConfig::Bubble).then(|| ListMotionTiming {
        settle: Duration::from_millis(config.ui.sort_motion_settle_ms),
        step: Duration::from_millis(config.ui.sort_motion_step_ms.max(1)),
        easing: match config.ui.sort_motion_easing {
            crate::config::SortMotionEasingConfig::Linear => ListMotionEasing::Linear,
            crate::config::SortMotionEasingConfig::Bubble => ListMotionEasing::Bubble,
        },
    })
}

pub(super) fn workspace_priority_sort(snapshot: &ClientShellSnapshot) -> bool {
    snapshot
        .resource_facts
        .as_ref()
        .and_then(|facts| facts.workspace_sort.as_deref())
        == Some("priority")
}

/// The motion key of each contiguous unit in `entries`: a worktree group
/// moves as one unit, every other space on its own.
fn workspace_units(
    snapshot: &ClientShellSnapshot,
    grouped: &HashSet<&str>,
    entries: &[WorkspaceEntry],
) -> Vec<(String, std::ops::Range<usize>)> {
    let mut units: Vec<(String, std::ops::Range<usize>)> = Vec::new();
    for (position, entry) in entries.iter().enumerate() {
        let workspace = &snapshot.workspaces[entry.index];
        let group = workspace
            .worktree
            .as_ref()
            .map(|worktree| worktree.key.as_str())
            .filter(|key| grouped.contains(key));
        if let (Some(group), Some((key, range))) = (group, units.last_mut()) {
            if key.strip_prefix("space:") == Some(group) {
                range.end = position + 1;
                continue;
            }
        }
        let key = match group {
            Some(group) => format!("space:{group}"),
            None => format!("ws:{}", workspace.workspace_id),
        };
        units.push((key, position..position + 1));
    }
    units
}

impl SortMotion {
    pub(super) fn from_config(config: &Config) -> Self {
        Self {
            timing: timing_from_config(config),
            ..Self::default()
        }
    }

    /// Switching to `instant` drops every engine, so the lists snap to their
    /// sorted order; a timing change keeps rows already in flight.
    pub(super) fn apply_live_config(&mut self, config: &Config) {
        match timing_from_config(config) {
            Some(timing) => self.timing = Some(timing),
            None => *self = Self::default(),
        }
    }

    /// Reorders the priority-sorted space list through its displayed order.
    pub(super) fn project_workspaces(
        &self,
        snapshot: &ClientShellSnapshot,
        grouped: &HashSet<&str>,
        entries: &mut Vec<WorkspaceEntry>,
    ) {
        if self.timing.is_none() || !workspace_priority_sort(snapshot) {
            return;
        }
        let Some(engine) = self.workspaces.get(&snapshot.boot_id) else {
            return;
        };
        let units = workspace_units(snapshot, grouped, entries);
        let target = units.iter().map(|(key, _)| key.clone()).collect::<Vec<_>>();
        let order = engine.project(&target);
        if order == target {
            return;
        }
        let projected = order
            .iter()
            .filter_map(|key| units.iter().find(|(unit, _)| unit == key))
            .flat_map(|(_, range)| entries[range.clone()].iter().copied())
            .collect();
        *entries = projected;
    }

    /// Reorders a priority-sorted agent panel through its displayed order.
    pub(super) fn project_agents(&self, boot_id: &str, ids: Vec<String>) -> Vec<String> {
        if self.timing.is_none() {
            return ids;
        }
        match self.agents.get(boot_id) {
            Some(engine) => engine.project(&ids),
            None => ids,
        }
    }

    /// Reorders the priority-sorted federated agent list through its
    /// displayed order.
    pub(super) fn project_aggregate<'a>(
        &self,
        rows: Vec<AggregateAgentRow<'a>>,
    ) -> Vec<AggregateAgentRow<'a>> {
        if self.timing.is_none() {
            return rows;
        }
        let target = aggregate_keys(&rows);
        let order = self.aggregate.project(&target);
        if order == target {
            return rows;
        }
        let mut rows = rows.into_iter().map(Some).collect::<Vec<_>>();
        order
            .iter()
            .filter_map(|key| {
                let position = target.iter().position(|other| other == key)?;
                rows[position].take()
            })
            .collect()
    }
}

fn aggregate_keys(rows: &[AggregateAgentRow<'_>]) -> Vec<(String, String)> {
    rows.iter()
        .map(|row| {
            (
                row.endpoint.snapshot.boot_id.clone(),
                row.agent.pane_id.clone(),
            )
        })
        .collect()
}

fn advance<K: Eq + std::hash::Hash + Clone>(
    engine: &mut ListMotion<K>,
    now: Instant,
    target: &[K],
    timing: ListMotionTiming,
    deadline: &mut Option<Instant>,
) -> bool {
    let before = engine.project(target);
    let changed = engine.tick(now, target, timing) != before.as_slice();
    if let Some(due) = engine.next_due(timing) {
        *deadline = Some(deadline.map_or(due, |current| current.min(due)));
    }
    changed
}

impl ClientShellState {
    /// Advances every sidebar list toward its sorted order. Returns true when
    /// a displayed order moved and the chrome needs recomposing. The client
    /// loop calls this on each timer wake, so a new sort order starts its
    /// settle delay within one wake; the only mutation point for motion.
    pub(crate) fn tick_sort_motion(&mut self, now: Instant) -> bool {
        let Some(timing) = self.config.sort_motion.timing else {
            return false;
        };
        let agent_priority =
            self.config.agent_panel_sort == crate::config::AgentPanelSortConfig::Priority;
        let mut snapshots = Vec::new();
        snapshots.extend(self.snapshot.as_deref());
        for endpoint in &self.endpoints {
            if let Some(snapshot) = endpoint.snapshot.as_deref() {
                if !snapshots.iter().any(|s| s.boot_id == snapshot.boot_id) {
                    snapshots.push(snapshot);
                }
            }
        }
        let mut workspace_targets = Vec::new();
        let mut agent_targets = Vec::new();
        for snapshot in &snapshots {
            if workspace_priority_sort(snapshot) {
                let entries = super::sidebar::workspace_entries(snapshot, &HashSet::new(), None);
                let grouped = super::sidebar::grouped_worktree_keys(snapshot);
                let units = workspace_units(snapshot, &grouped, &entries);
                workspace_targets.push((
                    snapshot.boot_id.clone(),
                    units.into_iter().map(|(key, _)| key).collect::<Vec<_>>(),
                ));
            }
            if agent_priority {
                agent_targets.push((
                    snapshot.boot_id.clone(),
                    super::agent_sidebar::ordered_agent_pane_ids(
                        snapshot,
                        self.config.agent_panel_sort,
                        None,
                    ),
                ));
            }
        }
        let aggregate_target = (agent_priority
            && (self.endpoints.len() > 1 || self.mobile_layout_active()))
        .then(|| {
            aggregate_keys(&super::aggregate_navigation::aggregate_agent_rows(
                &self.endpoints,
                &self.active_endpoint_id,
                self.config.agent_panel_sort,
                None,
            ))
        });

        let motion = &mut self.config.sort_motion;
        motion.deadline = None;
        let mut changed = false;
        motion
            .workspaces
            .retain(|boot, _| workspace_targets.iter().any(|(target, _)| target == boot));
        for (boot, target) in &workspace_targets {
            let engine = motion.workspaces.entry(boot.clone()).or_default();
            changed |= advance(engine, now, target, timing, &mut motion.deadline);
        }
        motion
            .agents
            .retain(|boot, _| agent_targets.iter().any(|(target, _)| target == boot));
        for (boot, target) in &agent_targets {
            let engine = motion.agents.entry(boot.clone()).or_default();
            changed |= advance(engine, now, target, timing, &mut motion.deadline);
        }
        match aggregate_target {
            Some(target) => {
                changed |= advance(
                    &mut motion.aggregate,
                    now,
                    &target,
                    timing,
                    &mut motion.deadline,
                );
            }
            None => motion.aggregate.reset(),
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schema::AgentStatus;

    const SETTLE: Duration = Duration::from_millis(2000);
    const STEP: Duration = Duration::from_millis(150);

    /// ws_1, then "a", then the worktree group "repo" (parent + child), all
    /// idle, under the endpoint's priority space sort.
    fn spaces() -> ClientShellSnapshot {
        let mut snapshot = super::super::tests::snapshot();
        let mut a = snapshot.workspaces[0].clone();
        a.workspace_id = "a".into();
        let mut parent = a.clone();
        parent.workspace_id = "parent".into();
        parent.worktree = Some(crate::protocol::ClientShellWorktree {
            key: "repo".into(),
            label: "repo".into(),
            is_linked_worktree: false,
        });
        let mut child = parent.clone();
        child.workspace_id = "child".into();
        child.worktree.as_mut().unwrap().is_linked_worktree = true;
        snapshot.workspaces.extend([a, parent, child]);
        for workspace in &mut snapshot.workspaces {
            workspace.agent_status = AgentStatus::Idle;
        }
        snapshot.resource_facts = Some(crate::protocol::ClientShellResourceFacts {
            workspace_sort: Some("priority".into()),
            ..Default::default()
        });
        snapshot
    }

    fn with_status(
        mut snapshot: ClientShellSnapshot,
        id: &str,
        status: AgentStatus,
    ) -> ClientShellSnapshot {
        snapshot.revision += 1;
        for workspace in &mut snapshot.workspaces {
            if workspace.workspace_id == id {
                workspace.agent_status = status;
            }
        }
        snapshot
    }

    fn drawn_spaces(state: &ClientShellState) -> Vec<String> {
        let snapshot = state.snapshot.as_deref().unwrap();
        state
            .navigation_workspace_entries(snapshot)
            .iter()
            .map(|entry| snapshot.workspaces[entry.index].workspace_id.clone())
            .collect()
    }

    fn agent(pane: &str, status: &str) -> crate::protocol::ClientShellAgent {
        serde_json::from_value(serde_json::json!({
            "pane_id":pane,"workspace_id":"ws_1","tab_id":"tab_1","name":null,"display_agent":null,"agent":null,"title":null,
            "terminal_title":null,"terminal_title_stripped":null,"agent_status":status,"state_change_seq":0,"state_labels":[],"tokens":[],"focused":false
        }))
        .unwrap()
    }

    fn drawn_agents(state: &ClientShellState) -> Vec<String> {
        super::super::agent_sidebar::ordered_agent_pane_ids(
            state.snapshot.as_deref().unwrap(),
            state.config.agent_panel_sort,
            Some(&state.config.sort_motion),
        )
    }

    #[test]
    fn a_resorted_space_holds_then_bubbles_one_unit_per_step_with_its_group_whole() {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        state.set_snapshot(Box::new(spaces()));
        let t0 = Instant::now();
        assert!(!state.tick_sort_motion(t0));
        assert_eq!(drawn_spaces(&state), ["ws_1", "a", "parent", "child"]);
        assert_eq!(state.config.sort_motion.deadline, None);

        // The linked worktree blocks: the whole group sorts to the top.
        state.set_snapshot(Box::new(with_status(
            spaces(),
            "child",
            AgentStatus::Blocked,
        )));
        assert_eq!(drawn_spaces(&state), ["ws_1", "a", "parent", "child"]);
        assert!(!state.tick_sort_motion(t0));
        assert_eq!(drawn_spaces(&state), ["ws_1", "a", "parent", "child"]);
        assert_eq!(state.config.sort_motion.deadline, Some(t0 + SETTLE));
        assert!(state.timer_delay(t0) <= Duration::from_millis(100));

        assert!(!state.tick_sort_motion(t0 + SETTLE / 2));
        assert!(state.tick_sort_motion(t0 + SETTLE));
        assert_eq!(drawn_spaces(&state), ["ws_1", "parent", "child", "a"]);
        assert_eq!(state.config.sort_motion.deadline, Some(t0 + SETTLE + STEP));
        assert!(!state.tick_sort_motion(t0 + SETTLE + STEP / 2));
        assert!(state.tick_sort_motion(t0 + SETTLE + STEP));
        assert_eq!(drawn_spaces(&state), ["parent", "child", "ws_1", "a"]);
        assert_eq!(state.config.sort_motion.deadline, None);
        let snapshot = state.snapshot.as_deref().unwrap();
        let entries = state.navigation_workspace_entries(snapshot);
        assert_eq!(
            entries.iter().map(|e| e.visible_index).collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
    }

    #[test]
    fn instant_motion_manual_sort_and_reload_to_instant_resort_at_once() {
        let mut config = Config::default();
        config.ui.sort_motion = crate::config::SortMotionConfig::Instant;
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
        state.set_snapshot(Box::new(spaces()));
        let t0 = Instant::now();
        assert!(!state.tick_sort_motion(t0));
        state.set_snapshot(Box::new(with_status(spaces(), "a", AgentStatus::Blocked)));
        assert!(!state.tick_sort_motion(t0));
        assert_eq!(drawn_spaces(&state)[0], "a");
        assert_eq!(state.config.sort_motion.deadline, None);

        // Manual space order never animates and drops its engine.
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        let mut manual = spaces();
        manual.resource_facts = None;
        state.set_snapshot(Box::new(manual.clone()));
        assert!(!state.tick_sort_motion(t0));
        assert!(state.config.sort_motion.workspaces.is_empty());

        // Bubble in flight, then a reload to instant snaps to the target.
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        state.set_snapshot(Box::new(spaces()));
        state.tick_sort_motion(t0);
        state.set_snapshot(Box::new(with_status(spaces(), "a", AgentStatus::Blocked)));
        state.tick_sort_motion(t0);
        assert_eq!(drawn_spaces(&state)[0], "ws_1");
        state.config.apply_live_config(&config, &[], &[]);
        assert_eq!(drawn_spaces(&state)[0], "a");
        assert_eq!(state.config.sort_motion.deadline, None);
        assert!(!state.tick_sort_motion(t0 + SETTLE));
    }

    #[test]
    fn agent_panel_jump_order_follows_the_drawn_motion_order() {
        let mut config = Config::default();
        config.ui.agent_panel_sort = crate::config::AgentPanelSortConfig::Priority;
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
        let mut snapshot = super::super::tests::snapshot();
        snapshot.agents = vec![agent("pane_1", "idle"), agent("pane_2", "idle")];
        state.set_snapshot(Box::new(snapshot.clone()));
        let t0 = Instant::now();
        assert!(!state.tick_sort_motion(t0));
        assert_eq!(drawn_agents(&state), ["pane_1", "pane_2"]);

        snapshot.revision += 1;
        snapshot.agents[1] = agent("pane_2", "blocked");
        state.set_snapshot(Box::new(snapshot));
        assert!(!state.tick_sort_motion(t0));
        assert_eq!(drawn_agents(&state), ["pane_1", "pane_2"]);
        let rows = super::super::agent_sidebar::agent_rows(
            state.snapshot.as_deref().unwrap(),
            &state.config,
            None,
        );
        assert_eq!(
            rows.iter()
                .map(|row| (row.pane_id.as_str(), row.jump_index))
                .collect::<Vec<_>>(),
            [("pane_1", 0), ("pane_2", 1)]
        );
        assert!(state.tick_sort_motion(t0 + SETTLE));
        assert_eq!(drawn_agents(&state), ["pane_2", "pane_1"]);
        assert_eq!(state.config.sort_motion.deadline, None);

        // Leaving priority sort drops the engine and the panel follows its own order.
        state.config.agent_panel_sort = crate::config::AgentPanelSortConfig::Spaces;
        assert!(!state.tick_sort_motion(t0 + SETTLE + STEP));
        assert!(state.config.sort_motion.agents.is_empty());
    }
}
