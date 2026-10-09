use std::collections::HashMap;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
    widgets::{Paragraph, Widget},
};

use super::state_presentation::Background;
use super::*;

pub(super) struct AgentRow {
    pub(super) todos: crate::ui::TodoDisplaySummary,
    pub(super) state_change_seq: u64,
    pub(super) background: Background,
    pub(super) jump_index: usize,
    pub(super) pin_rank: Option<usize>,
    pub(super) pane_id: String,
    pub(super) status: crate::api::schema::AgentStatus,
    pub(super) focused: bool,
    pub(super) rows: Vec<Vec<crate::ui::ResolvedToken>>,
}

pub(super) fn ordered_agent_pane_ids(
    snapshot: &ClientShellSnapshot,
    sort: crate::config::AgentPanelSortConfig,
) -> Vec<String> {
    let mut ids = if snapshot.agent_view_label.is_some() {
        snapshot
            .agent_order
            .iter()
            .filter(|id| snapshot.agents.iter().any(|agent| &agent.pane_id == *id))
            .cloned()
            .collect::<Vec<_>>()
    } else {
        let mut agents = snapshot.agents.iter().collect::<Vec<_>>();
        if sort == crate::config::AgentPanelSortConfig::Priority {
            agents.sort_by_key(|agent| {
                (
                    std::cmp::Reverse(status_priority(agent.agent_status)),
                    std::cmp::Reverse(agent.state_change_seq),
                )
            });
        }
        agents
            .into_iter()
            .map(|agent| agent.pane_id.clone())
            .collect()
    };
    if snapshot
        .resource_facts
        .as_ref()
        .and_then(|f| f.pane_pins.as_ref())
        .is_some_and(|pins| !pins.is_empty())
    {
        ids.sort_by_key(|id| {
            let pin = super::pins::pane_pin(snapshot, id);
            (pin.is_none(), pin.unwrap_or(u64::MAX))
        });
    }
    ids
}

pub(super) fn render_agent_panel(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    agent_scroll: &mut usize,
    hits: &mut ShellHitMap,
) {
    if !render_agent_panel_header(
        buffer,
        area,
        snapshot.agent_view_label.as_deref(),
        config,
        hits,
    ) {
        return;
    }

    let rows = agent_rows(snapshot, config, None);
    render_agent_list(
        buffer,
        area,
        &rows,
        snapshot
            .agent_view_label
            .as_ref()
            .map(|_| " no matching agents"),
        config,
        agent_scroll,
        hits,
        |row| row.rows.len(),
        |row| super::sidebar_overflow::item(row.status, row.pin_rank),
        |row| row.focused,
        |buffer, rect, row, hits| {
            if row.pin_rank.is_some() {
                if let Some(marker) = super::pins::marker_rect(rect, 1, config) {
                    hits.pin_markers.push((
                        marker,
                        ClientEndpointId::Local,
                        crate::api::schema::Method::AgentUnpin(crate::api::schema::AgentTarget {
                            target: row.pane_id.clone(),
                        }),
                    ));
                }
            }
            hits.agents.push((rect, row.pane_id.clone()));
            render_agent_row(buffer, rect, row, config);
        },
    );
}

pub(super) fn render_agent_panel_header(
    buffer: &mut Buffer,
    area: Rect,
    agent_view_label: Option<&str>,
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) -> bool {
    if area.height == 0 {
        return false;
    }
    put_text(
        buffer,
        area.x,
        area.y,
        area.width,
        &"─".repeat(area.width as usize),
        Style::default().fg(config.palette.surface_dim),
    );
    if area.height < 2 {
        return false;
    }
    put_text(
        buffer,
        area.x,
        area.y + 1,
        area.width,
        if config.sidebar_style == crate::config::SidebarStyleConfig::Editorial {
            " AGENTS"
        } else {
            " agents"
        },
        super::sidebar_chrome::header_style(config),
    );
    let sort_label = agent_view_label.unwrap_or(match config.agent_panel_sort {
        crate::config::AgentPanelSortConfig::Spaces => "grouped",
        crate::config::AgentPanelSortConfig::Priority => "priority",
    });
    let sort_width = display_width(sort_label).min(area.width as usize) as u16;
    let sort_rect = Rect::new(
        area.right().saturating_sub(sort_width),
        area.y + 1,
        sort_width,
        1,
    );
    hits.agent_sort_toggle = if config.mouse_capture && agent_view_label.is_none() {
        sort_rect
    } else {
        Rect::default()
    };
    put_text(
        buffer,
        sort_rect.x,
        sort_rect.y,
        sort_rect.width,
        sort_label,
        Style::default()
            .fg(if agent_view_label.is_some() {
                config.palette.accent
            } else {
                config.palette.overlay0
            })
            .add_modifier(Modifier::BOLD),
    );
    true
}

pub(super) fn render_agent_list<T>(
    buffer: &mut Buffer,
    area: Rect,
    rows: &[T],
    empty_message: Option<&str>,
    config: &ClientShellConfig,
    agent_scroll: &mut usize,
    hits: &mut ShellHitMap,
    row_lines: impl Fn(&T) -> usize,
    edge_item: impl Fn(&T) -> super::sidebar_overflow::EdgeItem,
    exempt: impl Fn(&T) -> bool,
    mut render_row: impl FnMut(&mut Buffer, Rect, &T, &mut ShellHitMap),
) {
    let body = Rect::new(
        area.x,
        area.y.saturating_add(3),
        area.width,
        area.height.saturating_sub(3),
    );
    hits.agent_body = body;
    if body.is_empty() || rows.is_empty() {
        *agent_scroll = 0;
        if let Some(message) = empty_message.filter(|_| !body.is_empty()) {
            put_text(
                buffer,
                body.x,
                body.y,
                body.width,
                message,
                Style::default()
                    .fg(config.palette.overlay0)
                    .add_modifier(Modifier::DIM),
            );
        }
        return;
    }

    let row_heights = rows
        .iter()
        .map(|row| row_lines(row).max(1).min(u16::MAX as usize) as u16)
        .collect::<Vec<_>>();
    let gaps = rows
        .iter()
        .enumerate()
        .map(|(index, _)| {
            if index + 1 < rows.len() {
                config.agents.row_gap
            } else {
                0
            }
        })
        .collect::<Vec<_>>();
    if config.follow_agent {
        if let Some(target) = rows.iter().position(&exempt) {
            *agent_scroll = super::sidebar_overflow::reveal_start(
                &row_heights,
                &gaps,
                body.height,
                *agent_scroll,
                target,
                config.sidebar_overflow.edge_rows(),
            );
        }
    }
    let (metrics, reserve, count) = super::sidebar_overflow::list_metrics(
        &row_heights,
        &gaps,
        body.height,
        *agent_scroll,
        config.sidebar_overflow.edge_rows(),
    );
    hits.agent_max_scroll = metrics.max_offset_from_bottom;
    hits.agent_scroll_metrics = Some(metrics);
    *agent_scroll = metrics
        .max_offset_from_bottom
        .saturating_sub(metrics.offset_from_bottom);
    let show_scrollbar = metrics.max_offset_from_bottom > 0 && body.width > 1;
    let content_width = body.width.saturating_sub(u16::from(show_scrollbar));
    let items = rows.iter().map(edge_item).collect::<Vec<_>>();
    let mut visible = Vec::new();
    let mut y = body.y + reserve.top;
    let bottom = body.bottom().saturating_sub(reserve.bottom);
    for (index, row) in rows.iter().enumerate().skip(*agent_scroll).take(count) {
        let height =
            row_heights[index].min(body.height.saturating_sub(reserve.top + reserve.bottom));
        if y.saturating_add(height) > bottom {
            break;
        }
        let rect = Rect::new(body.x, y, content_width, height);
        visible.push(super::sidebar_overflow::VisibleItem {
            index,
            rect,
            exempt: exempt(row),
        });
        render_row(buffer, rect, row, hits);
        y = y
            .saturating_add(height)
            .saturating_add(if index + 1 < rows.len() {
                config.agents.row_gap
            } else {
                0
            });
    }

    let plan = super::sidebar_overflow::plan(
        config.sidebar_overflow.fog(),
        &items,
        *agent_scroll,
        &visible,
        Rect::new(body.x, body.y, content_width, body.height),
        reserve,
    );
    super::sidebar_overflow::paint(buffer, &plan, config, hits, true, count);
    if show_scrollbar {
        let track = Rect::new(body.right().saturating_sub(1), body.y, 1, body.height);
        hits.agent_scrollbar = track;
        super::scroll::render_list_scrollbar(buffer, track, metrics, &config.palette);
    }
}

pub(super) fn agent_rows(
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    machine: Option<&str>,
) -> Vec<AgentRow> {
    let mut pin_rank = 0;
    ordered_agent_pane_ids(snapshot, config.agent_panel_sort)
        .into_iter()
        .enumerate()
        .filter_map(|(index, pane_id)| {
            let mut row = agent_row(snapshot, &pane_id, config, machine)?;
            row.jump_index = index;
            if super::pins::pane_pin(snapshot, &row.pane_id).is_some() {
                row.pin_rank = Some(pin_rank);
                pin_rank += 1;
            }
            Some(row)
        })
        .collect()
}

pub(super) fn agent_row(
    snapshot: &ClientShellSnapshot,
    pane_id: &str,
    config: &ClientShellConfig,
    machine: Option<&str>,
) -> Option<AgentRow> {
    let agent = snapshot
        .agents
        .iter()
        .find(|agent| agent.pane_id == pane_id)?;
    let workspace = snapshot
        .workspaces
        .iter()
        .find(|workspace| workspace.workspace_id == agent.workspace_id)?;
    let tab = snapshot.tabs.iter().find(|tab| tab.tab_id == agent.tab_id);
    let pane = snapshot
        .panes
        .iter()
        .find(|pane| pane.pane_id == agent.pane_id);
    let tab_count = snapshot
        .tabs
        .iter()
        .filter(|candidate| candidate.workspace_id == agent.workspace_id)
        .count();
    let tab_label = tab
        .filter(|tab| tab_count > 1 || tab.custom_label)
        .map(|tab| tab.label.as_str());
    let agent_label = agent
        .display_agent
        .as_deref()
        .or(agent.name.as_deref())
        .or(agent.agent.as_deref())
        .or(agent.title.as_deref());
    let labels = agent
        .state_labels
        .iter()
        .cloned()
        .collect::<HashMap<_, _>>();
    let tokens = agent.tokens.iter().cloned().collect::<HashMap<_, _>>();
    let state_text = labels
        .get(status_text(agent.agent_status))
        .map(String::as_str)
        .unwrap_or_else(|| sidebar_status_text(agent.agent_status));
    let canonical_agent = agent
        .agent
        .as_deref()
        .and_then(crate::detect::parse_agent_label);
    let rows = crate::ui::sidebar_agent_rows(
        &config.agents,
        crate::ui::AgentTokenContext {
            machine,
            workspace: &workspace.label,
            tab: tab_label,
            pane: agent
                .title
                .as_deref()
                .or_else(|| pane.and_then(|pane| pane.label.as_deref())),
            agent_label,
            terminal_title: agent.terminal_title.as_deref(),
            terminal_title_stripped: agent.terminal_title_stripped.as_deref(),
            canonical_agent,
            tokens: &tokens,
        },
        state_text,
    );
    Some(AgentRow {
        todos: snapshot
            .resource_facts
            .as_ref()
            .and_then(|f| f.pane_todos.as_ref())
            .and_then(|facts| facts.get(&agent.pane_id))
            .map(crate::ui::TodoDisplaySummary::from_fact)
            .unwrap_or_default(),
        state_change_seq: agent.state_change_seq,
        background: Background::from_snapshot(snapshot, &agent.pane_id),
        jump_index: 0,
        pin_rank: None,
        pane_id: agent.pane_id.clone(),
        status: agent.agent_status,
        focused: agent.focused,
        rows,
    })
}

pub(super) fn render_agent_row(
    buffer: &mut Buffer,
    rect: Rect,
    row: &AgentRow,
    config: &ClientShellConfig,
) {
    let palette = &config.palette;
    let row_style = if row.focused {
        Style::default().bg(palette.active_row_bg)
    } else {
        Style::default()
    };
    let name_style = if row.focused {
        Style::default()
            .fg(palette.text)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(palette.subtext0)
            .add_modifier(Modifier::BOLD)
    };
    let status_style = Style::default().fg(config.state_presentation.agent_color(
        row.status,
        row.background,
        palette,
    ));
    let editorial = config.sidebar_style == crate::config::SidebarStyleConfig::Editorial;
    let jump = config
        .show_agent_numbers
        .then(|| crate::config::jump_symbol(row.jump_index))
        .flatten();
    let label = super::sidebar_chrome::number_label(jump, &config.agent_number_prefix);
    let todo_label = match row.todos.open {
        0 => String::new(),
        n if n > 99 => "τ 99+".to_owned(),
        n => format!("τ {n}"),
    };
    let todo_color = crate::ui::todo_priority_color(row.todos.priority, palette, config.todo_color);
    let jump_row = usize::from(!todo_label.is_empty());
    let number_color = config.agent_number_color.unwrap_or(palette.overlay0);
    let bar =
        u16::from(config.sidebar_active_border == crate::config::SidebarActiveBorderConfig::Left);
    let secondary = Style::default().fg(palette.overlay0);
    let secondary = if editorial && !row.focused {
        secondary.add_modifier(Modifier::DIM)
    } else {
        secondary
    };
    let icon = (
        config.state_presentation.agent_icon(
            row.status,
            row.background,
            row.state_change_seq,
            config,
        ),
        Style::default().fg(config.state_presentation.agent_color(
            row.status,
            row.background,
            palette,
        )),
    );
    let rows = if row.rows.is_empty() {
        vec![vec![crate::ui::ResolvedToken {
            kind: crate::ui::ResolvedTokenKind::StateIcon,
            style: Default::default(),
        }]]
    } else {
        row.rows.clone()
    };
    for (index, tokens) in rows.iter().take(rect.height as usize).enumerate() {
        let indent = bar + if index == 0 { 1 } else { 3 };
        let reserve = if index == 0 && !todo_label.is_empty() {
            super::sidebar_chrome::number_reserve(&todo_label, config.sidebar_active_border)
        } else if editorial && index == jump_row {
            super::sidebar_chrome::number_reserve(&label, config.sidebar_active_border)
        } else {
            0
        };
        let mut spans = vec![ratatui::text::Span::raw(" ".repeat(usize::from(indent)))];
        spans.extend(crate::ui::resolved_token_spans(
            tokens,
            icon,
            status_style,
            name_style,
            secondary,
            secondary,
            palette,
            rect.width.saturating_sub(indent + reserve) as usize,
        ));
        Paragraph::new(Line::from(spans)).style(row_style).render(
            Rect::new(rect.x, rect.y + index as u16, rect.width, 1),
            buffer,
        );
    }
    if let Some(rank) = row.pin_rank {
        if rect.width > bar {
            buffer.set_stringn(
                rect.x + bar,
                rect.y,
                "↑",
                1,
                Style::default().fg(super::pins::marker_color(rank)),
            );
        }
    }
    if !todo_label.is_empty() {
        super::sidebar_chrome::draw_number(
            buffer,
            rect,
            rect.y,
            &todo_label,
            todo_color,
            config.sidebar_active_border,
        );
        let width = display_width(&todo_label).min(usize::from(u16::MAX)) as u16;
        if rect.width >= width.saturating_add(4) {
            let end = rect.right().saturating_sub(u16::from(
                config.sidebar_active_border == crate::config::SidebarActiveBorderConfig::Right,
            ));
            for x in end - width..end {
                buffer[(x, rect.y)].set_style(Style::default().add_modifier(Modifier::BOLD));
            }
        }
    }
    if editorial {
        super::sidebar_chrome::draw_number(
            buffer,
            rect,
            rect.y.saturating_add(jump_row as u16),
            &label,
            number_color,
            config.sidebar_active_border,
        );
    } else if let Some(jump) = jump {
        if rect.height > 1 {
            buffer.set_stringn(
                rect.x + bar + 1,
                rect.y + 1,
                jump.to_string(),
                1,
                Style::default().fg(number_color),
            );
        }
    }
    super::sidebar_chrome::draw_active_border(
        buffer,
        rect,
        row.focused,
        config,
        config.agents.row_gap,
    );
}

fn put_text(buffer: &mut Buffer, x: u16, y: u16, width: u16, text: &str, style: Style) {
    for (offset, character) in text.chars().take(width as usize).enumerate() {
        if let Some(cell) = buffer.cell_mut((x + offset as u16, y)) {
            cell.set_char(character).set_style(style);
        }
    }
}

fn display_width(text: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(text)
}

fn sidebar_status_text(status: crate::api::schema::AgentStatus) -> &'static str {
    use crate::api::schema::AgentStatus;
    match status {
        AgentStatus::Blocked => "blocked",
        AgentStatus::Done => "done",
        AgentStatus::Working => "working",
        AgentStatus::Idle | AgentStatus::Unknown => "idle",
    }
}
