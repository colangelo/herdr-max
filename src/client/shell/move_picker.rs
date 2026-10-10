//! The pane-move picker (fork 9c3e219b, 74ccda9d, b32ad6be, ac7abfa4,
//! 5ed89450, 73c2b2a0, 77da76e2) and the other pane-move keys: break the
//! focused pane into a new tab, or move it to the next or previous tab.
//!
//! The picker is a tree of every space and its tabs, read once from the
//! snapshot when it opens, with a "here" row for the pane's own tab, a "new
//! tab" row per space and a "new space" row. Choosing a row closes it and
//! sends `pane.move`; the server moves the pane and focus follows it.

use super::render::{display_width, put_text};
use super::todo_panel::render_panel_shell;
use super::todo_text::{apply_text_key, Shape, TextField};
use super::*;
use crate::api::schema::{
    AgentStatus, Method, PaneMoveDestination, PaneMoveParams, PaneMoveReason, ResponseResult,
    SplitDirection,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::style::Color;
use std::collections::HashSet;

const MIN_WIDTH: u16 = 48;
const MAX_WIDTH: u16 = 120;
/// Title, search, rule.
const HEADER_ROWS: u16 = 3;
/// A rule and one line of text, dropped when the list would be too short.
const DETAIL_ROWS: u16 = 2;
const FOOTER_ROWS: u16 = 2;
const STATUS_MAX_COLUMNS: usize = 36;
/// Gutter, branch, icon and space, and twelve cells of label.
const LABEL_FLOOR: usize = 3 + 4 + 2 + 12;
const SEARCH_MAX_CHARS: usize = 256;
const BUTTON_GAP: u16 = 2;
const HERE: &str = "you are here";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum MoveTarget {
    Tab { tab_id: String },
    NewTab { workspace_id: String },
    NewSpace,
}

/// A tab row, a new-tab row or the new-space row. Its facts are read when
/// the picker opens and never refreshed while it is open.
#[derive(Clone, Debug)]
pub(super) struct MoveEntry {
    /// `None` only for the new-space row.
    pub(super) workspace_id: Option<String>,
    /// The tab's position in its space, from 1; 0 for the action rows.
    pub(super) number: usize,
    /// The tab's custom name, or empty.
    pub(super) label: String,
    pub(super) target: MoveTarget,
    pub(super) space: String,
    pub(super) pane_names: Vec<String>,
    pub(super) status: AgentStatus,
}

#[derive(Clone, Debug)]
pub(super) enum MoveItem {
    SpaceHeading {
        label: String,
        pane_count: usize,
        activity: String,
        status: AgentStatus,
    },
    /// The tab the pane is in: context, never selectable.
    Here(MoveEntry),
    Destination(MoveEntry),
    Gap,
}

impl MoveItem {
    /// Drawn with a tree branch under its space's heading.
    fn is_branch(&self) -> bool {
        match self {
            Self::Here(_) => true,
            Self::Destination(entry) => entry.workspace_id.is_some(),
            _ => false,
        }
    }
}

fn row_label(entry: &MoveEntry) -> String {
    match entry.target {
        MoveTarget::Tab { .. } if entry.label.is_empty() => format!("tab {}", entry.number),
        MoveTarget::Tab { .. } => format!("tab {} · {}", entry.number, entry.label),
        MoveTarget::NewTab { .. } => "new tab".to_owned(),
        MoveTarget::NewSpace => "new space".to_owned(),
    }
}

fn search_text(entry: &MoveEntry) -> String {
    format!(
        "{} {} {}",
        entry.space,
        row_label(entry),
        entry.pane_names.join(" ")
    )
}

/// Every word of the query is somewhere in the text, ignoring case.
pub(super) fn matches_query(text: &str, query: &str) -> bool {
    let text = text.to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|word| text.contains(word))
}

#[derive(Debug)]
pub(super) struct ClientMovePickerOverlay {
    pub(super) boot_id: String,
    pub(super) source_pane_id: String,
    pub(super) source_label: String,
    /// "{space} › {tab}" for the title.
    pub(super) source_place: String,
    pub(super) all_items: Vec<MoveItem>,
    /// `all_items` narrowed by the search.
    pub(super) items: Vec<MoveItem>,
    /// An index into `items`.
    pub(super) selected: usize,
    pub(super) scroll: usize,
    pub(super) search: TextField,
    pub(super) search_focused: bool,
}

impl ClientMovePickerOverlay {
    fn new(boot_id: String, source_pane_id: String, items: Vec<MoveItem>) -> Self {
        let mut picker = Self {
            boot_id,
            source_pane_id,
            source_label: String::new(),
            source_place: String::new(),
            all_items: items.clone(),
            items,
            selected: 0,
            scroll: 0,
            search: TextField::new(SEARCH_MAX_CHARS),
            search_focused: false,
        };
        picker.select_first_destination();
        picker
    }

    fn query(&self) -> &str {
        self.search.text().trim()
    }

    fn select_first_destination(&mut self) {
        self.selected = self
            .items
            .iter()
            .position(|item| matches!(item, MoveItem::Destination(_)))
            .unwrap_or(0);
        self.scroll = 0;
    }

    /// Narrow the rows to the query. A heading that matches keeps every row
    /// of its space; otherwise a space keeps the rows that match, with its
    /// heading and its here row, and goes when none do.
    fn refilter(&mut self) {
        let query = self.query().to_owned();
        if query.is_empty() {
            self.items = self.all_items.clone();
            self.select_first_destination();
            return;
        }
        let mut groups: Vec<Vec<MoveItem>> = Vec::new();
        let mut index = 0;
        while index < self.all_items.len() {
            match &self.all_items[index] {
                heading @ MoveItem::SpaceHeading { label, .. } => {
                    let heading_matches = matches_query(label, &query);
                    let mut group = vec![heading.clone()];
                    let mut kept = false;
                    index += 1;
                    while let Some(item) = self.all_items.get(index).filter(|item| item.is_branch())
                    {
                        match item {
                            MoveItem::Destination(entry)
                                if heading_matches
                                    || matches_query(&search_text(entry), &query) =>
                            {
                                group.push(item.clone());
                                kept = true;
                            }
                            MoveItem::Here(_) => group.push(item.clone()),
                            _ => {}
                        }
                        index += 1;
                    }
                    if kept {
                        groups.push(group);
                    }
                }
                item @ MoveItem::Destination(entry) => {
                    if matches_query(&search_text(entry), &query) {
                        groups.push(vec![item.clone()]);
                    }
                    index += 1;
                }
                MoveItem::Here(_) | MoveItem::Gap => index += 1,
            }
        }
        let mut items = Vec::new();
        for group in groups {
            if !items.is_empty() {
                items.push(MoveItem::Gap);
            }
            items.extend(group);
        }
        self.items = items;
        self.select_first_destination();
    }

    /// Step to the nearest destination above (`-1`) or below (`1`); stay put
    /// when there is none.
    fn step(&mut self, delta: isize) -> bool {
        let mut index = self.selected;
        loop {
            let Some(next) = index.checked_add_signed(delta) else {
                return false;
            };
            let Some(item) = self.items.get(next) else {
                return false;
            };
            if matches!(item, MoveItem::Destination(_)) {
                self.selected = next;
                return true;
            }
            index = next;
        }
    }

    fn select_destination(&mut self, index: usize) -> bool {
        if matches!(self.items.get(index), Some(MoveItem::Destination(_))) {
            self.selected = index;
            return true;
        }
        false
    }

    fn selected_destination(&self) -> Option<&MoveEntry> {
        match self.items.get(self.selected) {
            Some(MoveItem::Destination(entry)) => Some(entry),
            _ => None,
        }
    }

    /// Keep the selection in view, and its space's heading too when both fit.
    fn reveal(&mut self, visible: usize) {
        let len = self.items.len();
        let mut heading = None;
        for index in (0..self.selected).rev() {
            match self.items[index] {
                MoveItem::Gap => break,
                MoveItem::SpaceHeading { .. } => {
                    heading = Some(index);
                    break;
                }
                _ => {}
            }
        }
        let scroll = reveal_scroll(self.scroll, self.selected, visible, len);
        self.scroll = match heading {
            Some(heading)
                if self.selected - heading < visible && visible > 0 && heading < scroll =>
            {
                heading
            }
            _ => scroll,
        };
    }

    fn start(&self, visible: usize) -> usize {
        self.scroll.min(self.items.len().saturating_sub(visible))
    }

    fn destination_count(&self) -> usize {
        self.all_items
            .iter()
            .filter(|item| matches!(item, MoveItem::Destination(_)))
            .count()
    }
}

/// The smallest scroll that keeps `index` inside a `visible`-row window.
pub(super) fn reveal_scroll(scroll: usize, index: usize, visible: usize, len: usize) -> usize {
    if visible == 0 {
        return 0;
    }
    let scroll = if index < scroll {
        index
    } else if index >= scroll.saturating_add(visible) {
        index.saturating_add(1).saturating_sub(visible)
    } else {
        scroll
    };
    scroll.min(len.saturating_sub(visible))
}

/// The fork's pane name: its title, manual label, terminal title, agent
/// name, agent, else "pane N".
pub(super) fn pane_display_label(
    snapshot: &ClientShellSnapshot,
    pane: &crate::protocol::ClientShellPane,
) -> String {
    let agent = snapshot
        .agents
        .iter()
        .find(|agent| agent.pane_id == pane.pane_id);
    [
        agent.and_then(|agent| agent.title.as_deref()),
        pane.label.as_deref(),
        agent.and_then(|agent| agent.terminal_title_stripped.as_deref()),
        agent.and_then(|agent| agent.name.as_deref()),
        agent.and_then(|agent| agent.display_agent.as_deref()),
        agent.and_then(|agent| agent.agent.as_deref()),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .find(|label| !label.is_empty())
    .map(str::to_owned)
    .unwrap_or_else(|| pane_id_label(&pane.pane_id))
}

/// "pane N" from a public pane id: what a pane with no name of its own is
/// called, and what a pane the snapshot does not hold is called.
pub(super) fn pane_id_label(pane_id: &str) -> String {
    let suffix = pane_id
        .rsplit_once(":p")
        .map_or(pane_id, |(_, suffix)| suffix);
    match crate::workspace::decode_public_number(suffix) {
        Some(number) => format!("pane {number}"),
        None => format!("pane {suffix}"),
    }
}

/// "2 blocked · 1 working · 1 done", or empty.
fn activity_summary(snapshot: &ClientShellSnapshot, workspace_id: &str) -> String {
    let count = |status| {
        snapshot
            .agents
            .iter()
            .filter(|agent| agent.workspace_id == workspace_id && agent.agent_status == status)
            .count()
    };
    [
        (count(AgentStatus::Blocked), "blocked"),
        (count(AgentStatus::Working), "working"),
        (count(AgentStatus::Done), "done"),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, word)| format!("{count} {word}"))
    .collect::<Vec<_>>()
    .join(" · ")
}

/// Build the picker for the focused pane, or say why there is none.
#[cfg(test)]
pub(super) fn move_picker_for_snapshot(
    snapshot: &ClientShellSnapshot,
) -> Result<ClientMovePickerOverlay, &'static str> {
    move_picker_in_drawn_order(snapshot, None)
}

/// Destinations in the space list's drawn order at open time, bubble motion
/// included, as the fork's picker listed them.
pub(super) fn move_picker_in_drawn_order(
    snapshot: &ClientShellSnapshot,
    motion: Option<&super::sort_motion::SortMotion>,
) -> Result<ClientMovePickerOverlay, &'static str> {
    let source_pane = snapshot
        .focused_pane_id
        .as_deref()
        .filter(|_| snapshot.focused_workspace_id.is_some())
        .and_then(|pane_id| snapshot.panes.iter().find(|pane| pane.pane_id == pane_id))
        .ok_or("no focused pane")?;
    let source_workspace = snapshot
        .workspaces
        .iter()
        .find(|workspace| workspace.workspace_id == source_pane.workspace_id)
        .ok_or("workspace disappeared")?;
    let source_tab = snapshot
        .tabs
        .iter()
        .find(|tab| tab.tab_id == source_pane.tab_id)
        .ok_or("tab disappeared")?;
    if source_tab.zoomed {
        return Err("pane moves are unavailable while the source tab is zoomed");
    }
    let panes_of = |tab_id: &str| -> Vec<&crate::protocol::ClientShellPane> {
        snapshot
            .panes
            .iter()
            .filter(|pane| pane.tab_id == tab_id)
            .collect()
    };
    let tabs_of = |workspace_id: &str| -> Vec<&crate::protocol::ClientShellTab> {
        snapshot
            .tabs
            .iter()
            .filter(|tab| tab.workspace_id == workspace_id)
            .collect()
    };
    let source_pane_is_alone = panes_of(&source_tab.tab_id).len() <= 1;

    // The source space first, then the sidebar's order.
    let mut order = vec![source_workspace];
    for entry in super::sidebar::workspace_entries(snapshot, &HashSet::new(), motion) {
        if let Some(workspace) = snapshot.workspaces.get(entry.index) {
            if workspace.workspace_id != source_workspace.workspace_id {
                order.push(workspace);
            }
        }
    }

    let mut items = Vec::new();
    let mut source_place = String::new();
    for workspace in order {
        let is_source = workspace.workspace_id == source_workspace.workspace_id;
        let space = workspace.label.clone();
        let mut branches = Vec::new();
        let mut destination_count = 0;
        for (position, tab) in tabs_of(&workspace.workspace_id).into_iter().enumerate() {
            let entry = MoveEntry {
                workspace_id: Some(workspace.workspace_id.clone()),
                number: position + 1,
                label: if tab.custom_label {
                    tab.label.clone()
                } else {
                    String::new()
                },
                target: MoveTarget::Tab {
                    tab_id: tab.tab_id.clone(),
                },
                space: space.clone(),
                pane_names: panes_of(&tab.tab_id)
                    .into_iter()
                    .map(|pane| pane_display_label(snapshot, pane))
                    .collect(),
                status: tab.agent_status,
            };
            if is_source && tab.tab_id == source_tab.tab_id {
                source_place = format!("{space} › {}", row_label(&entry));
                branches.push(MoveItem::Here(entry));
            } else if !tab.zoomed {
                branches.push(MoveItem::Destination(entry));
                destination_count += 1;
            }
        }
        if !(is_source && source_pane_is_alone) {
            branches.push(MoveItem::Destination(MoveEntry {
                workspace_id: Some(workspace.workspace_id.clone()),
                number: 0,
                label: String::new(),
                target: MoveTarget::NewTab {
                    workspace_id: workspace.workspace_id.clone(),
                },
                space: space.clone(),
                pane_names: Vec::new(),
                status: AgentStatus::Unknown,
            }));
            destination_count += 1;
        }
        if destination_count == 0 {
            continue;
        }
        if !items.is_empty() {
            items.push(MoveItem::Gap);
        }
        items.push(MoveItem::SpaceHeading {
            label: space,
            pane_count: snapshot
                .panes
                .iter()
                .filter(|pane| pane.workspace_id == workspace.workspace_id)
                .count(),
            activity: activity_summary(snapshot, &workspace.workspace_id),
            status: workspace.agent_status,
        });
        items.extend(branches);
    }

    let whole_session = snapshot.workspaces.len() == 1
        && tabs_of(&source_workspace.workspace_id).len() == 1
        && source_pane_is_alone;
    if !whole_session {
        if !items.is_empty() {
            items.push(MoveItem::Gap);
        }
        items.push(MoveItem::Destination(MoveEntry {
            workspace_id: None,
            number: 0,
            label: String::new(),
            target: MoveTarget::NewSpace,
            space: String::new(),
            pane_names: Vec::new(),
            status: AgentStatus::Unknown,
        }));
    }
    if !items
        .iter()
        .any(|item| matches!(item, MoveItem::Destination(_)))
    {
        return Err("there is nowhere to move this pane");
    }

    let mut picker =
        ClientMovePickerOverlay::new(snapshot.boot_id.clone(), source_pane.pane_id.clone(), items);
    picker.source_label = pane_display_label(snapshot, source_pane);
    picker.source_place = source_place;
    Ok(picker)
}

// -- geometry -------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MoveButton {
    Move,
    Cancel,
}

fn button_text(button: MoveButton) -> &'static str {
    match button {
        MoveButton::Move => " ↵ move ",
        MoveButton::Cancel => " esc cancel ",
    }
}

const BUTTONS: [MoveButton; 2] = [MoveButton::Move, MoveButton::Cancel];

fn buttons_width() -> u16 {
    BUTTONS
        .iter()
        .map(|button| display_width(button_text(*button)))
        .sum::<u16>()
        + BUTTON_GAP
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct MovePickerLayout {
    pub(super) outer: Rect,
    pub(super) header: Rect,
    pub(super) list: Rect,
    pub(super) detail: Option<Rect>,
    pub(super) footer_row: Option<Rect>,
    pub(super) buttons: Vec<(Rect, MoveButton)>,
    /// The first item drawn on the list's first row.
    pub(super) start: usize,
}

impl MovePickerLayout {
    pub(super) fn search_row(&self) -> Rect {
        Rect::new(
            self.header.x,
            self.header.y.saturating_add(1),
            self.header.width,
            u16::from(self.header.height >= 2),
        )
    }

    fn row_at(&self, x: u16, y: u16, len: usize) -> Option<usize> {
        if !super::contains(self.list, (x, y)) {
            return None;
        }
        let index = self.start + usize::from(y - self.list.y);
        (index < len).then_some(index)
    }
}

fn left_width(item: &MoveItem) -> usize {
    let text = match item {
        MoveItem::SpaceHeading {
            label, pane_count, ..
        } => format!("{label} ({pane_count})"),
        MoveItem::Here(entry) | MoveItem::Destination(entry) => row_label(entry),
        MoveItem::Gap => return 0,
    };
    3 + if item.is_branch() { 4 } else { 0 } + 2 + usize::from(display_width(&text)) + 1
}

fn item_status(item: &MoveItem) -> String {
    match item {
        MoveItem::SpaceHeading { activity, .. } => activity.clone(),
        MoveItem::Here(_) => HERE.to_owned(),
        MoveItem::Destination(entry) => entry.pane_names.join(", "),
        MoveItem::Gap => String::new(),
    }
}

/// The status column: 0 when no row has one, else the widest (capped) plus
/// padding.
fn status_width(items: &[MoveItem]) -> usize {
    let widest = items
        .iter()
        .map(|item| usize::from(display_width(&item_status(item))))
        .max()
        .unwrap_or(0);
    if widest == 0 {
        0
    } else {
        widest.min(STATUS_MAX_COLUMNS) + 2
    }
}

fn widest_left(items: &[MoveItem]) -> usize {
    items.iter().map(left_width).max().unwrap_or(0)
}

fn content_width(items: &[MoveItem]) -> usize {
    (widest_left(items) + status_width(items) + 2).max(usize::from(buttons_width()) + 2)
}

/// Centred on the screen and sized from every row, so a query never resizes
/// it.
pub(super) fn move_picker_layout(
    picker: &ClientMovePickerOverlay,
    screen: Rect,
) -> Option<MovePickerLayout> {
    if screen.width == 0 || screen.height == 0 {
        return None;
    }
    let width = (content_width(&picker.all_items)
        .clamp(usize::from(MIN_WIDTH), usize::from(MAX_WIDTH)) as u16)
        .min(screen.width.max(1));
    let rows = picker.all_items.len() + usize::from(DETAIL_ROWS);
    let height = (usize::from(HEADER_ROWS) + rows + 2 + usize::from(FOOTER_ROWS))
        .min(usize::from(screen.height.max(1))) as u16;
    let outer = Rect::new(
        screen.x + (screen.width - width) / 2,
        screen.y + (screen.height - height) / 2,
        width,
        height,
    );
    let inner = Rect::new(
        outer.x.saturating_add(1),
        outer.y.saturating_add(1),
        outer.width.saturating_sub(2),
        outer.height.saturating_sub(2),
    );
    let (mut list, footer_row) = if inner.width > 0 && inner.height >= FOOTER_ROWS {
        (
            Rect::new(inner.x, inner.y, inner.width, inner.height - FOOTER_ROWS),
            Some(Rect::new(inner.x, inner.bottom() - 1, inner.width, 1)),
        )
    } else {
        (inner, None)
    };
    let header_height = HEADER_ROWS.min(list.height);
    let header = Rect::new(list.x, list.y, list.width, header_height);
    list = Rect::new(
        list.x,
        list.y + header_height,
        list.width,
        list.height - header_height,
    );
    let detail = (list.height >= DETAIL_ROWS + 3).then(|| {
        list.height -= DETAIL_ROWS;
        Rect::new(list.x, list.bottom(), list.width, DETAIL_ROWS)
    });
    let buttons = footer_row
        .map(|row| {
            let mut x = row.x + row.width.saturating_sub(buttons_width()) / 2;
            BUTTONS
                .iter()
                .map(|button| {
                    let width =
                        display_width(button_text(*button)).min(row.right().saturating_sub(x));
                    let rect = Rect::new(x, row.y, width, 1);
                    x = x.saturating_add(width + BUTTON_GAP).min(row.right());
                    (rect, *button)
                })
                .collect()
        })
        .unwrap_or_default();
    Some(MovePickerLayout {
        outer,
        header,
        list,
        detail,
        footer_row,
        buttons,
        start: picker.start(usize::from(list.height)),
    })
}

// -- render ---------------------------------------------------------------------

fn contrast(p: &Palette) -> Color {
    super::panel_contrast_fg(p)
}

/// Every name if they fit; else as many as fit with "+N" for the rest; else
/// the whole list cut short.
pub(super) fn fit_pane_names(names: &[String], budget: usize) -> String {
    let all = names.join(", ");
    if usize::from(display_width(&all)) <= budget {
        return all;
    }
    for shown in (1..names.len()).rev() {
        let text = format!("{}, +{}", names[..shown].join(", "), names.len() - shown);
        if usize::from(display_width(&text)) <= budget {
            return text;
        }
    }
    crate::ui::text::truncate_end(&all, budget)
}

fn detail_text(picker: &ClientMovePickerOverlay) -> String {
    let Some(entry) = picker.selected_destination() else {
        return String::new();
    };
    match entry.target {
        MoveTarget::Tab { .. } => format!(
            "{} › {}: {}",
            entry.space,
            row_label(entry),
            entry.pane_names.join(", ")
        ),
        MoveTarget::NewTab { .. } => format!("a new tab in {}", entry.space),
        MoveTarget::NewSpace if picker.source_label.is_empty() => "a new space".to_owned(),
        MoveTarget::NewSpace => format!("a new space holding {}", picker.source_label),
    }
}

pub(super) fn render_rule(b: &mut Buffer, x: u16, y: u16, width: u16, p: &Palette) {
    put_text(
        b,
        x,
        y,
        width,
        &"─".repeat(usize::from(width)),
        Style::default().fg(p.surface1).bg(p.panel_bg),
    );
}

/// Spans drawn left to right, each whole while it fits; the first that does
/// not is cut with `…` and the rest are dropped.
fn put_spans(b: &mut Buffer, x: u16, y: u16, budget: usize, spans: &[(String, Style)]) {
    let mut used = 0usize;
    for (text, style) in spans {
        let width = usize::from(display_width(text));
        let room = budget.saturating_sub(used);
        if width <= room {
            put_text(b, x + used as u16, y, width as u16, text, *style);
            used += width;
            continue;
        }
        let cut = crate::ui::text::truncate_end(text, room);
        let cut_width = usize::from(display_width(&cut));
        put_text(b, x + used as u16, y, cut_width as u16, &cut, *style);
        break;
    }
}

#[allow(clippy::too_many_arguments)] // one row of a tree: each fact is drawn
fn render_row(
    b: &mut Buffer,
    row: Rect,
    item: &MoveItem,
    selected: bool,
    last_branch: bool,
    status_width: usize,
    config: &ClientShellConfig,
) {
    let p = &config.palette;
    let bar = Style::default()
        .fg(contrast(p))
        .bg(p.accent)
        .add_modifier(Modifier::BOLD);
    let pick = |style: Style| {
        if selected {
            bar
        } else {
            style.bg(p.panel_bg)
        }
    };
    b.set_style(row, pick(Style::default()));
    let dim = Style::default().fg(p.overlay0);
    let icon = |status| config.state_icon(status).to_owned();
    let mut spans = Vec::new();
    spans.push((
        if matches!(item, MoveItem::Here(_)) {
            " ◆ "
        } else {
            "   "
        }
        .to_owned(),
        pick(Style::default().fg(p.accent)),
    ));
    if item.is_branch() {
        spans.push((
            if last_branch {
                "└── "
            } else {
                "├── "
            }
            .to_owned(),
            pick(Style::default().fg(p.surface1)),
        ));
    }
    let status_style = match item {
        MoveItem::SpaceHeading {
            label,
            pane_count,
            status,
            ..
        } => {
            spans.push((
                icon(*status),
                pick(Style::default().fg(config.state_color(*status))),
            ));
            spans.push((" ".to_owned(), pick(Style::default())));
            spans.push((
                format!("{label} ({pane_count})"),
                pick(Style::default().fg(p.accent).add_modifier(Modifier::BOLD)),
            ));
            dim
        }
        MoveItem::Here(entry) => {
            spans.push((icon(entry.status), pick(dim)));
            spans.push((" ".to_owned(), pick(Style::default())));
            spans.push((row_label(entry), pick(dim)));
            dim.add_modifier(Modifier::ITALIC)
        }
        MoveItem::Destination(entry) => match entry.target {
            MoveTarget::Tab { .. } => {
                spans.push((
                    icon(entry.status),
                    pick(Style::default().fg(config.state_color(entry.status))),
                ));
                spans.push((" ".to_owned(), pick(Style::default())));
                spans.push(("tab ".to_owned(), pick(dim)));
                spans.push((
                    entry.number.to_string(),
                    pick(Style::default().fg(p.text).add_modifier(Modifier::BOLD)),
                ));
                if !entry.label.is_empty() {
                    spans.push((" · ".to_owned(), pick(dim)));
                    spans.push((entry.label.clone(), pick(Style::default().fg(p.text))));
                }
                Style::default().fg(p.subtext0)
            }
            MoveTarget::NewTab { .. } | MoveTarget::NewSpace => {
                spans.push(("+".to_owned(), pick(Style::default().fg(p.accent))));
                spans.push((" ".to_owned(), pick(Style::default())));
                spans.push((row_label(entry), pick(Style::default().fg(p.overlay1))));
                dim
            }
        },
        MoveItem::Gap => return,
    };
    put_spans(
        b,
        row.x,
        row.y,
        usize::from(row.width).saturating_sub(status_width),
        &spans,
    );
    if status_width == 0 {
        return;
    }
    let budget = status_width.saturating_sub(2);
    let status = match item {
        MoveItem::Destination(entry) => fit_pane_names(&entry.pane_names, budget),
        other => crate::ui::text::truncate_end(&item_status(other), budget),
    };
    let x = row.right().saturating_sub(status_width as u16);
    put_text(
        b,
        x,
        row.y,
        status_width as u16,
        &format!(" {status}"),
        pick(status_style),
    );
}

fn render_title(b: &mut Buffer, area: Rect, picker: &ClientMovePickerOverlay, p: &Palette) {
    let mut spans = vec![(
        "move pane".to_owned(),
        Style::default()
            .fg(p.text)
            .bg(p.panel_bg)
            .add_modifier(Modifier::BOLD),
    )];
    if !picker.source_label.is_empty() {
        spans.push(("  ".to_owned(), Style::default().bg(p.panel_bg)));
        spans.push((
            picker.source_label.clone(),
            Style::default()
                .fg(p.accent)
                .bg(p.panel_bg)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if !picker.source_place.is_empty() {
        spans.push((
            format!("  from {}", picker.source_place),
            Style::default().fg(p.overlay0).bg(p.panel_bg),
        ));
    }
    put_spans(b, area.x, area.y, usize::from(area.width), &spans);
}

/// The search row; returns the caret's cell while the search is focused.
fn render_search(
    b: &mut Buffer,
    row: Rect,
    picker: &ClientMovePickerOverlay,
    p: &Palette,
) -> Option<crate::protocol::CursorState> {
    let n = picker.destination_count();
    let count = format!(
        "{n} {}",
        if n == 1 {
            "destination"
        } else {
            "destinations"
        }
    );
    render_search_row(
        b,
        row,
        &picker.search,
        picker.search_focused,
        "search destinations",
        &count,
        p,
    )
}

/// A search row shared by the list overlays: ` / `, the query or its
/// placeholder, the count at the row's end. Returns the caret's cell while the
/// search is focused.
pub(super) fn render_search_row(
    b: &mut Buffer,
    row: Rect,
    search: &TextField,
    focused: bool,
    placeholder: &str,
    count: &str,
    p: &Palette,
) -> Option<crate::protocol::CursorState> {
    if row.width == 0 || row.height == 0 {
        return None;
    }
    let slash = if focused {
        Style::default()
            .fg(p.accent)
            .bg(p.panel_bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(p.overlay0).bg(p.panel_bg)
    };
    put_text(b, row.x, row.y, 3.min(row.width), " / ", slash);
    let text_x = row.x + 3.min(row.width);
    let room = row.right().saturating_sub(text_x);
    let query = search.text();
    let used = if query.is_empty() {
        put_text(
            b,
            text_x,
            row.y,
            room,
            placeholder,
            Style::default().fg(p.overlay0).bg(p.panel_bg),
        );
        3 + display_width(placeholder).min(room)
    } else {
        put_text(
            b,
            text_x,
            row.y,
            room,
            query,
            Style::default().fg(p.text).bg(p.panel_bg),
        );
        3 + display_width(query).min(room)
    };
    let count_width = display_width(count);
    if used + 1 + count_width < row.width {
        put_text(
            b,
            row.right() - 1 - count_width,
            row.y,
            count_width,
            count,
            Style::default().fg(p.overlay0).bg(p.panel_bg),
        );
    }
    focused.then(|| crate::protocol::CursorState {
        x: (row.x as usize + 3 + search.cursor_column()).min(usize::from(row.right() - 1)) as u16,
        y: row.y,
        visible: true,
        shape: 0,
    })
}

pub(super) fn render_scrollbar(b: &mut Buffer, list: Rect, start: usize, len: usize, p: &Palette) {
    let visible = usize::from(list.height);
    if list.width <= 1 || visible == 0 || len <= visible {
        return;
    }
    let metrics = crate::pane::ScrollMetrics {
        viewport_rows: visible,
        offset_from_bottom: len - visible - start,
        max_offset_from_bottom: len - visible,
    };
    let track = Rect::new(list.right() - 1, list.y, 1, list.height);
    let Some(thumb) = crate::ui::scrollbar_thumb(metrics, track) else {
        return;
    };
    for y in track.y..track.bottom() {
        let color = if y >= thumb.top && y < thumb.top + thumb.len {
            p.overlay0
        } else {
            p.surface_dim
        };
        let cell = &mut b[(track.x, y)];
        cell.set_symbol("▕");
        cell.set_style(Style::default().fg(color));
    }
}

/// Dim the screen, then draw the picker centred on it. Returns its layout
/// and, while the search is focused, the caret.
pub(super) fn render_move_picker(
    b: &mut Buffer,
    picker: &ClientMovePickerOverlay,
    config: &ClientShellConfig,
) -> Option<(MovePickerLayout, Option<crate::protocol::CursorState>)> {
    let p = &config.palette;
    for y in b.area.y..b.area.bottom() {
        for x in b.area.x..b.area.right() {
            let cell = &mut b[(x, y)];
            cell.set_style(cell.style().add_modifier(Modifier::DIM));
        }
    }
    let layout = move_picker_layout(picker, b.area)?;
    render_panel_shell(b, layout.outer, p.accent, p.panel_bg)?;
    let header = layout.header;
    if header.height >= 2 {
        render_title(
            b,
            Rect::new(header.x + 1, header.y, header.width.saturating_sub(2), 1),
            picker,
            p,
        );
    }
    let cursor = render_search(b, layout.search_row(), picker, p);
    if header.height >= 3 {
        render_rule(b, header.x, header.y + 2, header.width, p);
    }

    let list = layout.list;
    let all_status = status_width(&picker.all_items);
    let draw_status = all_status.min(
        usize::from(list.width).saturating_sub(LABEL_FLOOR.min(widest_left(&picker.all_items))),
    );
    for (offset, item) in picker
        .items
        .iter()
        .enumerate()
        .skip(layout.start)
        .take(usize::from(list.height))
    {
        let row = Rect::new(
            list.x,
            list.y + (offset - layout.start) as u16,
            list.width,
            1,
        );
        let last_branch = !picker
            .items
            .get(offset + 1)
            .is_some_and(MoveItem::is_branch);
        render_row(
            b,
            row,
            item,
            offset == picker.selected,
            last_branch,
            draw_status,
            config,
        );
    }
    render_scrollbar(b, list, layout.start, picker.items.len(), p);
    if picker.items.is_empty() && list.height > 0 {
        put_text(
            b,
            list.x,
            list.y,
            list.width,
            " no match",
            Style::default().fg(p.overlay0).bg(p.panel_bg),
        );
    }

    if let Some(detail) = layout.detail {
        render_rule(b, detail.x, detail.y, detail.width, p);
        let text = crate::ui::text::middle_elide(
            &detail_text(picker),
            usize::from(detail.width.saturating_sub(2)),
        );
        put_text(
            b,
            detail.x,
            detail.y + 1,
            detail.width,
            &format!(" {text}"),
            Style::default().fg(p.overlay0).bg(p.panel_bg),
        );
    }

    for (rect, button) in &layout.buttons {
        let style = match button {
            MoveButton::Move => Style::default()
                .fg(contrast(p))
                .bg(p.accent)
                .add_modifier(Modifier::BOLD),
            MoveButton::Cancel => Style::default()
                .fg(p.text)
                .bg(p.surface0)
                .add_modifier(Modifier::BOLD),
        };
        b.set_style(*rect, style);
        put_text(b, rect.x, rect.y, rect.width, button_text(*button), style);
    }
    Some((layout, cursor))
}

// -- input and requests -----------------------------------------------------------

/// The fork's `list_chord`: what moves the selection rather than editing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Chord {
    Prev,
    Next,
    HalfPageUp,
    HalfPageDown,
    First,
    Last,
}

impl Chord {
    /// Where this chord takes `selected` in a list of `len` rows shown
    /// `visible` at a time: one row, half a page, or an end. Clamped, never
    /// wrapping.
    pub(super) fn target(self, selected: usize, visible: usize, len: usize) -> usize {
        let half = (visible / 2).max(1);
        let last = len.saturating_sub(1);
        match self {
            Self::Prev => selected.saturating_sub(1),
            Self::Next => selected.saturating_add(1).min(last),
            Self::HalfPageUp => selected.saturating_sub(half),
            Self::HalfPageDown => selected.saturating_add(half).min(last),
            Self::First => 0,
            Self::Last => last,
        }
    }
}

/// While the search has focus plain letters are text; otherwise `j`/`k` and
/// `ctrl+u`/`ctrl+d` move too.
pub(super) fn list_chord(
    code: KeyCode,
    modifiers: KeyModifiers,
    plain_chars_are_text: bool,
) -> Option<Chord> {
    let bare = modifiers.is_empty();
    let ctrl = modifiers == KeyModifiers::CONTROL;
    match code {
        KeyCode::Up if bare => Some(Chord::Prev),
        KeyCode::Down if bare => Some(Chord::Next),
        KeyCode::Home if bare => Some(Chord::First),
        KeyCode::End if bare => Some(Chord::Last),
        KeyCode::Char('k' | 'p') if ctrl => Some(Chord::Prev),
        KeyCode::Char('j' | 'n') if ctrl => Some(Chord::Next),
        KeyCode::Char('k') if bare && !plain_chars_are_text => Some(Chord::Prev),
        KeyCode::Char('j') if bare && !plain_chars_are_text => Some(Chord::Next),
        KeyCode::Char('u') if ctrl && !plain_chars_are_text => Some(Chord::HalfPageUp),
        KeyCode::Char('d') if ctrl && !plain_chars_are_text => Some(Chord::HalfPageDown),
        _ => None,
    }
}

fn reason_text(reason: Option<&PaneMoveReason>) -> &'static str {
    match reason {
        Some(PaneMoveReason::ZoomedTab) => {
            "pane moves are unavailable while a source or target tab is zoomed"
        }
        Some(PaneMoveReason::SameTab) => "the pane is already in that tab",
        None => "the pane was not moved",
    }
}

impl ClientShellState {
    fn move_picker_mut(&mut self) -> Option<&mut ClientMovePickerOverlay> {
        match self.overlay.as_mut() {
            Some(ClientShellOverlay::MovePicker(picker)) => Some(picker),
            _ => None,
        }
    }

    /// The fork's pane-move feedback: a toast titled with what happened.
    fn pane_move_feedback(
        &mut self,
        title: &str,
        body: impl Into<String>,
        outcome: &mut ClientShellInput,
    ) {
        outcome.repaint |= self.push_feedback_toast(title, body);
    }

    pub(super) fn open_move_picker(&mut self, outcome: &mut ClientShellInput) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        match move_picker_in_drawn_order(snapshot, Some(&self.config.sort_motion)) {
            Ok(picker) => {
                self.overlay = Some(ClientShellOverlay::MovePicker(picker));
            }
            Err(message) => {
                self.pane_move_feedback("pane move unavailable", message, outcome);
                self.mode = ClientShellMode::Terminal;
            }
        }
        outcome.repaint = true;
    }

    fn send_pane_move(
        &mut self,
        pane_id: String,
        destination: PaneMoveDestination,
        outcome: &mut ClientShellInput,
    ) {
        self.push_endpoint_method_with_kind(
            Method::PaneMove(PaneMoveParams {
                pane_id,
                destination,
                focus: true,
            }),
            PendingEndpointKind::PaneMove,
            outcome,
        );
    }

    fn move_pane_to_tab(
        &mut self,
        pane_id: String,
        tab_id: String,
        outcome: &mut ClientShellInput,
    ) {
        self.send_pane_move(
            pane_id,
            PaneMoveDestination::Tab {
                tab_id,
                target_pane_id: None,
                split: SplitDirection::Right,
                ratio: None,
            },
            outcome,
        );
    }

    /// The focused pane, its workspace and its tab, or the fork's feedback.
    fn pane_move_source(
        &mut self,
        outcome: &mut ClientShellInput,
    ) -> Option<(String, String, String)> {
        let source = self.snapshot.as_deref().and_then(|snapshot| {
            let pane_id = snapshot.focused_pane_id.as_deref()?;
            snapshot.focused_workspace_id.as_deref()?;
            let pane = snapshot.panes.iter().find(|pane| pane.pane_id == pane_id)?;
            Some((
                pane.pane_id.clone(),
                pane.workspace_id.clone(),
                pane.tab_id.clone(),
            ))
        });
        if source.is_none() {
            self.pane_move_feedback("pane move unavailable", "no focused pane", outcome);
        }
        source
    }

    /// Break the focused pane out into a new tab of its own space.
    pub(super) fn break_pane(&mut self, outcome: &mut ClientShellInput) {
        let Some((pane_id, workspace_id, tab_id)) = self.pane_move_source(outcome) else {
            return;
        };
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        if !snapshot
            .workspaces
            .iter()
            .any(|workspace| workspace.workspace_id == workspace_id)
        {
            self.pane_move_feedback("pane move unavailable", "workspace disappeared", outcome);
            return;
        }
        if !snapshot.tabs.iter().any(|tab| tab.tab_id == tab_id) {
            self.pane_move_feedback("pane move unavailable", "tab disappeared", outcome);
            return;
        }
        if snapshot
            .panes
            .iter()
            .filter(|pane| pane.tab_id == tab_id)
            .count()
            <= 1
        {
            self.pane_move_feedback(
                "nothing to break out",
                "the focused pane is already alone in its tab",
                outcome,
            );
            return;
        }
        self.send_pane_move(
            pane_id,
            PaneMoveDestination::NewTab {
                workspace_id: Some(workspace_id),
                label: None,
            },
            outcome,
        );
    }

    /// Move the focused pane into the next (`1`) or previous (`-1`) tab of
    /// its space, without wrapping.
    pub(super) fn move_pane_to_adjacent_tab(
        &mut self,
        delta: isize,
        outcome: &mut ClientShellInput,
    ) {
        let Some((pane_id, workspace_id, tab_id)) = self.pane_move_source(outcome) else {
            return;
        };
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        if !snapshot
            .workspaces
            .iter()
            .any(|workspace| workspace.workspace_id == workspace_id)
        {
            self.pane_move_feedback("pane move unavailable", "workspace disappeared", outcome);
            return;
        }
        let tabs = snapshot
            .tabs
            .iter()
            .filter(|tab| tab.workspace_id == workspace_id)
            .collect::<Vec<_>>();
        let target = tabs
            .iter()
            .position(|tab| tab.tab_id == tab_id)
            .and_then(|current| current.checked_add_signed(delta))
            .and_then(|target| tabs.get(target))
            .map(|tab| tab.tab_id.clone());
        let Some(target) = target else {
            let direction = if delta < 0 { "previous" } else { "next" };
            self.pane_move_feedback(
                "no adjacent tab",
                format!("there is no {direction} tab to move into"),
                outcome,
            );
            return;
        };
        self.move_pane_to_tab(pane_id, target, outcome);
    }

    /// The answer to a `pane.move`. Errors are titled by the shared endpoint
    /// error path; a move the server declined says why.
    pub(super) fn handle_pane_move_result(
        &mut self,
        result: &Result<ResponseResult, ClientShellEndpointError>,
        outcome: &mut ClientShellInput,
    ) {
        match result {
            Ok(ResponseResult::PaneMove { move_result }) if !move_result.changed => {
                self.pane_move_feedback(
                    "pane move unavailable",
                    reason_text(move_result.reason.as_ref()),
                    outcome,
                );
            }
            Ok(ResponseResult::PaneMove { .. }) | Err(_) => {}
            Ok(_) => {
                tracing::warn!("unexpected pane.move response");
                self.pane_move_feedback("pane move failed", "unexpected runtime response", outcome);
            }
        }
    }

    fn visible_move_picker_rows(&self) -> usize {
        self.hits
            .move_picker
            .as_ref()
            .map_or(0, |layout| usize::from(layout.list.height))
    }

    fn move_move_picker_selection(&mut self, chord: Chord) {
        let visible = self.visible_move_picker_rows();
        let Some(picker) = self.move_picker_mut() else {
            return;
        };
        let half = (visible / 2).max(1);
        match chord {
            Chord::Prev => {
                picker.step(-1);
            }
            Chord::Next => {
                picker.step(1);
            }
            Chord::HalfPageUp => (0..half).for_each(|_| {
                picker.step(-1);
            }),
            Chord::HalfPageDown => (0..half).for_each(|_| {
                picker.step(1);
            }),
            Chord::First => (0..picker.items.len()).for_each(|_| {
                picker.step(-1);
            }),
            Chord::Last => (0..picker.items.len()).for_each(|_| {
                picker.step(1);
            }),
        }
        picker.reveal(visible);
    }

    fn refilter_move_picker(&mut self) {
        let visible = self.visible_move_picker_rows();
        if let Some(picker) = self.move_picker_mut() {
            picker.refilter();
            picker.reveal(visible);
        }
    }

    fn close_move_picker(&mut self, outcome: &mut ClientShellInput) {
        self.overlay = None;
        self.mode = ClientShellMode::Terminal;
        outcome.repaint = true;
    }

    /// Close first, then send: a failure shows over the view, not the picker.
    /// With nothing selected (a search that matched nothing) it stays open.
    fn submit_move_picker(&mut self, outcome: &mut ClientShellInput) {
        let Some(picker) = self.move_picker_mut() else {
            return;
        };
        let Some(target) = picker
            .selected_destination()
            .map(|entry| entry.target.clone())
        else {
            return;
        };
        let pane_id = picker.source_pane_id.clone();
        self.close_move_picker(outcome);
        match target {
            MoveTarget::Tab { tab_id } => self.move_pane_to_tab(pane_id, tab_id, outcome),
            MoveTarget::NewTab { workspace_id } => self.send_pane_move(
                pane_id,
                PaneMoveDestination::NewTab {
                    workspace_id: Some(workspace_id),
                    label: None,
                },
                outcome,
            ),
            MoveTarget::NewSpace => self.send_pane_move(
                pane_id,
                PaneMoveDestination::NewWorkspace {
                    label: None,
                    tab_label: None,
                },
                outcome,
            ),
        }
    }

    /// Text arriving as text: only while the search has focus.
    pub(super) fn insert_move_picker_text(&mut self, text: &str) -> bool {
        let Some(picker) = self.move_picker_mut() else {
            return false;
        };
        if !picker.search_focused {
            return false;
        }
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        if picker.search.insert_str(&text) {
            self.refilter_move_picker();
        }
        true
    }

    /// Close the picker when its server boot is gone.
    pub(crate) fn tick_move_picker(&mut self, outcome: &mut ClientShellInput) {
        let Some(ClientShellOverlay::MovePicker(picker)) = self.overlay.as_ref() else {
            return;
        };
        if self
            .snapshot
            .as_deref()
            .is_some_and(|snapshot| snapshot.boot_id != picker.boot_id)
        {
            self.close_move_picker(outcome);
        }
    }

    pub(super) fn route_move_picker_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) {
        if key.kind == crossterm::event::KeyEventKind::Release {
            return;
        }
        let Some(picker) = self.move_picker_mut() else {
            return;
        };
        let (code, modifiers) = (key.code, key.modifiers);
        outcome.repaint = true;
        if picker.search_focused {
            if code == KeyCode::Esc {
                picker.search_focused = false;
                return;
            }
            if code == KeyCode::Enter {
                self.submit_move_picker(outcome);
                return;
            }
            if let Some(chord) = list_chord(code, modifiers, true) {
                self.move_move_picker_selection(chord);
                return;
            }
            let before = picker.search.text().to_owned();
            // Text the host composed is authoritative (AltGr, dead keys).
            let handled = match key.generated_text.as_deref().filter(|text| {
                !text.is_empty() && !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            }) {
                Some(text) => {
                    let text: String = text.chars().filter(|c| !c.is_control()).collect();
                    picker.search.insert_str(&text);
                    true
                }
                None => apply_text_key(
                    &mut picker.search,
                    KeyEvent::new(code, modifiers),
                    Shape::SingleLine,
                ),
            };
            if handled && picker.search.text() != before {
                self.refilter_move_picker();
            }
            return;
        }
        match code {
            KeyCode::Esc if !picker.query().is_empty() => {
                picker.search.clear();
                self.refilter_move_picker();
            }
            KeyCode::Esc => self.close_move_picker(outcome),
            KeyCode::Enter => self.submit_move_picker(outcome),
            KeyCode::Char('/') if modifiers.is_empty() => picker.search_focused = true,
            _ => match list_chord(code, modifiers, false) {
                Some(chord) => self.move_move_picker_selection(chord),
                None => outcome.repaint = false,
            },
        }
    }

    /// Mouse while the picker is open. A click on a destination moves there
    /// at once; a click outside the box cancels; the rest is inert.
    pub(super) fn route_move_picker_mouse(
        &mut self,
        mouse: MouseEvent,
        outcome: &mut ClientShellInput,
    ) {
        let point = (mouse.column, mouse.row);
        let Some(layout) = self.hits.move_picker.clone() else {
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                self.close_move_picker(outcome);
            }
            return;
        };
        let visible = usize::from(layout.list.height);
        let Some(picker) = self.move_picker_mut() else {
            return;
        };
        let hovered = layout.row_at(mouse.column, mouse.row, picker.items.len());
        let on_destination = hovered
            .filter(|index| matches!(picker.items.get(*index), Some(MoveItem::Destination(_))));
        match mouse.kind {
            MouseEventKind::Moved => {
                if let Some(index) = on_destination {
                    outcome.repaint |= picker.selected != index;
                    picker.select_destination(index);
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                picker.step(if mouse.kind == MouseEventKind::ScrollUp {
                    -1
                } else {
                    1
                });
                picker.reveal(visible);
                outcome.repaint = true;
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(index) = on_destination {
                    picker.select_destination(index);
                    self.submit_move_picker(outcome);
                    return;
                }
                if hovered.is_some() {
                    return;
                }
                if let Some((_, button)) = layout
                    .buttons
                    .iter()
                    .find(|(rect, _)| super::contains(*rect, point))
                {
                    match button {
                        MoveButton::Move => self.submit_move_picker(outcome),
                        MoveButton::Cancel => self.close_move_picker(outcome),
                    }
                    return;
                }
                if layout.footer_row.is_some_and(|row| mouse.row == row.y) {
                    return;
                }
                if super::contains(layout.search_row(), point) {
                    picker.search_focused = true;
                    outcome.repaint = true;
                    return;
                }
                if !super::contains(layout.outer, point) {
                    self.close_move_picker(outcome);
                }
            }
            _ => {}
        }
    }
}
