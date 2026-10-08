use super::*;
use ratatui::style::Color;

pub(super) fn pane_pin(snapshot: &ClientShellSnapshot, pane_id: &str) -> Option<u64> {
    snapshot
        .resource_facts
        .as_ref()?
        .pane_pins
        .as_ref()?
        .get(pane_id)
        .copied()
}

pub(super) fn workspace_pin(snapshot: &ClientShellSnapshot, workspace_id: &str) -> Option<u64> {
    snapshot
        .resource_facts
        .as_ref()?
        .workspace_pins
        .as_ref()?
        .get(workspace_id)
        .copied()
}

pub(super) fn marker_color(rank: usize) -> Color {
    const RAMP: [(u8, u8, u8); 4] = [
        (0xf4, 0x7b, 0x7b),
        (0xf4, 0xab, 0x7b),
        (0xf4, 0xdc, 0x7b),
        (0xdc, 0xf4, 0x7b),
    ];
    if let Some(&(r, g, b)) = RAMP.get(rank) {
        return Color::Rgb(r, g, b);
    }
    let k = rank - 4;
    let steps = if k == 0 { 0 } else { k.saturating_mul(2) + 1 };
    let channel = |top: u8, floor: u8| {
        let mut v = u32::from(top);
        for _ in 0..steps.min(64) {
            v = v * 90 / 100;
        }
        u8::try_from(v.max(u32::from(floor))).unwrap_or(floor)
    };
    Color::Rgb(
        channel(0xb5, 0x4f),
        channel(0xf0, 0x8a),
        channel(0xb0, 0x4a),
    )
}

pub(super) fn order_workspaces(
    snapshot: &ClientShellSnapshot,
    grouped: &HashSet<&str>,
    entries: &mut [WorkspaceEntry],
) {
    let priority = snapshot
        .resource_facts
        .as_ref()
        .and_then(|facts| facts.workspace_sort.as_deref())
        == Some("priority");
    let has_pins = snapshot
        .resource_facts
        .as_ref()
        .and_then(|f| f.workspace_pins.as_ref())
        .is_some_and(|pins| !pins.is_empty());
    if !priority && !has_pins {
        return;
    }
    let mut attention = HashMap::<&str, (u8, u64)>::new();
    for agent in &snapshot.agents {
        let entry = attention.entry(&agent.workspace_id).or_default();
        entry.0 = entry.0.max(status_priority(agent.agent_status));
        entry.1 = entry.1.max(agent.state_change_seq);
    }
    let mut group_ranks = HashMap::<&str, (Option<u64>, u8, u64)>::new();
    for workspace in &snapshot.workspaces {
        if let Some(worktree) = workspace
            .worktree
            .as_ref()
            .filter(|w| grouped.contains(w.key.as_str()))
        {
            let entry = group_ranks.entry(&worktree.key).or_default();
            if let Some(pin) = workspace_pin(snapshot, &workspace.workspace_id) {
                entry.0 = Some(entry.0.map_or(pin, |old| old.min(pin)));
            }
            let state = attention
                .get(workspace.workspace_id.as_str())
                .copied()
                .unwrap_or((status_priority(workspace.agent_status), 0));
            entry.1 = entry.1.max(state.0);
            entry.2 = entry.2.max(state.1);
        }
    }
    // Every member of a group has one rank and the stable sort keeps its tree intact.
    entries.sort_by_key(|entry| {
        let ws = &snapshot.workspaces[entry.index];
        let rank = ws
            .worktree
            .as_ref()
            .and_then(|w| group_ranks.get(w.key.as_str()))
            .copied()
            .unwrap_or_else(|| {
                let (state, seq) = attention
                    .get(ws.workspace_id.as_str())
                    .copied()
                    .unwrap_or((status_priority(ws.agent_status), 0));
                (workspace_pin(snapshot, &ws.workspace_id), state, seq)
            });
        (
            rank.0.is_none(),
            rank.0.unwrap_or(u64::MAX),
            std::cmp::Reverse(if priority { rank.1 } else { 0 }),
            std::cmp::Reverse(if priority { rank.2 } else { 0 }),
        )
    });
    let mut pins = snapshot
        .resource_facts
        .as_ref()
        .and_then(|f| f.workspace_pins.as_ref())
        .map(|pins| pins.values().copied().collect::<Vec<_>>())
        .unwrap_or_default();
    pins.sort_unstable();
    pins.dedup();
    for entry in entries {
        entry.pin_rank = workspace_pin(snapshot, &snapshot.workspaces[entry.index].workspace_id)
            .and_then(|pin| pins.binary_search(&pin).ok());
    }
}

pub(super) fn marker_rect(rect: Rect, lead: u16, config: &ClientShellConfig) -> Option<Rect> {
    let offset =
        u16::from(config.sidebar_active_border == crate::config::SidebarActiveBorderConfig::Left)
            + lead
            - 1;
    (rect.height > 0 && rect.width > offset).then(|| Rect::new(rect.x + offset, rect.y, 1, 1))
}
pub(super) fn workspace_marker(
    rect: Rect,
    entry: WorkspaceEntry,
    config: &ClientShellConfig,
) -> Option<Rect> {
    entry.pin_rank?;
    marker_rect(
        rect,
        if entry.indented {
            3
        } else if entry.group_collapsed.is_some() {
            2
        } else {
            1
        },
        config,
    )
}

impl ClientShellState {
    pub(super) fn unpin_marker_at(
        &mut self,
        point: (u16, u16),
        outcome: &mut ClientShellInput,
    ) -> bool {
        let Some((_, endpoint_id, method)) = self
            .hits
            .pin_markers
            .iter()
            .find(|(rect, _, _)| super::contains(*rect, point))
            .cloned()
        else {
            return false;
        };
        if endpoint_id == self.active_endpoint_id {
            self.push_endpoint_method(method, outcome);
        } else if let Some(endpoint) = self.endpoints.iter().find(|endpoint| {
            endpoint.endpoint_id == endpoint_id && endpoint.status == ClientEndpointStatus::Online
        }) {
            let method_name = crate::api::api_method_name(&method).to_owned();
            if endpoint
                .methods
                .as_ref()
                .is_some_and(|methods| !methods.contains(&method_name))
            {
                return true;
            }
            let Some(snapshot) = endpoint.snapshot.as_ref() else {
                return true;
            };
            let boot_id = snapshot.boot_id.clone();
            let id = format!("client-shell:{}", self.next_request_id);
            self.next_request_id = self.next_request_id.saturating_add(1);
            self.pending_requests.insert(
                id.clone(),
                PendingEndpointRequest {
                    boot_id: boot_id.clone(),
                    method_name,
                    confirmation_workspace_id: None,
                    kind: PendingEndpointKind::Generic,
                },
            );
            outcome.actions.push(ClientShellAction::Endpoint {
                endpoint_id,
                boot_id,
                request: Box::new(crate::api::schema::Request { id, method }),
            });
        }
        true
    }

    pub(super) fn toggle_workspace_pin(
        &mut self,
        workspace_id: String,
        outcome: &mut ClientShellInput,
    ) {
        let Some(pins) = self
            .snapshot
            .as_deref()
            .and_then(|s| s.resource_facts.as_ref())
            .and_then(|f| f.workspace_pins.as_ref())
        else {
            return;
        };
        let target = crate::api::schema::WorkspaceTarget {
            workspace_id: workspace_id.clone(),
        };
        let method = if pins.contains_key(&workspace_id) {
            crate::api::schema::Method::WorkspaceUnpin(target)
        } else {
            crate::api::schema::Method::WorkspacePin(target)
        };
        self.push_endpoint_method(method, outcome);
    }
    pub(super) fn toggle_agent_pin(&mut self, pane_id: String, outcome: &mut ClientShellInput) {
        let Some(pins) = self
            .snapshot
            .as_deref()
            .and_then(|s| s.resource_facts.as_ref())
            .and_then(|f| f.pane_pins.as_ref())
        else {
            return;
        };
        let target = crate::api::schema::AgentTarget {
            target: pane_id.clone(),
        };
        let method = if pins.contains_key(&pane_id) {
            crate::api::schema::Method::AgentUnpin(target)
        } else {
            crate::api::schema::Method::AgentPin(target)
        };
        self.push_endpoint_method(method, outcome);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clicking_pin_marker_unpins_without_changing_focus_and_neighbor_does_nothing() {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        state.set_snapshot(Box::new(super::super::tests::snapshot()));
        let focus = state
            .snapshot
            .as_ref()
            .unwrap()
            .focused_workspace_id
            .clone();
        state.hits.pin_markers.push((
            Rect::new(2, 3, 1, 1),
            ClientEndpointId::Local,
            crate::api::schema::Method::WorkspaceUnpin(crate::api::schema::WorkspaceTarget {
                workspace_id: "ws_1".into(),
            }),
        ));
        let mut outcome = ClientShellInput::default();
        assert!(!state.unpin_marker_at((3, 3), &mut outcome));
        assert!(outcome.actions.is_empty());
        assert!(state.unpin_marker_at((2, 3), &mut outcome));
        assert!(
            matches!(&outcome.actions[0], ClientShellAction::Endpoint { request, .. } if matches!(&request.method, crate::api::schema::Method::WorkspaceUnpin(target) if target.workspace_id == "ws_1"))
        );
        assert_eq!(state.snapshot.as_ref().unwrap().focused_workspace_id, focus);
    }

    #[test]
    fn pin_markers_use_the_fork_ramp_and_a_bounded_green_floor() {
        assert_eq!(marker_color(0), Color::Rgb(0xf4, 0x7b, 0x7b));
        assert_eq!(marker_color(1), Color::Rgb(0xf4, 0xab, 0x7b));
        assert_eq!(marker_color(2), Color::Rgb(0xf4, 0xdc, 0x7b));
        assert_eq!(marker_color(3), Color::Rgb(0xdc, 0xf4, 0x7b));
        assert_eq!(marker_color(4), Color::Rgb(0xb5, 0xf0, 0xb0));
        assert_eq!(marker_color(100), Color::Rgb(0x4f, 0x8a, 0x4a));
    }
    #[test]
    fn pinned_spaces_lead_in_pin_order_under_manual_and_priority() {
        let mut snapshot = super::super::tests::snapshot();
        let mut second = snapshot.workspaces[0].clone();
        second.workspace_id = "second".into();
        snapshot.workspaces.push(second);
        snapshot.resource_facts = Some(crate::protocol::ClientShellResourceFacts {
            workspace_pins: Some([("second".into(), 0)].into_iter().collect()),
            workspace_sort: Some("priority".into()),
            ..Default::default()
        });
        let entries = render::workspace_entries(&snapshot, &HashSet::new());
        assert_eq!(snapshot.workspaces[entries[0].index].workspace_id, "second");
        assert_eq!(entries[0].pin_rank, Some(0));
        assert_eq!(entries[0].visible_index, 0);
    }
    #[test]
    fn pin_key_uses_public_id_and_never_mutates_shared_pin_fact_optimistically() {
        let mut snapshot = super::super::tests::snapshot();
        snapshot.resource_facts = Some(crate::protocol::ClientShellResourceFacts {
            workspace_pins: Some(Default::default()),
            pane_pins: Some(Default::default()),
            ..Default::default()
        });
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        state.set_snapshot(Box::new(snapshot));
        let mut outcome = ClientShellInput::default();
        state.toggle_workspace_pin("ws_1".into(), &mut outcome);
        assert!(
            matches!(&outcome.actions[0],ClientShellAction::Endpoint {request,..} if matches!(&request.method,crate::api::schema::Method::WorkspacePin(target) if target.workspace_id=="ws_1"))
        );
        assert!(state
            .snapshot
            .as_ref()
            .unwrap()
            .resource_facts
            .as_ref()
            .unwrap()
            .workspace_pins
            .as_ref()
            .unwrap()
            .is_empty());
    }
    #[test]
    fn a_group_is_pinned_when_a_hidden_member_is_and_moves_whole() {
        let mut snapshot = super::super::tests::snapshot();
        let mut parent = snapshot.workspaces[0].clone();
        parent.workspace_id = "parent".into();
        parent.worktree = Some(crate::protocol::ClientShellWorktree {
            key: "repo".into(),
            label: "repo".into(),
            is_linked_worktree: false,
        });
        let mut child = parent.clone();
        child.workspace_id = "child".into();
        child.worktree.as_mut().unwrap().is_linked_worktree = true;
        snapshot.workspaces.extend([parent, child]);
        snapshot.resource_facts = Some(crate::protocol::ClientShellResourceFacts {
            workspace_pins: Some([("child".into(), 0)].into_iter().collect()),
            ..Default::default()
        });
        let entries = render::workspace_entries(&snapshot, &HashSet::new());
        assert_eq!(
            entries
                .iter()
                .map(|e| snapshot.workspaces[e.index].workspace_id.as_str())
                .collect::<Vec<_>>(),
            vec!["parent", "child", "ws_1"]
        );
        let entries = render::workspace_entries(&snapshot, &HashSet::from(["repo".into()]));
        assert_eq!(snapshot.workspaces[entries[0].index].workspace_id, "parent");
    }
}
