use ratatui::{
    buffer::Buffer,
    layout::{Direction, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use super::scrollbar::{render_pane_scrollbar, should_show_scrollbar};
#[cfg(test)]
use super::text::display_width;
use super::text::{display_width_u16, truncate_end};
use super::widgets::panel_contrast_fg;
use crate::app::state::Palette;
use crate::app::AppState;
use crate::layout::{PaneId, PaneInfo};
use crate::config::PaneBorderActiveStyleConfig;
use crate::popup_size::resolve_popup_geometry;
use crate::terminal::{TerminalRuntime, TerminalRuntimeRegistry};

pub(crate) fn pane_is_scrolled_back(rt: &TerminalRuntime) -> bool {
    rt.scroll_metrics()
        .is_some_and(|metrics| metrics.offset_from_bottom > 0)
}

fn pane_border_title(label: &str, pane_width: u16, _focused: bool) -> Option<String> {
    let label = label.trim();
    if label.is_empty() || pane_width <= 4 {
        return None;
    }
    let max_label_width = pane_width.saturating_sub(4) as usize;
    Some(format!(" {} ", truncate_end(label, max_label_width)))
}

// Full view computation reaches this helper for active and background panes.
// Keep terminal queries narrow, allocation-free, and short under the core lock.
fn terminal_inner_rect(rt: &TerminalRuntime, pane_inner: Rect, pane_scrollbars: bool) -> Rect {
    terminal_inner_rect_for(pane_inner, pane_scrollbars && !rt.alternate_screen_active())
}

fn terminal_inner_rect_for(pane_inner: Rect, scrollbar_gutter: bool) -> Rect {
    if !scrollbar_gutter || pane_inner.width <= 4 {
        return pane_inner;
    }

    Rect::new(
        pane_inner.x,
        pane_inner.y,
        pane_inner.width.saturating_sub(1),
        pane_inner.height,
    )
}

fn zoomed_pane_borders(app: &AppState, multi_pane: bool) -> Borders {
    if app.pane_borders.shows_borders(multi_pane) && app.pane_outer_borders {
        Borders::ALL
    } else {
        Borders::NONE
    }
}

/// Where a pane that is about to be created will sit in its tab.
#[derive(Debug, Clone, Copy)]
pub(crate) enum NewPanePlacement {
    /// The only pane of a new tab or workspace.
    Alone,
    /// The pane created by splitting `target` in workspace `ws_idx`.
    Split {
        ws_idx: usize,
        target: PaneId,
        direction: Direction,
        ratio: f32,
    },
    /// A pane split off and immediately zoomed over its tab.
    ZoomedOverlay,
    /// An existing pane in workspace `ws_idx` getting a new terminal.
    Existing { ws_idx: usize, pane: PaneId },
}

/// Terminal rows and columns a new pane gets once its tab is laid out in
/// `area`, so its program starts at that size instead of being resized.
pub(crate) fn new_pane_terminal_size(
    app: &AppState,
    area: Rect,
    placement: NewPanePlacement,
) -> (u16, u16) {
    let laid_out = |panes: Vec<PaneInfo>, index: usize| {
        let infos = apply_pane_chrome(
            panes,
            app.pane_borders,
            app.pane_gaps,
            app.pane_outer_borders,
        );
        pane_inner_rect(infos[index].rect, infos[index].borders)
    };
    // A lone pane has no neighbors, so its chrome matches a zoomed single pane.
    let alone = || pane_inner_rect(area, zoomed_pane_borders(app, false));
    let tab_for = |ws_idx: usize, pane: PaneId| {
        let ws = app.workspaces.get(ws_idx)?;
        ws.tabs.get(ws.find_tab_index_for_pane(pane)?)
    };
    let pane_inner = match placement {
        NewPanePlacement::Alone => alone(),
        NewPanePlacement::Existing { ws_idx, pane } => tab_for(ws_idx, pane)
            .and_then(|tab| {
                if tab.zoomed && tab.layout.focused() == pane {
                    let multi_pane = tab.layout.pane_count() > 1;
                    return Some(pane_inner_rect(area, zoomed_pane_borders(app, multi_pane)));
                }
                let panes = tab.layout.panes(area);
                let index = panes.iter().position(|info| info.id == pane)?;
                Some(laid_out(panes, index))
            })
            .unwrap_or_else(alone),
        NewPanePlacement::ZoomedOverlay => pane_inner_rect(area, zoomed_pane_borders(app, true)),
        NewPanePlacement::Split {
            ws_idx,
            target,
            direction,
            ratio,
        } => tab_for(ws_idx, target)
            .and_then(|tab| tab.layout.panes_after_split(area, target, direction, ratio))
            .map(|(panes, new_index)| laid_out(panes, new_index))
            .unwrap_or_else(alone),
    };
    new_terminal_size(app, pane_inner)
}

/// Terminal rows and columns for every pane of `layout` laid out in `area`, in
/// pane order, so a multi-pane layout can start each program at its final size.
pub(crate) fn new_layout_terminal_sizes(
    app: &AppState,
    area: Rect,
    layout: &crate::layout::TileLayout,
) -> Vec<(u16, u16)> {
    apply_pane_chrome(
        layout.panes(area),
        app.pane_borders,
        app.pane_gaps,
        app.pane_outer_borders,
    )
    .into_iter()
    .map(|info| new_terminal_size(app, pane_inner_rect(info.rect, info.borders)))
    .collect()
}

fn new_terminal_size(app: &AppState, pane_inner: Rect) -> (u16, u16) {
    // A new program starts on the primary screen, which reserves the gutter.
    let inner = terminal_inner_rect_for(pane_inner, app.pane_scrollbars);
    (
        inner.height.max(crate::pane::MIN_PANE_ROWS),
        inner.width.max(crate::pane::MIN_PANE_COLS),
    )
}

pub(crate) fn pane_inner_rect(area: Rect, borders: Borders) -> Rect {
    if borders.is_empty() {
        area
    } else {
        Block::default().borders(borders).inner(area)
    }
}

fn ranges_overlap(a_start: u16, a_len: u16, b_start: u16, b_len: u16) -> bool {
    a_start < b_start.saturating_add(b_len) && b_start < a_start.saturating_add(a_len)
}

fn pane_to_right<'a>(info: &PaneInfo, panes: &'a [PaneInfo]) -> Option<&'a PaneInfo> {
    let right = info.rect.x.saturating_add(info.rect.width);
    panes.iter().find(|other| {
        other.id != info.id
            && other.rect.x == right
            && ranges_overlap(
                info.rect.y,
                info.rect.height,
                other.rect.y,
                other.rect.height,
            )
    })
}

fn pane_below<'a>(info: &PaneInfo, panes: &'a [PaneInfo]) -> Option<&'a PaneInfo> {
    let bottom = info.rect.y.saturating_add(info.rect.height);
    panes.iter().find(|other| {
        other.id != info.id
            && other.rect.y == bottom
            && ranges_overlap(info.rect.x, info.rect.width, other.rect.x, other.rect.width)
    })
}

fn shrink_for_one_cell_gap(size: u16) -> u16 {
    if size > 1 {
        size - 1
    } else {
        size
    }
}

pub(crate) fn apply_pane_chrome(
    panes: Vec<PaneInfo>,
    pane_borders: crate::config::PaneBordersConfig,
    pane_gaps: bool,
    pane_outer_borders: bool,
) -> Vec<PaneInfo> {
    let multi_pane = panes.len() > 1;
    let bordered = pane_borders.shows_borders(multi_pane);
    let outer_left = panes.iter().map(|info| info.rect.x).min().unwrap_or(0);
    let outer_top = panes.iter().map(|info| info.rect.y).min().unwrap_or(0);
    let outer_right = panes
        .iter()
        .map(|info| info.rect.x.saturating_add(info.rect.width))
        .max()
        .unwrap_or(0);
    let outer_bottom = panes
        .iter()
        .map(|info| info.rect.y.saturating_add(info.rect.height))
        .max()
        .unwrap_or(0);
    panes
        .iter()
        .cloned()
        .map(|mut info| {
            let right_neighbor = multi_pane.then(|| pane_to_right(&info, &panes)).flatten();
            let below_neighbor = multi_pane.then(|| pane_below(&info, &panes)).flatten();

            if multi_pane && pane_gaps && !pane_borders.draws_borders() {
                if right_neighbor.is_some() {
                    info.rect.width = shrink_for_one_cell_gap(info.rect.width);
                }
                if below_neighbor.is_some() {
                    info.rect.height = shrink_for_one_cell_gap(info.rect.height);
                }
            }

            info.borders = if !bordered {
                Borders::NONE
            } else {
                let mut borders = Borders::ALL;
                if !pane_gaps {
                    if right_neighbor.is_some() {
                        borders.remove(Borders::RIGHT);
                    }
                    if below_neighbor.is_some() {
                        borders.remove(Borders::BOTTOM);
                    }
                }
                if !pane_outer_borders {
                    if info.rect.x == outer_left {
                        borders.remove(Borders::LEFT);
                    }
                    if info.rect.y == outer_top {
                        borders.remove(Borders::TOP);
                    }
                    if info.rect.x.saturating_add(info.rect.width) == outer_right {
                        borders.remove(Borders::RIGHT);
                    }
                    if info.rect.y.saturating_add(info.rect.height) == outer_bottom {
                        borders.remove(Borders::BOTTOM);
                    }
                }
                borders
            };
            info
        })
        .collect()
}

fn runtime_for_tab_pane<'a>(
    _app: &'a AppState,
    terminal_runtimes: &'a TerminalRuntimeRegistry,
    _workspace_index: usize,
    tab: &'a crate::workspace::Tab,
    pane_id: crate::layout::PaneId,
) -> Option<(&'a crate::terminal::TerminalId, &'a TerminalRuntime)> {
    let terminal_id = tab.terminal_id(pane_id)?;
    #[cfg(test)]
    if let Some(runtime) = _app
        .workspaces
        .get(_workspace_index)?
        .test_runtimes
        .get(&pane_id)
        .or_else(|| tab.runtimes.get(&pane_id))
    {
        return Some((terminal_id, runtime));
    }
    terminal_runtimes
        .get(terminal_id)
        .map(|runtime| (terminal_id, runtime))
}

fn stable_scrollbar_gutter(
    rt: &TerminalRuntime,
    pane_inner: Rect,
    pane_scrollbars: bool,
) -> (Rect, Option<Rect>) {
    let inner_rect = terminal_inner_rect(rt, pane_inner, pane_scrollbars);
    if inner_rect == pane_inner {
        return (inner_rect, None);
    }
    let gutter = Rect::new(
        pane_inner.x + pane_inner.width.saturating_sub(1),
        pane_inner.y,
        1,
        pane_inner.height,
    );
    let scrollbar_rect = rt
        .scroll_metrics()
        .filter(|metrics| should_show_scrollbar(*metrics))
        .map(|_| gutter);

    (inner_rect, scrollbar_rect)
}

/// Resize every visible runtime in a tab to the geometry it would receive if the tab were selected.
pub(super) fn resize_tab_panes(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    workspace_index: usize,
    tab: &crate::workspace::Tab,
    area: Rect,
    cell_size: crate::kitty_graphics::HostCellSize,
) {
    let multi_pane = tab.layout.pane_count() > 1;

    if tab.zoomed {
        let focused_id = tab.layout.focused();
        if let Some((terminal_id, rt)) =
            runtime_for_tab_pane(app, terminal_runtimes, workspace_index, tab, focused_id)
        {
            let pane_inner = pane_inner_rect(area, zoomed_pane_borders(app, multi_pane));
            let inner_rect = terminal_inner_rect(rt, pane_inner, app.pane_scrollbars);
            if !app.direct_attach_resize_locks.contains(terminal_id) {
                rt.resize(
                    inner_rect.height,
                    inner_rect.width,
                    cell_size.width_px,
                    cell_size.height_px,
                );
            }
        }
        return;
    }

    for info in apply_pane_chrome(
        tab.layout.panes(area),
        app.pane_borders,
        app.pane_gaps,
        app.pane_outer_borders,
    ) {
        let pane_inner = pane_inner_rect(info.rect, info.borders);

        if let Some((terminal_id, rt)) =
            runtime_for_tab_pane(app, terminal_runtimes, workspace_index, tab, info.id)
        {
            let inner_rect = terminal_inner_rect(rt, pane_inner, app.pane_scrollbars);
            if !app.direct_attach_resize_locks.contains(terminal_id) {
                rt.resize(
                    inner_rect.height,
                    inner_rect.width,
                    cell_size.width_px,
                    cell_size.height_px,
                );
            }
        }
    }
}

/// Compute pane layout info and optionally resize pane runtimes to match.
pub(super) fn compute_pane_infos_for_tab(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    ws_idx: usize,
    tab_idx: usize,
    area: Rect,
    resize_panes: bool,
    cell_size: crate::kitty_graphics::HostCellSize,
) -> Vec<PaneInfo> {
    let Some(tab) = app
        .workspaces
        .get(ws_idx)
        .and_then(|workspace| workspace.tabs.get(tab_idx))
    else {
        return Vec::new();
    };

    let multi_pane = tab.layout.pane_count() > 1;

    if tab.zoomed {
        let focused_id = tab.layout.focused();
        let borders = zoomed_pane_borders(app, multi_pane);
        let pane_inner = pane_inner_rect(area, borders);
        let mut inner_rect = pane_inner;
        let mut scrollbar_rect = None;
        if let Some(rt) = app.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, focused_id) {
            (inner_rect, scrollbar_rect) =
                stable_scrollbar_gutter(rt, pane_inner, app.pane_scrollbars);
            if resize_panes
                && tab.terminal_id(focused_id).is_some_and(|terminal_id| {
                    !app.direct_attach_resize_locks.contains(terminal_id)
                })
            {
                rt.resize(
                    inner_rect.height,
                    inner_rect.width,
                    cell_size.width_px,
                    cell_size.height_px,
                );
            }
        }
        return vec![PaneInfo {
            id: focused_id,
            rect: area,
            inner_rect,
            scrollbar_rect,
            borders,
            is_focused: true,
        }];
    }

    let mut pane_infos = apply_pane_chrome(
        tab.layout.panes(area),
        app.pane_borders,
        app.pane_gaps,
        app.pane_outer_borders,
    );

    for info in &mut pane_infos {
        let pane_inner = pane_inner_rect(info.rect, info.borders);

        let mut inner_rect = pane_inner;
        let mut scrollbar_rect = None;
        if let Some(rt) = app.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, info.id) {
            (inner_rect, scrollbar_rect) =
                stable_scrollbar_gutter(rt, pane_inner, app.pane_scrollbars);
            if resize_panes
                && tab.terminal_id(info.id).is_some_and(|terminal_id| {
                    !app.direct_attach_resize_locks.contains(terminal_id)
                })
            {
                rt.resize(
                    inner_rect.height,
                    inner_rect.width,
                    cell_size.width_px,
                    cell_size.height_px,
                );
            }
        }

        info.inner_rect = inner_rect;
        info.scrollbar_rect = scrollbar_rect;
    }

    pane_infos
}

#[cfg(test)]
fn compute_pane_infos(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    area: Rect,
    resize_panes: bool,
    cell_size: crate::kitty_graphics::HostCellSize,
) -> Vec<PaneInfo> {
    let Some(workspace_index) = app.active else {
        return Vec::new();
    };
    let Some(tab_index) = app
        .workspaces
        .get(workspace_index)
        .map(crate::workspace::Workspace::active_tab_index)
    else {
        return Vec::new();
    };
    compute_pane_infos_for_tab(
        app,
        terminal_runtimes,
        workspace_index,
        tab_index,
        area,
        resize_panes,
        cell_size,
    )
}

pub(super) fn render_panes(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    frame: &mut Frame,
    target: Option<super::tab_surface::TabSurfaceTarget>,
    pane_infos: &[PaneInfo],
    split_borders: &[crate::layout::SplitBorder],
) {
    let Some(target) = target else {
        return;
    };
    let ws_idx = target.workspace_index;
    let Some(ws) = app.workspaces.get(ws_idx) else {
        return;
    };

    for info in pane_infos {
        if let Some(rt) = app.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, info.id) {
            let show_cursor = info.is_focused
                && !pane_is_scrolled_back(rt)
                && app.pane_exposes_host_cursor(ws_idx, info.id);
            rt.render(frame, info.inner_rect, show_cursor);
            render_pane_scrollbar(app, frame, info, rt);
        } else if let Some(reason) = ws
            .tabs
            .get(target.tab_index)
            .and_then(|tab| tab.terminal_id(info.id))
            .and_then(|id| app.terminals.get(id))
            .and_then(|terminal| terminal.restore_error.as_deref())
        {
            frame.render_widget(
                Paragraph::new(reason).wrap(Wrap { trim: false }),
                info.inner_rect,
            );
        }
    }

    render_pane_borders(app, ws, pane_infos, split_borders, frame);
}

pub(crate) fn popup_pane_rects(app: &AppState, area: Rect) -> Option<(Rect, Rect)> {
    let popup = app.popup_pane.as_ref()?;
    resolve_popup_geometry(popup.width, popup.height, area)
        .map(|geometry| (geometry.outer, geometry.inner))
}

pub(super) fn resize_popup_pane(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    area: Rect,
    cell_size: crate::kitty_graphics::HostCellSize,
) {
    let Some(popup) = app.popup_pane.as_ref() else {
        return;
    };
    let Some((_outer, inner)) = popup_pane_rects(app, area) else {
        return;
    };
    if app.direct_attach_resize_locks.contains(&popup.terminal_id) {
        return;
    }
    if let Some(rt) = terminal_runtimes.get(&popup.terminal_id) {
        rt.resize(
            inner.height,
            inner.width,
            cell_size.width_px,
            cell_size.height_px,
        );
    }
}

#[derive(Clone, Copy, Default)]
struct LineCell {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
}

fn render_pane_borders(
    app: &AppState,
    ws: &crate::workspace::Workspace,
    pane_infos: &[PaneInfo],
    split_borders: &[crate::layout::SplitBorder],
    frame: &mut Frame,
) {
    if !app.pane_borders.draws_borders() || pane_infos.iter().all(|info| info.borders.is_empty()) {
        return;
    }

    let mut cells = std::collections::HashMap::<(u16, u16), LineCell>::new();
    for info in pane_infos {
        add_pane_border_cells(&mut cells, info);
    }
    add_split_border_cells(app.pane_gaps, split_borders, &mut cells);

    let sync_tab = ws
        .active_tab()
        .filter(|tab| tab.is_syncing() && app.display_panes().is_none());
    let buf = frame.buffer_mut();
    let area = buf.area;
    for ((x, y), line) in cells {
        if x < area.x
            || x >= area.x.saturating_add(area.width)
            || y < area.y
            || y >= area.y.saturating_add(area.height)
        {
            continue;
        }
        let focused = pane_infos
            .iter()
            .any(|info| info.is_focused && line_touches_pane(x, y, info, app.pane_gaps));
        let glyph_style = if focused {
            app.pane_border_active_style
        } else {
            PaneBorderActiveStyleConfig::Light
        };
        let symbol = line_cell_symbol(line, glyph_style);
        if symbol.is_empty() {
            continue;
        }
        // Sync mode (fork issues 141, 155): a line any group member touches is
        // yellow, every other line gray. The labels view stays red over it.
        let color = match sync_tab {
            Some(tab)
                if pane_infos.iter().any(|info| {
                    tab.pane_synced(info.id) && line_touches_pane(x, y, info, app.pane_gaps)
                }) =>
            {
                crate::app::state::SYNC_YELLOW
            }
            Some(_) => app.palette.sync_outsider(),
            None => app.pane_border_color(focused),
        };
        let cell = &mut buf[(x, y)];
        cell.set_symbol(symbol);
        cell.set_style(Style::default().fg(color));
    }

    render_pane_border_titles(app, ws, pane_infos, frame);
}

fn add_split_border_cells(
    pane_gaps: bool,
    split_borders: &[crate::layout::SplitBorder],
    cells: &mut std::collections::HashMap<(u16, u16), LineCell>,
) {
    if pane_gaps {
        return;
    }

    for split in split_borders {
        match split.direction {
            ratatui::layout::Direction::Horizontal => {
                let x = split.pos;
                let end = split.area.y.saturating_add(split.area.height);
                for y in split.area.y..=end {
                    if !cells.contains_key(&(x, y)) {
                        continue;
                    }
                    let left = x
                        .checked_sub(1)
                        .and_then(|left_x| cells.get(&(left_x, y)))
                        .is_some_and(|cell| cell.left || cell.right);
                    let right = cells
                        .get(&(x.saturating_add(1), y))
                        .is_some_and(|cell| cell.left || cell.right);
                    let cell = cells.entry((x, y)).or_default();
                    cell.up |= y > split.area.y;
                    cell.down |= y + 1 < end;
                    cell.left |= left;
                    cell.right |= right;
                }
            }
            ratatui::layout::Direction::Vertical => {
                let y = split.pos;
                let end = split.area.x.saturating_add(split.area.width);
                for x in split.area.x..=end {
                    if !cells.contains_key(&(x, y)) {
                        continue;
                    }
                    let up = y
                        .checked_sub(1)
                        .and_then(|up_y| cells.get(&(x, up_y)))
                        .is_some_and(|cell| cell.up || cell.down);
                    let down = cells
                        .get(&(x, y.saturating_add(1)))
                        .is_some_and(|cell| cell.up || cell.down);
                    let cell = cells.entry((x, y)).or_default();
                    cell.left |= x > split.area.x;
                    cell.right |= x + 1 < end;
                    cell.up |= up;
                    cell.down |= down;
                }
            }
        }
    }
}

fn add_pane_border_cells(
    cells: &mut std::collections::HashMap<(u16, u16), LineCell>,
    info: &PaneInfo,
) {
    let rect = info.rect;
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let right = rect.x.saturating_add(rect.width).saturating_sub(1);
    let bottom = rect.y.saturating_add(rect.height).saturating_sub(1);

    if info.borders.contains(Borders::TOP) {
        for x in rect.x..=right {
            let cell = cells.entry((x, rect.y)).or_default();
            cell.left |= x > rect.x;
            cell.right |= x < right;
        }
    }
    if info.borders.contains(Borders::BOTTOM) {
        for x in rect.x..=right {
            let cell = cells.entry((x, bottom)).or_default();
            cell.left |= x > rect.x;
            cell.right |= x < right;
        }
    }
    if info.borders.contains(Borders::LEFT) {
        for y in rect.y..=bottom {
            let cell = cells.entry((rect.x, y)).or_default();
            cell.up |= y > rect.y;
            cell.down |= y < bottom;
        }
    }
    if info.borders.contains(Borders::RIGHT) {
        for y in rect.y..=bottom {
            let cell = cells.entry((right, y)).or_default();
            cell.up |= y > rect.y;
            cell.down |= y < bottom;
        }
    }
}

fn line_touches_pane(x: u16, y: u16, info: &PaneInfo, pane_gaps: bool) -> bool {
    let rect = info.rect;
    if rect.width == 0 || rect.height == 0 {
        return false;
    }
    let right = rect.x.saturating_add(rect.width).saturating_sub(1);
    let bottom = rect.y.saturating_add(rect.height).saturating_sub(1);
    let in_rows = y >= rect.y && y <= bottom;
    let in_cols = x >= rect.x && x <= right;
    let own_border =
        (in_rows && (x == rect.x || x == right)) || (in_cols && (y == rect.y || y == bottom));

    if pane_gaps {
        return own_border;
    }

    let shared_right = rect.x.saturating_add(rect.width);
    let shared_bottom = rect.y.saturating_add(rect.height);
    own_border
        || (in_rows && x == shared_right)
        || (in_cols && y == shared_bottom)
        || (x == shared_right && y == shared_bottom)
}

/// Where a pane's todo indicator lives and what it says. The renderer and the
/// mouse hit-test both read this one value, which is what keeps the drawn
/// glyph and the click target from drifting.
pub(crate) struct PaneTodoIndicator {
    /// Exactly the cells the label is drawn into.
    pub rect: Rect,
    pub label: String,
    pub state: PaneTodoIndicatorState,
}

/// Which of three things a pane's indicator is saying. Tone rather than shape
/// separates them, which keeps the reserved width identical in every state so
/// the title's budget does not shift as todos come and go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaneTodoIndicatorState {
    /// Outstanding todos, coloured by the highest priority among them.
    Outstanding {
        count: usize,
        priority: Option<crate::terminal::todo::TodoPriority>,
    },
    /// Todos, every one of them done.
    AllDone,
    /// No todos at all — the affordance standing on its own, so an empty pane
    /// can still be opened by mouse.
    Empty,
}

impl PaneTodoIndicatorState {
    fn of(
        total: usize,
        outstanding: usize,
        priority: Option<crate::terminal::todo::TodoPriority>,
    ) -> Self {
        if outstanding > 0 {
            Self::Outstanding {
                count: outstanding,
                priority,
            }
        } else if total > 0 {
            Self::AllDone
        } else {
            Self::Empty
        }
    }

    /// Finished work stays legible; an empty affordance recedes further still,
    /// so the two never read as the same thing.
    fn color(self, app: &AppState) -> ratatui::style::Color {
        match self {
            Self::Outstanding { priority, .. } => app.pane_todo_indicator_color(priority),
            Self::AllDone => app.palette.overlay1,
            Self::Empty => app.palette.overlay0,
        }
    }
}

/// `▾ τ N` for N outstanding todos and a bare `▾` otherwise: `▾` is the
/// handle that marks the place on the pane, `τ` names the count the way the
/// tab-bar corner does. Every pane gets one so the control sits in the same
/// place on all of them; the empty and all-done states are told apart by
/// [`PaneTodoIndicatorState::color`], not by width. Same spacing grammar as
/// the notification `и`. The count is at most two digits because `add_todo`
/// caps a pane at `MAX_TODOS_PER_PANE` (50).
fn pane_todo_indicator_label(state: PaneTodoIndicatorState) -> String {
    match state {
        PaneTodoIndicatorState::Outstanding { count, .. } => format!(" ▾ τ {count} "),
        PaneTodoIndicatorState::AllDone | PaneTodoIndicatorState::Empty => " ▾ ".to_string(),
    }
}

/// The entry point for a caller holding only a pane id — the mouse hit-test,
/// which must read the same cells the renderer drew.
pub(crate) fn pane_todo_indicator(app: &AppState, info: &PaneInfo) -> Option<PaneTodoIndicator> {
    pane_todo_indicator_for(app, info, app.pane_terminal(info.id)?)
}

/// [`pane_todo_indicator`] for a caller that already holds the pane's
/// terminal. The render path resolves it from the workspace it was handed, so
/// it must not pay `AppState::pane_terminal`'s scan of every workspace once
/// per pane per frame.
fn pane_todo_indicator_for(
    app: &AppState,
    info: &PaneInfo,
    terminal: &crate::terminal::TerminalState,
) -> Option<PaneTodoIndicator> {
    // No top border means no place to put it: a single-pane tab or
    // `ui.pane_borders = false` draws no chrome at all, and the keybinding is
    // the discoverable path there.
    if !app.show_pane_todo_indicator || !info.borders.contains(Borders::TOP) {
        return None;
    }
    let state = PaneTodoIndicatorState::of(
        terminal.todos().len(),
        terminal.outstanding_todo_count(),
        terminal.highest_outstanding_todo_priority(),
    );
    let label = pane_todo_indicator_label(state);
    let width = display_width_u16(&label);
    // The label plus both corner glyphs is the whole requirement: the title is
    // laid out in what is left over and drops itself when that is too narrow,
    // so the control is what survives a squeeze.
    if width == 0 || info.rect.width < width.saturating_add(2) {
        return None;
    }
    let x = info
        .rect
        .x
        .saturating_add(info.rect.width)
        .saturating_sub(1)
        .saturating_sub(width);
    Some(PaneTodoIndicator {
        rect: Rect::new(x, info.rect.y, width, 1),
        label,
        state,
    })
}

fn render_pane_border_titles(
    app: &AppState,
    ws: &crate::workspace::Workspace,
    pane_infos: &[PaneInfo],
    frame: &mut Frame,
) {
    let buf = frame.buffer_mut();
    let area = buf.area;
    for info in pane_infos {
        if !info.borders.contains(Borders::TOP) || info.rect.width <= 4 {
            continue;
        }
        let y = info.rect.y;
        if y < area.y || y >= area.y.saturating_add(area.height) {
            continue;
        }

        // One lookup feeds both the indicator and the title.
        let terminal = ws
            .pane_state(info.id)
            .and_then(|pane| app.terminals.get(&pane.attached_terminal_id));

        // The indicator claims the far right of the border before the title is
        // laid out, so a narrow pane truncates the title instead of dropping
        // the control.
        let indicator = terminal.and_then(|terminal| pane_todo_indicator_for(app, info, terminal));
        let reserved = indicator
            .as_ref()
            .map(|indicator| indicator.rect.width)
            .unwrap_or(0);

        if let Some(title) = terminal
            .and_then(|terminal| terminal.border_label(app.show_agent_labels_on_pane_borders))
            .and_then(|label| {
                pane_border_title(
                    &label,
                    info.rect.width.saturating_sub(reserved),
                    info.is_focused,
                )
            })
        {
            let start_x = info.rect.x.saturating_add(1);
            let end_x = info
                .rect
                .x
                .saturating_add(info.rect.width)
                .saturating_sub(1)
                .saturating_sub(reserved)
                .min(area.x.saturating_add(area.width));
            if start_x < end_x {
                let mut style = Style::default().fg(app.pane_title_color(info.is_focused));
                if info.is_focused {
                    style = style.add_modifier(Modifier::BOLD);
                }
                buf.set_stringn(
                    start_x,
                    y,
                    title,
                    end_x.saturating_sub(start_x) as usize,
                    style,
                );
            }
        }

        if let Some(indicator) = indicator {
            let style = Style::default().fg(indicator.state.color(app));
            buf.set_stringn(
                indicator.rect.x,
                indicator.rect.y,
                &indicator.label,
                indicator.rect.width as usize,
                style,
            );
            // With work outstanding the count is what matters: `τ N` goes
            // bold, the `▾` handle in front of it stays plain. The handle
            // and its padding are the first three cells of the label.
            if matches!(indicator.state, PaneTodoIndicatorState::Outstanding { .. }) {
                let count_x = indicator.rect.x.saturating_add(3);
                let count_end = indicator.rect.x.saturating_add(indicator.rect.width);
                for x in count_x..count_end {
                    buf[(x, indicator.rect.y)].set_style(style.add_modifier(Modifier::BOLD));
                }
            }
        }
    }
}

fn line_cell_symbol(line: LineCell, style: PaneBorderActiveStyleConfig) -> &'static str {
    // cross, tee-left, tee-right, tee-up, tee-down, vertical, horizontal,
    // corner-tl, corner-tr, corner-bl, corner-br
    const LIGHT: [&str; 11] = ["┼", "┤", "├", "┴", "┬", "│", "─", "┌", "┐", "└", "┘"];
    const HEAVY: [&str; 11] = ["╋", "┫", "┣", "┻", "┳", "┃", "━", "┏", "┓", "┗", "┛"];
    const DOUBLE: [&str; 11] = ["╬", "╣", "╠", "╩", "╦", "║", "═", "╔", "╗", "╚", "╝"];

    let index = match (line.up, line.down, line.left, line.right) {
        (true, true, true, true) => 0,
        (true, true, true, false) => 1,
        (true, true, false, true) => 2,
        (true, false, true, true) => 3,
        (false, true, true, true) => 4,
        (true, true, false, false) | (true, false, false, false) | (false, true, false, false) => 5,
        (false, false, true, true) | (false, false, true, false) | (false, false, false, true) => 6,
        (false, true, false, true) => 7,
        (false, true, true, false) => 8,
        (true, false, false, true) => 9,
        (true, false, true, false) => 10,
        _ => return "",
    };
    match style {
        PaneBorderActiveStyleConfig::Light => LIGHT[index],
        PaneBorderActiveStyleConfig::Heavy => HEAVY[index],
        PaneBorderActiveStyleConfig::Double => DOUBLE[index],
    }
}

pub(crate) fn render_selection_highlight<P: PartialEq>(
    selection: Option<&crate::selection::Selection<P>>,
    buffer: &mut Buffer,
    pane_id: &P,
    inner: Rect,
    scroll_metrics: Option<crate::pane::ScrollMetrics>,
    p: &Palette,
    host_theme: crate::terminal_theme::TerminalTheme,
) {
    let Some(selection) =
        selection.filter(|selection| selection.is_visible() && &selection.pane_id == pane_id)
    else {
        return;
    };
    let style = automatic_selection_style(p, host_theme);
    for y in 0..inner.height {
        for x in 0..inner.width {
            if selection.contains(y, x, scroll_metrics) {
                buffer[(inner.x + x, inner.y + y)].set_style(style);
            }
        }
    }
}

type Rgb = (u8, u8, u8);

fn automatic_selection_style(
    p: &Palette,
    host_theme: crate::terminal_theme::TerminalTheme,
) -> Style {
    let bg = automatic_selection_bg(p, host_theme);
    Style::reset().fg(selection_fg_for_bg(bg, p)).bg(bg)
}

fn automatic_selection_bg(p: &Palette, host_theme: crate::terminal_theme::TerminalTheme) -> Color {
    let fallback = selection_palette_background(p);
    let Some(background) = host_theme
        .background
        .map(|color| (color.r, color.g, color.b))
        .or(match fallback {
            Color::Rgb(r, g, b) => Some((r, g, b)),
            _ => None,
        })
    else {
        return fallback;
    };

    let target = if relative_luminance(background) < 0.5 {
        (255, 255, 255)
    } else {
        (0, 0, 0)
    };
    let selected = mix_rgb(background, target, 0.28);
    Color::Rgb(selected.0, selected.1, selected.2)
}

fn selection_palette_background(p: &Palette) -> Color {
    if p.panel_bg == Color::Reset {
        p.surface_dim
    } else {
        p.panel_bg
    }
}

fn selection_fg_for_bg(bg: Color, p: &Palette) -> Color {
    if let Color::Rgb(r, g, b) = bg {
        let luminance = relative_luminance((r, g, b));
        let black_contrast = (luminance + 0.05) / 0.05;
        let white_contrast = 1.05 / (luminance + 0.05);
        return if black_contrast > white_contrast {
            Color::Rgb(0, 0, 0)
        } else {
            Color::Rgb(255, 255, 255)
        };
    }

    color_to_rgb(bg)
        .map(|bg| {
            if relative_luminance(bg) < 0.5 {
                Color::White
            } else {
                Color::Black
            }
        })
        .unwrap_or_else(|| panel_contrast_fg(p))
}

fn mix_rgb(base: Rgb, target: Rgb, amount: f32) -> Rgb {
    fn channel(base: u8, target: u8, amount: f32) -> u8 {
        (f32::from(base) + (f32::from(target) - f32::from(base)) * amount).round() as u8
    }
    (
        channel(base.0, target.0, amount),
        channel(base.1, target.1, amount),
        channel(base.2, target.2, amount),
    )
}

fn relative_luminance(color: Rgb) -> f32 {
    fn channel(value: u8) -> f32 {
        let value = f32::from(value) / 255.0;
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(color.0) + 0.7152 * channel(color.1) + 0.0722 * channel(color.2)
}

fn color_to_rgb(color: Color) -> Option<Rgb> {
    match color {
        Color::Reset => None,
        Color::Black => Some((0, 0, 0)),
        Color::Red => Some((128, 0, 0)),
        Color::Green => Some((0, 128, 0)),
        Color::Yellow => Some((128, 128, 0)),
        Color::Blue => Some((0, 0, 128)),
        Color::Magenta => Some((128, 0, 128)),
        Color::Cyan => Some((0, 128, 128)),
        Color::Gray => Some((192, 192, 192)),
        Color::DarkGray => Some((128, 128, 128)),
        Color::LightRed => Some((255, 0, 0)),
        Color::LightGreen => Some((0, 255, 0)),
        Color::LightYellow => Some((255, 255, 0)),
        Color::LightBlue => Some((0, 0, 255)),
        Color::LightMagenta => Some((255, 0, 255)),
        Color::LightCyan => Some((0, 255, 255)),
        Color::White => Some((255, 255, 255)),
        Color::Rgb(r, g, b) => Some((r, g, b)),
        Color::Indexed(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PaneBordersConfig;
    use crate::layout::PaneId;
    use crate::selection::Selection;
    use crate::terminal::todo::{TodoPriority, TodoUpdate};
    use crate::terminal::TerminalRuntime;
    use crate::terminal::TerminalState;
    use crate::workspace::Workspace;

    fn render_view_pane_borders(
        app: &AppState,
        ws: &Workspace,
        split_borders: &[crate::layout::SplitBorder],
        frame: &mut Frame,
    ) {
        render_pane_borders(app, ws, &app.view.pane_infos, split_borders, frame);
    }

    #[test]
    fn unavailable_pane_renders_restore_failure_without_a_runtime() {
        let mut app = AppState::test_new();
        app.workspaces = vec![Workspace::test_new("unavailable")];
        app.active = Some(0);
        app.ensure_test_terminals();
        let pane_id = app.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.workspaces[0].terminal_id(pane_id).unwrap().clone();
        app.terminals.get_mut(&terminal_id).unwrap().restore_error =
            Some("Saved directory is unavailable. Restart to retry.".into());
        let runtimes = TerminalRuntimeRegistry::new();
        let area = Rect::new(0, 0, 80, 24);
        let layout = crate::ui::compute_tab_surface_for(
            &app,
            &runtimes,
            Some(crate::ui::TabSurfaceTarget {
                workspace_index: 0,
                tab_index: 0,
            }),
            area,
            false,
            Default::default(),
        );
        let (buffer, cursor, _, _) =
            crate::server::render_stream::render_tab_surface_virtual(&app, &runtimes, layout, area);
        let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert!(text.contains("Saved directory is unavailable."));
        assert!(cursor.is_none_or(|cursor| !cursor.visible));
    }

    #[test]
    fn pane_border_title_trims_and_truncates() {
        assert_eq!(
            pane_border_title(" claude ", 20, false).as_deref(),
            Some(" claude ")
        );
        assert_eq!(
            pane_border_title(" claude ", 20, true).as_deref(),
            Some(" claude ")
        );
        assert_eq!(pane_border_title("", 20, false), None);
        assert_eq!(
            pane_border_title("abcdef", 8, false).as_deref(),
            Some(" abc… ")
        );
        assert_eq!(
            pane_border_title("abcdef", 8, true).as_deref(),
            Some(" abc… ")
        );
        assert_eq!(pane_border_title("abcdef", 4, false), None);
    }

    #[test]
    fn line_cell_symbol_maps_every_shape_per_style() {
        let shapes: [(LineCell, [&str; 3]); 11] = [
            (cell(true, true, true, true), ["┼", "╋", "╬"]),
            (cell(true, true, true, false), ["┤", "┫", "╣"]),
            (cell(true, true, false, true), ["├", "┣", "╠"]),
            (cell(true, false, true, true), ["┴", "┻", "╩"]),
            (cell(false, true, true, true), ["┬", "┳", "╦"]),
            (cell(true, true, false, false), ["│", "┃", "║"]),
            (cell(false, false, true, true), ["─", "━", "═"]),
            (cell(false, true, false, true), ["┌", "┏", "╔"]),
            (cell(false, true, true, false), ["┐", "┓", "╗"]),
            (cell(true, false, false, true), ["└", "┗", "╚"]),
            (cell(true, false, true, false), ["┘", "┛", "╝"]),
        ];
        for (line, [light, heavy, double]) in shapes {
            assert_eq!(
                line_cell_symbol(line, PaneBorderActiveStyleConfig::Light),
                light
            );
            assert_eq!(
                line_cell_symbol(line, PaneBorderActiveStyleConfig::Heavy),
                heavy
            );
            assert_eq!(
                line_cell_symbol(line, PaneBorderActiveStyleConfig::Double),
                double
            );
        }
        // dangling stubs render as plain lines; empty cells render nothing
        assert_eq!(
            line_cell_symbol(
                cell(true, false, false, false),
                PaneBorderActiveStyleConfig::Heavy
            ),
            "┃"
        );
        for style in [
            PaneBorderActiveStyleConfig::Light,
            PaneBorderActiveStyleConfig::Heavy,
            PaneBorderActiveStyleConfig::Double,
        ] {
            assert_eq!(
                line_cell_symbol(cell(false, false, false, false), style),
                ""
            );
        }
    }

    fn cell(up: bool, down: bool, left: bool, right: bool) -> LineCell {
        LineCell {
            up,
            down,
            left,
            right,
        }
    }

    #[test]
    fn pane_border_and_title_colors_resolve_with_fallbacks() {
        let mut app = AppState::test_new();

        // all unset: theme defaults, title follows border
        assert_eq!(app.pane_border_color(true), app.palette.accent);
        assert_eq!(app.pane_border_color(false), app.palette.overlay0);
        assert_eq!(app.pane_title_color(true), app.palette.accent);
        assert_eq!(app.pane_title_color(false), app.palette.overlay0);

        // border colors set: titles follow them
        app.pane_border_active_color = Some(Color::Rgb(215, 135, 0));
        app.pane_border_inactive_color = Some(Color::Rgb(74, 74, 74));
        assert_eq!(app.pane_border_color(true), Color::Rgb(215, 135, 0));
        assert_eq!(app.pane_border_color(false), Color::Rgb(74, 74, 74));
        assert_eq!(app.pane_title_color(true), Color::Rgb(215, 135, 0));
        assert_eq!(app.pane_title_color(false), Color::Rgb(74, 74, 74));

        // explicit title colors decouple from the border
        app.pane_title_active_color = Some(Color::Rgb(255, 215, 0));
        app.pane_title_inactive_color = Some(Color::Rgb(122, 122, 122));
        assert_eq!(app.pane_title_color(true), Color::Rgb(255, 215, 0));
        assert_eq!(app.pane_title_color(false), Color::Rgb(122, 122, 122));
        assert_eq!(app.pane_border_color(true), Color::Rgb(215, 135, 0));
    }

    #[test]
    fn pane_border_title_truncates_cjk_by_display_width() {
        let title = pane_border_title("1 模块组织（已定）", 12, false).unwrap();

        assert_eq!(title, " 1 模块… ");
        assert!(display_width(title.as_str()) <= 10);
    }

    #[test]
    fn pane_border_renderer_places_adjacent_cjk_by_display_width() {
        let mut app = AppState::test_new();
        // 15 rather than 12: the always-on todo indicator reserves three
        // columns of every top border, and this test is about where wide
        // glyphs land, not about how much title survives. Widening by exactly
        // the reserved width leaves the title the same 12 columns it was
        // written against, so the expected cells stay put.
        app.view.terminal_area = Rect::new(0, 0, 15, 3);
        let ws = Workspace::test_new("test");
        let pane_id = ws.tabs[0].root_pane;
        app.view.pane_infos = vec![PaneInfo {
            id: pane_id,
            rect: Rect::new(0, 0, 15, 3),
            inner_rect: Rect::default(),
            scrollbar_rect: None,
            borders: Borders::ALL,
            is_focused: false,
        }];

        let terminal_id = ws.tabs[0].panes[&pane_id].attached_terminal_id.clone();
        let mut terminal_state = TerminalState::new(terminal_id.clone(), "/tmp".into());
        terminal_state.set_manual_label("1 模块组织（已定）".into());
        app.terminals.insert(terminal_id, terminal_state);

        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(15, 3)).unwrap();
        terminal
            .draw(|frame| render_view_pane_borders(&app, &ws, &[], frame))
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(4, 0)].symbol(), "模");
        assert_eq!(buffer[(5, 0)].symbol(), " ");
        assert_eq!(buffer[(6, 0)].symbol(), "块");
    }

    #[test]
    fn default_horizontal_split_uses_one_shared_divider_column() {
        let mut workspace = Workspace::test_new("test");
        let root = workspace.tabs[0].root_pane;
        let right = workspace.test_split(ratatui::layout::Direction::Horizontal);
        workspace.tabs[0].layout.focus_pane(root);

        let infos = apply_pane_chrome(
            workspace.tabs[0].layout.panes(Rect::new(0, 0, 100, 20)),
            PaneBordersConfig::Auto,
            false,
            true,
        );
        let left = infos.iter().find(|info| info.id == root).unwrap();
        let right = infos.iter().find(|info| info.id == right).unwrap();

        assert_eq!(left.rect.x + left.rect.width, right.rect.x);
        assert!(!left.borders.contains(Borders::RIGHT));
        assert!(right.borders.contains(Borders::LEFT));
    }

    #[test]
    fn default_vertical_split_uses_one_shared_divider_row() {
        let mut workspace = Workspace::test_new("test");
        let root = workspace.tabs[0].root_pane;
        let bottom = workspace.test_split(ratatui::layout::Direction::Vertical);
        workspace.tabs[0].layout.focus_pane(root);

        let infos = apply_pane_chrome(
            workspace.tabs[0].layout.panes(Rect::new(0, 0, 100, 20)),
            PaneBordersConfig::Auto,
            false,
            true,
        );
        let top = infos.iter().find(|info| info.id == root).unwrap();
        let bottom = infos.iter().find(|info| info.id == bottom).unwrap();

        assert_eq!(top.rect.y + top.rect.height, bottom.rect.y);
        assert!(!top.borders.contains(Borders::BOTTOM));
        assert!(bottom.borders.contains(Borders::TOP));
    }

    #[test]
    fn disabled_outer_borders_keep_only_shared_pane_dividers() {
        let mut workspace = Workspace::test_new("test");
        let root = workspace.tabs[0].root_pane;
        let right = workspace.test_split(ratatui::layout::Direction::Horizontal);
        workspace.tabs[0].layout.focus_pane(root);

        let infos = apply_pane_chrome(
            workspace.tabs[0].layout.panes(Rect::new(0, 0, 100, 20)),
            PaneBordersConfig::Auto,
            false,
            false,
        );
        let left = infos.iter().find(|info| info.id == root).unwrap();
        let right = infos.iter().find(|info| info.id == right).unwrap();

        assert_eq!(left.borders, Borders::NONE);
        assert_eq!(right.borders, Borders::LEFT);
    }

    #[test]
    fn pane_gaps_keep_independent_bordered_panes() {
        let mut workspace = Workspace::test_new("test");
        let root = workspace.tabs[0].root_pane;
        let right = workspace.test_split(ratatui::layout::Direction::Horizontal);
        workspace.tabs[0].layout.focus_pane(root);

        let infos = apply_pane_chrome(
            workspace.tabs[0].layout.panes(Rect::new(0, 0, 100, 20)),
            PaneBordersConfig::Auto,
            true,
            true,
        );
        let left = infos.iter().find(|info| info.id == root).unwrap();
        let right = infos.iter().find(|info| info.id == right).unwrap();

        assert_eq!(left.rect.x + left.rect.width, right.rect.x);
        assert_eq!(left.borders, Borders::ALL);
        assert_eq!(right.borders, Borders::ALL);
    }

    #[test]
    fn borderless_pane_gaps_add_one_empty_cell_between_panes() {
        let mut workspace = Workspace::test_new("test");
        let root = workspace.tabs[0].root_pane;
        let right = workspace.test_split(ratatui::layout::Direction::Horizontal);
        workspace.tabs[0].layout.focus_pane(root);

        let infos = apply_pane_chrome(
            workspace.tabs[0].layout.panes(Rect::new(0, 0, 100, 20)),
            PaneBordersConfig::Off,
            true,
            true,
        );
        let left = infos.iter().find(|info| info.id == root).unwrap();
        let right = infos.iter().find(|info| info.id == right).unwrap();

        assert_eq!(left.rect, Rect::new(0, 0, 49, 20));
        assert_eq!(right.rect, Rect::new(50, 0, 50, 20));
        assert!(left.borders.is_empty());
        assert!(right.borders.is_empty());
    }

    #[test]
    fn disabled_pane_borders_make_inner_rect_equal_visual_rect() {
        let mut workspace = Workspace::test_new("test");
        workspace.test_split(ratatui::layout::Direction::Horizontal);

        let infos = apply_pane_chrome(
            workspace.tabs[0].layout.panes(Rect::new(0, 0, 100, 20)),
            PaneBordersConfig::Off,
            false,
            true,
        );

        for info in infos {
            assert!(info.borders.is_empty());
            assert_eq!(pane_inner_rect(info.rect, info.borders), info.rect);
        }
    }

    #[test]
    fn always_pane_borders_frame_lone_pane() {
        let workspace = Workspace::test_new("test");
        let area = Rect::new(0, 0, 100, 20);

        let default_infos = apply_pane_chrome(
            workspace.tabs[0].layout.panes(area),
            PaneBordersConfig::Auto,
            false,
            true,
        );
        assert_eq!(default_infos[0].borders, Borders::NONE);

        let framed_infos = apply_pane_chrome(
            workspace.tabs[0].layout.panes(area),
            PaneBordersConfig::Always,
            false,
            true,
        );
        assert_eq!(framed_infos[0].borders, Borders::ALL);

        let no_outer_infos = apply_pane_chrome(
            workspace.tabs[0].layout.panes(area),
            PaneBordersConfig::Always,
            false,
            false,
        );
        assert_eq!(no_outer_infos[0].borders, Borders::NONE);
    }

    #[test]
    fn global_pane_border_renderer_composes_junctions_and_focus_style() {
        let mut app = AppState::test_new();
        app.view.terminal_area = Rect::new(0, 0, 4, 4);
        app.view.pane_infos = vec![
            PaneInfo {
                id: PaneId::from_raw(1),
                rect: Rect::new(0, 0, 2, 2),
                inner_rect: Rect::default(),
                scrollbar_rect: None,
                borders: Borders::TOP | Borders::LEFT,
                is_focused: true,
            },
            PaneInfo {
                id: PaneId::from_raw(2),
                rect: Rect::new(2, 0, 2, 2),
                inner_rect: Rect::default(),
                scrollbar_rect: None,
                borders: Borders::TOP | Borders::LEFT | Borders::RIGHT,
                is_focused: false,
            },
            PaneInfo {
                id: PaneId::from_raw(3),
                rect: Rect::new(0, 2, 2, 2),
                inner_rect: Rect::default(),
                scrollbar_rect: None,
                borders: Borders::TOP | Borders::LEFT | Borders::BOTTOM,
                is_focused: false,
            },
            PaneInfo {
                id: PaneId::from_raw(4),
                rect: Rect::new(2, 2, 2, 2),
                inner_rect: Rect::default(),
                scrollbar_rect: None,
                borders: Borders::ALL,
                is_focused: false,
            },
        ];
        let split_borders = vec![
            crate::layout::SplitBorder {
                pos: 2,
                direction: ratatui::layout::Direction::Horizontal,
                ratio: 0.5,
                area: Rect::new(0, 0, 4, 4),
                path: vec![],
            },
            crate::layout::SplitBorder {
                pos: 2,
                direction: ratatui::layout::Direction::Vertical,
                ratio: 0.5,
                area: Rect::new(0, 0, 4, 4),
                path: vec![false],
            },
        ];
        let ws = Workspace::test_new("test");
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(4, 4)).unwrap();

        terminal
            .draw(|frame| render_view_pane_borders(&app, &ws, &split_borders, frame))
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(2, 2)].symbol(), "┼");
        assert_eq!(buffer[(2, 2)].style().fg, Some(app.palette.accent));
        assert_eq!(buffer[(2, 1)].symbol(), "│");
        assert_eq!(buffer[(2, 1)].style().fg, Some(app.palette.accent));
    }

    #[test]
    fn gapped_pane_focus_does_not_color_neighbor_border() {
        let mut app = AppState::test_new();
        app.pane_gaps = true;
        app.view.terminal_area = Rect::new(0, 0, 4, 3);
        app.view.pane_infos = vec![
            PaneInfo {
                id: PaneId::from_raw(1),
                rect: Rect::new(0, 0, 2, 3),
                inner_rect: Rect::default(),
                scrollbar_rect: None,
                borders: Borders::ALL,
                is_focused: true,
            },
            PaneInfo {
                id: PaneId::from_raw(2),
                rect: Rect::new(2, 0, 2, 3),
                inner_rect: Rect::default(),
                scrollbar_rect: None,
                borders: Borders::ALL,
                is_focused: false,
            },
        ];
        let ws = Workspace::test_new("test");
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(4, 3)).unwrap();

        terminal
            .draw(|frame| render_view_pane_borders(&app, &ws, &[], frame))
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(1, 1)].style().fg, Some(app.palette.accent));
        assert_eq!(buffer[(2, 1)].style().fg, Some(app.palette.overlay0));
    }

    #[tokio::test]
    async fn pane_scrollbar_gutter_is_reserved_before_scrollback_exists() {
        let mut app = AppState::test_new();
        let mut workspace = Workspace::test_new("test");
        let root_pane = workspace.tabs[0].root_pane;
        workspace.tabs[0].runtimes.insert(
            root_pane,
            TerminalRuntime::test_with_scrollback_bytes(40, 8, 1024, b"ready\n"),
        );
        app.workspaces = vec![workspace];
        app.active = Some(0);

        let area = Rect::new(10, 3, 40, 8);
        let terminal_runtimes = TerminalRuntimeRegistry::new();
        let infos = compute_pane_infos(
            &app,
            &terminal_runtimes,
            area,
            false,
            crate::kitty_graphics::HostCellSize::default(),
        );
        let info = &infos[0];

        assert_eq!(info.rect, area);
        assert_eq!(info.scrollbar_rect, None);
        assert_eq!(info.inner_rect, Rect::new(10, 3, 39, 8));
    }

    #[tokio::test]
    async fn alternate_screen_reclaims_scrollbar_gutter_and_restores_it_on_exit() {
        let mut app = AppState::test_new();
        let mut workspace = Workspace::test_new("test");
        let root_pane = workspace.tabs[0].root_pane;
        workspace.tabs[0].runtimes.insert(
            root_pane,
            TerminalRuntime::test_with_scrollback_bytes(
                40,
                8,
                1024,
                b"one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n",
            ),
        );
        app.workspaces = vec![workspace];
        app.active = Some(0);

        let area = Rect::new(10, 3, 40, 8);
        let terminal_runtimes = TerminalRuntimeRegistry::new();
        let assert_geometry = |expected_width, has_scrollbar| {
            let infos = compute_pane_infos(
                &app,
                &terminal_runtimes,
                area,
                true,
                crate::kitty_graphics::HostCellSize::default(),
            );
            assert_eq!(
                infos[0].inner_rect,
                Rect::new(area.x, area.y, expected_width, area.height)
            );
            assert_eq!(infos[0].scrollbar_rect.is_some(), has_scrollbar);
            assert_eq!(
                app.workspaces[0].tabs[0].runtimes[&root_pane].current_size(),
                (area.height, expected_width)
            );
        };

        assert_geometry(39, true);
        app.workspaces[0].tabs[0].runtimes[&root_pane].test_process_pty_bytes(b"\x1b[?1049h");
        assert_geometry(40, false);
        app.workspaces[0].tabs[0].runtimes[&root_pane].test_process_pty_bytes(b"\x1b[?1049l");
        assert_geometry(39, true);
    }

    #[tokio::test]
    async fn zoomed_pane_scrollbar_gutter_is_reserved_before_scrollback_exists() {
        let mut app = AppState::test_new();
        let mut workspace = Workspace::test_new("test");
        workspace.zoomed = true;
        let root_pane = workspace.tabs[0].root_pane;
        workspace.tabs[0].runtimes.insert(
            root_pane,
            TerminalRuntime::test_with_scrollback_bytes(40, 8, 1024, b"ready\n"),
        );
        app.workspaces = vec![workspace];
        app.active = Some(0);

        let area = Rect::new(10, 3, 40, 8);
        let terminal_runtimes = TerminalRuntimeRegistry::new();
        let infos = compute_pane_infos(
            &app,
            &terminal_runtimes,
            area,
            false,
            crate::kitty_graphics::HostCellSize::default(),
        );
        let info = &infos[0];

        assert_eq!(info.rect, area);
        assert_eq!(info.scrollbar_rect, None);
        assert_eq!(info.inner_rect, Rect::new(10, 3, 39, 8));
    }

    #[tokio::test]
    async fn zoomed_multi_pane_keeps_border_space() {
        let mut app = AppState::test_new();
        let mut workspace = Workspace::test_new("test");
        let focused_pane = workspace.test_split(ratatui::layout::Direction::Horizontal);
        workspace.zoomed = true;
        workspace.tabs[0].runtimes.insert(
            focused_pane,
            TerminalRuntime::test_with_scrollback_bytes(40, 8, 1024, b"ready\n"),
        );
        app.workspaces = vec![workspace];
        app.active = Some(0);

        let area = Rect::new(10, 3, 40, 8);
        let terminal_runtimes = TerminalRuntimeRegistry::new();
        let infos = compute_pane_infos(
            &app,
            &terminal_runtimes,
            area,
            false,
            crate::kitty_graphics::HostCellSize::default(),
        );
        let info = &infos[0];

        assert_eq!(info.id, focused_pane);
        assert_eq!(info.rect, area);
        assert_eq!(info.scrollbar_rect, None);
        assert_eq!(info.inner_rect, Rect::new(11, 4, 37, 6));
    }

    #[tokio::test]
    async fn tiny_pane_does_not_reserve_scrollbar_gutter() {
        let mut app = AppState::test_new();
        let mut workspace = Workspace::test_new("test");
        let root_pane = workspace.tabs[0].root_pane;
        workspace.tabs[0].runtimes.insert(
            root_pane,
            TerminalRuntime::test_with_scrollback_bytes(4, 8, 1024, b"ready\n"),
        );
        app.workspaces = vec![workspace];
        app.active = Some(0);

        let area = Rect::new(10, 3, 4, 8);
        let terminal_runtimes = TerminalRuntimeRegistry::new();
        let infos = compute_pane_infos(
            &app,
            &terminal_runtimes,
            area,
            false,
            crate::kitty_graphics::HostCellSize::default(),
        );
        let info = &infos[0];

        assert_eq!(info.rect, area);
        assert_eq!(info.scrollbar_rect, None);
        assert_eq!(info.inner_rect, area);
    }

    #[tokio::test]
    async fn pane_scrollbar_setting_controls_reserved_column() {
        let mut app = AppState::test_new();
        let mut workspace = Workspace::test_new("test");
        let root_pane = workspace.tabs[0].root_pane;
        workspace.tabs[0].runtimes.insert(
            root_pane,
            TerminalRuntime::test_with_scrollback_bytes(
                40,
                8,
                1024,
                b"one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n",
            ),
        );
        app.workspaces = vec![workspace];
        app.active = Some(0);

        let area = Rect::new(10, 3, 40, 8);
        let terminal_runtimes = TerminalRuntimeRegistry::new();
        let infos = compute_pane_infos(
            &app,
            &terminal_runtimes,
            area,
            false,
            crate::kitty_graphics::HostCellSize::default(),
        );
        let info = &infos[0];

        assert_eq!(info.rect, area);
        assert_eq!(info.scrollbar_rect, Some(Rect::new(49, 3, 1, 8)));
        assert_eq!(info.inner_rect, Rect::new(10, 3, 39, 8));

        app.pane_scrollbars = false;
        let infos = compute_pane_infos(
            &app,
            &terminal_runtimes,
            area,
            false,
            crate::kitty_graphics::HostCellSize::default(),
        );
        let info = &infos[0];

        assert_eq!(info.rect, area);
        assert_eq!(info.scrollbar_rect, None);
        assert_eq!(info.inner_rect, area);
    }

    #[test]
    fn selection_highlight_uses_one_uniform_style() {
        let palette = Palette::catppuccin();
        let host_theme = crate::terminal_theme::TerminalTheme {
            foreground: None,
            background: Some(crate::terminal_theme::RgbColor {
                r: 12,
                g: 14,
                b: 16,
            }),
            ..Default::default()
        };
        let expected_style = automatic_selection_style(&palette, host_theme);
        let selection = Some(Selection::absolute_range(
            PaneId::from_raw(1),
            (0, 0),
            (0, 2),
        ));
        let backend = ratatui::backend::TestBackend::new(4, 1);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                let buf = frame.buffer_mut();
                buf[(0, 0)].set_style(
                    Style::default()
                        .fg(Color::Rgb(10, 220, 120))
                        .bg(Color::Black),
                );
                buf[(1, 0)].set_style(
                    Style::default()
                        .fg(Color::Rgb(220, 180, 40))
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                );
                buf[(2, 0)].set_style(Style::default().fg(Color::Blue).bg(Color::Reset));
                render_selection_highlight(
                    selection.as_ref(),
                    frame.buffer_mut(),
                    &PaneId::from_raw(1),
                    Rect::new(0, 0, 4, 1),
                    None,
                    &palette,
                    host_theme,
                );
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let first = buffer[(0, 0)].style();
        let second = buffer[(1, 0)].style();
        let third = buffer[(2, 0)].style();

        assert_eq!(first.fg, expected_style.fg);
        assert_eq!(second.fg, expected_style.fg);
        assert_eq!(third.fg, expected_style.fg);
        assert_eq!(first.bg, expected_style.bg);
        assert_eq!(second.bg, expected_style.bg);
        assert_eq!(third.bg, expected_style.bg);
        assert_eq!(first.add_modifier, expected_style.add_modifier);
        assert_eq!(second.add_modifier, expected_style.add_modifier);
        assert_eq!(third.add_modifier, expected_style.add_modifier);
        assert!(!second.add_modifier.contains(Modifier::BOLD));
    }

    /// One 30x4 pane with `Borders::ALL`, whose terminal carries `todos` given
    /// as (done, priority) pairs. The workspace lives in `app.workspaces` and
    /// `app.active` points at it, because the indicator resolves a pane's
    /// terminal the same way `render_panes` does.
    fn app_with_pane_todos(todos: &[(bool, TodoPriority)]) -> AppState {
        let mut app = AppState::test_new();
        app.mode = Mode::Terminal;
        app.workspaces = vec![Workspace::test_new("todos")];
        app.active = Some(0);
        app.ensure_test_terminals();

        let pane_id = app.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let terminal = app
            .terminals
            .get_mut(&terminal_id)
            .expect("test terminal should exist");
        for (index, (done, priority)) in todos.iter().enumerate() {
            let todo = terminal
                .add_todo(&format!("todo {index}"), *priority, None, 100)
                .expect("todo should be added");
            if *done {
                terminal
                    .update_todo(
                        todo.id,
                        TodoUpdate {
                            done: Some(true),
                            ..TodoUpdate::default()
                        },
                        200,
                    )
                    .expect("todo should be updated");
            }
        }

        app.view.terminal_area = Rect::new(0, 0, 30, 4);
        app.view.pane_infos = vec![PaneInfo {
            id: pane_id,
            rect: Rect::new(0, 0, 30, 4),
            inner_rect: Rect::new(1, 1, 28, 2),
            scrollbar_rect: None,
            borders: Borders::ALL,
            is_focused: false,
        }];
        app
    }

    fn draw_pane_borders(app: &AppState) -> ratatui::buffer::Buffer {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(30, 4)).unwrap();
        terminal
            .draw(|frame| render_view_pane_borders(app, &app.workspaces[0], frame))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn row_text(buffer: &ratatui::buffer::Buffer, row: u16, width: u16) -> String {
        crate::ui::test_support::row_text(buffer, Rect::new(0, row, width, 1))
    }

    #[test]
    fn pane_todo_indicator_counts_only_outstanding_todos() {
        let app = app_with_pane_todos(&[
            (false, TodoPriority::High),
            (false, TodoPriority::Normal),
            (false, TodoPriority::Low),
            (true, TodoPriority::High),
        ]);

        let indicator =
            pane_todo_indicator(&app, &app.view.pane_infos[0]).expect("indicator should exist");

        assert_eq!(indicator.label, " ▾ τ 3 ");
        assert_eq!(
            indicator.state,
            PaneTodoIndicatorState::Outstanding {
                count: 3,
                priority: Some(TodoPriority::High),
            }
        );
    }

    /// With work outstanding the `τ N` is bold and the `▾` handle is not;
    /// once everything is done nothing on the badge is bold.
    #[test]
    fn the_outstanding_count_is_bold_and_the_handle_is_not() {
        let app = app_with_pane_todos(&[(false, TodoPriority::High), (false, TodoPriority::Low)]);
        let indicator =
            pane_todo_indicator(&app, &app.view.pane_infos[0]).expect("indicator should exist");
        assert_eq!(indicator.label, " ▾ τ 2 ");
        let buffer = draw_pane_borders(&app);
        let (x, y) = (indicator.rect.x, indicator.rect.y);
        let bold = |dx: u16| {
            buffer[(x + dx, y)]
                .style()
                .add_modifier
                .contains(Modifier::BOLD)
        };
        assert_eq!(buffer[(x + 1, y)].symbol(), "▾");
        assert!(!bold(1), "the handle stays plain");
        assert_eq!(buffer[(x + 3, y)].symbol(), "τ");
        assert!(bold(3), "tau is bold");
        assert_eq!(buffer[(x + 5, y)].symbol(), "2");
        assert!(bold(5), "the count is bold");
        assert_eq!(buffer[(x + 3, y)].style().fg, Some(app.palette.red));

        let done = app_with_pane_todos(&[(true, TodoPriority::High)]);
        let indicator = pane_todo_indicator(&done, &done.view.pane_infos[0]).expect("indicator");
        let buffer = draw_pane_borders(&done);
        assert!(
            (0..indicator.rect.width).all(|dx| !buffer[(indicator.rect.x + dx, indicator.rect.y)]
                .style()
                .add_modifier
                .contains(Modifier::BOLD)),
            "nothing bold once all todos are done"
        );
    }

    /// Spec: "the cells that respond to a click are exactly the cells drawn".
    #[test]
    fn pane_todo_indicator_draws_exactly_the_cells_it_claims() {
        let app = app_with_pane_todos(&[(false, TodoPriority::High), (false, TodoPriority::Low)]);
        let indicator =
            pane_todo_indicator(&app, &app.view.pane_infos[0]).expect("indicator should exist");
        let buffer = draw_pane_borders(&app);

        let drawn: String = (indicator.rect.x..indicator.rect.x + indicator.rect.width)
            .map(|x| buffer[(x, indicator.rect.y)].symbol())
            .collect();
        assert_eq!(drawn, indicator.label, "claimed cells must hold the label");
        assert_eq!(
            buffer[(indicator.rect.x - 1, indicator.rect.y)].symbol(),
            "─",
            "the cell before the indicator is still border"
        );
        assert_eq!(
            buffer[(indicator.rect.x + indicator.rect.width, indicator.rect.y)].symbol(),
            "┐",
            "the corner glyph is never overwritten"
        );
        assert_eq!(
            indicator.rect.x + indicator.rect.width,
            app.view.pane_infos[0].rect.x + app.view.pane_infos[0].rect.width - 1,
            "the indicator hugs the far right of the top border"
        );
    }

    /// Spec: "an empty pane still offers the affordance". The indicator used
    /// to be drawn only for panes already holding todos, which left the one
    /// pane you would want to add a todo to — an empty one — with nothing to
    /// click.
    #[test]
    fn a_pane_with_no_todos_still_offers_the_affordance() {
        let app = app_with_pane_todos(&[]);
        let indicator = pane_todo_indicator(&app, &app.view.pane_infos[0])
            .expect("every bordered pane offers the affordance");

        assert_eq!(indicator.state, PaneTodoIndicatorState::Empty);
        assert_eq!(indicator.label, " ▾ ", "nothing to count");

        let buffer = draw_pane_borders(&app);
        let drawn: String = (indicator.rect.x..indicator.rect.x + indicator.rect.width)
            .map(|x| buffer[(x, indicator.rect.y)].symbol())
            .collect();
        assert_eq!(
            drawn, indicator.label,
            "the cells it claims for the click target are the cells drawn"
        );
    }

    #[test]
    fn an_all_done_pane_shows_a_bare_dimmed_glyph() {
        let app = app_with_pane_todos(&[(true, TodoPriority::High), (true, TodoPriority::Normal)]);
        let indicator =
            pane_todo_indicator(&app, &app.view.pane_infos[0]).expect("indicator should exist");

        assert_eq!(indicator.label, " ▾ ", "no count once everything is done");
        assert_eq!(indicator.state, PaneTodoIndicatorState::AllDone);

        let buffer = draw_pane_borders(&app);
        assert_eq!(
            buffer[(indicator.rect.x + 1, indicator.rect.y)].style().fg,
            Some(app.palette.overlay1),
            "a finished pane's indicator is muted"
        );
    }

    /// Spec: "empty is distinguishable from all-done". Both draw a bare glyph
    /// of the same width, so tone is the only thing left to separate "nothing
    /// to do" from "everything done". Drawing the indicator on every pane took
    /// away the old signal — its absence — and this is what replaces it.
    #[test]
    fn an_empty_pane_and_a_finished_pane_read_differently() {
        let empty = app_with_pane_todos(&[]);
        let finished = app_with_pane_todos(&[(true, TodoPriority::High)]);

        let empty_indicator =
            pane_todo_indicator(&empty, &empty.view.pane_infos[0]).expect("indicator should exist");
        let finished_indicator = pane_todo_indicator(&finished, &finished.view.pane_infos[0])
            .expect("indicator should exist");

        assert_eq!(
            empty_indicator.label, finished_indicator.label,
            "same glyph and same reserved width in both states"
        );
        assert_ne!(
            empty_indicator.state.color(&empty),
            finished_indicator.state.color(&finished),
            "so the tone has to carry the whole difference"
        );
    }

    #[test]
    fn indicator_color_follows_the_highest_outstanding_priority() {
        let high = app_with_pane_todos(&[(false, TodoPriority::High), (false, TodoPriority::Low)]);
        let normal = app_with_pane_todos(&[(false, TodoPriority::Normal)]);

        assert_eq!(
            high.pane_todo_indicator_color(Some(TodoPriority::High)),
            high.palette.red
        );
        assert_eq!(
            normal.pane_todo_indicator_color(Some(TodoPriority::Normal)),
            normal.palette.yellow
        );

        let mut pinned = app_with_pane_todos(&[(false, TodoPriority::High)]);
        pinned.pane_todo_color = Some(ratatui::style::Color::Magenta);
        assert_eq!(
            pinned.pane_todo_indicator_color(Some(TodoPriority::High)),
            ratatui::style::Color::Magenta,
            "ui.pane_todo_color pins the outstanding colour"
        );
        assert_eq!(
            pinned.pane_todo_indicator_color(None),
            pinned.palette.overlay0,
            "an all-done indicator stays muted even when pinned"
        );
    }

    fn set_manual_pane_label(app: &mut AppState, label: &str) {
        let pane_id = app.view.pane_infos[0].id;
        let terminal_id = app.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        app.terminals
            .get_mut(&terminal_id)
            .expect("test terminal should exist")
            .set_manual_label(label.into());
    }

    /// Spec: "the indicator SHALL be laid out before the pane title so the
    /// title truncates instead of the control disappearing".
    #[test]
    fn the_indicator_reserves_its_cells_before_the_title() {
        let mut app = app_with_pane_todos(&[(false, TodoPriority::High)]);
        set_manual_pane_label(&mut app, "a very long pane label indeed");

        let indicator =
            pane_todo_indicator(&app, &app.view.pane_infos[0]).expect("indicator should exist");
        let buffer = draw_pane_borders(&app);

        let drawn: String = (indicator.rect.x..indicator.rect.x + indicator.rect.width)
            .map(|x| buffer[(x, indicator.rect.y)].symbol())
            .collect();
        assert_eq!(drawn, indicator.label, "the control survives intact");
        assert!(
            row_text(&buffer, 0, 30).contains('…'),
            "the title truncates instead"
        );
    }

    /// Spec: "when pane width forces a choice, the indicator SHALL be laid out
    /// before the pane title". At 10 columns the 7-cell label still fits, so
    /// the title is what has to give way — not the other way round.
    #[test]
    fn a_squeezed_pane_keeps_the_control_and_drops_the_title() {
        let mut app = app_with_pane_todos(&[(false, TodoPriority::High)]);
        set_manual_pane_label(&mut app, "label");
        app.view.pane_infos[0].rect = Rect::new(0, 0, 10, 4);
        app.view.pane_infos[0].inner_rect = Rect::new(1, 1, 8, 2);

        let indicator =
            pane_todo_indicator(&app, &app.view.pane_infos[0]).expect("indicator should exist");
        assert_eq!(indicator.label, " ▾ τ 1 ");

        let buffer = draw_pane_borders(&app);
        assert_eq!(
            row_text(&buffer, 0, 10),
            "┌─ ▾ τ 1 ┐",
            "the control survives whole and the title drops out"
        );
    }

    #[test]
    fn the_indicator_is_hidden_by_config_and_on_borderless_panes() {
        let mut off = app_with_pane_todos(&[(false, TodoPriority::High)]);
        off.show_pane_todo_indicator = false;
        assert!(pane_todo_indicator(&off, &off.view.pane_infos[0]).is_none());

        let mut borderless = app_with_pane_todos(&[(false, TodoPriority::High)]);
        borderless.view.pane_infos[0].borders = Borders::NONE;
        assert!(pane_todo_indicator(&borderless, &borderless.view.pane_infos[0]).is_none());

        let mut narrow = app_with_pane_todos(&[(false, TodoPriority::High)]);
        narrow.view.pane_infos[0].rect = Rect::new(0, 0, 6, 4);
        narrow.view.pane_infos[0].inner_rect = Rect::new(1, 1, 4, 2);
        assert!(
            pane_todo_indicator(&narrow, &narrow.view.pane_infos[0]).is_none(),
            "a 5-cell label plus both corner glyphs needs 7 columns"
        );
        assert_eq!(
            row_text(&draw_pane_borders(&narrow), 0, 6),
            "┌────┐",
            "and nothing is drawn over the border it would not fit in"
        );
    }

    #[test]
    fn automatic_selection_background_uses_host_background() {
        let bg = automatic_selection_bg(
            &Palette::terminal(),
            crate::terminal_theme::TerminalTheme {
                foreground: Some(crate::terminal_theme::RgbColor {
                    r: 230,
                    g: 230,
                    b: 230,
                }),
                background: Some(crate::terminal_theme::RgbColor {
                    r: 12,
                    g: 14,
                    b: 16,
                }),
                ..Default::default()
            },
        );

        let Color::Rgb(r, g, b) = bg else {
            panic!("selection background should resolve to rgb");
        };
        assert!(relative_luminance((r, g, b)) > relative_luminance((12, 14, 16)));
    }

    #[test]
    fn automatic_selection_rgb_style_is_readable_with_or_without_host_background() {
        for (background, selected_bg, selected_fg) in [
            ((239, 241, 245), (172, 174, 176), (0, 0, 0)),
            ((26, 27, 38), (90, 91, 99), (255, 255, 255)),
            ((45, 53, 59), (104, 110, 114), (255, 255, 255)),
        ] {
            let mut palette = Palette::catppuccin();
            let (r, g, b) = background;
            palette.panel_bg = Color::Rgb(r, g, b);
            let expected = Style::reset()
                .bg(Color::Rgb(selected_bg.0, selected_bg.1, selected_bg.2))
                .fg(Color::Rgb(selected_fg.0, selected_fg.1, selected_fg.2));

            assert_eq!(
                automatic_selection_style(&palette, Default::default()),
                expected
            );
            assert_eq!(
                automatic_selection_style(
                    &Palette::terminal(),
                    crate::terminal_theme::TerminalTheme {
                        background: Some(crate::terminal_theme::RgbColor { r, g, b }),
                        ..Default::default()
                    },
                ),
                expected
            );
        }
    }

    #[test]
    fn automatic_selection_preserves_symbolic_palette_fallbacks() {
        let mut palette = Palette::terminal();
        assert_eq!(
            automatic_selection_style(&palette, Default::default()),
            Style::reset().fg(Color::White).bg(Color::DarkGray)
        );
        for fallback in [Color::Blue, Color::White, Color::Indexed(42), Color::Reset] {
            palette.surface_dim = fallback;
            assert_eq!(
                automatic_selection_bg(&palette, Default::default()),
                fallback
            );
        }
    }
}
