mod tokens;

use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use self::tokens::{ResolvedToken, ResolvedTokenKind, SpaceTokenContext};
use super::scrollbar::{render_scrollbar, should_show_scrollbar};
use super::sidebar_overflow::{
    self as overflow, EdgeItem, EdgeReserve, EdgeRow, OverflowPlan, VisibleItem,
};
use super::status::{agent_state_icon, state_icon, state_label, state_label_color};
use super::text::{display_width, display_width_u16, truncate_end, truncate_start};
use crate::agent_priority::{attention_priority, display_priority};
use crate::app::state::{AgentPanelSort, Palette, WorkspaceSort};
use crate::app::{AppState, Mode};
use crate::detect::AgentState;
use crate::terminal::TerminalRuntimeRegistry;

const WORKSPACE_SECTION_HEADER_ROWS: u16 = 2;
const AGENT_PANEL_HEADER_ROWS: u16 = 3;

pub(crate) struct AgentPanelEntry {
    pub ws_idx: usize,
    pub tab_idx: usize,
    pub pane_id: crate::layout::PaneId,
    pub primary_label: String,
    pub primary_tab_label: Option<String>,
    pub pane_label: Option<String>,
    pub terminal_title: Option<String>,
    pub terminal_title_stripped: Option<String>,
    /// See `TerminalState::background_work`.
    pub background_work: bool,
    /// Not-yet-done todos on the pane, and the highest priority among them.
    pub outstanding_todos: usize,
    pub highest_todo_priority: Option<crate::terminal::todo::TodoPriority>,
    pub agent_label: Option<String>,
    pub agent_kind_label: Option<String>,
    pub agent: Option<crate::detect::Agent>,
    pub state: AgentState,
    pub seen: bool,
    pub last_agent_state_change_seq: Option<u64>,
    /// See `TerminalState::pin_order`.
    pub pin_order: Option<u64>,
    pub state_labels: std::collections::HashMap<String, String>,
    pub tokens: std::collections::HashMap<String, String>,
}

impl AgentPanelEntry {
    /// The shared spinner frame shifted by when this agent last changed
    /// state, so rows that started working at different moments sit at
    /// different frames instead of marching in lockstep. Same tick, same
    /// cost; only the phase differs.
    fn spinner_frame(&self, shared: Option<u8>) -> Option<u8> {
        let phase = self.last_agent_state_change_seq.unwrap_or(0) as u8;
        shared.map(|frame| frame.wrapping_add(phase))
    }
}

fn sidebar_section_heights(total_h: u16, split_ratio: f32) -> (u16, u16) {
    if total_h == 0 {
        return (0, 0);
    }

    if total_h < 6 {
        let ws_h = total_h.div_ceil(2);
        return (ws_h, total_h.saturating_sub(ws_h));
    }

    let ratio = split_ratio.clamp(0.1, 0.9);
    let ws_h = ((total_h as f32) * ratio).round() as u16;
    let ws_h = ws_h.clamp(3, total_h.saturating_sub(3));
    let detail_h = total_h.saturating_sub(ws_h);
    (ws_h, detail_h)
}

pub(crate) fn expanded_sidebar_sections(area: Rect, split_ratio: f32) -> (Rect, Rect) {
    let content = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    if content.width == 0 || content.height == 0 {
        return (Rect::default(), Rect::default());
    }

    let (ws_h, detail_h) = sidebar_section_heights(content.height, split_ratio);
    let ws_area = Rect::new(content.x, content.y, content.width, ws_h);
    let detail_area = Rect::new(content.x, content.y + ws_h, content.width, detail_h);
    (ws_area, detail_area)
}

pub(crate) fn sidebar_section_divider_rect(area: Rect, split_ratio: f32) -> Rect {
    let content = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    if content.width == 0 || content.height < 6 {
        return Rect::default();
    }

    let (ws_h, _) = sidebar_section_heights(content.height, split_ratio);
    Rect::new(content.x, content.y + ws_h, content.width, 1)
}

fn agent_panel_sort_label(sort: AgentPanelSort) -> &'static str {
    match sort {
        AgentPanelSort::Spaces => "grouped",
        AgentPanelSort::Priority => "priority",
    }
}

pub(crate) fn agent_panel_toggle_rect(area: Rect, sort: AgentPanelSort) -> Rect {
    agent_panel_header_label_rect(area, agent_panel_sort_label(sort))
}

fn agent_panel_header_label_rect(area: Rect, label: &str) -> Rect {
    if area.width == 0 || area.height < 2 {
        return Rect::default();
    }

    let width = display_width_u16(label).min(area.width);
    Rect::new(
        area.x + area.width.saturating_sub(width),
        area.y + 1,
        width,
        1,
    )
}

fn active_agent_view_label(app: &AppState) -> Option<&str> {
    app.agent_view_override
        .as_ref()
        .map(|view| view.label.as_deref().unwrap_or("filtered"))
}

pub(crate) fn agent_panel_entries(app: &AppState) -> Vec<AgentPanelEntry> {
    agent_panel_entries_with_runtimes(app, None)
}

pub(crate) fn all_agent_panel_entries(app: &AppState) -> Vec<AgentPanelEntry> {
    collect_agent_panel_entries_with_runtimes(app, None)
}

pub(crate) fn agent_panel_entries_from(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
) -> Vec<AgentPanelEntry> {
    agent_panel_entries_with_runtimes(app, Some(terminal_runtimes))
}

fn agent_panel_entries_with_runtimes(
    app: &AppState,
    terminal_runtimes: Option<&TerminalRuntimeRegistry>,
) -> Vec<AgentPanelEntry> {
    let mut entries = collect_agent_panel_entries_with_runtimes(app, terminal_runtimes);
    crate::app::agent_view::apply_agent_view(app, &mut entries);
    if matches!(app.agent_panel_sort, AgentPanelSort::Priority) {
        entries = apply_agent_panel_motion(app, entries);
    }
    entries
}

fn collect_agent_panel_entries_with_runtimes(
    app: &AppState,
    terminal_runtimes: Option<&TerminalRuntimeRegistry>,
) -> Vec<AgentPanelEntry> {
    let empty_runtimes;
    let terminal_runtimes = match terminal_runtimes {
        Some(terminal_runtimes) => terminal_runtimes,
        None => {
            empty_runtimes = TerminalRuntimeRegistry::new();
            &empty_runtimes
        }
    };

    app.workspaces
        .iter()
        .enumerate()
        .flat_map(|(ws_idx, ws)| {
            let multi_tab = ws.tabs.len() > 1;
            let workspace_label = ws.display_name_from(&app.terminals, terminal_runtimes);
            ws.pane_details(&app.terminals)
                .into_iter()
                .map(move |detail| {
                    let show_tab = multi_tab
                        || ws
                            .tabs
                            .get(detail.tab_idx)
                            .is_some_and(|tab| !tab.is_auto_named());
                    AgentPanelEntry {
                        ws_idx,
                        tab_idx: detail.tab_idx,
                        pane_id: detail.pane_id,
                        primary_label: workspace_label.clone(),
                        primary_tab_label: show_tab.then_some(detail.tab_label),
                        pane_label: detail.pane_label,
                        terminal_title: detail.terminal_title,
                        terminal_title_stripped: detail.terminal_title_stripped,
                        background_work: detail.background_work,
                        outstanding_todos: detail.outstanding_todos,
                        highest_todo_priority: detail.highest_todo_priority,
                        agent_label: Some(detail.agent_label),
                        agent_kind_label: detail.agent_kind_label,
                        agent: detail.agent,
                        state: detail.state,
                        seen: detail.seen,
                        last_agent_state_change_seq: detail.last_agent_state_change_seq,
                        pin_order: detail.pin_order,
                        state_labels: detail.state_labels,
                        tokens: detail.tokens,
                    }
                })
        })
        .collect()
}

/// Reorders priority-sorted agent entries through the panel's bubble-motion
/// display order. Pure between motion ticks, so render and click-time
/// hit-testing observe the same order.
fn apply_agent_panel_motion(app: &AppState, entries: Vec<AgentPanelEntry>) -> Vec<AgentPanelEntry> {
    if !agent_panel_motion_active(app) {
        return entries;
    }
    // Pinned entries are in the target, first and in pin order: they move
    // only when a pin changes, never for a priority change.
    let target: Vec<crate::layout::PaneId> = entries.iter().map(|entry| entry.pane_id).collect();
    let order = app.agent_panel_motion.project(&target);
    let mut by_key: std::collections::HashMap<crate::layout::PaneId, AgentPanelEntry> = entries
        .into_iter()
        .map(|entry| (entry.pane_id, entry))
        .collect();
    order
        .into_iter()
        .filter_map(|key| by_key.remove(&key))
        .collect()
}

pub(crate) fn agent_panel_motion_active(app: &AppState) -> bool {
    app.sort_motion_bubble && matches!(app.agent_panel_sort, AgentPanelSort::Priority)
}

/// Target order for the agent panel's bubble motion: the live priority-sorted
/// pane ids, before motion is applied.
pub(crate) fn agent_panel_target_keys(app: &AppState) -> Vec<crate::layout::PaneId> {
    let mut keyed: Vec<(Option<u64>, u8, Option<u64>, crate::layout::PaneId)> = app
        .workspaces
        .iter()
        .flat_map(|ws| ws.pane_details(&app.terminals))
        .map(|detail| {
            (
                detail.pin_order,
                attention_priority(detail.state, detail.seen),
                detail.last_agent_state_change_seq,
                detail.pane_id,
            )
        })
        .collect();
    // Pinned first in pin order (`None` last), then the priority order.
    keyed.sort_by_key(|(pin, priority, seq, _)| {
        (
            pin.is_none(),
            *pin,
            std::cmp::Reverse(*priority),
            std::cmp::Reverse(*seq),
        )
    });
    keyed
        .into_iter()
        .map(|(_, _, _, pane_id)| pane_id)
        .collect()
}

pub(super) fn agent_panel_status_key(state: AgentState, seen: bool) -> &'static str {
    match (state, seen) {
        (AgentState::Idle, false) => "done",
        (AgentState::Idle, true) => "idle",
        (AgentState::Working, _) => "working",
        (AgentState::Blocked, _) => "blocked",
        (AgentState::Unknown, _) => "unknown",
    }
}

/// Drawn in the one-cell lead of a pinned row's name line. One cell, no emoji.
pub(crate) const PIN_MARKER: &str = "↑";

/// Where the green fade starts, and the darkest it ever gets (fork issues 151,
/// 154). Each step keeps 90% of every channel, and the floor keeps a long
/// block readable on a dark background and clearly green.
const PIN_MARKER_TOP: (u8, u8, u8) = (0xb5, 0xf0, 0xb0);
const PIN_MARKER_FLOOR: (u8, u8, u8) = (0x4f, 0x8a, 0x4a);
const PIN_MARKER_STEP_PERCENT: u32 = 90;

/// How many spaces are pinned ahead of `ws`: its rank in the pinned block.
fn workspace_pin_rank(app: &AppState, ws: &crate::workspace::Workspace) -> usize {
    let Some(own) = ws.pin_order else { return 0 };
    app.workspaces
        .iter()
        .filter(|other| other.pin_order.is_some_and(|order| order < own))
        .count()
}

/// How many agents are pinned ahead of `entry` in the panel.
fn agent_pin_rank(entries: &[AgentPanelEntry], entry: &AgentPanelEntry) -> usize {
    let Some(own) = entry.pin_order else { return 0 };
    entries
        .iter()
        .filter(|other| other.pin_order.is_some_and(|order| order < own))
        .count()
}

/// The first four pins run red, orange, yellow, then yellow-green (fork issue
/// 154), then the green fade starts.
const PIN_MARKER_RAMP: [(u8, u8, u8); 4] = [
    (0xf4, 0x7b, 0x7b),
    (0xf4, 0xab, 0x7b),
    (0xf4, 0xdc, 0x7b),
    (0xdc, 0xf4, 0x7b),
];

/// The marker colour for the pinned row at `rank` (0 is the top pin of its own
/// list: the spaces list and the agent panel each count from their own top).
///
/// A rank always has the same colour, however many rows are pinned. Ranks 0 to
/// 3 take [`PIN_MARKER_RAMP`]. From rank 4 the green fade begins at the start
/// colour and drops two steps per rung after that, down to the floor.
pub(crate) fn pin_marker_color(rank: usize) -> Color {
    if let Some(&(r, g, b)) = PIN_MARKER_RAMP.get(rank) {
        return Color::Rgb(r, g, b);
    }
    let k = rank - PIN_MARKER_RAMP.len();
    let steps = if k == 0 { 0 } else { k.saturating_mul(2) + 1 };
    marker_color_after(steps)
}

/// The start colour dimmed by `steps` steps, down to the floor.
fn marker_color_after(steps: usize) -> Color {
    let channel = |top: u8, floor: u8| {
        let mut value = u32::from(top);
        for _ in 0..steps.min(64) {
            value = value * PIN_MARKER_STEP_PERCENT / 100;
        }
        u8::try_from(value.max(u32::from(floor))).unwrap_or(floor)
    };
    Color::Rgb(
        channel(PIN_MARKER_TOP.0, PIN_MARKER_FLOOR.0),
        channel(PIN_MARKER_TOP.1, PIN_MARKER_FLOOR.1),
        channel(PIN_MARKER_TOP.2, PIN_MARKER_FLOOR.2),
    )
}

fn workspace_row_height(app: &AppState, ws: &crate::workspace::Workspace, indented: bool) -> u16 {
    // The stored-cwd name, not the live one the row draws: sidebar geometry
    // is measured without the runtime registry, and the name only matters
    // here when it changes how many rows the row's tokens wrap to.
    let (state, seen) = ws.display_state(&app.terminals);
    let label = if indented {
        grouped_child_display_label(
            &ws.display_name_from_terminals(&app.terminals),
            ws.head_label().as_deref(),
            ws.custom_name.is_some(),
        )
    } else {
        ws.display_name_from_terminals(&app.terminals)
    };
    let token_values = ws.metadata_tokens.values();
    tokens::space_rows(
        &app.sidebar_spaces,
        SpaceTokenContext {
            workspace: &label,
            branch: ws.head_label().as_deref(),
            state_text: state_label(state, seen),
            ahead_behind: ws.git_ahead_behind(),
            tokens: &token_values,
            suppress_git_details: indented,
        },
    )
    .len()
    .max(1)
    .min(u16::MAX as usize) as u16
}

fn workspace_row_height_in_body(
    app: &AppState,
    workspace: &crate::workspace::Workspace,
    indented: bool,
    body_height: u16,
) -> u16 {
    workspace_row_height(app, workspace, indented).min(body_height)
}

fn workspace_entry_gap(app: &AppState, entries: &[WorkspaceListEntry], entry_idx: usize) -> u16 {
    if entry_idx + 1 < entries.len() && !next_entry_is_indented_workspace(entries, entry_idx) {
        app.sidebar_spaces.row_gap
    } else {
        0
    }
}

fn space_workspaces<'a>(
    app: &'a AppState,
    key: &'a str,
) -> impl Iterator<Item = &'a crate::workspace::Workspace> + 'a {
    app.workspaces
        .iter()
        .filter(move |ws| ws.worktree_space().is_some_and(|space| space.key == key))
}

/// The state a worktree space *is*, shown on its collapsed group row.
fn space_display_state(app: &AppState, key: &str) -> (AgentState, bool) {
    space_workspaces(app, key)
        .map(|ws| ws.display_state(&app.terminals))
        .max_by_key(|(state, seen)| display_priority(*state, *seen))
        .unwrap_or((AgentState::Unknown, true))
}

/// The state a worktree space *wants the user for*, used to rank the whole
/// group under `workspace_sort = priority`.
fn space_attention_state(app: &AppState, key: &str) -> (AgentState, bool) {
    space_workspaces(app, key)
        .map(|ws| ws.attention_state(&app.terminals))
        .max_by_key(|(state, seen)| attention_priority(*state, *seen))
        .unwrap_or((AgentState::Unknown, true))
}

fn space_last_agent_state_change_seq(app: &AppState, key: &str) -> Option<u64> {
    app.workspaces
        .iter()
        .filter(|ws| ws.worktree_space().is_some_and(|space| space.key == key))
        .filter_map(|ws| ws.last_agent_state_change_seq(&app.terminals))
        .max()
}

pub(crate) fn workspace_parent_group_state(
    app: &AppState,
    ws_idx: usize,
) -> Option<(String, bool)> {
    let space = app.workspaces.get(ws_idx)?.worktree_space()?;
    if space.is_linked_worktree {
        return None;
    }
    let member_count = app
        .workspaces
        .iter()
        .filter(|ws| {
            ws.worktree_space()
                .is_some_and(|member| member.key == space.key)
        })
        .count();
    (member_count >= 2).then(|| {
        (
            space.key.clone(),
            app.collapsed_space_keys.contains(&space.key),
        )
    })
}

pub(crate) fn grouped_child_display_label(
    label: &str,
    branch: Option<&str>,
    has_custom_name: bool,
) -> String {
    if has_custom_name {
        return label.to_string();
    }
    let Some(branch) = branch else {
        return label.to_string();
    };
    branch
        .strip_prefix("worktree/")
        .unwrap_or(branch)
        .to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkspaceListEntry {
    Workspace { ws_idx: usize, indented: bool },
}

pub(crate) fn next_entry_is_indented_workspace(entries: &[WorkspaceListEntry], idx: usize) -> bool {
    matches!(
        entries.get(idx.saturating_add(1)),
        Some(WorkspaceListEntry::Workspace { indented: true, .. })
    )
}

pub(crate) fn normalized_workspace_scroll(app: &AppState, area: Rect, requested: usize) -> usize {
    let ws_area = workspace_list_rect(area, app.sidebar_section_split);
    let body = workspace_list_body_rect(ws_area, false);
    if body.height == 0 {
        return requested;
    }

    if workspace_list_entries(app).is_empty() {
        0
    } else {
        requested.min(workspace_list_bottom_start(app, ws_area))
    }
}

pub(crate) fn workspace_list_entries(app: &AppState) -> Vec<WorkspaceListEntry> {
    workspace_list_entries_inner(app, false)
}

/// Like [`workspace_list_entries`] but always expands worktree groups, ignoring
/// `collapsed_space_keys`. The mobile switcher has no collapse affordance and
/// always shows the full worktree tree.
pub(crate) fn workspace_list_entries_expanded(app: &AppState) -> Vec<WorkspaceListEntry> {
    workspace_list_entries_inner(app, true)
}

fn workspace_list_entries_inner(app: &AppState, force_expanded: bool) -> Vec<WorkspaceListEntry> {
    let units = workspace_sorted_units(app, force_expanded);
    let units = apply_workspace_motion(app, units);
    units.into_iter().flat_map(|unit| unit.entries).collect()
}

/// Reorders sorted units through the workspace list's bubble-motion display
/// order. Pure between motion ticks, so render, jump numbers, and hit-testing
/// all observe the same order.
fn apply_workspace_motion(app: &AppState, units: Vec<WorkspaceUnit>) -> Vec<WorkspaceUnit> {
    if !workspace_motion_active(app) {
        return units;
    }
    // Pinned units are part of the target, first and in pin order, so they
    // move only when a pin changes: pinning bubbles a row up into the block,
    // unpinning bubbles it back down. A priority change never reorders them.
    let target: Vec<String> = units.iter().map(|unit| unit.key.clone()).collect();
    let order = app.workspace_list_motion.project(&target);
    let mut by_key: std::collections::HashMap<String, WorkspaceUnit> = units
        .into_iter()
        .map(|unit| (unit.key.clone(), unit))
        .collect();
    order
        .into_iter()
        .filter_map(|key| by_key.remove(&key))
        .collect()
}

pub(crate) fn workspace_motion_active(app: &AppState) -> bool {
    app.sort_motion_bubble && matches!(app.workspace_sort, WorkspaceSort::Priority)
}

/// Target order for the workspace list's bubble motion: the live
/// priority-sorted unit keys, before motion is applied.
pub(crate) fn workspace_unit_target_keys(app: &AppState) -> Vec<String> {
    workspace_sorted_units(app, false)
        .into_iter()
        .map(|unit| unit.key)
        .collect()
}

/// A top-level unit: an ungrouped workspace, or a whole worktree group
/// (parent + member rows). Units are what priority sort reorders; group
/// members always stay contiguous under their parent.
struct WorkspaceUnit {
    /// Stable motion key: `ws:<workspace id>` or `space:<worktree space key>`.
    key: String,
    priority: u8,
    last_change_seq: Option<u64>,
    /// Pin order of the unit: a workspace's own, or the smallest across a
    /// worktree group's members. `Some` units sit above every other unit.
    pin_order: Option<u64>,
    entries: Vec<WorkspaceListEntry>,
}

fn workspace_sorted_units(app: &AppState, force_expanded: bool) -> Vec<WorkspaceUnit> {
    let mut members_by_key = std::collections::HashMap::<String, Vec<usize>>::new();
    for (ws_idx, ws) in app.workspaces.iter().enumerate() {
        if let Some(space) = ws.worktree_space() {
            members_by_key
                .entry(space.key.clone())
                .or_default()
                .push(ws_idx);
        }
    }
    let grouped_keys = members_by_key
        .iter()
        .filter(|(_, members)| {
            members.len() >= 2
                && members.iter().any(|idx| {
                    app.workspaces
                        .get(*idx)
                        .and_then(|ws| ws.worktree_space())
                        .is_some_and(|space| !space.is_linked_worktree)
                })
        })
        .map(|(key, _)| key.clone())
        .collect::<std::collections::HashSet<_>>();

    let visible_group_idx = if matches!(app.mode, Mode::Navigate) {
        Some(app.selected)
    } else {
        app.active
    };
    let active_group = visible_group_idx.and_then(|idx| {
        app.workspaces
            .get(idx)
            .and_then(|ws| ws.worktree_space())
            .map(|space| space.key.clone())
    });

    let prioritize = matches!(app.workspace_sort, WorkspaceSort::Priority);
    let workspace_rank = |ws: &crate::workspace::Workspace| {
        if !prioritize {
            return (0, None);
        }
        let (state, seen) = ws.attention_state(&app.terminals);
        (
            attention_priority(state, seen),
            ws.last_agent_state_change_seq(&app.terminals),
        )
    };

    let mut emitted_groups = std::collections::HashSet::<String>::new();
    let mut units = Vec::new();
    for (ws_idx, ws) in app.workspaces.iter().enumerate() {
        let Some(space) = ws
            .worktree_space()
            .filter(|space| grouped_keys.contains(&space.key))
        else {
            let (priority, last_change_seq) = workspace_rank(ws);
            units.push(WorkspaceUnit {
                key: format!("ws:{}", ws.id),
                priority,
                last_change_seq,
                pin_order: ws.pin_order,
                entries: vec![WorkspaceListEntry::Workspace {
                    ws_idx,
                    indented: false,
                }],
            });
            continue;
        };

        if !emitted_groups.insert(space.key.clone()) {
            continue;
        }

        let Some(members) = members_by_key.get(&space.key) else {
            continue;
        };
        let Some(parent_idx) = members.iter().copied().find(|idx| {
            app.workspaces
                .get(*idx)
                .and_then(|member| member.worktree_space())
                .is_some_and(|member_space| !member_space.is_linked_worktree)
        }) else {
            let (priority, last_change_seq) = workspace_rank(ws);
            units.push(WorkspaceUnit {
                key: format!("ws:{}", ws.id),
                priority,
                last_change_seq,
                pin_order: ws.pin_order,
                entries: vec![WorkspaceListEntry::Workspace {
                    ws_idx,
                    indented: false,
                }],
            });
            continue;
        };
        let collapsed = !force_expanded && app.collapsed_space_keys.contains(&space.key);
        let mut entries = vec![WorkspaceListEntry::Workspace {
            ws_idx: parent_idx,
            indented: false,
        }];

        if collapsed {
            if let Some(active_idx) = visible_group_idx
                .filter(|idx| *idx != parent_idx)
                .filter(|_| active_group.as_deref() == Some(space.key.as_str()))
            {
                entries.push(WorkspaceListEntry::Workspace {
                    ws_idx: active_idx,
                    indented: true,
                });
            }
        } else {
            for member_idx in members {
                if *member_idx == parent_idx {
                    continue;
                }
                entries.push(WorkspaceListEntry::Workspace {
                    ws_idx: *member_idx,
                    indented: true,
                });
            }
        }

        // Rank a group by its whole space, even when collapsed rows are hidden.
        let (priority, last_change_seq) = if prioritize {
            let (state, seen) = space_attention_state(app, &space.key);
            (
                attention_priority(state, seen),
                space_last_agent_state_change_seq(app, &space.key),
            )
        } else {
            (0, None)
        };
        units.push(WorkspaceUnit {
            key: format!("space:{}", space.key),
            priority,
            last_change_seq,
            pin_order: members
                .iter()
                .filter_map(|idx| app.workspaces.get(*idx).and_then(|ws| ws.pin_order))
                .min(),
            entries,
        });
    }

    if prioritize {
        // Stable sort: within a tier, most recent state change first, then
        // the manual order.
        units.sort_by_key(|unit| {
            (
                std::cmp::Reverse(unit.priority),
                std::cmp::Reverse(unit.last_change_seq),
            )
        });
    }

    // Pinned units go first in pin order, under every sort. Unpinned units
    // keep the order the sort gave them.
    units.sort_by_key(|unit| unit.pin_order.is_none());
    let pinned = units.iter().take_while(|u| u.pin_order.is_some()).count();
    units[..pinned].sort_by_key(|unit| unit.pin_order);

    units
}

pub(crate) fn workspace_list_rect(area: Rect, split_ratio: f32) -> Rect {
    let (ws_area, _) = expanded_sidebar_sections(area, split_ratio);
    ws_area
}

pub(crate) fn workspace_list_body_rect(area: Rect, has_scrollbar: bool) -> Rect {
    if area.width == 0 || area.height <= WORKSPACE_SECTION_HEADER_ROWS {
        return Rect::default();
    }

    let body_y = area.y.saturating_add(WORKSPACE_SECTION_HEADER_ROWS);
    let footer_y = area.y + area.height.saturating_sub(1);
    let body_height = footer_y.saturating_sub(body_y);
    let body_width = area.width.saturating_sub(u16::from(has_scrollbar));
    Rect::new(area.x, body_y, body_width, body_height)
}

/// How many entries from `scroll` fit in `avail` rows of the spaces list.
fn workspace_fit_count(
    app: &AppState,
    entries: &[WorkspaceListEntry],
    scroll: usize,
    avail: u16,
    body_height: u16,
) -> usize {
    let mut used_rows = 0u16;
    let mut visible = 0usize;
    for (entry_idx, entry) in entries.iter().enumerate().skip(scroll) {
        let (row_height, gap) = match entry {
            WorkspaceListEntry::Workspace { ws_idx, indented } => {
                let Some(ws) = app.workspaces.get(*ws_idx) else {
                    continue;
                };
                (
                    workspace_row_height_in_body(app, ws, *indented, body_height),
                    workspace_entry_gap(app, entries, entry_idx),
                )
            }
        };
        if used_rows.saturating_add(row_height) > avail {
            break;
        }
        used_rows = used_rows.saturating_add(row_height);
        visible += 1;
        used_rows = used_rows.saturating_add(gap).min(avail);
    }
    visible
}

/// The edge rows of the spaces list at `scroll` and how many entries it shows.
fn workspace_list_window(app: &AppState, area: Rect, scroll: usize) -> (EdgeReserve, usize) {
    let body = workspace_list_body_rect(area, false);
    if body.width == 0 || body.height == 0 {
        return (EdgeReserve::default(), 0);
    }
    let entries = workspace_list_entries(app);
    overflow::window(
        app.sidebar_overflow.edge_rows(),
        body.height,
        entries.len(),
        scroll,
        |avail| workspace_fit_count(app, &entries, scroll, avail, body.height),
    )
}

fn workspace_list_visible_count(app: &AppState, area: Rect, scroll: usize) -> usize {
    workspace_list_window(app, area, scroll).1
}

/// The first entry of the last full window of `entries` in `avail` rows.
fn workspace_start_in(
    app: &AppState,
    entries: &[WorkspaceListEntry],
    avail: u16,
    body_height: u16,
) -> usize {
    let mut used_rows = 0u16;
    let mut start = entries.len();
    for (entry_idx, entry) in entries.iter().enumerate().rev() {
        let WorkspaceListEntry::Workspace { ws_idx, indented } = entry;
        let Some(workspace) = app.workspaces.get(*ws_idx) else {
            continue;
        };
        let gap = workspace_entry_gap(app, entries, entry_idx);
        let needed = workspace_row_height_in_body(app, workspace, *indented, body_height)
            .saturating_add(gap);
        if used_rows.saturating_add(needed) > avail {
            break;
        }
        used_rows = used_rows.saturating_add(needed);
        start = entry_idx;
    }
    start.min(entries.len().saturating_sub(1))
}

fn workspace_list_bottom_start(app: &AppState, area: Rect) -> usize {
    let body = workspace_list_body_rect(area, false);
    let entries = workspace_list_entries(app);
    overflow::last_window_start(app.sidebar_overflow.edge_rows(), body.height, |avail| {
        workspace_start_in(app, &entries, avail, body.height)
    })
}

pub(crate) fn workspace_list_scroll_metrics(
    app: &AppState,
    area: Rect,
) -> crate::pane::ScrollMetrics {
    let max_scroll = workspace_list_bottom_start(app, area);
    let scroll = app.workspace_scroll.min(max_scroll);
    let viewport_rows = workspace_list_visible_count(app, area, scroll);

    crate::pane::ScrollMetrics {
        offset_from_bottom: max_scroll.saturating_sub(scroll),
        max_offset_from_bottom: max_scroll,
        viewport_rows,
    }
}

pub(crate) fn workspace_list_scrollbar_rect(app: &AppState, area: Rect) -> Option<Rect> {
    let metrics = workspace_list_scroll_metrics(app, area);
    let body = workspace_list_body_rect(area, true);
    (should_show_scrollbar(metrics) && body.width > 0 && body.height > 0).then_some(Rect::new(
        area.x + area.width.saturating_sub(1),
        body.y,
        1,
        body.height,
    ))
}

pub(crate) fn agent_panel_body_rect(area: Rect, has_scrollbar: bool) -> Rect {
    if area.width == 0 || area.height <= AGENT_PANEL_HEADER_ROWS {
        return Rect::default();
    }

    let body_y = area.y.saturating_add(AGENT_PANEL_HEADER_ROWS);
    let body_height = (area.y + area.height).saturating_sub(body_y);
    let body_width = area.width.saturating_sub(u16::from(has_scrollbar));
    Rect::new(area.x, body_y, body_width, body_height)
}

fn resolved_agent_rows(app: &AppState, entry: &AgentPanelEntry) -> Vec<Vec<ResolvedToken>> {
    let label = entry
        .state_labels
        .get(agent_panel_status_key(entry.state, entry.seen))
        .map(String::as_str)
        .unwrap_or_else(|| state_label(entry.state, entry.seen));
    tokens::agent_rows(&app.sidebar_agents, entry, label)
}

pub(crate) fn agent_entry_height_in_body(
    app: &AppState,
    entry: &AgentPanelEntry,
    body_height: u16,
) -> u16 {
    (resolved_agent_rows(app, entry)
        .len()
        .max(1)
        .min(u16::MAX as usize) as u16)
        .min(body_height)
}

pub(crate) fn agent_entry_gap(app: &AppState, entry_idx: usize, entry_count: usize) -> u16 {
    if entry_idx + 1 < entry_count {
        app.sidebar_agents.row_gap
    } else {
        0
    }
}

/// How many entries from `scroll` fit in `avail` rows of the agent panel.
fn agent_fit_count(
    app: &AppState,
    entries: &[AgentPanelEntry],
    scroll: usize,
    avail: u16,
    body_height: u16,
) -> usize {
    let mut used_rows = 0u16;
    let mut visible = 0usize;
    for (index, entry) in entries.iter().enumerate().skip(scroll) {
        let height = agent_entry_height_in_body(app, entry, body_height);
        if used_rows.saturating_add(height) > avail {
            break;
        }
        used_rows = used_rows.saturating_add(height);
        visible += 1;
        used_rows = used_rows
            .saturating_add(agent_entry_gap(app, index, entries.len()))
            .min(avail);
    }
    visible
}

/// The edge rows of the agent panel at `scroll` and how many entries it shows.
pub(crate) fn agent_panel_window(
    app: &AppState,
    area: Rect,
    scroll: usize,
) -> (EdgeReserve, usize) {
    let body = agent_panel_body_rect(area, false);
    if body.width == 0 || body.height == 0 {
        return (EdgeReserve::default(), 0);
    }
    let entries = agent_panel_entries(app);
    overflow::window(
        app.sidebar_overflow.edge_rows(),
        body.height,
        entries.len(),
        scroll,
        |avail| agent_fit_count(app, &entries, scroll, avail, body.height),
    )
}

fn agent_panel_visible_count_from(app: &AppState, area: Rect, scroll: usize) -> usize {
    agent_panel_window(app, area, scroll).1
}

fn agent_start_in(
    app: &AppState,
    entries: &[AgentPanelEntry],
    avail: u16,
    body_height: u16,
) -> usize {
    let mut used_rows = 0u16;
    let mut start = entries.len();
    for (index, entry) in entries.iter().enumerate().rev() {
        let gap = agent_entry_gap(app, index, entries.len());
        let needed = agent_entry_height_in_body(app, entry, body_height).saturating_add(gap);
        if used_rows.saturating_add(needed) > avail {
            break;
        }
        used_rows = used_rows.saturating_add(needed);
        start = index;
    }
    start.min(entries.len().saturating_sub(1))
}

fn agent_panel_bottom_start(app: &AppState, area: Rect) -> usize {
    let body = agent_panel_body_rect(area, false);
    let entries = agent_panel_entries(app);
    overflow::last_window_start(app.sidebar_overflow.edge_rows(), body.height, |avail| {
        agent_start_in(app, &entries, avail, body.height)
    })
}

pub(crate) fn agent_panel_scroll_for_target(
    app: &AppState,
    area: Rect,
    current_scroll: usize,
    target: usize,
) -> usize {
    let max_scroll = agent_panel_bottom_start(app, area);
    if target < current_scroll {
        return target.min(max_scroll);
    }
    let mut scroll = current_scroll.min(max_scroll);
    while scroll < target {
        let visible = agent_panel_visible_count_from(app, area, scroll);
        if visible > 0 && target < scroll.saturating_add(visible) {
            break;
        }
        scroll += 1;
    }
    scroll.min(max_scroll)
}

pub(crate) fn agent_panel_scroll_metrics(app: &AppState, area: Rect) -> crate::pane::ScrollMetrics {
    let max_scroll = agent_panel_bottom_start(app, area);
    let scroll = app.agent_panel_scroll.min(max_scroll);
    let viewport_rows = agent_panel_visible_count_from(app, area, scroll);

    crate::pane::ScrollMetrics {
        offset_from_bottom: max_scroll.saturating_sub(scroll),
        max_offset_from_bottom: max_scroll,
        viewport_rows,
    }
}

pub(crate) fn agent_panel_scrollbar_rect(app: &AppState, area: Rect) -> Option<Rect> {
    let metrics = agent_panel_scroll_metrics(app, area);
    let body = agent_panel_body_rect(area, true);
    (should_show_scrollbar(metrics) && body.width > 0 && body.height > 0).then_some(Rect::new(
        area.x + area.width.saturating_sub(1),
        body.y,
        1,
        body.height,
    ))
}

/// The state a space wants the user for, as its list entry shows it.
fn workspace_edge_state(app: &AppState, ws_idx: usize) -> (AgentState, bool) {
    match workspace_parent_group_state(app, ws_idx) {
        Some((key, true)) => space_attention_state(app, &key),
        _ => app
            .workspaces
            .get(ws_idx)
            .map_or((AgentState::Unknown, true), |ws| {
                ws.attention_state(&app.terminals)
            }),
    }
}

/// What the edge rows and fog of the spaces list draw, for the list in `area`
/// (the whole spaces section, header and footer included).
pub(crate) fn workspace_overflow_plan(
    app: &AppState,
    area: Rect,
    cards: &[crate::app::state::WorkspaceCardArea],
) -> OverflowPlan {
    let metrics = workspace_list_scroll_metrics(app, area);
    let body = workspace_list_body_rect(area, should_show_scrollbar(metrics));
    if body.width == 0 || body.height == 0 {
        return OverflowPlan::default();
    }
    let entries = workspace_list_entries(app);
    let items: Vec<EdgeItem> = entries
        .iter()
        .map(|entry| {
            let WorkspaceListEntry::Workspace { ws_idx, .. } = entry;
            let (state, seen) = workspace_edge_state(app, *ws_idx);
            EdgeItem {
                pin_rank: app
                    .workspaces
                    .get(*ws_idx)
                    .filter(|ws| ws.pin_order.is_some())
                    .map(|ws| workspace_pin_rank(app, ws)),
                state,
                seen,
            }
        })
        .collect();
    let is_navigating = matches!(app.mode, Mode::Navigate);
    let dragged = match app.drag.as_ref().map(|drag| &drag.target) {
        Some(crate::app::state::DragTarget::WorkspaceReorder { source_ws_idx, .. }) => {
            Some(*source_ws_idx)
        }
        _ => None,
    };
    let scroll = app.workspace_scroll.min(metrics.max_offset_from_bottom);
    let (reserve, _) = workspace_list_window(app, area, scroll);
    let visible: Vec<VisibleItem> = cards
        .iter()
        .filter_map(|card| {
            let index = entries.iter().position(|entry| {
                matches!(entry, WorkspaceListEntry::Workspace { ws_idx, .. } if *ws_idx == card.ws_idx)
            })?;
            Some(VisibleItem {
                index,
                rect: card.rect,
                exempt: Some(card.ws_idx) == app.active
                    || (is_navigating && card.ws_idx == app.selected)
                    || dragged == Some(card.ws_idx),
            })
        })
        .collect();
    overflow::plan(
        app.sidebar_overflow.fog(),
        &items,
        scroll,
        &visible,
        body,
        reserve,
    )
}

/// The edge rows of the spaces list, for hit-testing a click on one.
pub(crate) fn workspace_edge_rows(app: &AppState) -> Vec<EdgeRow> {
    if app.sidebar_collapsed || !app.sidebar_overflow.edge_rows() {
        return Vec::new();
    }
    let area = workspace_list_rect(app.view.sidebar_rect, app.sidebar_section_split);
    let cards = if app.view.workspace_card_areas.is_empty() {
        compute_workspace_card_areas(app, app.view.sidebar_rect)
    } else {
        app.view.workspace_card_areas.clone()
    };
    workspace_overflow_plan(app, area, &cards).rows
}

/// What the edge rows and fog of the agent panel draw, for the panel in `area`.
pub(crate) fn agent_overflow_plan(
    app: &AppState,
    details: &[AgentPanelEntry],
    area: Rect,
) -> OverflowPlan {
    let metrics = agent_panel_scroll_metrics(app, area);
    let body = agent_panel_body_rect(area, should_show_scrollbar(metrics));
    if body.width == 0 || body.height == 0 {
        return OverflowPlan::default();
    }
    let items: Vec<EdgeItem> = details
        .iter()
        .map(|detail| EdgeItem {
            pin_rank: detail
                .pin_order
                .is_some()
                .then(|| agent_pin_rank(details, detail)),
            state: detail.state,
            seen: detail.seen,
        })
        .collect();
    let scroll = app.agent_panel_scroll.min(metrics.max_offset_from_bottom);
    let (reserve, count) = agent_panel_window(app, area, scroll);
    let mut row_y = body.y + reserve.top;
    let mut visible = Vec::new();
    for (index, detail) in details.iter().enumerate().skip(scroll).take(count) {
        let height = agent_entry_height_in_body(app, detail, body.height);
        visible.push(VisibleItem {
            index,
            rect: Rect::new(body.x, row_y, body.width, height),
            exempt: app.is_active_pane(detail.ws_idx, detail.tab_idx, detail.pane_id),
        });
        row_y =
            row_y
                .saturating_add(height)
                .saturating_add(agent_entry_gap(app, index, details.len()));
    }
    overflow::plan(
        app.sidebar_overflow.fog(),
        &items,
        scroll,
        &visible,
        body,
        reserve,
    )
}

/// The edge rows of the agent panel, for hit-testing a click on one.
pub(crate) fn agent_edge_rows(app: &AppState) -> Vec<EdgeRow> {
    if app.sidebar_collapsed || !app.sidebar_overflow.edge_rows() {
        return Vec::new();
    }
    let (_, detail_area) =
        expanded_sidebar_sections(app.view.sidebar_rect, app.sidebar_section_split);
    agent_overflow_plan(app, &agent_panel_entries(app), detail_area).rows
}

/// The colour the fog lifts from: what the sidebar really shows. The sidebar's
/// own background when it is RGB; else the host terminal's background, when
/// herdr already learned it (the OSC 11 reply kept in `host_terminal_theme`);
/// else the palette's panel background. Read from state, never queried here.
fn fog_base(app: &AppState) -> Color {
    let p = &app.palette;
    if matches!(p.sidebar_bg, Color::Rgb(..)) {
        return p.sidebar_bg;
    }
    match app.host_terminal_theme.background {
        Some(bg) => Color::Rgb(bg.r, bg.g, bg.b),
        None => p.panel_bg,
    }
}

/// Paints the plan: the fog first, so row content keeps its own colours, then
/// the edge rows over the space the list reserved for them.
fn render_overflow(app: &AppState, frame: &mut Frame, plan: &OverflowPlan) {
    let p = &app.palette;
    let base = fog_base(app);
    {
        let buf = frame.buffer_mut();
        for band in &plan.fog {
            let tint = band
                .tint
                .map(|(state, seen)| state_label_color(state, seen, &app.state_icon_colors()));
            let Some(bg) = overflow::fog_color(base, p.text, tint, band.level) else {
                continue;
            };
            for y in band.rect.y..band.rect.y + band.rect.height {
                for x in band.rect.x..band.rect.x + band.rect.width {
                    buf[(x, y)].set_style(Style::default().bg(bg));
                }
            }
        }
    }
    for row in &plan.rows {
        let urgent_color = row
            .summary
            .urgent
            .map(|(state, seen, _)| state_label_color(state, seen, &app.state_icon_colors()));
        let line = overflow::edge_row_line(
            row,
            row.rect.width,
            overflow::dim_style(p.overlay0),
            urgent_color,
        );
        // The row sits over space the list reserved, but a border line may
        // have been drawn there as "blank": start from an empty row.
        {
            let buf = frame.buffer_mut();
            for x in row.rect.x..row.rect.x + row.rect.width {
                let cell = &mut buf[(x, row.rect.y)];
                cell.set_symbol(" ");
                cell.set_style(Style::reset().bg(p.sidebar_bg));
            }
        }
        frame.render_widget(Paragraph::new(line), row.rect);
    }
}

pub(crate) fn compute_workspace_list_areas(
    app: &AppState,
    area: Rect,
) -> (Vec<crate::app::state::WorkspaceCardArea>, Vec<()>) {
    let ws_area = workspace_list_rect(area, app.sidebar_section_split);
    if ws_area == Rect::default() {
        return (Vec::new(), Vec::new());
    }

    let metrics = workspace_list_scroll_metrics(app, ws_area);
    let body = workspace_list_body_rect(ws_area, should_show_scrollbar(metrics));
    if body.width == 0 || body.height == 0 {
        return (Vec::new(), Vec::new());
    }

    let scroll = app.workspace_scroll;
    let (reserve, _) = workspace_list_window(app, ws_area, scroll);
    let mut row_y = body.y + reserve.top;
    let body_bottom = (body.y + body.height).saturating_sub(reserve.bottom);
    let mut cards = Vec::new();
    let headers = Vec::new();

    let entries = workspace_list_entries(app);
    for (entry_idx, entry) in entries.iter().enumerate().skip(scroll) {
        match entry {
            WorkspaceListEntry::Workspace { ws_idx, indented } => {
                let Some(ws) = app.workspaces.get(*ws_idx) else {
                    continue;
                };
                let row_height = workspace_row_height_in_body(app, ws, *indented, body.height);
                let gap = workspace_entry_gap(app, &entries, entry_idx);
                if row_y.saturating_add(row_height) > body_bottom {
                    break;
                }
                cards.push(crate::app::state::WorkspaceCardArea {
                    ws_idx: *ws_idx,
                    rect: Rect::new(body.x, row_y, body.width, row_height),
                    indented: *indented,
                });
                row_y = row_y
                    .saturating_add(row_height)
                    .saturating_add(gap)
                    .min(body_bottom);
            }
        }
    }

    (cards, headers)
}

pub(crate) fn compute_workspace_card_areas(
    app: &AppState,
    area: Rect,
) -> Vec<crate::app::state::WorkspaceCardArea> {
    compute_workspace_list_areas(app, area).0
}

pub(crate) fn workspace_group_chevron_rect(card: &crate::app::state::WorkspaceCardArea) -> Rect {
    if card.rect.width == 0 || card.rect.height == 0 {
        return Rect::default();
    }

    Rect::new(
        card.rect.x + card.rect.width.saturating_sub(1),
        card.rect.y,
        1,
        1,
    )
}

/// Auto-scale sidebar width based on workspace identity + agent summary.
pub(crate) fn collapsed_sidebar_sections(area: Rect) -> (Rect, Option<u16>, Rect) {
    let content = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    if content.width == 0 || content.height == 0 {
        return (Rect::default(), None, Rect::default());
    }

    if content.height < 7 {
        return (content, None, Rect::default());
    }

    let total_h = content.height as usize;
    let ws_h = total_h.div_ceil(2);
    let detail_h = total_h.saturating_sub(ws_h + 1);
    if ws_h == 0 || detail_h == 0 {
        return (content, None, Rect::default());
    }

    let divider_y = content.y + ws_h as u16;
    let ws_area = Rect::new(content.x, content.y, content.width, ws_h as u16);
    let detail_area = Rect::new(content.x, divider_y + 1, content.width, detail_h as u16);
    (ws_area, Some(divider_y), detail_area)
}

fn workspace_selection_background(p: &Palette, is_active: bool) -> Color {
    if is_active && p.selection_bg == Color::Reset {
        p.active_row_bg
    } else {
        p.selection_bg
    }
}

/// Collapsed sidebar: workspace glance on top, compact agent list below.
pub(super) fn render_sidebar_collapsed(app: &AppState, frame: &mut Frame, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let is_navigating = matches!(app.mode, Mode::Navigate);

    let p = &app.palette;
    frame
        .buffer_mut()
        .set_style(area, Style::default().bg(p.sidebar_bg));
    let sep_style = if is_navigating {
        Style::default().fg(p.accent)
    } else {
        Style::default().fg(p.surface_dim)
    };
    let sep_x = area.x + area.width.saturating_sub(1);
    let buf = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        buf[(sep_x, y)].set_symbol("│");
        buf[(sep_x, y)].set_style(sep_style);
    }

    let (ws_area, divider_y, detail_area) = collapsed_sidebar_sections(area);
    if ws_area == Rect::default() {
        render_sidebar_toggle(app, frame, area, true, p);
        return;
    }

    // `left`/`right` get a dedicated edge column for the active bar (the
    // collapsed width grows by one) so the bar never overwrites the jump
    // symbol or the state icon; `left` shifts all rows right by one cell.
    let border_mode = app.sidebar_active_border;
    let has_bar_column = matches!(
        border_mode,
        crate::config::SidebarActiveBorderConfig::Left
            | crate::config::SidebarActiveBorderConfig::Right
    );
    let bar_reserve: u16 = u16::from(border_mode == crate::config::SidebarActiveBorderConfig::Left);

    // Rows follow the visible list order so the labels stay the jump targets
    // `prefix+1..9,a..z` resolves via `workspace_at_visible_position`.
    for (visible_idx, ws_idx) in app.visible_workspace_order().into_iter().enumerate() {
        let Some(ws) = app.workspaces.get(ws_idx) else {
            continue;
        };
        let y = ws_area.y + visible_idx as u16;
        if y >= ws_area.y + ws_area.height {
            break;
        }
        let (agg_state, agg_seen) = ws.display_state(&app.terminals);
        let (icon, icon_style) = state_icon(
            agg_state,
            agg_seen,
            &app.state_icon_symbols(),
            &app.state_icon_colors(),
        );
        let symbol = crate::config::jump_symbol(visible_idx).unwrap_or(' ');
        let is_selected = visible_idx == app.selected && is_navigating;
        let is_active = Some(visible_idx) == app.active;
        let selection_bg = workspace_selection_background(p, is_active);
        let row_style = if is_selected {
            Style::default().bg(selection_bg)
        } else if is_active {
            Style::default().bg(p.active_row_bg)
        } else {
            Style::default()
        };
        let num_style = if is_selected {
            Style::default().fg(p.overlay1).bg(selection_bg)
        } else if is_active {
            Style::default()
                .fg(p.text)
                .bg(p.active_row_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(app.workspace_number_color.unwrap_or(p.overlay0))
        };

        if is_selected || is_active {
            let buf = frame.buffer_mut();
            for x in ws_area.x..ws_area.x + ws_area.width {
                buf[(x, y)].set_style(row_style);
            }
        }

        let mut spans = Vec::new();
        if bar_reserve > 0 {
            spans.push(Span::styled(" ", row_style));
        }
        spans.push(Span::styled(symbol.to_string(), num_style));
        spans.push(Span::styled(" ", row_style));
        spans.push(Span::styled(icon, icon_style));
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect::new(ws_area.x, y, ws_area.width, 1),
        );

        // Bar draws after the row so it wins the edge cell.
        if is_active && has_bar_column {
            let x = if border_mode == crate::config::SidebarActiveBorderConfig::Left {
                ws_area.x
            } else {
                ws_area.x + ws_area.width.saturating_sub(1)
            };
            draw_sidebar_active_border_bar(app, frame, x, y, 1, ws_area.y + ws_area.height);
        }
    }

    if let Some(divider_y) = divider_y {
        let buf = frame.buffer_mut();
        let divider_color = if app.agent_view_override.is_some() {
            p.accent
        } else {
            p.surface_dim
        };
        for x in ws_area.x..ws_area.x + ws_area.width {
            buf[(x, divider_y)].set_symbol("─");
            buf[(x, divider_y)].set_style(Style::default().fg(divider_color));
        }
    }

    let detail_content_area = Rect::new(
        detail_area.x,
        detail_area.y,
        detail_area.width,
        detail_area.height.saturating_sub(1),
    );
    if detail_content_area != Rect::default() {
        for (detail_idx, detail) in agent_panel_entries(app).iter().enumerate() {
            let y = detail_content_area.y + detail_idx as u16;
            if y >= detail_content_area.y + detail_content_area.height {
                break;
            }
            let symbol = crate::config::jump_symbol(detail_idx).unwrap_or(' ');
            let is_active = app.is_active_pane(detail.ws_idx, detail.tab_idx, detail.pane_id);
            let row_style = if is_active {
                Style::default().bg(p.active_row_bg)
            } else {
                Style::default()
            };
            let position_style = if is_active {
                Style::default()
                    .fg(p.text)
                    .bg(p.active_row_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(app.agent_number_color.unwrap_or(p.overlay0))
            };
            let (icon, icon_style) = agent_state_icon(
                detail.state,
                detail.seen,
                detail.background_work,
                detail.spinner_frame(app.working_spinner_frame()),
                &app.state_icon_symbols(),
                &app.state_icon_colors(),
            );

            if is_active {
                let buf = frame.buffer_mut();
                for x in detail_content_area.x..detail_content_area.x + detail_content_area.width {
                    buf[(x, y)].set_style(row_style);
                }
            }

            let mut spans = Vec::new();
            if bar_reserve > 0 {
                spans.push(Span::styled(" ", row_style));
            }
            spans.push(Span::styled(format!("{symbol:<2}"), position_style));
            spans.push(Span::styled(icon, icon_style));
            frame.render_widget(
                Paragraph::new(Line::from(spans)),
                Rect::new(detail_content_area.x, y, detail_content_area.width, 1),
            );

            // Bar draws after the row so it wins the edge cell.
            if is_active && has_bar_column {
                let x = if border_mode == crate::config::SidebarActiveBorderConfig::Left {
                    detail_content_area.x
                } else {
                    detail_content_area.x + detail_content_area.width.saturating_sub(1)
                };
                draw_sidebar_active_border_bar(
                    app,
                    frame,
                    x,
                    y,
                    1,
                    detail_content_area.y + detail_content_area.height,
                );
            }
        }
    }

    render_sidebar_toggle(app, frame, area, true, p);
}

pub(crate) fn workspace_drop_slots(
    app: &AppState,
    cards: &[crate::app::state::WorkspaceCardArea],
    area: Rect,
) -> Vec<(crate::app::state::WorkspaceDropTarget, u16)> {
    if area.height == 0 || cards.is_empty() {
        return Vec::new();
    }
    let list_bottom = area.y + area.height.saturating_sub(1);
    let entries = workspace_list_entries(app);
    let entry_position = |ws_idx| {
        entries.iter().position(|entry| {
            matches!(
                entry,
                WorkspaceListEntry::Workspace {
                    ws_idx: entry_ws_idx,
                    ..
                } if *entry_ws_idx == ws_idx
            )
        })
    };
    let block_root_at = |entry_idx: usize| {
        entries[..=entry_idx]
            .iter()
            .rev()
            .find_map(|entry| match entry {
                WorkspaceListEntry::Workspace {
                    ws_idx,
                    indented: false,
                } => Some(*ws_idx),
                WorkspaceListEntry::Workspace { .. } => None,
            })
    };

    let mut slots = Vec::new();
    let mut previous_root = None;
    for card in cards {
        let Some(entry_idx) = entry_position(card.ws_idx) else {
            continue;
        };
        let Some(root_idx) = block_root_at(entry_idx) else {
            continue;
        };
        if previous_root == Some(root_idx) {
            continue;
        }
        previous_root = Some(root_idx);
        if let Some(row) = card.rect.y.checked_sub(1).filter(|row| *row < list_bottom) {
            slots.push((
                crate::app::state::WorkspaceDropTarget::Before(root_idx),
                row,
            ));
        }
    }

    let Some(last) = cards.last() else {
        return slots;
    };
    let Some(last_entry_idx) = entry_position(last.ws_idx) else {
        return slots;
    };
    let next_entry = entries.get(last_entry_idx.saturating_add(1));
    if matches!(
        next_entry,
        Some(WorkspaceListEntry::Workspace { indented: true, .. })
    ) {
        return slots;
    }
    let target = match next_entry {
        Some(WorkspaceListEntry::Workspace { ws_idx, .. }) => {
            crate::app::state::WorkspaceDropTarget::Before(*ws_idx)
        }
        None => crate::app::state::WorkspaceDropTarget::End,
    };
    let row = last.rect.y.saturating_add(last.rect.height);
    if row < list_bottom
        && slots
            .last()
            .is_none_or(|(last_target, _)| *last_target != target)
    {
        slots.push((target, row));
    }
    slots
}

pub(crate) fn workspace_drop_indicator_row(
    app: &AppState,
    cards: &[crate::app::state::WorkspaceCardArea],
    area: Rect,
    target: crate::app::state::WorkspaceDropTarget,
) -> Option<u16> {
    workspace_drop_slots(app, cards, area)
        .into_iter()
        .find_map(|(candidate, row)| (candidate == target).then_some(row))
}

pub(super) fn render_sidebar(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    frame: &mut Frame,
    area: Rect,
) {
    let p = &app.palette;
    frame
        .buffer_mut()
        .set_style(area, Style::default().bg(p.sidebar_bg));
    let is_navigating = matches!(app.mode, Mode::Navigate);
    let sep_style = if is_navigating {
        Style::default().fg(p.accent)
    } else {
        Style::default().fg(p.surface_dim)
    };

    let sep_x = area.x + area.width.saturating_sub(1);
    let buf = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        buf[(sep_x, y)].set_symbol("│");
        buf[(sep_x, y)].set_style(sep_style);
    }

    let (ws_area, detail_area) = expanded_sidebar_sections(area, app.sidebar_section_split);

    render_workspace_list(app, terminal_runtimes, frame, ws_area, is_navigating);
    render_agent_detail(app, terminal_runtimes, frame, detail_area);
    render_sidebar_toggle(app, frame, area, false, p);
}

fn resolved_token_spans(
    resolved: &[ResolvedToken],
    state_icon: (&str, Style),
    state_text_style: Style,
    workspace_style: Style,
    secondary_style: Style,
    custom_style: Style,
    p: &Palette,
    max_width: usize,
) -> Vec<Span<'static>> {
    let fixed_widths = resolved
        .iter()
        .map(|token| match &token.kind {
            ResolvedTokenKind::StateIcon => display_width(state_icon.0),
            ResolvedTokenKind::GitStatus { ahead, behind } => {
                usize::from(*ahead > 0) * display_width(&format!("↑{ahead}"))
                    + usize::from(*behind > 0) * display_width(&format!("↓{behind}"))
                    + usize::from(*ahead > 0 && *behind > 0)
            }
            _ => 0,
        })
        .collect::<Vec<_>>();
    let flexible_widths = resolved
        .iter()
        .map(|token| match &token.kind {
            ResolvedTokenKind::StateText(text)
            | ResolvedTokenKind::Workspace(text)
            | ResolvedTokenKind::Tab(text)
            | ResolvedTokenKind::Pane(text)
            | ResolvedTokenKind::Agent(text)
            | ResolvedTokenKind::TerminalTitle(text)
            | ResolvedTokenKind::Branch(text)
            | ResolvedTokenKind::Custom(text) => display_width(text),
            _ => 0,
        })
        .collect::<Vec<_>>();
    let minimum_width = |active: &[bool]| {
        let indices = active
            .iter()
            .enumerate()
            .filter_map(|(index, active)| active.then_some(index))
            .collect::<Vec<_>>();
        let content = indices
            .iter()
            .map(|index| fixed_widths[*index] + usize::from(flexible_widths[*index] > 0))
            .sum::<usize>();
        let separators = indices
            .windows(2)
            .map(|pair| display_width(tokens::separator(&resolved[pair[0]], &resolved[pair[1]])))
            .sum::<usize>();
        content + separators
    };
    // A kept token (`keep = true`) counts its full width when the row is
    // fitted, so the other flexible tokens shrink, and drop, before it does.
    let kept = resolved
        .iter()
        .zip(&flexible_widths)
        .map(|(token, width)| *width > 0 && token.style.keeps_width())
        .collect::<Vec<_>>();
    let needed_width = |active: &[bool]| {
        let kept_extra = active
            .iter()
            .enumerate()
            .filter(|(index, active)| **active && kept[*index])
            .map(|(index, _)| flexible_widths[index].saturating_sub(1))
            .sum::<usize>();
        minimum_width(active) + kept_extra
    };
    let mut active = resolved.iter().map(|_| true).collect::<Vec<_>>();
    if kept.iter().any(|kept| *kept) && needed_width(&active) > max_width {
        for (index, width) in flexible_widths.iter().enumerate() {
            if *width > 0 {
                active[index] = false;
            }
        }
        // Kept tokens first, last to first, at full width when that fits and
        // at their minimum otherwise; then the rest, which must leave the
        // kept tokens their full width.
        for index in (0..resolved.len()).rev().filter(|index| kept[*index]) {
            active[index] = true;
            if minimum_width(&active) > max_width {
                active[index] = false;
            }
        }
        for index in (0..resolved.len()).rev() {
            if flexible_widths[index] == 0 || kept[index] {
                continue;
            }
            active[index] = true;
            if needed_width(&active) > max_width {
                active[index] = false;
            }
        }
    } else if minimum_width(&active) > max_width {
        for (index, width) in flexible_widths.iter().enumerate() {
            if *width > 0 {
                active[index] = false;
            }
        }
        for index in (0..resolved.len()).rev() {
            if flexible_widths[index] == 0 {
                continue;
            }
            active[index] = true;
            if minimum_width(&active) > max_width {
                active[index] = false;
            }
        }
    }
    let visible_indices = active
        .iter()
        .enumerate()
        .filter_map(|(index, active)| active.then_some(index))
        .collect::<Vec<_>>();
    let separator_width = visible_indices
        .windows(2)
        .map(|pair| display_width(tokens::separator(&resolved[pair[0]], &resolved[pair[1]])))
        .sum::<usize>();
    let fixed_width = visible_indices
        .iter()
        .map(|index| fixed_widths[*index])
        .sum::<usize>();
    let mut budgets = flexible_widths
        .iter()
        .enumerate()
        .map(|(index, width)| usize::from(active[index] && *width > 0))
        .collect::<Vec<_>>();
    let minimum = budgets.iter().sum::<usize>();
    let mut remaining = max_width
        .saturating_sub(separator_width + fixed_width)
        .saturating_sub(minimum);
    // Kept tokens grow to their full width before anything else grows.
    for (index, budget) in budgets.iter_mut().enumerate() {
        if remaining == 0 {
            break;
        }
        if kept[index] && *budget > 0 {
            let grow = flexible_widths[index]
                .saturating_sub(*budget)
                .min(remaining);
            *budget += grow;
            remaining -= grow;
        }
    }
    while remaining > 0 {
        let mut grew = false;
        for (budget, width) in budgets.iter_mut().zip(&flexible_widths) {
            if *budget > 0 && *budget < *width {
                *budget += 1;
                remaining -= 1;
                grew = true;
                if remaining == 0 {
                    break;
                }
            }
        }
        if !grew {
            break;
        }
    }
    let cut = |token: &ResolvedToken, text: &str, budget: usize| {
        if token.style.truncates_start() {
            truncate_start(text, budget)
        } else {
            truncate_end(text, budget)
        }
    };
    let mut spans = Vec::new();
    for (position, index) in visible_indices.iter().copied().enumerate() {
        let token = &resolved[index];
        if position > 0 {
            let previous = &resolved[visible_indices[position - 1]];
            spans.push(Span::styled(
                tokens::separator(previous, token),
                Style::default().fg(p.overlay0).add_modifier(Modifier::DIM),
            ));
        }
        match &token.kind {
            ResolvedTokenKind::StateIcon => {
                spans.push(Span::styled(
                    state_icon.0.to_string(),
                    apply_token_style(state_icon.1, token.style),
                ));
            }
            ResolvedTokenKind::StateText(text) => {
                spans.push(Span::styled(
                    cut(token, text, budgets[index]),
                    apply_token_style(state_text_style, token.style),
                ));
            }
            ResolvedTokenKind::Workspace(text) => {
                spans.push(Span::styled(
                    cut(token, text, budgets[index]),
                    apply_token_style(workspace_style, token.style),
                ));
            }
            ResolvedTokenKind::Tab(text)
            | ResolvedTokenKind::Pane(text)
            | ResolvedTokenKind::Agent(text)
            | ResolvedTokenKind::Branch(text) => {
                spans.push(Span::styled(
                    cut(token, text, budgets[index]),
                    apply_token_style(secondary_style, token.style),
                ));
            }
            ResolvedTokenKind::GitStatus { ahead, behind } => {
                if *ahead > 0 {
                    spans.push(Span::styled(
                        format!("↑{ahead}"),
                        apply_token_style(Style::default().fg(p.green), token.style),
                    ));
                }
                if *ahead > 0 && *behind > 0 {
                    spans.push(Span::styled(
                        " ",
                        apply_token_style(Style::default(), token.style),
                    ));
                }
                if *behind > 0 {
                    spans.push(Span::styled(
                        format!("↓{behind}"),
                        apply_token_style(Style::default().fg(p.red), token.style),
                    ));
                }
            }
            ResolvedTokenKind::TerminalTitle(text) | ResolvedTokenKind::Custom(text) => {
                spans.push(Span::styled(
                    cut(token, text, budgets[index]),
                    apply_token_style(custom_style, token.style),
                ));
            }
        }
    }
    spans
}

fn apply_token_style(mut style: Style, patch: crate::config::SidebarTokenStyle) -> Style {
    if let Some(fg) = patch.fg {
        style = style.fg(fg.ratatui());
    }
    if let Some(bold) = patch.bold {
        style = if bold {
            style.add_modifier(Modifier::BOLD)
        } else {
            style.remove_modifier(Modifier::BOLD)
        };
    }
    if let Some(dim) = patch.dim {
        style = if dim {
            style.add_modifier(Modifier::DIM)
        } else {
            style.remove_modifier(Modifier::DIM)
        };
    }
    if let Some(italic) = patch.italic {
        style = if italic {
            style.add_modifier(Modifier::ITALIC)
        } else {
            style.remove_modifier(Modifier::ITALIC)
        };
    }
    style
}

fn sidebar_active_border_symbol(style: crate::config::PaneBorderActiveStyleConfig) -> &'static str {
    match style {
        crate::config::PaneBorderActiveStyleConfig::Light => "─",
        crate::config::PaneBorderActiveStyleConfig::Heavy => "━",
        crate::config::PaneBorderActiveStyleConfig::Double => "═",
    }
}

fn draw_sidebar_active_border_line(app: &AppState, frame: &mut Frame, x: u16, y: u16, width: u16) {
    if width == 0 {
        return;
    }
    let line = sidebar_active_border_symbol(app.pane_border_active_style).repeat(width as usize);
    frame.render_widget(
        Paragraph::new(Span::styled(
            line,
            Style::default().fg(app.pane_border_color(true)),
        )),
        Rect::new(x, y, width, 1),
    );
}

fn sidebar_active_border_vertical_symbol(
    style: crate::config::PaneBorderActiveStyleConfig,
) -> &'static str {
    match style {
        crate::config::PaneBorderActiveStyleConfig::Light => "│",
        crate::config::PaneBorderActiveStyleConfig::Heavy => "┃",
        crate::config::PaneBorderActiveStyleConfig::Double => "║",
    }
}

fn draw_sidebar_active_border_bar(
    app: &AppState,
    frame: &mut Frame,
    x: u16,
    y: u16,
    height: u16,
    clamp_bottom: u16,
) {
    let symbol = sidebar_active_border_vertical_symbol(app.pane_border_active_style);
    let color = app.pane_border_color(true);
    let buf = frame.buffer_mut();
    let area = buf.area;
    for row in y..y.saturating_add(height).min(clamp_bottom) {
        if x < area.x
            || x >= area.x.saturating_add(area.width)
            || row < area.y
            || row >= area.y.saturating_add(area.height)
        {
            continue;
        }
        let cell = &mut buf[(x, row)];
        cell.set_symbol(symbol);
        cell.set_style(Style::default().fg(color));
    }
}

fn sidebar_header_style(editorial: bool, p: &Palette) -> Style {
    if editorial {
        Style::default().fg(p.overlay0).add_modifier(Modifier::DIM)
    } else {
        Style::default().fg(p.overlay0).add_modifier(Modifier::BOLD)
    }
}

/// The editorial right-aligned jump label: an optional leader prefix plus the
/// jump symbol (e.g. "₽5", "₽⌥2", or bare "5"). Empty when numbers are off.
fn editorial_number_label(jump_number: Option<char>, prefix: &str) -> String {
    match jump_number {
        Some(symbol) => format!("{prefix}{symbol}"),
        None => String::new(),
    }
}

/// The agent row's todo count, `τ N`, shown at the right edge of the entry's
/// name row — the editorial jump label moves under it — so the open work on
/// a pane is visible without opening it. Empty when nothing is outstanding:
/// the row is for counts, the pane border keeps the always-present handle.
fn agent_todo_label(outstanding: usize) -> String {
    match outstanding {
        0 => String::new(),
        n if n > 99 => "τ 99+".to_string(),
        n => format!("τ {n}"),
    }
}

/// Columns reserved at the right edge of an editorial name row for the jump
/// label (its display width + one-cell gap, plus the right active-bar column
/// when that border mode is on).
fn editorial_number_reserve(
    applies: bool,
    label: &str,
    active_border: crate::config::SidebarActiveBorderConfig,
) -> u16 {
    if !applies || label.is_empty() {
        return 0;
    }
    display_width_u16(label)
        + 1
        + u16::from(active_border == crate::config::SidebarActiveBorderConfig::Right)
}

/// Draws the editorial right-aligned jump label on an entry's name row. The
/// label style carries no background, so the pre-filled active band (or the
/// row paragraph's band) shows through untouched.
fn draw_editorial_number(
    frame: &mut Frame,
    entry_rect: Rect,
    name_row_y: u16,
    bottom: u16,
    label: &str,
    style: Style,
    active_border: crate::config::SidebarActiveBorderConfig,
) {
    let width = display_width_u16(label);
    if name_row_y >= bottom || label.is_empty() || entry_rect.width < width + 4 {
        return;
    }
    let right_reserve =
        width + u16::from(active_border == crate::config::SidebarActiveBorderConfig::Right);
    let x = entry_rect.x + entry_rect.width.saturating_sub(right_reserve);
    frame.render_widget(
        Paragraph::new(Span::styled(label.to_string(), style)),
        Rect::new(x, name_row_y, width, 1),
    );
}

fn render_workspace_list(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    frame: &mut Frame,
    area: Rect,
    is_navigating: bool,
) {
    let p = &app.palette;
    let dragged_ws_idx = match app.drag.as_ref().map(|drag| &drag.target) {
        Some(crate::app::state::DragTarget::WorkspaceReorder { source_ws_idx, .. }) => {
            Some(*source_ws_idx)
        }
        _ => None,
    };
    let insertion_row = match app.drag.as_ref().map(|drag| &drag.target) {
        Some(crate::app::state::DragTarget::WorkspaceReorder {
            drop_target: Some(drop_target),
            ..
        }) => workspace_drop_indicator_row(app, &app.view.workspace_card_areas, area, *drop_target),
        _ => None,
    };

    let editorial = matches!(
        app.sidebar_style,
        crate::config::SidebarStyleConfig::Editorial
    );
    let list_bottom = area.y + area.height.saturating_sub(1);
    if area.height > 0 {
        let header_text = if editorial { " SPACES" } else { " spaces" };
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                header_text,
                sidebar_header_style(editorial, p),
            )])),
            Rect::new(area.x, area.y, area.width, 1),
        );
        // Right-align the server host name on the same header row, styled as
        // part of the header. Casing follows the header (uppercase editorial,
        // lowercase otherwise). Hidden entirely — never truncated — when the
        // row cannot fit the header text, a two-cell gap, the label, and one
        // cell of right padding.
        if app.show_host {
            if let Some(host) = &app.host_label {
                let label = if editorial {
                    host.to_uppercase()
                } else {
                    host.to_lowercase()
                };
                let label_width = display_width_u16(&label);
                let needed = display_width_u16(header_text)
                    .saturating_add(2)
                    .saturating_add(label_width)
                    .saturating_add(1);
                if label_width > 0 && area.width >= needed {
                    let x = area.x + area.width.saturating_sub(label_width + 1);
                    frame.render_widget(
                        Paragraph::new(Line::from(vec![Span::styled(
                            label,
                            sidebar_header_style(editorial, p),
                        )])),
                        Rect::new(x, area.y, label_width, 1),
                    );
                }
            }
        }
    }

    let metrics = workspace_list_scroll_metrics(app, area);
    let scrollbar_rect = workspace_list_scrollbar_rect(app, area);
    let cards = &app.view.workspace_card_areas;
    // Jump labels follow the full visible order so they match what
    // `keys.switch_workspace` (prefix+1..9, a..z) resolves via
    // `workspace_at_visible_position`, regardless of scroll or priority sort.
    let visible_order = app
        .show_workspace_numbers
        .then(|| app.visible_workspace_order());
    // `left` reserves a dedicated leftmost column for the active bar, so it never
    // overwrites the chevron/dot/number/name; all rows shift right by one cell.
    let bar_reserve: u16 =
        u16::from(app.sidebar_active_border == crate::config::SidebarActiveBorderConfig::Left);

    for card in cards {
        let i = card.ws_idx;
        let ws = &app.workspaces[i];
        let row_y = card.rect.y;
        let row_height = card.rect.height;
        let selected = i == app.selected && is_navigating;
        let is_active = Some(i) == app.active;
        let is_dragged = dragged_ws_idx == Some(i);
        let highlighted = selected || is_active || is_dragged;
        let (agg_state, agg_seen) = ws.display_state(&app.terminals);

        // Jump number for this card, if enabled and within the label range.
        let jump_number = visible_order.as_ref().and_then(|order| {
            let pos = order.iter().position(|idx| *idx == i)?;
            crate::config::jump_symbol(pos)
        });

        if highlighted {
            let bg = if selected {
                workspace_selection_background(p, is_active)
            } else if is_dragged {
                p.surface1
            } else {
                p.active_row_bg
            };
            let buf = frame.buffer_mut();
            for y in row_y..row_y + row_height {
                if y >= list_bottom {
                    break;
                }
                for x in card.rect.x..card.rect.x + card.rect.width {
                    buf[(x, y)].set_style(Style::default().bg(bg));
                }
            }
        }

        // Active-space border lines live in the blank gap rows between cards,
        // so they never shift the list layout. They only appear when a
        // `row_gap` leaves a blank row to draw into. Bar modes render after the
        // card loop so the bar wins the cell over row content.
        let border_mode = app.sidebar_active_border;
        if is_active
            && matches!(
                border_mode,
                crate::config::SidebarActiveBorderConfig::Above
                    | crate::config::SidebarActiveBorderConfig::Below
                    | crate::config::SidebarActiveBorderConfig::Both
            )
        {
            let row_is_blank = |y: u16| {
                !cards
                    .iter()
                    .any(|c| y >= c.rect.y && y < c.rect.y + c.rect.height)
            };
            if border_mode != crate::config::SidebarActiveBorderConfig::Below {
                if let Some(top_y) = row_y.checked_sub(1) {
                    // area.y holds the " spaces" header; the row below it is padding.
                    if top_y > area.y && row_is_blank(top_y) {
                        draw_sidebar_active_border_line(
                            app,
                            frame,
                            card.rect.x,
                            top_y,
                            card.rect.width,
                        );
                    }
                }
            }
            let bottom_y = row_y + row_height;
            if border_mode != crate::config::SidebarActiveBorderConfig::Above
                && bottom_y < list_bottom
                && row_is_blank(bottom_y)
            {
                draw_sidebar_active_border_line(app, frame, card.rect.x, bottom_y, card.rect.width);
            }
        }

        let name_style = if selected || is_active || is_dragged {
            Style::default().fg(p.text).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(p.subtext0)
        };

        let label = ws.display_name_from(&app.terminals, terminal_runtimes);
        let display_label = if card.indented {
            grouped_child_display_label(
                &label,
                ws.head_label().as_deref(),
                ws.custom_name.is_some(),
            )
        } else {
            label
        };
        let parent_group = (!card.indented)
            .then(|| workspace_parent_group_state(app, i))
            .flatten();
        let (display_state, display_seen) = parent_group
            .as_ref()
            .filter(|(_, collapsed)| *collapsed)
            .map(|(key, _)| space_display_state(app, key))
            .unwrap_or((agg_state, agg_seen));
        let state_icon = state_icon(
            display_state,
            display_seen,
            &app.state_icon_symbols(),
            &app.state_icon_colors(),
        );
        let state_text_style = Style::default()
            .fg(state_label_color(
                display_state,
                display_seen,
                &app.state_icon_colors(),
            ))
            .add_modifier(Modifier::DIM);
        let branch_style = {
            let style = Style::default().fg(if selected || is_active {
                p.mauve
            } else {
                p.overlay0
            });
            // Editorial: inactive meta lines recede; the active entry keeps
            // its accent branch color undimmed.
            if editorial && !(selected || is_active) {
                style.add_modifier(Modifier::DIM)
            } else {
                style
            }
        };
        let token_values = ws.metadata_tokens.values();
        let rows = tokens::space_rows(
            &app.sidebar_spaces,
            SpaceTokenContext {
                workspace: &display_label,
                branch: ws.head_label().as_deref(),
                state_text: state_label(display_state, display_seen),
                ahead_behind: ws.git_ahead_behind(),
                tokens: &token_values,
                suppress_git_details: card.indented,
            },
        );

        // Row 0's leading prefix width doubles as the dot column; the jump number
        // sits under the dot on the second row, matching the pre-0.7.4 layout.
        let lead0: u16 = if card.indented {
            3
        } else if parent_group.is_some() {
            2
        } else {
            1
        };
        let lead_rest: u16 = if card.indented { 5 } else { 3 };
        let has_second_row = rows.len() >= 2;
        let number_label = editorial_number_label(jump_number, &app.workspace_number_prefix);

        for (row_index, resolved) in rows.iter().enumerate() {
            if row_index as u16 >= row_height || row_y + row_index as u16 >= list_bottom {
                break;
            }
            let mut spans = Vec::new();
            if bar_reserve > 0 {
                spans.push(Span::raw(" "));
            }
            if row_index == 0 {
                // The pin marker takes the last cell of the lead, so pinning
                // moves no text and changes no row height.
                let lead_mark = |pinned: bool| {
                    if pinned {
                        Span::styled(
                            PIN_MARKER,
                            Style::default().fg(pin_marker_color(workspace_pin_rank(app, ws))),
                        )
                    } else {
                        Span::raw(" ")
                    }
                };
                let pinned = ws.pin_order.is_some();
                if card.indented {
                    spans.push(Span::raw("  "));
                    spans.push(lead_mark(pinned));
                } else if let Some((_, collapsed)) = parent_group.as_ref() {
                    spans.push(Span::styled(
                        if *collapsed { "▸" } else { "▾" },
                        Style::default().fg(p.accent),
                    ));
                    spans.push(lead_mark(pinned));
                } else {
                    spans.push(lead_mark(pinned));
                }
            } else if !editorial && row_index == 1 && has_second_row && jump_number.is_some() {
                // Jump number in the dot column of the second row (under the dot).
                let symbol = jump_number.unwrap_or(' ');
                spans.push(Span::raw(" ".repeat(lead0 as usize)));
                spans.push(Span::styled(
                    symbol.to_string(),
                    Style::default().fg(app.workspace_number_color.unwrap_or(p.overlay0)),
                ));
                spans.push(Span::raw(
                    " ".repeat(lead_rest.saturating_sub(lead0).saturating_sub(1) as usize),
                ));
            } else {
                spans.push(Span::raw(" ".repeat(lead_rest as usize)));
            }
            let prefix_width = bar_reserve + if row_index == 0 { lead0 } else { lead_rest };
            // Editorial: the name row reserves the right edge for the jump
            // label so long names truncate before reaching it.
            let number_reserve = editorial_number_reserve(
                editorial && row_index == 0,
                &number_label,
                app.sidebar_active_border,
            );
            spans.extend(resolved_token_spans(
                resolved,
                state_icon,
                state_text_style,
                name_style,
                branch_style,
                branch_style,
                p,
                card.rect
                    .width
                    .saturating_sub(prefix_width + number_reserve) as usize,
            ));
            frame.render_widget(
                Paragraph::new(Line::from(spans)),
                Rect::new(card.rect.x, row_y + row_index as u16, card.rect.width, 1),
            );
        }
        if editorial {
            draw_editorial_number(
                frame,
                card.rect,
                row_y,
                list_bottom,
                &number_label,
                Style::default().fg(app.workspace_number_color.unwrap_or(p.overlay0)),
                app.sidebar_active_border,
            );
        }
    }

    // Bar modes overlay the active card's edge column after its content is
    // rendered, so the bar wins the cell.
    if matches!(
        app.sidebar_active_border,
        crate::config::SidebarActiveBorderConfig::Left
            | crate::config::SidebarActiveBorderConfig::Right
    ) {
        if let Some(card) = cards.iter().find(|card| Some(card.ws_idx) == app.active) {
            let x = if app.sidebar_active_border == crate::config::SidebarActiveBorderConfig::Left {
                card.rect.x
            } else {
                card.rect.x + card.rect.width.saturating_sub(1)
            };
            draw_sidebar_active_border_bar(
                app,
                frame,
                x,
                card.rect.y,
                card.rect.height,
                list_bottom,
            );
        }
    }

    if let Some(y) = insertion_row.filter(|y| *y < list_bottom) {
        let indicator_right = scrollbar_rect
            .map(|rect| rect.x)
            .unwrap_or(area.x + area.width);
        let buf = frame.buffer_mut();
        for x in area.x..indicator_right {
            buf[(x, y)].set_symbol("─");
            buf[(x, y)].set_style(Style::default().fg(p.accent));
        }
    }

    render_overflow(app, frame, &workspace_overflow_plan(app, area, cards));

    if let Some(track) = scrollbar_rect {
        render_scrollbar(frame, metrics, track, p.surface_dim, p.overlay0, "▕");
    }

    if app.mouse_capture && list_bottom > area.y {
        let new_rect = app.sidebar_new_button_rect();
        frame.render_widget(
            Paragraph::new(Span::styled(" new", Style::default().fg(p.overlay0))),
            new_rect,
        );

        let menu_rect = app.global_launcher_rect();
        let menu_line = if app.global_menu_attention_badge_visible() {
            Line::from(vec![
                Span::styled(
                    "● ",
                    Style::default().fg(p.accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled("menu", Style::default().fg(p.overlay0)),
            ])
        } else {
            Line::from(vec![Span::styled("menu", Style::default().fg(p.overlay0))])
        };
        frame.render_widget(
            Paragraph::new(menu_line).alignment(Alignment::Right),
            menu_rect,
        );
    }
}

fn render_agent_detail(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    frame: &mut Frame,
    area: Rect,
) {
    let p = &app.palette;

    if area.height < 3 {
        return;
    }

    let sep_line = "─".repeat(area.width as usize);
    frame.render_widget(
        Paragraph::new(Span::styled(&sep_line, Style::default().fg(p.surface_dim))),
        Rect::new(area.x, area.y, area.width, 1),
    );

    let editorial = matches!(
        app.sidebar_style,
        crate::config::SidebarStyleConfig::Editorial
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            if editorial { " AGENTS" } else { " agents" },
            sidebar_header_style(editorial, p),
        )])),
        Rect::new(area.x, area.y + 1, area.width, 1),
    );
    let control_label = active_agent_view_label(app)
        .unwrap_or_else(|| agent_panel_sort_label(app.agent_panel_sort));
    let toggle_rect = agent_panel_header_label_rect(area, control_label);
    if toggle_rect != Rect::default() {
        let color = if app.agent_view_override.is_some() {
            p.accent
        } else {
            p.overlay0
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                control_label,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ))
            .alignment(Alignment::Right),
            toggle_rect,
        );
    }

    let details = agent_panel_entries_from(app, terminal_runtimes);
    let metrics = agent_panel_scroll_metrics(app, area);
    let scrollbar_rect = agent_panel_scrollbar_rect(app, area);
    let body = agent_panel_body_rect(area, should_show_scrollbar(metrics));
    if body == Rect::default() {
        return;
    }
    if details.is_empty() && app.agent_view_override.is_some() {
        frame.render_widget(
            Paragraph::new(" no matching agents")
                .style(Style::default().fg(p.overlay0).add_modifier(Modifier::DIM)),
            Rect::new(body.x, body.y, body.width, 1),
        );
        return;
    }

    let scroll = app.agent_panel_scroll.min(metrics.max_offset_from_bottom);
    let (reserve, _) = agent_panel_window(app, area, scroll);
    let mut row_y = body.y + reserve.top;
    let body_bottom = (body.y + body.height).saturating_sub(reserve.bottom);
    // `left` reserves a dedicated leftmost column for the active bar so it never
    // overwrites the dot/number/name; all rows shift right by one cell.
    let bar_reserve: u16 =
        u16::from(app.sidebar_active_border == crate::config::SidebarActiveBorderConfig::Left);
    // Tracks whether the row directly above the current entry is a blank
    // spacer (only true when the previous entry left a `row_gap`).
    let mut prev_gap: u16 = 0;
    for (index, detail) in details.iter().enumerate().skip(scroll) {
        let colors = app.state_icon_colors();
        // The row's state text follows its icon: an agent parked behind its own
        // background work is coloured apart from one mid-turn.
        let label_color = if detail.background_work {
            colors.background
        } else {
            state_label_color(detail.state, detail.seen, &colors)
        };
        let rows = resolved_agent_rows(app, detail);
        let height = (rows.len().max(1) as u16).min(body.height);
        if row_y.saturating_add(height) > body_bottom {
            break;
        }

        let is_active = app.is_active_pane(detail.ws_idx, detail.tab_idx, detail.pane_id);

        // Jump label follows the visible panel order (matches keys.focus_agent
        // resolution), shown under the dot on the entry's second row.
        let jump_number = app
            .show_agent_numbers
            .then(|| crate::config::jump_symbol(index))
            .flatten();
        let has_second_row = rows.len() >= 2;
        let number_label = editorial_number_label(jump_number, &app.agent_number_prefix);
        // The todo count sits at the right edge of the name row in both
        // styles, with the editorial jump label under it on the second row.
        // A one-row entry keeps its jump label and has no room for the count.
        let todo_label = if has_second_row {
            agent_todo_label(detail.outstanding_todos)
        } else {
            String::new()
        };
        let editorial_number_row = u16::from(has_second_row);

        let gap = agent_entry_gap(app, index, details.len());
        // Active-agent border lines live in blank spacer rows between entries,
        // so they never shift layout; they only appear when a `row_gap` exists.
        // Bar modes render after the entry so the bar wins the cell.
        let border_mode = app.sidebar_active_border;
        if is_active
            && matches!(
                border_mode,
                crate::config::SidebarActiveBorderConfig::Above
                    | crate::config::SidebarActiveBorderConfig::Below
                    | crate::config::SidebarActiveBorderConfig::Both
            )
        {
            if border_mode != crate::config::SidebarActiveBorderConfig::Below
                && prev_gap > 0
                && row_y > body.y
            {
                draw_sidebar_active_border_line(app, frame, body.x, row_y - 1, body.width);
            }
            let bottom_y = row_y + height;
            if border_mode != crate::config::SidebarActiveBorderConfig::Above
                && gap > 0
                && bottom_y < body_bottom
            {
                draw_sidebar_active_border_line(app, frame, body.x, bottom_y, body.width);
            }
        }
        let active_bar_top = (is_active
            && matches!(
                border_mode,
                crate::config::SidebarActiveBorderConfig::Left
                    | crate::config::SidebarActiveBorderConfig::Right
            ))
        .then_some(row_y);

        let row_style = if is_active {
            Style::default().bg(p.active_row_bg)
        } else {
            Style::default()
        };
        let name_style = if is_active {
            Style::default().fg(p.text).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(p.subtext0).add_modifier(Modifier::BOLD)
        };
        let status_style = if is_active {
            Style::default().fg(label_color)
        } else {
            Style::default().fg(label_color).add_modifier(Modifier::DIM)
        };
        let agent_style = Style::default().fg(p.overlay0).add_modifier(Modifier::DIM);
        let state_icon = agent_state_icon(
            detail.state,
            detail.seen,
            detail.background_work,
            detail.spinner_frame(app.working_spinner_frame()),
            &app.state_icon_symbols(),
            &app.state_icon_colors(),
        );

        for (row_index, resolved) in rows.iter().take(height as usize).enumerate() {
            let mut spans = Vec::new();
            if bar_reserve > 0 {
                spans.push(Span::raw(" "));
            }
            if !editorial && row_index == 1 && has_second_row && jump_number.is_some() {
                // Jump number in the dot column of the second row (under the dot).
                let symbol = jump_number.unwrap_or(' ');
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    symbol.to_string(),
                    Style::default().fg(app.agent_number_color.unwrap_or(p.overlay0)),
                ));
                spans.push(Span::raw(" "));
            } else if row_index == 0 && detail.pin_order.is_some() {
                // The pin marker takes the one-cell lead of the name row, so
                // pinning moves no text and changes no row height.
                spans.push(Span::styled(
                    PIN_MARKER,
                    Style::default().fg(pin_marker_color(agent_pin_rank(&details, detail))),
                ));
            } else {
                spans.push(Span::raw(if row_index == 0 { " " } else { "   " }));
            }
            let prefix_width = bar_reserve + if row_index == 0 { 1 } else { 3 };
            // Each row reserves the right edge for the label drawn there so
            // long text truncates before reaching it.
            let number_reserve = editorial_number_reserve(
                editorial && row_index as u16 == editorial_number_row,
                &number_label,
                app.sidebar_active_border,
            ) + editorial_number_reserve(
                row_index == 0,
                &todo_label,
                app.sidebar_active_border,
            );
            spans.extend(resolved_token_spans(
                resolved,
                state_icon,
                status_style,
                name_style,
                agent_style,
                agent_style,
                p,
                body.width.saturating_sub(prefix_width + number_reserve) as usize,
            ));
            frame.render_widget(
                Paragraph::new(Line::from(spans)).style(row_style),
                Rect::new(body.x, row_y + row_index as u16, body.width, 1),
            );
        }
        if editorial {
            draw_editorial_number(
                frame,
                Rect::new(body.x, row_y, body.width, height),
                row_y + editorial_number_row,
                body_bottom,
                &number_label,
                Style::default().fg(app.agent_number_color.unwrap_or(p.overlay0)),
                app.sidebar_active_border,
            );
        }
        if !todo_label.is_empty() {
            draw_editorial_number(
                frame,
                Rect::new(body.x, row_y, body.width, height),
                row_y,
                body_bottom,
                &todo_label,
                Style::default().fg(app.pane_todo_indicator_color(detail.highest_todo_priority)),
                app.sidebar_active_border,
            );
        }

        // Bar modes overlay the entry's edge column after its rows render, so
        // the bar wins the cell over row content.
        if let Some(top) = active_bar_top {
            let x = if border_mode == crate::config::SidebarActiveBorderConfig::Left {
                body.x
            } else {
                body.x + body.width.saturating_sub(1)
            };
            draw_sidebar_active_border_bar(app, frame, x, top, height, body_bottom);
        }

        row_y = row_y
            .saturating_add(height)
            .saturating_add(gap)
            .min(body_bottom);
        prev_gap = gap;
    }

    render_overflow(app, frame, &agent_overflow_plan(app, &details, area));

    if let Some(track) = scrollbar_rect {
        render_scrollbar(frame, metrics, track, p.surface_dim, p.overlay0, "▕");
    }
}

pub(crate) fn collapsed_sidebar_toggle_rect(area: Rect) -> Rect {
    let bottom_y = area.y + area.height.saturating_sub(1);
    let content_w = area.width.saturating_sub(1);
    if content_w == 0 || area.height == 0 {
        return Rect::default();
    }
    let x = area.x + content_w / 2;
    Rect::new(x, bottom_y, 1, 1)
}

pub(crate) fn expanded_sidebar_toggle_rect(area: Rect) -> Rect {
    if area.width <= 1 || area.height == 0 {
        return Rect::default();
    }
    Rect::new(
        area.x + area.width.saturating_sub(2),
        area.y + area.height.saturating_sub(1),
        1,
        1,
    )
}

fn render_sidebar_toggle(
    app: &AppState,
    frame: &mut Frame,
    area: Rect,
    collapsed: bool,
    p: &Palette,
) {
    let toggle_area = if collapsed {
        collapsed_sidebar_toggle_rect(area)
    } else {
        expanded_sidebar_toggle_rect(area)
    };
    if toggle_area == Rect::default() {
        return;
    }
    let icon = if collapsed { "»" } else { "«" };
    let icon_style = if collapsed && app.global_menu_attention_badge_visible() {
        Style::default().fg(p.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(p.overlay0)
    };
    frame.render_widget(Paragraph::new(Span::styled(icon, icon_style)), toggle_area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{detect::Agent, layout::PaneId, workspace::Workspace};
    use ratatui::{backend::TestBackend, layout::Direction, Terminal};

    fn row_text(buffer: &ratatui::buffer::Buffer, row: u16, width: u16) -> String {
        crate::ui::test_support::row_text_trimmed(buffer, Rect::new(0, row, width, 1))
    }

    fn find_symbol_x(buffer: &ratatui::buffer::Buffer, row: u16, width: u16, symbol: &str) -> u16 {
        (0..width)
            .find(|x| buffer[(*x, row)].symbol() == symbol)
            .unwrap_or_else(|| {
                panic!(
                    "missing symbol {symbol:?} in row {}",
                    row_text(buffer, row, width)
                )
            })
    }

    /// Renders the sidebar and returns the trimmed text of the "SPACES" header
    /// row within the workspace-list area (excluding the sidebar frame border),
    /// which is where the host label is drawn.
    fn render_sidebar_header_row(app: &mut crate::app::state::AppState, width: u16) -> String {
        app.ensure_test_terminals();
        let area = Rect::new(0, 0, width, 20);
        let ws_rect = workspace_list_rect(area, app.sidebar_section_split);
        let cards = compute_workspace_card_areas(&*app, ws_rect);
        app.view.workspace_card_areas = cards;
        let mut terminal = Terminal::new(TestBackend::new(width, 20)).unwrap();
        terminal
            .draw(|frame| render_sidebar(app, &TerminalRuntimeRegistry::new(), frame, area))
            .unwrap();
        let (ws_area, _) = expanded_sidebar_sections(area, app.sidebar_section_split);
        let buffer = terminal.backend().buffer();
        (ws_area.x..ws_area.x + ws_area.width)
            .map(|x| buffer[(x, ws_area.y)].symbol())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn sidebar_header_renders_host_label_right_aligned_editorial() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![Workspace::test_new("one")];
        app.sidebar_style = crate::config::SidebarStyleConfig::Editorial;
        app.show_host = true;
        app.host_label = Some("mbm5".into());

        let header = render_sidebar_header_row(&mut app, 26);
        assert!(header.contains("SPACES"), "header: {header:?}");
        assert!(
            header.trim_end().ends_with("MBM5"),
            "host label should be uppercased and right-aligned: {header:?}"
        );
    }

    #[test]
    fn sidebar_header_lowercases_host_label_in_default_style() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![Workspace::test_new("one")];
        app.show_host = true;
        app.host_label = Some("MBM5".into());

        let header = render_sidebar_header_row(&mut app, 26);
        assert!(
            header.trim_end().ends_with("mbm5"),
            "host label should be lowercased in default style: {header:?}"
        );
    }

    #[test]
    fn sidebar_header_hides_host_label_when_disabled() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![Workspace::test_new("one")];
        app.sidebar_style = crate::config::SidebarStyleConfig::Editorial;
        app.show_host = false;
        app.host_label = Some("mbm5".into());

        let header = render_sidebar_header_row(&mut app, 26);
        assert!(header.contains("SPACES"), "header: {header:?}");
        assert!(
            !header.to_lowercase().contains("mbm5"),
            "disabled host label must not render: {header:?}"
        );
    }

    #[test]
    fn sidebar_header_hides_host_label_when_row_too_narrow() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![Workspace::test_new("one")];
        app.sidebar_style = crate::config::SidebarStyleConfig::Editorial;
        app.show_host = true;
        // Far wider than " SPACES" + a two-cell gap + right pad allow at 26 cols.
        app.host_label = Some("a-really-long-hostname".into());

        let header = render_sidebar_header_row(&mut app, 26);
        assert!(header.contains("SPACES"), "header: {header:?}");
        assert!(
            !header.to_uppercase().contains("REALLY"),
            "overflowing host label must be hidden, not truncated: {header:?}"
        );
    }

    #[test]
    fn expanded_and_collapsed_sidebars_use_custom_background() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces.clear();
        app.active = None;
        app.palette.sidebar_bg = ratatui::style::Color::Rgb(12, 34, 56);
        let area = Rect::new(0, 0, 26, 20);

        let mut expanded = Terminal::new(TestBackend::new(26, 20)).unwrap();
        expanded
            .draw(|frame| render_sidebar(&app, &TerminalRuntimeRegistry::new(), frame, area))
            .unwrap();
        assert!(expanded
            .backend()
            .buffer()
            .content
            .iter()
            .all(|cell| cell.bg == app.palette.sidebar_bg));

        let mut collapsed = Terminal::new(TestBackend::new(26, 20)).unwrap();
        collapsed
            .draw(|frame| render_sidebar_collapsed(&app, frame, area))
            .unwrap();
        assert!(collapsed
            .backend()
            .buffer()
            .content
            .iter()
            .all(|cell| cell.bg == app.palette.sidebar_bg));
    }

    #[test]
    fn editorial_number_prefix_precedes_the_number() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![Workspace::test_new("one")];
        app.ensure_test_terminals();
        app.sidebar_style = crate::config::SidebarStyleConfig::Editorial;
        app.show_workspace_numbers = true;
        app.workspace_number_prefix = "₽".into();

        let area = Rect::new(0, 0, 26, 20);
        let ws_rect = workspace_list_rect(area, app.sidebar_section_split);
        app.view.workspace_card_areas = compute_workspace_card_areas(&app, ws_rect);
        let mut terminal = Terminal::new(TestBackend::new(26, 20)).unwrap();
        terminal
            .draw(|frame| render_sidebar(&app, &TerminalRuntimeRegistry::new(), frame, area))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let name_y = (0..20)
            .find(|y| row_text(buffer, *y, 25).contains("one"))
            .expect("workspace name row rendered");
        let name_row = row_text(buffer, name_y, 25);
        assert!(
            name_row.trim_end().ends_with("₽1"),
            "prefixed number should end the name row: {name_row:?}"
        );
    }

    #[test]
    fn editorial_number_label_and_reserve() {
        use crate::config::SidebarActiveBorderConfig;
        assert_eq!(editorial_number_label(Some('3'), ""), "3");
        assert_eq!(editorial_number_label(Some('3'), "₽"), "₽3");
        assert_eq!(editorial_number_label(Some('2'), "₽⌥"), "₽⌥2");
        assert_eq!(editorial_number_label(None, "₽"), "");

        // Bare number reserves symbol + gap; a "₽⌥2" label reserves its full
        // display width + gap.
        assert_eq!(
            editorial_number_reserve(true, "3", SidebarActiveBorderConfig::Left),
            2
        );
        assert_eq!(
            editorial_number_reserve(true, "₽⌥2", SidebarActiveBorderConfig::Left),
            4
        );
        assert_eq!(
            editorial_number_reserve(false, "₽⌥2", SidebarActiveBorderConfig::Left),
            0
        );
        assert_eq!(
            editorial_number_reserve(true, "", SidebarActiveBorderConfig::Left),
            0
        );
    }

    #[test]
    fn state_icon_symbols_layer_overrides_on_the_active_style() {
        let mut app = crate::app::state::AppState::test_new();
        let dots = app.state_icon_symbols();
        assert_eq!(
            (
                dots.blocked,
                dots.working,
                dots.done,
                dots.idle,
                dots.unknown
            ),
            ("●", "●", "●", "○", "·")
        );

        app.state_symbol_overrides.done = Some("◆".into());
        app.state_symbol_overrides.idle = Some("✔".into());
        let resolved = app.state_icon_symbols();
        assert_eq!(resolved.done, "◆");
        assert_eq!(resolved.idle, "✔");
        assert_eq!(resolved.working, "●");

        // The same overrides sit on top of whichever style is active.
        app.status_indicators = crate::config::StatusIndicatorStyle::Symbols;
        let resolved = app.state_icon_symbols();
        assert_eq!(resolved.done, "◆");
        assert_eq!(resolved.idle, "✔");
        assert_eq!(resolved.working, "◐");
        assert_eq!(resolved.blocked, "×");
        assert_eq!(resolved.symbol(AgentState::Idle, false), "◆");
        assert_eq!(resolved.symbol(AgentState::Idle, true), "✔");
    }

    #[test]
    fn state_icon_colors_resolve_overrides_with_theme_fallback() {
        use ratatui::style::Color;
        let mut app = crate::app::state::AppState::test_new();
        let defaults = app.state_icon_colors();
        assert_eq!(defaults.working, app.palette.yellow);
        assert_eq!(defaults.idle, app.palette.green);
        assert_eq!(defaults.done, app.palette.teal);
        assert_eq!(defaults.blocked, app.palette.red);
        assert_eq!(defaults.unknown, app.palette.overlay0);

        assert_eq!(
            defaults.background, app.palette.yellow,
            "unset background follows the working colour"
        );

        app.state_color_overrides.working = Some(Color::Rgb(255, 200, 50));
        app.state_color_overrides.idle = Some(Color::Rgb(74, 222, 128));
        let resolved = app.state_icon_colors();
        assert_eq!(resolved.working, Color::Rgb(255, 200, 50));
        assert_eq!(resolved.idle, Color::Rgb(74, 222, 128));
        assert_eq!(resolved.done, app.palette.teal);
        assert_eq!(
            resolved.background,
            Color::Rgb(255, 200, 50),
            "background still follows working until it is set itself"
        );
        app.state_color_overrides.background = Some(Color::Rgb(0, 200, 200));
        assert_eq!(app.state_icon_colors().background, Color::Rgb(0, 200, 200));

        let symbols = crate::app::state::StateIconSymbols::for_style(
            crate::config::StatusIndicatorStyle::Dots,
        );
        let (_, working_style) = state_icon(AgentState::Working, true, &symbols, &resolved);
        assert_eq!(working_style.fg, Some(Color::Rgb(255, 200, 50)));
        let (_, idle_style) = state_icon(AgentState::Idle, true, &symbols, &resolved);
        assert_eq!(idle_style.fg, Some(Color::Rgb(74, 222, 128)));
    }

    /// Puts a workspace's single root pane into the given agent state. Panics
    /// if the workspace has been split; the state fixtures here are all
    /// one pane per workspace.
    fn set_single_pane_state(
        app: &mut crate::app::state::AppState,
        ws_idx: usize,
        state: AgentState,
        seen: bool,
        seq: Option<u64>,
    ) {
        let workspace = &mut app.workspaces[ws_idx];
        let pane_id = workspace.tabs[0].root_pane;
        let terminal_id = workspace.tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        workspace.tabs[0].panes.get_mut(&pane_id).unwrap().seen = seen;
        let terminal = app.terminals.get_mut(&terminal_id).unwrap();
        terminal.detected_agent = Some(Agent::Pi);
        terminal.state = state;
        terminal.last_agent_state_change_seq = seq;
    }

    /// One single-pane workspace per `(state, seen, seq)` triple, in the order
    /// given, so a test can name an expected order by index.
    fn app_with_workspace_states(
        states: &[(AgentState, bool, Option<u64>)],
    ) -> crate::app::state::AppState {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = (0..states.len())
            .map(|idx| Workspace::test_new(&format!("w{idx}")))
            .collect();
        app.ensure_test_terminals();
        for (ws_idx, (state, seen, seq)) in states.iter().enumerate() {
            set_single_pane_state(&mut app, ws_idx, *state, *seen, *seq);
        }
        app
    }

    /// A collapsed worktree-space row aggregates across its member workspaces
    /// through a second, independent `max_by_key`. It is a display path too:
    /// fixing only the per-workspace aggregate would leave collapsed groups
    /// still masking a working agent behind a finished sibling.
    #[test]
    fn space_display_state_working_beats_done_unseen_across_members() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
        ];
        app.ensure_test_terminals();
        set_single_pane_state(&mut app, 0, AgentState::Idle, false, Some(1));
        set_single_pane_state(&mut app, 1, AgentState::Working, true, Some(2));

        assert_eq!(
            space_display_state(&app, "repo-key"),
            (AgentState::Working, true)
        );
        // The group's rank under `workspace_sort = priority` is unaffected.
        assert_eq!(
            space_attention_state(&app, "repo-key"),
            (AgentState::Idle, false)
        );
    }

    /// A fixture covering all five `(state, seen)` combinations, one per
    /// workspace. Attention order over it is blocked, done, working, idle,
    /// unknown — indices 3, 4, 1, 0, 2.
    fn mixed_state_workspaces() -> crate::app::state::AppState {
        app_with_workspace_states(&[
            (AgentState::Idle, true, Some(1)),
            (AgentState::Working, true, Some(2)),
            (AgentState::Unknown, true, Some(3)),
            (AgentState::Blocked, true, Some(4)),
            (AgentState::Idle, false, Some(5)),
        ])
    }

    fn workspace_order(app: &crate::app::state::AppState) -> Vec<usize> {
        workspace_list_entries(app)
            .iter()
            .map(|entry| match entry {
                WorkspaceListEntry::Workspace { ws_idx, .. } => *ws_idx,
            })
            .collect()
    }

    /// Characterization test: the agent panel's priority sort is an
    /// *attention* order, in which a pane that finished while unseen outranks
    /// an actively working one. Splitting display state away from attention
    /// must leave this ordering exactly as it is — surfacing a finished agent
    /// above a working one is the point of the sort.
    #[test]
    fn agent_panel_priority_sort_is_attention_ordered() {
        let mut app = mixed_state_workspaces();
        app.agent_panel_sort = AgentPanelSort::Priority;
        app.sort_motion_bubble = false;

        let entries = agent_panel_entries(&app);
        let order: Vec<_> = entries
            .iter()
            .map(|entry| agent_panel_status_key(entry.state, entry.seen))
            .collect();

        assert_eq!(order, vec!["blocked", "done", "working", "idle", "unknown"]);

        // The bubble-motion target must animate toward the order the sort
        // actually produced, across every state combination — not just the
        // three the older motion test covers.
        let entry_ids: Vec<_> = entries.iter().map(|entry| entry.pane_id).collect();
        assert_eq!(agent_panel_target_keys(&app), entry_ids);
    }

    /// Characterization test: the workspace list's priority sort is likewise
    /// an attention order, and the bubble-motion target agrees with it. The
    /// two must not diverge, or the motion chases an order the sort never
    /// produces and never settles.
    #[test]
    fn workspace_priority_sort_is_attention_ordered_and_matches_motion_target() {
        let mut app = mixed_state_workspaces();
        app.workspace_sort = WorkspaceSort::Priority;
        app.sort_motion_bubble = false;

        let order = workspace_order(&app);
        assert_eq!(order, vec![3, 4, 1, 0, 2]);

        let expected_keys: Vec<String> = order
            .iter()
            .map(|ws_idx| format!("ws:{}", app.workspaces[*ws_idx].id))
            .collect();
        assert_eq!(workspace_unit_target_keys(&app), expected_keys);
    }

    #[test]
    fn pinned_spaces_lead_in_pin_order_under_manual_and_priority() {
        let mut app = mixed_state_workspaces();
        app.sort_motion_bubble = false;
        app.workspace_sort = WorkspaceSort::Manual;
        assert_eq!(workspace_order(&app), vec![0, 1, 2, 3, 4]);

        app.pin_workspace(3);
        app.pin_workspace(1);
        assert_eq!(workspace_order(&app), vec![3, 1, 0, 2, 4]);

        app.workspace_sort = WorkspaceSort::Priority;
        // Priority order alone is 3, 4, 1, 0, 2; pins pull 3 then 1 first.
        assert_eq!(workspace_order(&app), vec![3, 1, 4, 0, 2]);

        app.unpin_workspace(3);
        assert_eq!(workspace_order(&app), vec![1, 3, 4, 0, 2]);
        app.unpin_workspace(1);
        assert_eq!(workspace_order(&app), vec![3, 4, 1, 0, 2]);
    }

    #[test]
    fn pinned_spaces_head_the_motion_target_in_pin_order() {
        let mut app = mixed_state_workspaces();
        app.workspace_sort = WorkspaceSort::Priority;
        app.pin_workspace(0);
        app.pin_workspace(2);
        let keys = workspace_unit_target_keys(&app);
        assert_eq!(keys.len(), 5);
        assert_eq!(keys[0], format!("ws:{}", app.workspaces[0].id));
        assert_eq!(keys[1], format!("ws:{}", app.workspaces[2].id));
    }

    #[test]
    fn pinned_spaces_hold_still_while_the_rest_bubbles() {
        let mut app = mixed_state_workspaces();
        app.workspace_sort = WorkspaceSort::Priority;
        app.sort_motion_bubble = true;
        app.pin_workspace(0);
        assert_eq!(workspace_order(&app), vec![0, 3, 4, 1, 2]);
    }

    #[test]
    fn agent_panel_target_keys_match_priority_entries_order() {
        // `agent_panel_target_keys` re-implements the panel's priority
        // comparator for the motion tick; pin the two against drift.
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![
            Workspace::test_new("one"),
            Workspace::test_new("two"),
            Workspace::test_new("three"),
        ];
        app.ensure_test_terminals();
        app.agent_panel_sort = AgentPanelSort::Priority;
        app.sort_motion_bubble = false;
        let states = [
            (AgentState::Idle, true, Some(3)),
            (AgentState::Working, true, Some(9)),
            (AgentState::Blocked, false, Some(5)),
        ];
        for (workspace, (state, seen, seq)) in app.workspaces.iter_mut().zip(states) {
            let pane_id = workspace.tabs[0].root_pane;
            let terminal_id = workspace.tabs[0].panes[&pane_id]
                .attached_terminal_id
                .clone();
            workspace.tabs[0].panes.get_mut(&pane_id).unwrap().seen = seen;
            let terminal = app.terminals.get_mut(&terminal_id).unwrap();
            terminal.detected_agent = Some(Agent::Pi);
            terminal.state = state;
            terminal.last_agent_state_change_seq = seq;
        }

        let entry_ids: Vec<_> = agent_panel_entries(&app)
            .into_iter()
            .map(|entry| entry.pane_id)
            .collect();
        assert_eq!(agent_panel_target_keys(&app), entry_ids);
    }

    fn three_agent_app(sort: AgentPanelSort) -> crate::app::state::AppState {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![
            Workspace::test_new("one"),
            Workspace::test_new("two"),
            Workspace::test_new("three"),
        ];
        app.ensure_test_terminals();
        app.agent_panel_sort = sort;
        app.sort_motion_bubble = false;
        let states = [
            (AgentState::Idle, true, Some(3)),
            (AgentState::Working, true, Some(9)),
            (AgentState::Blocked, false, Some(5)),
        ];
        for (workspace, (state, seen, seq)) in app.workspaces.iter_mut().zip(states) {
            let pane_id = workspace.tabs[0].root_pane;
            let terminal_id = workspace.tabs[0].panes[&pane_id]
                .attached_terminal_id
                .clone();
            workspace.tabs[0].panes.get_mut(&pane_id).unwrap().seen = seen;
            let terminal = app.terminals.get_mut(&terminal_id).unwrap();
            terminal.detected_agent = Some(Agent::Pi);
            terminal.state = state;
            terminal.last_agent_state_change_seq = seq;
        }
        app
    }

    fn agent_pane(app: &crate::app::state::AppState, ws_idx: usize) -> crate::layout::PaneId {
        app.workspaces[ws_idx].tabs[0].root_pane
    }

    fn agent_panel_order(app: &crate::app::state::AppState) -> Vec<usize> {
        agent_panel_entries(app)
            .iter()
            .map(|entry| entry.ws_idx)
            .collect()
    }

    fn motion_timing() -> crate::ui::list_motion::ListMotionTiming {
        crate::ui::list_motion::ListMotionTiming {
            settle: std::time::Duration::ZERO,
            step: std::time::Duration::from_millis(100),
            easing: crate::ui::list_motion::ListMotionEasing::Linear,
        }
    }

    /// One motion tick, the way the runtime does it.
    fn tick_workspace_motion(app: &mut crate::app::state::AppState, now: std::time::Instant) {
        let target = workspace_unit_target_keys(app);
        app.workspace_list_motion
            .tick(now, &target, motion_timing());
    }

    #[test]
    fn pinning_a_space_bubbles_it_up_through_the_frames_between() {
        let mut app = mixed_state_workspaces();
        app.workspace_sort = WorkspaceSort::Priority;
        app.sort_motion_bubble = true;
        let t0 = std::time::Instant::now();
        tick_workspace_motion(&mut app, t0);
        assert_eq!(workspace_order(&app), vec![3, 4, 1, 0, 2]);

        // ws 2 sits last; pinning aims it at the top.
        app.pin_workspace(2);
        assert_eq!(workspace_order(&app), vec![3, 4, 1, 0, 2], "no jump");
        let mut frames = Vec::new();
        for step in 1..=8u32 {
            tick_workspace_motion(&mut app, t0 + std::time::Duration::from_millis(100) * step);
            let order = workspace_order(&app);
            // Jump numbers and hit-testing read this same order.
            assert_eq!(
                workspace_list_entries(&app).len(),
                order.len(),
                "the rendered order is the one entries report"
            );
            frames.push(order);
        }
        assert!(
            frames
                .iter()
                .any(|f| f != &vec![3, 4, 1, 0, 2] && f != &vec![2, 3, 4, 1, 0]),
            "a frame between the old and the new place: {frames:?}"
        );
        assert_eq!(frames.last().unwrap(), &vec![2, 3, 4, 1, 0]);

        // Unpinning bubbles it back down to where priority puts it.
        app.unpin_workspace(2);
        let mut last = Vec::new();
        for step in 9..=24u32 {
            tick_workspace_motion(&mut app, t0 + std::time::Duration::from_millis(100) * step);
            last = workspace_order(&app);
        }
        assert_eq!(last, vec![3, 4, 1, 0, 2]);
    }

    #[test]
    fn a_priority_change_never_moves_a_pinned_space() {
        let mut app = mixed_state_workspaces();
        app.workspace_sort = WorkspaceSort::Priority;
        app.sort_motion_bubble = true;
        app.pin_workspace(0);
        let t0 = std::time::Instant::now();
        for step in 0..8u32 {
            tick_workspace_motion(&mut app, t0 + std::time::Duration::from_millis(100) * step);
        }
        assert_eq!(workspace_order(&app)[0], 0);

        // Make ws 4 the most urgent: it climbs, ws 0 keeps the top slot in
        // every frame.
        let terminal_id = {
            let ws = &app.workspaces[1];
            let pane = ws.tabs[0].root_pane;
            ws.tabs[0].panes[&pane].attached_terminal_id.clone()
        };
        app.terminals.get_mut(&terminal_id).unwrap().state = AgentState::Blocked;
        for step in 8..24u32 {
            tick_workspace_motion(&mut app, t0 + std::time::Duration::from_millis(100) * step);
            assert_eq!(workspace_order(&app)[0], 0, "pinned row held its place");
        }
        assert!(
            workspace_order(&app)[1..3].contains(&1),
            "the urgent row climbed to just under the pin"
        );
    }

    fn tick_agent_motion(app: &mut crate::app::state::AppState, now: std::time::Instant) {
        let target = agent_panel_target_keys(app);
        app.agent_panel_motion.tick(now, &target, motion_timing());
    }

    #[test]
    fn pinning_an_agent_bubbles_it_up_and_a_priority_change_never_moves_a_pin() {
        let mut app = three_agent_app(AgentPanelSort::Priority);
        app.sort_motion_bubble = true;
        let t0 = std::time::Instant::now();
        let at = |step: u32| t0 + std::time::Duration::from_millis(100) * step;
        tick_agent_motion(&mut app, t0);
        let before = agent_panel_order(&app);
        assert_eq!(before, vec![2, 1, 0]);

        // The last-ranked agent is pinned: it climbs frame by frame.
        let pane = agent_pane(&app, 0);
        app.pin_agent(pane);
        assert_eq!(agent_panel_order(&app), before, "no jump");
        let mut frames = Vec::new();
        for step in 1..=6 {
            tick_agent_motion(&mut app, at(step));
            frames.push(agent_panel_order(&app));
        }
        assert!(
            frames.iter().any(|f| f != &before && f[0] != 0),
            "a frame between: {frames:?}"
        );
        assert_eq!(frames.last().unwrap(), &vec![0, 2, 1]);

        // A priority change underneath leaves the pinned agent on top.
        let terminal_id = app.workspaces[1].tabs[0].panes[&app.workspaces[1].tabs[0].root_pane]
            .attached_terminal_id
            .clone();
        app.terminals.get_mut(&terminal_id).unwrap().state = AgentState::Blocked;
        app.terminals
            .get_mut(&terminal_id)
            .unwrap()
            .last_agent_state_change_seq = Some(99);
        for step in 7..20 {
            tick_agent_motion(&mut app, at(step));
            assert_eq!(agent_panel_order(&app)[0], 0, "pinned agent held its place");
        }

        // Unpinned, it bubbles back down to where priority puts it.
        app.unpin_agent(pane);
        for step in 20..40 {
            tick_agent_motion(&mut app, at(step));
        }
        assert_eq!(agent_panel_order(&app)[2], 0);
    }

    #[test]
    fn pinned_agents_lead_in_pin_order_under_spaces_and_priority() {
        for sort in [AgentPanelSort::Spaces, AgentPanelSort::Priority] {
            let mut app = three_agent_app(sort);
            let unpinned = agent_panel_order(&app);
            // Pin the lowest-ranked agents first and second.
            let (a, b) = (agent_pane(&app, 0), agent_pane(&app, 1));
            app.pin_agent(b);
            app.pin_agent(a);
            let order = agent_panel_order(&app);
            assert_eq!(&order[..2], &[1, 0], "pinned lead in pin order");
            assert_eq!(order[2], 2);

            app.unpin_agent(b);
            app.unpin_agent(a);
            assert_eq!(agent_panel_order(&app), unpinned, "unpin returns the rows");
        }
    }

    #[test]
    fn pinned_agents_lead_over_a_view_sort_too() {
        let mut app = three_agent_app(AgentPanelSort::Spaces);
        app.agent_view_override = Some(crate::api::schema::AgentViewSetParams {
            source: "test".into(),
            label: None,
            filter: None,
            sort: vec![crate::api::schema::AgentViewSort {
                field: crate::api::schema::AgentViewSortField::Builtin(
                    crate::api::schema::AgentViewBuiltinSortField::Status,
                ),
                order: crate::api::schema::AgentViewSortOrder::Desc,
            }],
        });
        let pane = agent_pane(&app, 2);
        app.pin_agent(pane);
        assert_eq!(agent_panel_order(&app)[0], 2);
    }

    fn channels(color: Color) -> (u8, u8, u8) {
        match color {
            Color::Rgb(r, g, b) => (r, g, b),
            other => panic!("an rgb colour expected, got {other:?}"),
        }
    }

    /// Fork issue 154: red, orange, yellow, yellow-green, then the green fade
    /// down to its floor; a rank's colour never depends on the pin count.
    #[test]
    fn the_pin_marker_runs_red_orange_yellow_then_a_green_fade_to_a_floor() {
        let hex = |rank| {
            let (r, g, b) = channels(pin_marker_color(rank));
            format!("#{r:02x}{g:02x}{b:02x}")
        };
        let expected = [
            "#f47b7b", "#f4ab7b", "#f4dc7b", "#dcf47b", "#b5f0b0", "#82ae7f", "#698c66", "#548a51",
            "#4f8a4a",
        ];
        for (rank, want) in expected.iter().enumerate() {
            assert_eq!(&hex(rank), want, "rank {rank}");
        }
        assert_eq!(hex(1000), "#4f8a4a", "the floor is held");
        assert_eq!(pin_marker_color(40), pin_marker_color(1000));
    }

    #[test]
    fn pinned_space_markers_get_darker_down_the_block_and_agents_count_from_their_own_top() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![
            Workspace::test_new("a"),
            Workspace::test_new("b"),
            Workspace::test_new("c"),
        ];
        app.ensure_test_terminals();
        app.active = Some(0);
        app.mode = Mode::Terminal;
        app.pin_workspace(2);
        app.pin_workspace(0);
        app.pin_workspace(1);
        // One pinned agent: the agent panel starts at its own top, rank 0.
        let pane = agent_pane(&app, 1);
        let terminal_id = app.workspaces[1].tabs[0].panes[&pane]
            .attached_terminal_id
            .clone();
        app.terminals
            .get_mut(&terminal_id)
            .unwrap()
            .set_detected_state(Some(Agent::Pi), AgentState::Idle);
        app.pin_agent(pane);

        let area = Rect::new(0, 0, 30, 30);
        app.view.workspace_card_areas = compute_workspace_card_areas(&app, area);
        let list_area = workspace_list_rect(area, app.sidebar_section_split);
        let mut terminal = Terminal::new(TestBackend::new(30, 30)).unwrap();
        terminal
            .draw(|frame| {
                render_workspace_list(
                    &app,
                    &TerminalRuntimeRegistry::new(),
                    frame,
                    list_area,
                    false,
                );
                render_agent_detail(&app, &TerminalRuntimeRegistry::new(), frame, area);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let space_colors: Vec<Color> = app
            .view
            .workspace_card_areas
            .iter()
            .map(|card| buffer[(card.rect.x, card.rect.y)].style().fg.unwrap())
            .collect();
        assert_eq!(space_colors[0], pin_marker_color(0));
        assert_eq!(space_colors[1], pin_marker_color(1));
        assert_eq!(space_colors[2], pin_marker_color(2));
        let body = agent_panel_body_rect(area, false);
        assert_eq!(buffer[(body.x, body.y)].symbol(), PIN_MARKER);
        assert_eq!(
            buffer[(body.x, body.y)].style().fg,
            Some(pin_marker_color(0))
        );
    }

    #[test]
    fn pinned_agent_row_leads_with_the_marker_in_its_one_cell_lead() {
        let mut app = three_agent_app(AgentPanelSort::Spaces);
        let pane = agent_pane(&app, 1);
        app.pin_agent(pane);

        let area = Rect::new(0, 0, 30, 20);
        let body = agent_panel_body_rect(area, false);
        let mut terminal = Terminal::new(TestBackend::new(30, 20)).unwrap();
        terminal
            .draw(|frame| render_agent_detail(&app, &TerminalRuntimeRegistry::new(), frame, area))
            .unwrap();
        let buffer = terminal.backend().buffer();
        // The pinned agent is the first entry, so its name row is the first.
        assert_eq!(agent_panel_order(&app)[0], 1);
        let lead = (0..body.width)
            .map(|dx| buffer[(body.x + dx, body.y)].symbol().to_string())
            .collect::<String>();
        assert!(lead.contains(PIN_MARKER), "pinned name row: {lead:?}");
        let marks = (body.y..body.y + body.height)
            .filter(|y| (0..body.width).any(|dx| buffer[(body.x + dx, *y)].symbol() == PIN_MARKER))
            .count();
        assert_eq!(marks, 1, "only the pinned agent carries the marker");
    }

    #[test]
    fn pinned_agents_head_the_agent_motion_target_and_match_the_entries() {
        let mut app = three_agent_app(AgentPanelSort::Priority);
        let (a, b) = (agent_pane(&app, 0), agent_pane(&app, 1));
        app.pin_agent(b);
        app.pin_agent(a);
        let keys = agent_panel_target_keys(&app);
        assert_eq!(&keys[..2], &[b, a]);
        let entry_ids: Vec<_> = agent_panel_entries(&app)
            .into_iter()
            .map(|entry| entry.pane_id)
            .collect();
        assert_eq!(keys, entry_ids);
    }

    #[test]
    fn workspace_entries_hold_order_until_motion_ticks() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.ensure_test_terminals();
        app.workspace_sort = WorkspaceSort::Priority;
        app.sort_motion_bubble = true;

        let set_state = |app: &mut crate::app::state::AppState,
                         ws_idx: usize,
                         state: AgentState,
                         seq: Option<u64>| {
            let pane_id = app.workspaces[ws_idx].tabs[0].root_pane;
            let terminal_id = app.workspaces[ws_idx].tabs[0].panes[&pane_id]
                .attached_terminal_id
                .clone();
            let terminal = app.terminals.get_mut(&terminal_id).unwrap();
            terminal.detected_agent = Some(Agent::Pi);
            terminal.state = state;
            terminal.last_agent_state_change_seq = seq;
        };
        set_state(&mut app, 0, AgentState::Working, Some(1));
        set_state(&mut app, 1, AgentState::Idle, Some(2));

        let ws_order = |app: &crate::app::state::AppState| -> Vec<usize> {
            workspace_list_entries(app)
                .iter()
                .map(|entry| match entry {
                    WorkspaceListEntry::Workspace { ws_idx, .. } => *ws_idx,
                })
                .collect()
        };

        // Settle the motion state on the initial order: "one" (working) first.
        let t0 = std::time::Instant::now();
        let timing = app.sort_motion_timing;
        let target = workspace_unit_target_keys(&app);
        app.workspace_list_motion.tick(t0, &target, timing);
        assert_eq!(ws_order(&app), vec![0, 1]);

        // "two" starts working with a newer state change: target flips, but
        // without a motion tick the displayed order must not move.
        set_state(&mut app, 0, AgentState::Idle, Some(3));
        set_state(&mut app, 1, AgentState::Working, Some(4));
        assert_eq!(ws_order(&app), vec![0, 1]);

        // Before the settle delay a tick still holds the order.
        let target = workspace_unit_target_keys(&app);
        app.workspace_list_motion.tick(t0, &target, timing);
        assert_eq!(ws_order(&app), vec![0, 1]);

        // After the settle delay one step resolves the two-row swap.
        app.workspace_list_motion
            .tick(t0 + timing.settle, &target, timing);
        assert_eq!(ws_order(&app), vec![1, 0]);
    }

    #[test]
    fn default_agent_rows_remove_redundant_state_text() {
        let mut app = crate::app::state::AppState::test_new();
        let workspace = Workspace::test_new("one");
        let pane_id = workspace.tabs[0].root_pane;
        app.workspaces = vec![workspace];
        app.ensure_test_terminals();
        app.active = Some(0);
        let terminal_id = app.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let terminal_state = app.terminals.get_mut(&terminal_id).unwrap();
        terminal_state.detected_agent = Some(Agent::Pi);
        terminal_state.state = AgentState::Working;

        let area = Rect::new(0, 0, 26, 20);
        let mut terminal = Terminal::new(TestBackend::new(26, 20)).unwrap();
        terminal
            .draw(|frame| render_sidebar(&app, &TerminalRuntimeRegistry::new(), frame, area))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let (_, agent_area) = expanded_sidebar_sections(area, app.sidebar_section_split);
        let body = agent_panel_body_rect(agent_area, false);

        let first = row_text(buffer, body.y, 25);
        let second = row_text(buffer, body.y + 1, 25);
        assert!(first.contains("one"));
        assert_eq!(second, "   pi");
        assert!(!first.contains("working"));
        assert!(!second.contains("working"));

        let workspace_x = find_symbol_x(buffer, body.y, body.width, "o");
        let workspace_style = buffer[(workspace_x, body.y)].style();
        assert_eq!(workspace_style.fg, Some(app.palette.text));
        assert!(workspace_style.add_modifier.contains(Modifier::BOLD));
        assert!(!workspace_style.add_modifier.contains(Modifier::DIM));
        assert_eq!(workspace_style.bg, Some(app.palette.active_row_bg));

        let agent_x = find_symbol_x(buffer, body.y + 1, body.width, "p");
        let agent_style = buffer[(agent_x, body.y + 1)].style();
        assert_eq!(agent_style.fg, Some(app.palette.overlay0));
        assert!(agent_style.add_modifier.contains(Modifier::DIM));
        assert!(!agent_style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(agent_style.bg, Some(app.palette.active_row_bg));
    }

    #[test]
    fn occurrence_false_removes_default_workspace_bold_and_agent_dim() {
        let config: crate::config::Config = toml::from_str(
            r##"
[ui.sidebar.agents]
rows = [[{ token = "workspace", bold = false }, { token = "agent", dim = false }]]
"##,
        )
        .unwrap();
        let mut app = crate::app::state::AppState::test_new();
        app.sidebar_agents = config.ui.sidebar.agents;
        let workspace = Workspace::test_new("one");
        let pane_id = workspace.tabs[0].root_pane;
        app.workspaces = vec![workspace];
        app.ensure_test_terminals();
        app.active = Some(0);
        let terminal_id = app.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        app.terminals.get_mut(&terminal_id).unwrap().detected_agent = Some(Agent::Pi);

        let area = Rect::new(0, 0, 26, 20);
        let mut terminal = Terminal::new(TestBackend::new(26, 20)).unwrap();
        terminal
            .draw(|frame| render_sidebar(&app, &TerminalRuntimeRegistry::new(), frame, area))
            .unwrap();
        let (_, agent_area) = expanded_sidebar_sections(area, app.sidebar_section_split);
        let body = agent_panel_body_rect(agent_area, false);
        let buffer = terminal.backend().buffer();
        let workspace = buffer[(find_symbol_x(buffer, body.y, body.width, "o"), body.y)].style();
        let agent = buffer[(find_symbol_x(buffer, body.y, body.width, "p"), body.y)].style();

        assert_eq!(workspace.fg, Some(app.palette.text));
        assert!(!workspace.add_modifier.contains(Modifier::BOLD));
        assert_eq!(agent.fg, Some(app.palette.overlay0));
        assert!(!agent.add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn default_space_workspace_style_tracks_active_state() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.active = Some(0);
        app.mode = Mode::Terminal;
        let area = Rect::new(0, 0, 26, 20);
        app.view.workspace_card_areas = compute_workspace_card_areas(&app, area);
        let first_row = app.view.workspace_card_areas[0].rect.y;
        let second_row = app.view.workspace_card_areas[1].rect.y;
        let mut terminal = Terminal::new(TestBackend::new(26, 20)).unwrap();
        terminal
            .draw(|frame| render_sidebar(&app, &TerminalRuntimeRegistry::new(), frame, area))
            .unwrap();
        let buffer = terminal.backend().buffer();

        let active = buffer[(find_symbol_x(buffer, first_row, 25, "o"), first_row)].style();
        assert_eq!(active.fg, Some(app.palette.text));
        assert!(active.add_modifier.contains(Modifier::BOLD));
        assert!(!active.add_modifier.contains(Modifier::DIM));
        assert_eq!(active.bg, Some(app.palette.active_row_bg));

        let inactive = buffer[(find_symbol_x(buffer, second_row, 25, "t"), second_row)].style();
        assert_eq!(inactive.fg, Some(app.palette.subtext0));
        assert!(!inactive
            .add_modifier
            .intersects(Modifier::BOLD | Modifier::DIM));
        assert_eq!(inactive.bg, Some(ratatui::style::Color::Reset));
    }

    #[test]
    fn navigate_selection_keeps_its_existing_background_beside_active_workspace() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.active = Some(0);
        app.selected = 1;
        app.mode = Mode::Navigate;
        let area = Rect::new(0, 0, 26, 20);
        app.view.workspace_card_areas = compute_workspace_card_areas(&app, area);
        let active_row = app.view.workspace_card_areas[0].rect.y;
        let selected_row = app.view.workspace_card_areas[1].rect.y;
        let mut terminal = Terminal::new(TestBackend::new(26, 20)).unwrap();
        terminal
            .draw(|frame| render_sidebar(&app, &TerminalRuntimeRegistry::new(), frame, area))
            .unwrap();
        let buffer = terminal.backend().buffer();

        assert_eq!(
            buffer[(0, active_row)].bg,
            app.palette.active_row_bg,
            "active workspace should keep its dedicated background"
        );
        assert_eq!(
            buffer[(0, selected_row)].bg,
            app.palette.selection_bg,
            "navigate selection should use its dedicated cursor background"
        );
    }

    #[test]
    fn selected_active_workspace_resolves_expanded_background() {
        let mut app = crate::app::state::AppState::test_new();
        app.palette = crate::app::state::Palette::terminal();
        app.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.active = Some(0);
        app.selected = 0;
        app.mode = Mode::Navigate;
        let area = Rect::new(0, 0, 26, 20);
        app.view.workspace_card_areas = compute_workspace_card_areas(&app, area);
        let active_row = app.view.workspace_card_areas[0].rect.y;
        let inactive_row = app.view.workspace_card_areas[1].rect.y;
        let mut terminal = Terminal::new(TestBackend::new(26, 20)).unwrap();
        terminal
            .draw(|frame| render_sidebar(&app, &TerminalRuntimeRegistry::new(), frame, area))
            .unwrap();

        assert_eq!(
            terminal.backend().buffer()[(0, active_row)].bg,
            app.palette.active_row_bg
        );

        app.selected = 1;
        terminal
            .draw(|frame| render_sidebar(&app, &TerminalRuntimeRegistry::new(), frame, area))
            .unwrap();
        assert_eq!(
            terminal.backend().buffer()[(0, active_row)].bg,
            app.palette.active_row_bg
        );
        assert_eq!(
            terminal.backend().buffer()[(0, inactive_row)].bg,
            app.palette.selection_bg
        );

        app.palette = crate::app::state::Palette::catppuccin();
        app.selected = 0;
        terminal
            .draw(|frame| render_sidebar(&app, &TerminalRuntimeRegistry::new(), frame, area))
            .unwrap();
        assert_eq!(
            terminal.backend().buffer()[(0, active_row)].bg,
            app.palette.selection_bg
        );
    }

    #[test]
    fn selected_active_workspace_resolves_collapsed_background() {
        let mut app = crate::app::state::AppState::test_new();
        app.palette = crate::app::state::Palette::terminal();
        app.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.active = Some(0);
        app.selected = 0;
        app.mode = Mode::Navigate;
        let area = Rect::new(0, 0, 5, 8);
        let mut terminal = Terminal::new(TestBackend::new(5, 8)).unwrap();
        terminal
            .draw(|frame| render_sidebar_collapsed(&app, frame, area))
            .unwrap();

        let (workspace_area, _, _) = collapsed_sidebar_sections(area);
        assert_eq!(
            terminal.backend().buffer()[(workspace_area.x, workspace_area.y)].bg,
            app.palette.active_row_bg
        );

        app.selected = 1;
        terminal
            .draw(|frame| render_sidebar_collapsed(&app, frame, area))
            .unwrap();
        assert_eq!(
            terminal.backend().buffer()[(workspace_area.x, workspace_area.y)].bg,
            app.palette.active_row_bg
        );
        assert_eq!(
            terminal.backend().buffer()[(workspace_area.x, workspace_area.y + 1)].bg,
            app.palette.selection_bg
        );

        app.palette = crate::app::state::Palette::catppuccin();
        app.selected = 0;
        terminal
            .draw(|frame| render_sidebar_collapsed(&app, frame, area))
            .unwrap();
        assert_eq!(
            terminal.backend().buffer()[(workspace_area.x, workspace_area.y)].bg,
            app.palette.selection_bg
        );
    }

    #[test]
    fn space_occurrence_style_applies_without_styling_separator() {
        let config: crate::config::Config = toml::from_str(
            r##"
[ui.sidebar.spaces]
rows = [[{ token = "$hype", fg = "#abcdef", bold = true, dim = false }, "workspace"]]
"##,
        )
        .unwrap();
        let mut app = crate::app::state::AppState::test_new();
        app.sidebar_spaces = config.ui.sidebar.spaces;
        app.workspaces = vec![Workspace::test_new("one")];
        app.active = Some(0);
        app.mode = Mode::Terminal;
        app.workspaces[0].metadata_tokens.patch(
            std::collections::HashMap::from([("hype".into(), Some("HI".into()))]),
            None,
            std::time::Instant::now(),
        );

        let area = Rect::new(0, 0, 26, 20);
        app.view.workspace_card_areas = compute_workspace_card_areas(&app, area);
        let row = app.view.workspace_card_areas[0].rect.y;
        let mut terminal = Terminal::new(TestBackend::new(26, 20)).unwrap();
        terminal
            .draw(|frame| render_sidebar(&app, &TerminalRuntimeRegistry::new(), frame, area))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let h = buffer[(find_symbol_x(buffer, row, 25, "H"), row)].style();
        let i = buffer[(find_symbol_x(buffer, row, 25, "I"), row)].style();
        let separator = buffer[(find_symbol_x(buffer, row, 25, "·"), row)].style();

        for style in [h, i] {
            assert_eq!(style.fg, Some(ratatui::style::Color::Rgb(0xab, 0xcd, 0xef)));
            assert!(style.add_modifier.contains(Modifier::BOLD));
            assert!(!style.add_modifier.contains(Modifier::DIM));
            assert_eq!(style.bg, Some(app.palette.active_row_bg));
        }
        assert_eq!(separator.fg, Some(app.palette.overlay0));
        assert!(separator.add_modifier.contains(Modifier::DIM));
        assert!(!separator.add_modifier.contains(Modifier::BOLD));
        assert_eq!(separator.bg, Some(app.palette.active_row_bg));
    }

    // --- keep and truncate (fork issue 164) ---

    /// `[{branch}, git_status, {$asks, keep, truncate = start}]`, the target
    /// row, with `main`, `↓5` and `asks L:20 A:0`.
    fn asks_row(
        keep: Option<bool>,
        truncate: Option<crate::config::SidebarTokenTruncate>,
    ) -> Vec<ResolvedToken> {
        let style = crate::config::SidebarTokenStyle {
            keep,
            truncate,
            ..Default::default()
        };
        vec![
            ResolvedToken {
                kind: ResolvedTokenKind::Branch("main".into()),
                style: Default::default(),
            },
            ResolvedToken {
                kind: ResolvedTokenKind::GitStatus {
                    ahead: 0,
                    behind: 5,
                },
                style: Default::default(),
            },
            ResolvedToken {
                kind: ResolvedTokenKind::Custom("asks L:20 A:0".into()),
                style,
            },
        ]
    }

    fn fit_text(tokens: &[ResolvedToken], width: usize) -> String {
        let spans = resolved_token_spans(
            tokens,
            ("", Style::default()),
            Style::default(),
            Style::default(),
            Style::default(),
            Style::default(),
            &crate::app::state::Palette::catppuccin(),
            width,
        );
        let text: String = spans.iter().map(|span| span.content.to_string()).collect();
        // The fixed `↓5` always draws, so only a row that can hold it is checked.
        assert!(
            width < 2 || display_width(&text) <= width,
            "{text:?} wider than {width}"
        );
        text
    }

    #[test]
    fn a_kept_token_with_truncate_start_is_eaten_last_and_from_the_left() {
        use crate::config::SidebarTokenTruncate::Start;
        let row = asks_row(Some(true), Some(Start));
        for (width, expected) in [
            // Everything fits.
            (23, "main ↓5 · asks L:20 A:0"),
            // The branch shrinks first; asks stays whole.
            (22, "ma… ↓5 · asks L:20 A:0"),
            (20, "… ↓5 · asks L:20 A:0"),
            // The branch is dropped before asks loses a cell.
            (19, "↓5 · asks L:20 A:0"),
            // Then asks is cut from the start, the end stays.
            (14, "↓5 · …L:20 A:0"),
            (9, "↓5 · …A:0"),
            (8, "↓5 · …:0"),
            (6, "↓5 · …"),
            // No room for asks at all: the branch, which is not kept, is what
            // is left beside the fixed `↓5`.
            (5, "m… ↓5"),
        ] {
            assert_eq!(fit_text(&row, width), expected, "width {width}");
        }
    }

    #[test]
    fn keep_alone_keeps_the_token_whole_and_truncate_alone_changes_only_the_cut() {
        // keep without truncate: asks is whole, cut from the end when it must.
        let kept = asks_row(Some(true), None);
        assert_eq!(fit_text(&kept, 19), "↓5 · asks L:20 A:0");
        assert_eq!(fit_text(&kept, 9), "↓5 · ask…");
        // truncate = start without keep: the branch and asks share the width
        // as before, only asks is cut from the other end.
        let start = asks_row(None, Some(crate::config::SidebarTokenTruncate::Start));
        let plain = asks_row(None, None);
        for width in [23, 22, 19, 14, 10, 6] {
            assert_eq!(
                display_width(&fit_text(&start, width)),
                display_width(&fit_text(&plain, width)),
                "same widths at {width}"
            );
        }
        let narrow = fit_text(&start, 14);
        assert!(narrow.ends_with("A:0"), "{narrow:?}");
    }

    #[test]
    fn a_row_with_no_keep_or_truncate_fits_exactly_as_before() {
        use crate::config::SidebarTokenTruncate::End;
        let plain = asks_row(None, None);
        let explicit = asks_row(Some(false), Some(End));
        for width in 0..=26 {
            assert_eq!(
                fit_text(&plain, width),
                fit_text(&explicit, width),
                "width {width}"
            );
        }
        // The round-robin share and the drop order of the old fit.
        assert_eq!(fit_text(&plain, 23), "main ↓5 · asks L:20 A:0");
        assert_eq!(fit_text(&plain, 16), "main ↓5 · asks …");
        assert_eq!(fit_text(&plain, 12), "ma… ↓5 · as…");
        assert_eq!(fit_text(&plain, 9), "m… ↓5 · …");
    }

    #[test]
    fn keep_and_truncate_render_in_the_sidebar_row() {
        let config: crate::config::Config = toml::from_str(
            r##"
[ui.sidebar.spaces]
rows = [["workspace"], [{ token = "$asks", keep = true, truncate = "start", italic = true }]]
"##,
        )
        .unwrap();
        let mut app = crate::app::state::AppState::test_new();
        app.sidebar_spaces = config.ui.sidebar.spaces;
        app.workspaces = vec![Workspace::test_new("one")];
        app.active = Some(0);
        app.mode = Mode::Terminal;
        app.workspaces[0].metadata_tokens.patch(
            std::collections::HashMap::from([("asks".into(), Some("asks L:20 A:0".into()))]),
            None,
            std::time::Instant::now(),
        );

        // 12 cells wide: the metadata cell is cut from the start.
        let area = Rect::new(0, 0, 12, 20);
        app.view.workspace_card_areas = compute_workspace_card_areas(&app, area);
        let row = app.view.workspace_card_areas[0].rect.y + 1;
        let mut terminal = Terminal::new(TestBackend::new(12, 20)).unwrap();
        terminal
            .draw(|frame| render_sidebar(&app, &TerminalRuntimeRegistry::new(), frame, area))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let line: String = (0..11)
            .map(|x| buffer[(x, row)].symbol().to_string())
            .collect();
        assert!(
            line.contains("…") && line.trim_end().ends_with("A:0"),
            "{line:?}"
        );
    }

    #[test]
    fn fog_base_is_the_sidebar_then_the_host_terminal_then_the_panel_background() {
        let mut app = crate::app::state::AppState::test_new();
        app.palette.panel_bg = Color::Rgb(24, 24, 37);
        app.palette.sidebar_bg = Color::Reset;
        // Unknown host background: the palette's panel background.
        assert_eq!(fog_base(&app), Color::Rgb(24, 24, 37));
        // Known host background: what the sidebar really shows.
        app.host_terminal_theme.background = Some(crate::terminal_theme::RgbColor {
            r: 10,
            g: 10,
            b: 10,
        });
        assert_eq!(fog_base(&app), Color::Rgb(10, 10, 10));
        // An RGB sidebar background wins over both.
        app.palette.sidebar_bg = Color::Rgb(1, 2, 3);
        assert_eq!(fog_base(&app), Color::Rgb(1, 2, 3));
    }

    #[test]
    fn space_occurrence_italic_is_on_the_token_and_not_the_separator() {
        let config: crate::config::Config = toml::from_str(
            r##"
[ui.sidebar.spaces]
rows = [["workspace", { token = "$asks", italic = true }]]
"##,
        )
        .unwrap();
        let mut app = crate::app::state::AppState::test_new();
        app.sidebar_spaces = config.ui.sidebar.spaces;
        app.workspaces = vec![Workspace::test_new("one")];
        app.active = Some(0);
        app.mode = Mode::Terminal;
        app.workspaces[0].metadata_tokens.patch(
            std::collections::HashMap::from([("asks".into(), Some("QQ".into()))]),
            None,
            std::time::Instant::now(),
        );

        let area = Rect::new(0, 0, 26, 20);
        app.view.workspace_card_areas = compute_workspace_card_areas(&app, area);
        let row = app.view.workspace_card_areas[0].rect.y;
        let mut terminal = Terminal::new(TestBackend::new(26, 20)).unwrap();
        terminal
            .draw(|frame| render_sidebar(&app, &TerminalRuntimeRegistry::new(), frame, area))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let q = buffer[(find_symbol_x(buffer, row, 25, "Q"), row)].style();
        let separator = buffer[(find_symbol_x(buffer, row, 25, "·"), row)].style();
        let name = buffer[(find_symbol_x(buffer, row, 25, "o"), row)].style();

        assert!(q.add_modifier.contains(Modifier::ITALIC));
        assert!(!separator.add_modifier.contains(Modifier::ITALIC));
        assert!(!name.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn explicit_italic_false_removes_italic() {
        let patch = crate::config::SidebarTokenStyle {
            italic: Some(false),
            ..Default::default()
        };
        let style = apply_token_style(Style::default().add_modifier(Modifier::ITALIC), patch);
        assert!(!style.add_modifier.contains(Modifier::ITALIC));
        let untouched = apply_token_style(
            Style::default().add_modifier(Modifier::ITALIC),
            crate::config::SidebarTokenStyle::default(),
        );
        assert!(untouched.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn occurrence_foreground_flattens_composite_git_status_colors() {
        let config: crate::config::Config = toml::from_str(
            r##"[ui.sidebar.spaces]
rows = [[{ token = "git_status", fg = "#123456" }]]
"##,
        )
        .unwrap();
        let spans = resolved_token_spans(
            &[ResolvedToken {
                kind: ResolvedTokenKind::GitStatus {
                    ahead: 2,
                    behind: 1,
                },
                style: config.ui.sidebar.spaces.rows[0][0].parts().1,
            }],
            ("", Style::default()),
            Style::default(),
            Style::default(),
            Style::default(),
            Style::default(),
            &crate::app::state::AppState::test_new().palette,
            20,
        );

        assert_eq!(
            spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            "↑2 ↓1"
        );
        assert!(spans
            .iter()
            .all(|span| { span.style.fg == Some(ratatui::style::Color::Rgb(0x12, 0x34, 0x56)) }));
    }

    #[test]
    fn default_agent_row_gap_packs_rendering_and_scroll_geometry() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.ensure_test_terminals();
        for (workspace, agent) in app.workspaces.iter().zip([Agent::Pi, Agent::Claude]) {
            let pane_id = workspace.tabs[0].root_pane;
            let terminal_id = workspace.tabs[0].panes[&pane_id]
                .attached_terminal_id
                .clone();
            app.terminals.get_mut(&terminal_id).unwrap().detected_agent = Some(agent);
        }
        app.sidebar_agents.rows = vec![vec![crate::config::AgentSidebarToken::Agent]];
        assert_eq!(app.sidebar_agents.row_gap, 0);

        let area = Rect::new(0, 0, 20, 5);
        let metrics = agent_panel_scroll_metrics(&app, area);
        let body = agent_panel_body_rect(area, false);
        let mut terminal = Terminal::new(TestBackend::new(20, 5)).unwrap();
        terminal
            .draw(|frame| render_agent_detail(&app, &TerminalRuntimeRegistry::new(), frame, area))
            .unwrap();
        let buffer = terminal.backend().buffer();

        assert_eq!(metrics.viewport_rows, 2);
        assert_eq!(metrics.max_offset_from_bottom, 0);
        assert_eq!(row_text(buffer, body.y, body.width), " pi");
        assert_eq!(row_text(buffer, body.y + 1, body.width), " claude");
    }

    #[test]
    fn every_working_agent_row_draws_the_shared_spinner_frame() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.ensure_test_terminals();
        app.status_indicators = crate::config::StatusIndicatorStyle::Symbols;
        app.active = Some(0);
        for (workspace, seq) in app.workspaces.iter().zip([10u64, 13]) {
            let pane_id = workspace.tabs[0].root_pane;
            let terminal_id = workspace.tabs[0].panes[&pane_id]
                .attached_terminal_id
                .clone();
            let terminal = app.terminals.get_mut(&terminal_id).unwrap();
            terminal.detected_agent = Some(Agent::Claude);
            terminal.state = AgentState::Working;
            // When each agent last changed state sets its phase.
            terminal.last_agent_state_change_seq = Some(seq);
        }

        let area = Rect::new(0, 0, 4, 16);
        let (ws_area, _, detail_area) = collapsed_sidebar_sections(area);
        let icons = |app: &crate::app::state::AppState| {
            let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))
                .expect("test terminal should initialize");
            terminal
                .draw(|frame| render_sidebar_collapsed(app, frame, area))
                .expect("collapsed sidebar should render");
            let buffer = terminal.backend().buffer();
            (
                buffer[(detail_area.x + 2, detail_area.y)]
                    .symbol()
                    .to_string(),
                buffer[(detail_area.x + 2, detail_area.y + 1)]
                    .symbol()
                    .to_string(),
                buffer[(ws_area.x + 2, ws_area.y)].symbol().to_string(),
            )
        };

        // Focused or not, both working rows move on the same tick, each
        // from its own phase (frame 3 + 10 → ⠸, 3 + 13 → ⠦); the space row
        // keeps the static mark.
        app.spinner_frame = 3;
        assert_eq!(icons(&app), ("⠸".into(), "⠦".into(), "◐".into()));
        app.spinner_frame = 4;
        assert_eq!(icons(&app), ("⠼".into(), "⠧".into(), "◐".into()));

        // Off pins the static glyph.
        app.status_spinner = crate::config::StatusSpinnerConfig::Off;
        assert_eq!(icons(&app), ("◐".into(), "◐".into(), "◐".into()));
    }

    /// An agent parked at its prompt while a shell it launched keeps running
    /// pulses instead of spinning, so it does not read as an agent mid-turn.
    #[test]
    fn background_work_rows_pulse_instead_of_spinning() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![Workspace::test_new("cchv"), Workspace::test_new("infra")];
        app.ensure_test_terminals();
        app.status_indicators = crate::config::StatusIndicatorStyle::Symbols;
        app.active = Some(0);
        for (index, workspace) in app.workspaces.iter().enumerate() {
            let pane_id = workspace.tabs[0].root_pane;
            let terminal_id = workspace.tabs[0].panes[&pane_id]
                .attached_terminal_id
                .clone();
            let terminal = app.terminals.get_mut(&terminal_id).unwrap();
            terminal.detected_agent = Some(Agent::Claude);
            terminal.state = AgentState::Working;
            // Only the first pane is working because of a background shell.
            terminal.set_background_work_observed(index == 0);
        }

        let area = Rect::new(0, 0, 4, 16);
        let (_, _, detail_area) = collapsed_sidebar_sections(area);
        let icons = |app: &crate::app::state::AppState| {
            let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))
                .expect("test terminal should initialize");
            terminal
                .draw(|frame| render_sidebar_collapsed(app, frame, area))
                .expect("collapsed sidebar should render");
            let buffer = terminal.backend().buffer();
            (
                buffer[(detail_area.x + 2, detail_area.y)]
                    .symbol()
                    .to_string(),
                buffer[(detail_area.x + 2, detail_area.y + 1)]
                    .symbol()
                    .to_string(),
            )
        };

        // Eight shared ticks per pulse frame, against one snake step per tick.
        app.spinner_frame = 0;
        assert_eq!(icons(&app), ("■".into(), "⠋".into()));
        app.spinner_frame = 7;
        assert_eq!(icons(&app), ("■".into(), "⠧".into()));
        app.spinner_frame = 8;
        assert_eq!(icons(&app), ("◆".into(), "⠇".into()));
        app.spinner_frame = 16;
        assert_eq!(icons(&app), ("■".into(), "⠦".into()));

        // With the spinner off the pulse still marks the row apart.
        app.status_spinner = crate::config::StatusSpinnerConfig::Off;
        assert_eq!(icons(&app), ("■".into(), "◐".into()));

        // Overridable like every other state glyph.
        app.status_spinner = crate::config::StatusSpinnerConfig::On;
        app.state_symbol_overrides.background = Some("⊙".into());
        app.spinner_frame = 0;
        assert_eq!(icons(&app).0, "⊙");
    }

    #[test]
    fn agent_rows_show_the_open_todo_count_under_the_jump_label() {
        use crate::terminal::todo::TodoPriority;
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.ensure_test_terminals();
        app.sidebar_style = crate::config::SidebarStyleConfig::Editorial;
        app.sidebar_agents.rows = vec![
            vec![crate::config::AgentSidebarToken::Workspace],
            vec![crate::config::AgentSidebarToken::Agent],
        ];
        for (workspace, todos) in app.workspaces.iter().zip([
            vec![
                (false, TodoPriority::Normal),
                (false, TodoPriority::High),
                (true, TodoPriority::High),
            ],
            vec![(true, TodoPriority::Low)],
        ]) {
            let pane_id = workspace.tabs[0].root_pane;
            let terminal_id = workspace.tabs[0].panes[&pane_id]
                .attached_terminal_id
                .clone();
            let terminal = app.terminals.get_mut(&terminal_id).unwrap();
            terminal.detected_agent = Some(Agent::Claude);
            for (index, (done, priority)) in todos.iter().enumerate() {
                let todo = terminal
                    .add_todo(&format!("todo {index}"), *priority, None, 100)
                    .unwrap();
                if *done {
                    terminal
                        .update_todo(
                            todo.id,
                            crate::terminal::todo::TodoUpdate {
                                done: Some(true),
                                ..Default::default()
                            },
                            200,
                        )
                        .unwrap();
                }
            }
        }

        app.show_agent_numbers = true;
        let area = Rect::new(0, 0, 24, 10);
        let body = agent_panel_body_rect(area, false);
        let mut terminal = Terminal::new(TestBackend::new(24, 10)).unwrap();
        terminal
            .draw(|frame| render_agent_detail(&app, &TerminalRuntimeRegistry::new(), frame, area))
            .unwrap();
        let buffer = terminal.backend().buffer();

        // Two open todos (the done one does not count) on the name row,
        // coloured by the highest open priority; the jump label sits under it.
        let first_name = row_text(buffer, body.y, body.width);
        let first_second = row_text(buffer, body.y + 1, body.width);
        assert!(
            first_name.ends_with("τ 2"),
            "count on the name row: {first_name:?}"
        );
        assert!(
            first_second.ends_with('1'),
            "jump label on the second row: {first_second:?}"
        );
        let tau_x = body.x + body.width - 3;
        assert_eq!(buffer[(tau_x, body.y)].symbol(), "τ");
        assert_eq!(buffer[(tau_x, body.y)].style().fg, Some(app.palette.red));

        // Everything done: no count at all.
        let second_second = row_text(buffer, body.y + 3, body.width);
        assert!(
            !second_second.contains('τ'),
            "no count once all done: {second_second:?}"
        );

        // A one-row entry keeps its jump label on the name row and has
        // nowhere to put the count.
        app.sidebar_agents.rows = vec![vec![crate::config::AgentSidebarToken::Workspace]];
        let mut terminal = Terminal::new(TestBackend::new(24, 10)).unwrap();
        terminal
            .draw(|frame| render_agent_detail(&app, &TerminalRuntimeRegistry::new(), frame, area))
            .unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol().to_string())
            .collect::<String>();
        assert!(!rendered.contains('τ'), "{rendered}");
        let first_name = row_text(terminal.backend().buffer(), body.y, body.width);
        assert!(
            first_name.ends_with('1'),
            "jump label stays on the one row: {first_name:?}"
        );
    }

    #[test]
    fn narrow_agent_rows_preserve_later_tab_tokens() {
        let mut app = crate::app::state::AppState::test_new();
        let mut workspace = Workspace::test_new("very-long-workspace-name");
        let tab_idx = workspace.test_add_tab(Some("logs"));
        let pane_id = workspace.tabs[tab_idx].root_pane;
        app.workspaces = vec![workspace];
        app.ensure_test_terminals();
        let terminal_id = app.workspaces[0].tabs[tab_idx].panes[&pane_id]
            .attached_terminal_id
            .clone();
        app.terminals.get_mut(&terminal_id).unwrap().detected_agent = Some(Agent::Pi);

        let area = Rect::new(0, 0, 18, 20);
        let mut terminal = Terminal::new(TestBackend::new(18, 20)).unwrap();
        terminal
            .draw(|frame| render_sidebar(&app, &TerminalRuntimeRegistry::new(), frame, area))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let (_, agent_area) = expanded_sidebar_sections(area, app.sidebar_section_split);
        let body = agent_panel_body_rect(agent_area, false);
        let first = row_text(buffer, body.y, 17);

        assert!(first.contains("logs"), "rendered row: {first:?}");
        assert!(first.contains('·'), "rendered row: {first:?}");
    }

    #[test]
    fn stripped_terminal_title_renders_with_unicode_width_truncation() {
        let mut app = crate::app::state::AppState::test_new();
        let workspace = Workspace::test_new("one");
        let pane_id = workspace.tabs[0].root_pane;
        app.workspaces = vec![workspace];
        app.ensure_test_terminals();
        let terminal_id = app.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let terminal = app.terminals.get_mut(&terminal_id).unwrap();
        terminal.detected_agent = Some(Agent::Claude);
        terminal.set_terminal_title(Some("⠋ 修复🙂标题很长".into()));
        app.sidebar_agents.rows = vec![vec![
            crate::config::AgentSidebarToken::TerminalTitleStripped,
        ]];

        let area = Rect::new(0, 0, 10, 12);
        let mut renderer = Terminal::new(TestBackend::new(10, 12)).unwrap();
        renderer
            .draw(|frame| render_sidebar(&app, &TerminalRuntimeRegistry::new(), frame, area))
            .unwrap();
        let (_, agent_area) = expanded_sidebar_sections(area, app.sidebar_section_split);
        let body = agent_panel_body_rect(agent_area, false);
        let rendered = row_text(renderer.backend().buffer(), body.y, 9);

        assert!(!rendered.contains('⠋'));
        assert!(rendered.contains('修') && rendered.contains('复'));

        let spans = resolved_token_spans(
            &[ResolvedToken::unstyled(ResolvedTokenKind::TerminalTitle(
                "修复🙂标题很长".into(),
            ))],
            ("", Style::default()),
            Style::default(),
            Style::default(),
            Style::default(),
            Style::default(),
            &app.palette,
            8,
        );
        let text = spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(display_width(&text) <= 8, "resolved title: {text:?}");
    }

    #[test]
    fn variable_agent_heights_pack_the_bottom_and_reveal_targets() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![
            Workspace::test_new("one"),
            Workspace::test_new("two"),
            Workspace::test_new("three"),
        ];
        app.ensure_test_terminals();
        for workspace in &app.workspaces {
            let pane_id = workspace.tabs[0].root_pane;
            let terminal_id = workspace.tabs[0].panes[&pane_id]
                .attached_terminal_id
                .clone();
            app.terminals.get_mut(&terminal_id).unwrap().detected_agent = Some(Agent::Pi);
        }
        let first_pane = app.workspaces[0].tabs[0].root_pane;
        let first_terminal = app.workspaces[0].tabs[0].panes[&first_pane]
            .attached_terminal_id
            .clone();
        app.terminals
            .get_mut(&first_terminal)
            .unwrap()
            .metadata_tokens
            .patch(
                std::collections::HashMap::from([
                    ("a".into(), Some("a".into())),
                    ("b".into(), Some("b".into())),
                ]),
                None,
                std::time::Instant::now(),
            );
        app.sidebar_agents.rows = vec![
            vec![crate::config::AgentSidebarToken::Agent],
            vec![crate::config::AgentSidebarToken::Custom("a".into())],
            vec![crate::config::AgentSidebarToken::Custom("b".into())],
        ];
        let area = Rect::new(0, 0, 20, 6);

        let metrics = agent_panel_scroll_metrics(&app, area);
        assert_eq!(metrics.max_offset_from_bottom, 1);
        assert_eq!(agent_panel_scroll_for_target(&app, area, 0, 2), 1);
    }

    #[test]
    fn oversized_space_layout_is_clipped_to_the_section_body() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.sidebar_spaces.rows = vec![vec![crate::config::SpaceSidebarToken::Workspace]; 6];
        let area = Rect::new(0, 0, 20, 10);
        let workspace_area = workspace_list_rect(area, app.sidebar_section_split);
        let body = workspace_list_body_rect(workspace_area, false);

        let metrics = workspace_list_scroll_metrics(&app, workspace_area);
        let (cards, _) = compute_workspace_list_areas(&app, area);

        assert_eq!(metrics.viewport_rows, 1);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].ws_idx, 0);
        assert_eq!(cards[0].rect.height, body.height);
    }

    #[test]
    fn oversized_agent_override_is_clipped_to_the_panel_body() {
        let mut app = crate::app::state::AppState::test_new();
        let workspace = Workspace::test_new("one");
        let pane_id = workspace.tabs[0].root_pane;
        app.workspaces = vec![workspace];
        app.ensure_test_terminals();
        let terminal_id = app.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        app.terminals.get_mut(&terminal_id).unwrap().detected_agent = Some(Agent::Claude);
        app.sidebar_agents.rows_by_agent.insert(
            "claude".into(),
            vec![vec![crate::config::AgentSidebarToken::Agent]; 6],
        );
        let panel = Rect::new(0, 0, 20, 5);

        let metrics = agent_panel_scroll_metrics(&app, panel);

        assert_eq!(metrics.viewport_rows, 1);
        assert_eq!(metrics.max_offset_from_bottom, 0);
        let entry = agent_panel_entries(&app).pop().unwrap();
        assert_eq!(
            agent_entry_height_in_body(&app, &entry, agent_panel_body_rect(panel, false).height),
            agent_panel_body_rect(panel, false).height
        );
    }

    #[test]
    fn render_sidebar_toggle_draws_expanded_collapse_icon() {
        let app = crate::app::state::AppState::test_new();
        let area = Rect::new(0, 0, 26, 20);
        let mut terminal =
            Terminal::new(TestBackend::new(26, 20)).expect("test terminal should initialize");

        terminal
            .draw(|frame| render_sidebar_toggle(&app, frame, area, false, &app.palette))
            .expect("sidebar toggle should render");

        let toggle = expanded_sidebar_toggle_rect(area);
        assert_eq!(
            terminal.backend().buffer()[(toggle.x, toggle.y)].symbol(),
            "«"
        );
    }

    #[test]
    fn expanded_sidebar_toggle_sits_inside_sidebar_content() {
        let area = Rect::new(0, 0, 26, 20);
        let toggle = expanded_sidebar_toggle_rect(area);

        assert_eq!(toggle.x, area.x + area.width - 2);
        assert_eq!(toggle.y, area.y + area.height - 1);
    }

    #[test]
    fn agent_panel_tab_label_visibility_tracks_tab_identity() {
        let mut app = crate::app::state::AppState::test_new();
        let single_auto = Workspace::test_new("auto");
        let mut single_custom = Workspace::test_new("custom");
        single_custom.tabs[0].set_custom_name("focus".into());
        let mut multi = Workspace::test_new("multi");
        multi.test_add_tab(Some("logs"));

        app.workspaces = vec![single_auto, single_custom, multi];
        app.ensure_test_terminals();
        for (ws_idx, tab_idx, agent) in [
            (0, 0, Agent::Pi),
            (1, 0, Agent::Claude),
            (2, 0, Agent::Codex),
            (2, 1, Agent::Pi),
        ] {
            let pane_id = app.workspaces[ws_idx].tabs[tab_idx].root_pane;
            let terminal_id = app.workspaces[ws_idx].tabs[tab_idx].panes[&pane_id]
                .attached_terminal_id
                .clone();
            app.terminals.get_mut(&terminal_id).unwrap().detected_agent = Some(agent);
        }

        let entries = agent_panel_entries(&app);
        let labels: Vec<_> = entries
            .iter()
            .map(|entry| {
                (
                    entry.primary_label.as_str(),
                    entry.primary_tab_label.as_deref(),
                )
            })
            .collect();

        assert_eq!(
            labels,
            [
                ("auto", None),
                ("custom", Some("focus")),
                ("multi", Some("1")),
                ("multi", Some("logs")),
            ]
        );
    }

    #[test]
    fn priority_agent_panel_sort_uses_attention_then_space_order() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![
            Workspace::test_new("one"),
            Workspace::test_new("two"),
            Workspace::test_new("three"),
            Workspace::test_new("four"),
        ];
        app.ensure_test_terminals();
        app.active = Some(0);
        app.selected = 0;
        app.agent_panel_sort = crate::app::state::AgentPanelSort::Priority;

        let set_state = |app: &mut crate::app::state::AppState, ws_idx: usize, state| {
            let pane = app.workspaces[ws_idx].tabs[0].root_pane;
            let terminal_id = app.workspaces[ws_idx].tabs[0].panes[&pane]
                .attached_terminal_id
                .clone();
            let terminal = app.terminals.get_mut(&terminal_id).unwrap();
            terminal.detected_agent = Some(Agent::Claude);
            terminal.state = state;
        };
        set_state(&mut app, 0, AgentState::Working);
        set_state(&mut app, 1, AgentState::Idle);
        set_state(&mut app, 2, AgentState::Working);
        set_state(&mut app, 3, AgentState::Blocked);

        let done_pane = app.workspaces[1].tabs[0].root_pane;
        app.workspaces[1].tabs[0]
            .panes
            .get_mut(&done_pane)
            .unwrap()
            .seen = false;

        let labels: Vec<String> = agent_panel_entries(&app)
            .into_iter()
            .map(|entry| entry.primary_label)
            .collect();

        assert_eq!(labels, ["four", "two", "one", "three"]);
    }

    #[test]
    fn collapsed_sidebar_numbers_grouped_agents_by_list_position() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.ensure_test_terminals();

        for ws_idx in 0..app.workspaces.len() {
            let pane = app.workspaces[ws_idx].tabs[0].root_pane;
            let terminal_id = app.workspaces[ws_idx].tabs[0].panes[&pane]
                .attached_terminal_id
                .clone();
            app.terminals.get_mut(&terminal_id).unwrap().detected_agent = Some(Agent::Claude);
        }

        let area = Rect::new(0, 0, 4, 12);
        let (_, _, detail_area) = collapsed_sidebar_sections(area);
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))
            .expect("test terminal should initialize");

        terminal
            .draw(|frame| render_sidebar_collapsed(&app, frame, area))
            .expect("collapsed sidebar should render");

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(detail_area.x, detail_area.y)].symbol(), "1");
        assert_eq!(buffer[(detail_area.x, detail_area.y + 1)].symbol(), "2");
    }

    /// Two agent panes in one workspace plus a second workspace, so the
    /// assertions can tell pane-level highlighting apart from workspace-level.
    fn collapsed_agent_app() -> (crate::app::state::AppState, PaneId, PaneId) {
        let mut app = crate::app::state::AppState::test_new();
        let mut first = Workspace::test_new("one");
        let second_pane = first.test_split(Direction::Horizontal);
        let first_pane = first.tabs[0].root_pane;
        app.workspaces = vec![first, Workspace::test_new("two")];
        app.ensure_test_terminals();

        let terminal_ids: Vec<_> = app
            .workspaces
            .iter()
            .flat_map(|ws| ws.tabs.iter())
            .flat_map(|tab| tab.panes.values())
            .map(|pane| pane.attached_terminal_id.clone())
            .collect();
        for terminal_id in terminal_ids {
            app.terminals.get_mut(&terminal_id).unwrap().detected_agent = Some(Agent::Claude);
        }

        (app, first_pane, second_pane)
    }

    fn collapsed_agent_row_styles(
        app: &crate::app::state::AppState,
        area: Rect,
        detail_area: Rect,
        rows: u16,
    ) -> Vec<Vec<ratatui::style::Style>> {
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))
            .expect("test terminal should initialize");
        terminal
            .draw(|frame| render_sidebar_collapsed(app, frame, area))
            .expect("collapsed sidebar should render");
        let buffer = terminal.backend().buffer();
        (0..rows)
            .map(|row| {
                (detail_area.x..detail_area.x + detail_area.width)
                    .map(|x| buffer[(x, detail_area.y + row)].style())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn collapsed_sidebar_highlights_only_the_focused_agent_pane() {
        let (mut app, first_pane, second_pane) = collapsed_agent_app();
        app.active = Some(0);
        app.workspaces[0].tabs[0].layout.focus_pane(second_pane);
        assert!(app.is_active_pane(0, 0, second_pane));
        assert!(!app.is_active_pane(0, 0, first_pane));

        let area = Rect::new(0, 0, 4, 14);
        let (_, _, detail_area) = collapsed_sidebar_sections(area);
        let rows = collapsed_agent_row_styles(&app, area, detail_area, 3);

        let highlighted: Vec<_> = rows
            .iter()
            .filter(|cells| {
                cells
                    .iter()
                    .all(|style| style.bg == Some(app.palette.active_row_bg))
            })
            .collect();
        assert_eq!(
            highlighted.len(),
            1,
            "only the focused agent pane should be highlighted, across the whole row"
        );
        assert_eq!(highlighted[0][0].fg, Some(app.palette.text));

        let muted = rows
            .iter()
            .filter(|cells| cells[0].fg == Some(app.palette.overlay0))
            .count();
        assert_eq!(
            muted, 2,
            "the sibling pane in the active workspace and the other workspace stay muted"
        );
    }

    #[test]
    fn collapsed_sidebar_does_not_highlight_agents_without_active_workspace() {
        let (mut app, _, _) = collapsed_agent_app();
        app.active = None;

        let area = Rect::new(0, 0, 4, 14);
        let (_, _, detail_area) = collapsed_sidebar_sections(area);
        let rows = collapsed_agent_row_styles(&app, area, detail_area, 3);

        for cells in rows {
            assert_eq!(cells[0].fg, Some(app.palette.overlay0));
            for style in cells {
                assert_ne!(style.bg, Some(app.palette.active_row_bg));
            }
        }
    }

    #[test]
    fn collapsed_sidebar_labels_tenth_entry_with_jump_letter() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = (1..=10)
            .map(|idx| Workspace::test_new(&format!("workspace-{idx}")))
            .collect();
        app.ensure_test_terminals();

        for ws_idx in 0..app.workspaces.len() {
            let pane = app.workspaces[ws_idx].tabs[0].root_pane;
            let terminal_id = app.workspaces[ws_idx].tabs[0].panes[&pane]
                .attached_terminal_id
                .clone();
            app.terminals.get_mut(&terminal_id).unwrap().detected_agent = Some(Agent::Claude);
        }

        let area = Rect::new(0, 0, 4, 25);
        let (ws_area, _, detail_area) = collapsed_sidebar_sections(area);
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))
            .expect("test terminal should initialize");

        terminal
            .draw(|frame| render_sidebar_collapsed(&app, frame, area))
            .expect("collapsed sidebar should render");

        // The 10th entry is jumped with `a`, not a two-digit number, in both
        // sections; the state icon column never shifts.
        let tenth_row = detail_area.y + 9;
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(detail_area.x, tenth_row)].symbol(), "a");
        assert_eq!(buffer[(detail_area.x + 1, tenth_row)].symbol(), " ");
        // Upstream's static glyph set draws unknown agents as "·" (the fork's
        // old icon map used "○"); D2 keeps upstream's glyphs as the base.
        assert_eq!(buffer[(detail_area.x + 2, tenth_row)].symbol(), "·");
        assert_eq!(buffer[(ws_area.x, ws_area.y + 9)].symbol(), "a");
    }

    #[test]
    fn collapsed_sidebar_highlights_active_agent_row() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.ensure_test_terminals();

        for ws_idx in 0..app.workspaces.len() {
            let pane = app.workspaces[ws_idx].tabs[0].root_pane;
            let terminal_id = app.workspaces[ws_idx].tabs[0].panes[&pane]
                .attached_terminal_id
                .clone();
            app.terminals.get_mut(&terminal_id).unwrap().detected_agent = Some(Agent::Claude);
        }
        app.active = Some(1);

        let active_idx = agent_panel_entries(&app)
            .iter()
            .position(|entry| entry.ws_idx == 1)
            .expect("active workspace agent entry");

        let area = Rect::new(0, 0, 4, 12);
        let (_, _, detail_area) = collapsed_sidebar_sections(area);
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))
            .expect("test terminal should initialize");

        terminal
            .draw(|frame| render_sidebar_collapsed(&app, frame, area))
            .expect("collapsed sidebar should render");

        let buffer = terminal.backend().buffer();
        let active_row = detail_area.y + active_idx as u16;
        let inactive_row = detail_area.y + (1 - active_idx) as u16;
        let active_style = buffer[(detail_area.x, active_row)].style();
        assert_eq!(active_style.bg, Some(app.palette.active_row_bg));
        assert!(active_style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(active_style.fg, Some(app.palette.text));
        assert_ne!(
            buffer[(detail_area.x, inactive_row)].style().bg,
            Some(app.palette.active_row_bg)
        );
    }

    #[test]
    fn collapsed_sidebar_honours_number_color_overrides() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.ensure_test_terminals();
        app.workspace_number_color = Some(ratatui::style::Color::Rgb(1, 2, 3));
        app.agent_number_color = Some(ratatui::style::Color::Rgb(4, 5, 6));

        for ws_idx in 0..app.workspaces.len() {
            let pane = app.workspaces[ws_idx].tabs[0].root_pane;
            let terminal_id = app.workspaces[ws_idx].tabs[0].panes[&pane]
                .attached_terminal_id
                .clone();
            app.terminals.get_mut(&terminal_id).unwrap().detected_agent = Some(Agent::Claude);
        }

        let area = Rect::new(0, 0, 4, 12);
        let (ws_area, _, detail_area) = collapsed_sidebar_sections(area);
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))
            .expect("test terminal should initialize");

        terminal
            .draw(|frame| render_sidebar_collapsed(&app, frame, area))
            .expect("collapsed sidebar should render");

        let buffer = terminal.backend().buffer();
        // Row 0 is the navigate-mode selection; row 1 shows the plain number color.
        assert_eq!(
            buffer[(ws_area.x, ws_area.y + 1)].style().fg,
            Some(ratatui::style::Color::Rgb(1, 2, 3))
        );
        assert_eq!(
            buffer[(detail_area.x, detail_area.y)].style().fg,
            Some(ratatui::style::Color::Rgb(4, 5, 6))
        );
    }

    #[test]
    fn collapsed_sidebar_draws_left_active_border_bar() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.ensure_test_terminals();
        app.sidebar_active_border = crate::config::SidebarActiveBorderConfig::Left;
        app.active = Some(0);

        for ws_idx in 0..app.workspaces.len() {
            let pane = app.workspaces[ws_idx].tabs[0].root_pane;
            let terminal_id = app.workspaces[ws_idx].tabs[0].panes[&pane]
                .attached_terminal_id
                .clone();
            app.terminals.get_mut(&terminal_id).unwrap().detected_agent = Some(Agent::Claude);
        }

        let active_idx = agent_panel_entries(&app)
            .iter()
            .position(|entry| entry.ws_idx == 0)
            .expect("active workspace agent entry");

        // One column wider: the leftmost column is reserved for the bar.
        let area = Rect::new(0, 0, 5, 12);
        let (ws_area, _, detail_area) = collapsed_sidebar_sections(area);
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))
            .expect("test terminal should initialize");

        terminal
            .draw(|frame| render_sidebar_collapsed(&app, frame, area))
            .expect("collapsed sidebar should render");

        let buffer = terminal.backend().buffer();
        let accent = app.pane_border_color(true);

        // Active space row: bar in the reserved column, content shifted right.
        assert_eq!(buffer[(ws_area.x, ws_area.y)].symbol(), "│");
        assert_eq!(buffer[(ws_area.x, ws_area.y)].style().fg, Some(accent));
        assert_eq!(buffer[(ws_area.x + 1, ws_area.y)].symbol(), "1");
        // Inactive space row: reserved column stays blank.
        assert_eq!(buffer[(ws_area.x, ws_area.y + 1)].symbol(), " ");

        // Active agent row gets the bar too.
        let agent_row = detail_area.y + active_idx as u16;
        assert_eq!(buffer[(detail_area.x, agent_row)].symbol(), "│");
        assert_eq!(buffer[(detail_area.x, agent_row)].style().fg, Some(accent));
    }

    #[test]
    fn collapsed_sidebar_numbers_priority_agents_by_list_position() {
        let first = Workspace::test_new("one");
        let first_pane = first.tabs[0].root_pane;
        let mut second = Workspace::test_new("two");
        let second_pane = second.tabs[0].root_pane;
        let urgent_pane = second.test_split(ratatui::layout::Direction::Horizontal);

        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![first, second];
        app.ensure_test_terminals();
        app.agent_panel_sort = crate::app::state::AgentPanelSort::Priority;
        app.status_indicators = crate::config::StatusIndicatorStyle::Symbols;

        let set_state = |app: &mut crate::app::state::AppState, ws_idx: usize, pane_id, state| {
            let terminal_id = app.workspaces[ws_idx].tabs[0].panes[&pane_id]
                .attached_terminal_id
                .clone();
            let terminal = app.terminals.get_mut(&terminal_id).unwrap();
            terminal.detected_agent = Some(Agent::Claude);
            terminal.state = state;
        };
        set_state(&mut app, 0, first_pane, AgentState::Idle);
        set_state(&mut app, 1, second_pane, AgentState::Working);
        set_state(&mut app, 1, urgent_pane, AgentState::Blocked);
        app.workspaces[0].tabs[0]
            .panes
            .get_mut(&first_pane)
            .unwrap()
            .seen = false;

        assert_eq!(app.workspaces[1].public_pane_number(urgent_pane), Some(2));
        assert_eq!(agent_panel_entries(&app)[0].pane_id, urgent_pane);

        let area = Rect::new(0, 0, 4, 16);
        let (_, _, detail_area) = collapsed_sidebar_sections(area);
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height))
            .expect("test terminal should initialize");

        terminal
            .draw(|frame| render_sidebar_collapsed(&app, frame, area))
            .expect("collapsed sidebar should render");

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(detail_area.x, detail_area.y)].symbol(), "1");
        assert_eq!(buffer[(detail_area.x, detail_area.y + 1)].symbol(), "2");
        assert_eq!(buffer[(detail_area.x, detail_area.y + 2)].symbol(), "3");
        assert_eq!(buffer[(detail_area.x + 2, detail_area.y)].symbol(), "×");
        assert_eq!(
            buffer[(detail_area.x + 2, detail_area.y)].style().fg,
            Some(app.palette.red)
        );
        assert_eq!(buffer[(detail_area.x + 2, detail_area.y + 1)].symbol(), "□");
        assert_eq!(
            buffer[(detail_area.x + 2, detail_area.y + 1)].style().fg,
            Some(app.palette.teal)
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn all_workspaces_agent_panel_entries_use_live_root_runtime_cwd_for_workspace_label() {
        let unique = format!(
            "herdr-agent-panel-runtime-cwd-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        let stale_cwd = root.join("issue-264-nix-support");
        let live_cwd = root.join("herdr");
        std::fs::create_dir_all(stale_cwd.join(".git")).unwrap();
        std::fs::create_dir_all(live_cwd.join(".git")).unwrap();

        let mut app = crate::app::state::AppState::test_new();
        let mut workspace = Workspace::test_new("stale-name");
        workspace.custom_name = None;
        workspace.identity_cwd = stale_cwd.clone();
        let pane = workspace.tabs[0].root_pane;

        app.workspaces = vec![workspace];
        app.ensure_test_terminals();
        let terminal_id = app.workspaces[0].tabs[0].panes[&pane]
            .attached_terminal_id
            .clone();
        let terminal = app.terminals.get_mut(&terminal_id).unwrap();
        terminal.cwd = stale_cwd;
        terminal.detected_agent = Some(Agent::Pi);
        app.active = Some(0);
        app.selected = 0;

        let (events, _) = tokio::sync::mpsc::channel(4);
        let runtime = crate::terminal::TerminalRuntime::spawn(
            pane,
            24,
            80,
            live_cwd.clone(),
            0,
            crate::terminal_theme::TerminalTheme::default(),
            None,
            crate::pane::PaneShellConfig::new("/bin/sh", crate::config::ShellModeConfig::NonLogin),
            &crate::pane::PaneLaunchEnv::default(),
            events,
            std::sync::Arc::new(tokio::sync::Notify::new()),
            std::sync::Arc::new(crate::render_signal::RenderSignal::new()),
        )
        .unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while runtime.cwd() != Some(live_cwd.clone()) && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let mut runtime_registry = TerminalRuntimeRegistry::new();
        runtime_registry.insert(terminal_id, runtime);
        let entries = agent_panel_entries_from(&app, &runtime_registry);
        let primary_label = entries[0].primary_label.clone();

        for (_, runtime) in runtime_registry.drain() {
            runtime.shutdown();
        }
        let _ = std::fs::remove_dir_all(root);

        assert_eq!(primary_label, "herdr");
    }

    #[test]
    fn all_workspaces_agent_panel_entries_prefer_agent_names_for_agent_identity() {
        let mut app = crate::app::state::AppState::test_new();
        let workspace = Workspace::test_new("bridge");
        let first_pane = workspace.tabs[0].root_pane;

        app.workspaces = vec![workspace];
        app.ensure_test_terminals();
        let first_terminal_id = app.workspaces[0].tabs[0].panes[&first_pane]
            .attached_terminal_id
            .clone();
        app.terminals
            .get_mut(&first_terminal_id)
            .unwrap()
            .detected_agent = Some(Agent::Pi);
        app.terminals
            .get_mut(&first_terminal_id)
            .unwrap()
            .set_agent_name("planner".into());
        app.active = Some(0);
        app.selected = 0;

        let entries = agent_panel_entries(&app);
        assert_eq!(entries[0].primary_label, "bridge");
        assert_eq!(entries[0].agent_label.as_deref(), Some("planner"));
    }

    #[test]
    fn expanded_sidebar_sections_handle_tiny_heights() {
        let (ws_area, detail_area) = expanded_sidebar_sections(Rect::new(0, 0, 20, 5), 0.9);

        assert_eq!(ws_area, Rect::new(0, 0, 19, 3));
        assert_eq!(detail_area, Rect::new(0, 3, 19, 2));
    }

    #[test]
    fn sidebar_section_divider_is_hidden_for_tiny_heights() {
        let divider = sidebar_section_divider_rect(Rect::new(0, 0, 20, 5), 0.5);

        assert_eq!(divider, Rect::default());
    }

    #[test]
    fn grouped_child_label_keeps_custom_workspace_name() {
        assert_eq!(
            grouped_child_display_label("renamed issue", Some("worktree/issue-137"), true),
            "renamed issue"
        );
    }

    #[test]
    fn grouped_child_label_uses_short_branch_for_auto_named_workspace() {
        assert_eq!(
            grouped_child_display_label("herdr-issue", Some("worktree/issue-137"), false),
            "issue-137"
        );
    }

    #[test]
    fn workspace_list_truncates_cjk_branch_without_panic() {
        let mut app = crate::app::state::AppState::test_new();
        let mut ws = Workspace::test_new("repo");
        ws.cached_git_branch = Some("feature/中文-分支-644".into());
        app.workspaces = vec![ws];
        app.active = Some(0);
        app.selected = 0;
        app.mode = Mode::Terminal;
        app.view.workspace_card_areas = vec![crate::app::state::WorkspaceCardArea {
            ws_idx: 0,
            rect: Rect::new(0, 1, 15, 2),
            indented: false,
        }];

        let mut terminal = Terminal::new(TestBackend::new(15, 6)).expect("test terminal");
        let runtimes = crate::terminal::TerminalRuntimeRegistry::new();

        terminal
            .draw(|frame| {
                render_workspace_list(&app, &runtimes, frame, Rect::new(0, 0, 15, 6), false)
            })
            .expect("workspace list should render");
    }

    #[test]
    fn pinned_space_row_leads_with_the_marker_and_unpinned_rows_do_not() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![Workspace::test_new("alpha"), Workspace::test_new("beta")];
        app.active = Some(0);
        app.mode = Mode::Terminal;
        app.pin_workspace(1);

        let area = Rect::new(0, 0, 30, 30);
        app.view.workspace_card_areas = compute_workspace_card_areas(&app, area);
        let list_area = workspace_list_rect(area, app.sidebar_section_split);
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| {
                render_workspace_list(
                    &app,
                    &TerminalRuntimeRegistry::new(),
                    frame,
                    list_area,
                    false,
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let rows: Vec<String> = app
            .view
            .workspace_card_areas
            .iter()
            .map(|card| row_text(buffer, card.rect.y, card.rect.width))
            .collect();
        // beta is pinned, so it is the first card.
        assert_eq!(app.view.workspace_card_areas[0].ws_idx, 1);
        assert!(
            rows[0].contains("↑") && rows[0].contains("beta"),
            "pinned row: {:?}",
            rows[0]
        );
        assert_eq!(rows[0].chars().next(), Some('↑'), "marker is in the lead");
        assert!(!rows[1].contains(PIN_MARKER), "unpinned row: {:?}", rows[1]);
    }

    /// A detached checkout used to lose its whole second row (branch `None`
    /// resolved to no token, and an empty row is dropped). It now says where
    /// it is instead, and names the operation holding it there.
    #[test]
    fn detached_head_workspace_keeps_its_second_row_and_says_where_it_is() {
        let mut app = crate::app::state::AppState::test_new();
        let mut detached = Workspace::test_new("herdr");
        detached.cached_git_branch = None;
        detached.cached_git_detached_head = Some(crate::workspace::DetachedHead {
            short_oid: "a620c06".into(),
            operation: Some(crate::workspace::GitOperation::Rebase),
        });
        let on_branch = Workspace::test_new("infra");
        let mut no_repo = Workspace::test_new("notes");
        no_repo.cached_git_branch = None;
        app.workspaces = vec![detached, on_branch, no_repo];
        app.workspaces[1].cached_git_branch = Some("main".into());
        app.active = Some(0);
        app.mode = Mode::Terminal;

        assert_eq!(workspace_row_height(&app, &app.workspaces[0], false), 2);
        assert_eq!(workspace_row_height(&app, &app.workspaces[1], false), 2);
        assert_eq!(
            workspace_row_height(&app, &app.workspaces[2], false),
            1,
            "outside a repo there is still nothing to say"
        );

        let area = Rect::new(0, 0, 30, 12);
        app.view.workspace_card_areas = compute_workspace_card_areas(&app, area);
        let list_area = workspace_list_rect(area, app.sidebar_section_split);
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| {
                render_workspace_list(
                    &app,
                    &TerminalRuntimeRegistry::new(),
                    frame,
                    list_area,
                    false,
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let card = app.view.workspace_card_areas[0].rect;
        let second = row_text(buffer, card.y + 1, card.width);
        assert!(
            second.contains("rebase @a620c06"),
            "detached row: {second:?}"
        );

        // Off the rebase, just the commit.
        app.workspaces[0]
            .cached_git_detached_head
            .as_mut()
            .unwrap()
            .operation = None;
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| {
                render_workspace_list(
                    &app,
                    &TerminalRuntimeRegistry::new(),
                    frame,
                    list_area,
                    false,
                )
            })
            .unwrap();
        let second = row_text(terminal.backend().buffer(), card.y + 1, card.width);
        assert!(second.contains("@a620c06"), "detached row: {second:?}");
        assert!(!second.contains("rebase"), "detached row: {second:?}");
    }

    fn workspace_with_worktree_space(
        name: &str,
        key: Option<&str>,
        checkout_key: &str,
    ) -> crate::workspace::Workspace {
        let mut ws = crate::workspace::Workspace::test_new(name);
        if let Some(key) = key {
            ws.worktree_space = Some(crate::workspace::WorktreeSpaceMembership {
                key: key.into(),
                label: "herdr".into(),
                repo_root: std::path::PathBuf::from("/repo/herdr"),
                checkout_path: std::path::PathBuf::from(checkout_key),
                is_linked_worktree: name != "main",
            });
        }
        ws
    }

    fn workspace_with_git_space(name: &str, key: &str) -> crate::workspace::Workspace {
        let mut ws = crate::workspace::Workspace::test_new(name);
        ws.cached_git_space = Some(crate::workspace::GitSpaceMetadata {
            key: key.into(),
            checkout_key: format!("/repo/{name}"),
            repo_name: "herdr".into(),
            repo_root: std::path::PathBuf::from(format!("/repo/{name}")),
            is_linked_worktree: false,
        });
        ws
    }

    #[test]
    fn desktop_worktree_tree_aligns_parents_and_marks_children() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
            workspace_with_worktree_space("review", Some("repo-key"), "/repo/herdr-review"),
            Workspace::test_new("notes"),
        ];
        app.sidebar_spaces.rows = vec![vec![
            crate::config::SpaceSidebarToken::StateIcon,
            crate::config::SpaceSidebarToken::Workspace,
        ]];
        app.sidebar_spaces.row_gap = 0;
        let area = Rect::new(0, 0, 30, 20);
        app.view.workspace_card_areas = compute_workspace_card_areas(&app, area);
        let list_area = workspace_list_rect(area, app.sidebar_section_split);

        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| {
                render_workspace_list(
                    &app,
                    &TerminalRuntimeRegistry::new(),
                    frame,
                    list_area,
                    false,
                )
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let cards = &app.view.workspace_card_areas;
        // Fork layout: the group chevron leads the parent row inline and
        // children indent three columns; there is no tree rail (the fork's
        // compact prefix replaced upstream's connector glyphs).
        assert_eq!(buffer[(cards[0].rect.x, cards[0].rect.y)].symbol(), "▾");
        let parent_name_x = find_symbol_x(buffer, cards[0].rect.y, cards[0].rect.width, "m");
        let child_name_x = find_symbol_x(buffer, cards[1].rect.y, cards[1].rect.width, "i");
        let last_child_name_x = find_symbol_x(buffer, cards[2].rect.y, cards[2].rect.width, "r");
        assert_eq!(child_name_x, last_child_name_x, "children share one indent");
        assert!(
            child_name_x > parent_name_x,
            "children indent deeper than the parent name ({child_name_x} vs {parent_name_x})"
        );
    }

    #[test]
    fn parent_workspace_row_stays_clickable_when_grouped() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
        ];
        app.sidebar_spaces.row_gap = 1;

        let (cards, headers) = compute_workspace_list_areas(&app, Rect::new(0, 0, 30, 20));

        assert!(headers.is_empty());
        assert_eq!(cards[0].ws_idx, 0);
        assert!(!cards[0].indented);
        assert_eq!(cards[1].ws_idx, 1);
        assert!(cards[1].indented);
        assert_eq!(cards[1].rect.y, cards[0].rect.y + cards[0].rect.height);
    }

    #[test]
    fn space_row_gap_preserves_compact_worktree_children() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
            workspace_with_worktree_space("review", Some("repo-key"), "/repo/herdr-review"),
            Workspace::test_new("notes"),
        ];
        app.sidebar_spaces.rows = vec![vec![crate::config::SpaceSidebarToken::Workspace]];
        app.sidebar_spaces.row_gap = 2;
        // Plain row geometry: edge rows would take a row of the window.
        app.sidebar_overflow = crate::config::SidebarOverflowConfig::Off;

        let (spacious, _) = compute_workspace_list_areas(&app, Rect::new(0, 0, 30, 30));
        assert_eq!(
            spacious[1].rect.y,
            spacious[0].rect.y + spacious[0].rect.height
        );
        assert_eq!(
            spacious[2].rect.y,
            spacious[1].rect.y + spacious[1].rect.height
        );
        assert_eq!(
            spacious[3].rect.y,
            spacious[2].rect.y + spacious[2].rect.height + 2
        );
        let spacious_metrics = workspace_list_scroll_metrics(&app, Rect::new(0, 0, 30, 7));
        assert_eq!(spacious_metrics.viewport_rows, 3);
        assert_eq!(spacious_metrics.max_offset_from_bottom, 2);

        app.sidebar_spaces.row_gap = 0;
        let (packed, _) = compute_workspace_list_areas(&app, Rect::new(0, 0, 30, 30));
        assert!(packed
            .windows(2)
            .all(|pair| pair[1].rect.y == pair[0].rect.y + pair[0].rect.height));
        let packed_metrics = workspace_list_scroll_metrics(&app, Rect::new(0, 0, 30, 7));
        assert_eq!(packed_metrics.viewport_rows, 4);
        assert_eq!(packed_metrics.max_offset_from_bottom, 0);
    }

    #[test]
    fn packed_workspace_drag_indicator_overlays_an_internal_boundary() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            Workspace::test_new("a"),
            Workspace::test_new("b"),
            Workspace::test_new("c"),
        ];
        app.sidebar_spaces.rows = vec![vec![crate::config::SpaceSidebarToken::Workspace]];
        app.sidebar_spaces.row_gap = 0;
        let area = Rect::new(0, 0, 30, 20);
        app.view.workspace_card_areas = compute_workspace_card_areas(&app, area);
        let list_area = workspace_list_rect(area, app.sidebar_section_split);
        let indicator_row = workspace_drop_indicator_row(
            &app,
            &app.view.workspace_card_areas,
            list_area,
            crate::app::state::WorkspaceDropTarget::Before(2),
        )
        .unwrap();
        assert_eq!(indicator_row, app.view.workspace_card_areas[1].rect.y);
        app.drag = Some(crate::app::state::DragState {
            target: crate::app::state::DragTarget::WorkspaceReorder {
                source_id: 0,
                source_ws_idx: 0,
                drop_target: Some(crate::app::state::WorkspaceDropTarget::Before(2)),
            },
        });

        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|frame| {
                render_workspace_list(
                    &app,
                    &TerminalRuntimeRegistry::new(),
                    frame,
                    list_area,
                    false,
                )
            })
            .unwrap();

        assert_eq!(
            terminal.backend().buffer()[(list_area.x, indicator_row)].symbol(),
            "─"
        );
    }

    #[test]
    fn linked_only_worktree_members_do_not_form_parentless_group() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
            workspace_with_worktree_space("review", Some("repo-key"), "/repo/herdr-review"),
        ];

        let entries = workspace_list_entries(&app);

        assert_eq!(
            entries,
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: false
                },
            ]
        );
    }

    #[test]
    fn compact_space_group_scroll_clamps_when_all_entries_fit() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("one", Some("repo-key"), "/repo/herdr-one"),
            workspace_with_worktree_space("two", Some("repo-key"), "/repo/herdr-two"),
        ];
        let area = Rect::new(0, 0, 30, 20);
        app.workspace_scroll = normalized_workspace_scroll(&app, area, 2);

        let (cards, headers) = compute_workspace_list_areas(&app, area);

        assert!(headers.is_empty());
        assert_eq!(app.workspace_scroll, 0);
        assert_eq!(cards.len(), 3);
        assert_eq!(cards[2].ws_idx, 2);
    }

    #[test]
    fn workspace_scroll_metrics_count_display_entries_not_raw_workspaces() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
            Workspace::test_new("notes"),
        ];
        for workspace in &mut app.workspaces {
            workspace.cached_git_branch = Some("main".into());
        }
        app.collapsed_space_keys.insert("repo-key".into());
        app.active = None;
        app.mode = Mode::Terminal;

        let ws_area = Rect::new(0, 0, 30, 6);
        let metrics = workspace_list_scroll_metrics(&app, ws_area);

        assert_eq!(metrics.viewport_rows, 1);
        assert_eq!(metrics.max_offset_from_bottom, 1);
        assert_eq!(metrics.offset_from_bottom, 1);
    }

    #[test]
    fn workspace_scroll_offset_applies_to_group_children() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
            Workspace::test_new("notes"),
        ];
        app.collapsed_space_keys.insert("repo-key".into());
        app.active = None;
        app.mode = Mode::Terminal;
        app.workspace_scroll = 1;

        let (cards, headers) = compute_workspace_list_areas(&app, Rect::new(0, 0, 30, 12));

        assert!(headers.is_empty());
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].ws_idx, 2);
    }

    #[test]
    fn workspace_list_entries_group_multiple_workspaces_in_same_git_space() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
        ];

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: true,
                },
            ]
        );
    }

    #[test]
    fn workspace_list_entries_group_non_contiguous_explicit_members() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_git_space("normal", "other-key"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
        ];

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 2,
                    indented: true,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: false,
                },
            ]
        );
    }

    #[test]
    fn a_group_is_pinned_when_any_member_is_and_moves_whole() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_git_space("plain", "other-key"),
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
        ];
        // Pin the linked child only: the whole group still moves up together.
        app.pin_workspace(2);

        assert_eq!(workspace_order(&app), vec![1, 2, 0]);
    }

    #[test]
    fn workspace_list_entries_do_not_group_normal_git_workspaces() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_git_space("one", "repo-key"),
            workspace_with_git_space("two", "repo-key"),
        ];

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: false,
                },
            ]
        );
    }

    #[test]
    fn workspace_list_entries_do_not_auto_attach_normal_git_workspace_to_group() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_git_space("scratch", "repo-key"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
        ];

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 2,
                    indented: true,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: false,
                },
            ]
        );
    }

    #[test]
    fn workspace_list_entries_leave_single_git_and_non_git_workspaces_flat() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_git_space("one", "repo-key"),
            workspace_with_worktree_space("notes", None, "/notes"),
        ];

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: false,
                },
            ]
        );
    }

    #[test]
    fn collapsed_group_hides_inactive_children_but_keeps_active_visible() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
        ];
        app.active = Some(1);
        app.mode = Mode::Terminal;
        app.collapsed_space_keys.insert("repo-key".into());

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: true,
                },
            ]
        );

        app.active = None;
        app.mode = Mode::Terminal;
        assert_eq!(
            workspace_list_entries(&app),
            vec![WorkspaceListEntry::Workspace {
                ws_idx: 0,
                indented: false,
            }]
        );
    }

    #[test]
    fn collapsed_group_keeps_selected_child_visible_in_navigate_mode() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
        ];
        app.mode = Mode::Navigate;
        app.selected = 1;
        app.active = Some(1);
        app.collapsed_space_keys.insert("repo-key".into());

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: true,
                },
            ]
        );
    }
}
