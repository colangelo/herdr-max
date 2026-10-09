//! What is scrolled out of view in the spaces list and the agent panel (fork
//! issue 159): a summary row at an edge with rows hidden past it, and a lighter
//! background on the rows next to that edge.
//!
//! Everything here is pure. The two lists hand in what they already know (how
//! many entries, the scroll, a fit function for their row heights) and get back
//! plain geometry and colours, so render and hit-testing read the same answer.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use super::pins::marker_color as pin_marker_color;
fn state_label(state: crate::detect::AgentState, seen: bool) -> &'static str {
    match (state, seen) {
        (crate::detect::AgentState::Blocked, _) => "blocked",
        (crate::detect::AgentState::Working, _) => "working",
        (crate::detect::AgentState::Idle, false) => "done",
        (crate::detect::AgentState::Idle, true) => "idle",
        _ => "unknown",
    }
}
use crate::agent_priority::attention_priority;
use crate::detect::AgentState;
use crate::ui::text::{display_width, truncate_end};

/// A hidden state only gets named on an edge row, and only tints the fog, when
/// it is waiting on the user: blocked, or finished and unseen.
const ATTENTION_FLOOR: u8 = 3;

/// The rows a list gives up to its edge rows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct EdgeReserve {
    pub top: u16,
    pub bottom: u16,
}

/// Whether a list with a body this tall has room for edge rows at all: one for
/// each edge and at least one entry between them.
fn edge_rows_fit(edge_rows: bool, body_height: u16) -> bool {
    edge_rows && body_height >= 3
}

/// The edge rows of the window starting at `scroll`, and how many entries it
/// shows. `fit(avail)` is the number of entries from `scroll` that fit in
/// `avail` rows. The top row exists when the window starts past the first
/// entry; the bottom row exists when entries remain past the last one shown.
pub(crate) fn window(
    edge_rows: bool,
    body_height: u16,
    len: usize,
    scroll: usize,
    fit: impl Fn(u16) -> usize,
) -> (EdgeReserve, usize) {
    let on = edge_rows_fit(edge_rows, body_height);
    let top = u16::from(on && scroll > 0);
    let avail = body_height.saturating_sub(top);
    let mut count = fit(avail);
    let mut bottom = 0;
    if on && scroll.saturating_add(count) < len {
        let reduced = fit(avail.saturating_sub(1));
        if reduced > 0 {
            count = reduced;
            bottom = 1;
        }
    }
    (EdgeReserve { top, bottom }, count)
}

/// The furthest the list scrolls: the first entry of the last full window.
/// `start(avail)` is that entry for a body `avail` rows tall. Past the first
/// window the top edge row is always there, so the last window is one row
/// shorter.
pub(crate) fn last_window_start(
    edge_rows: bool,
    body_height: u16,
    start: impl Fn(u16) -> usize,
) -> usize {
    let first = start(body_height);
    if edge_rows_fit(edge_rows, body_height) && first > 0 {
        start(body_height - 1)
    } else {
        first
    }
}

/// One entry of a list, as far as the edge rows and the fog care.
#[derive(Debug, Clone, Copy)]
pub(crate) struct EdgeItem {
    /// The rank of the entry's pin marker, when it is pinned.
    pub pin_rank: Option<usize>,
    pub state: AgentState,
    pub seen: bool,
}

/// What the entries hidden past one edge add up to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct EdgeSummary {
    pub hidden: usize,
    /// The pin-marker ranks of the hidden pinned entries.
    pub pin_ranks: Vec<usize>,
    /// The most urgent hidden state that is waiting on the user, and how many
    /// hidden entries are in it.
    pub urgent: Option<(AgentState, bool, usize)>,
}

impl EdgeSummary {
    pub(crate) fn of(items: &[EdgeItem]) -> Self {
        let most_urgent = items
            .iter()
            .filter(|item| attention_priority(item.state, item.seen) >= ATTENTION_FLOOR)
            .max_by_key(|item| attention_priority(item.state, item.seen));
        let urgent = most_urgent.map(|top| {
            let count = items
                .iter()
                .filter(|item| item.state == top.state && item.seen == top.seen)
                .count();
            (top.state, top.seen, count)
        });
        Self {
            hidden: items.len(),
            pin_ranks: items.iter().filter_map(|item| item.pin_rank).collect(),
            urgent,
        }
    }
}

/// Which edge of a list a summary or a fog band belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Edge {
    Above,
    Below,
}

/// An edge row and what it says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EdgeRow {
    pub edge: Edge,
    pub rect: Rect,
    pub summary: EdgeSummary,
}

/// Everything a list draws for rows out of view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct OverflowPlan {
    pub rows: Vec<EdgeRow>,
    /// Rects to lighten, each with its row (0 is the nearest) and the
    /// state to tint it with.
    pub fog: Vec<FogBand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FogBand {
    pub rect: Rect,
    pub level: usize,
    pub tint: Option<(AgentState, bool)>,
}

/// One visible entry: its index in the list, its rect, and whether it is the
/// focused or selected one (which never fogs).
#[derive(Debug, Clone, Copy)]
pub(crate) struct VisibleItem {
    pub index: usize,
    pub rect: Rect,
    pub exempt: bool,
}

/// The plan for a list: `items` is every entry in order, `scroll` the first
/// visible one, `visible` the entries on screen, `body` the rect the list
/// draws in, `reserve` its edge rows.
pub(crate) fn plan(
    fog: bool,
    items: &[EdgeItem],
    scroll: usize,
    visible: &[VisibleItem],
    body: Rect,
    reserve: EdgeReserve,
) -> OverflowPlan {
    let mut plan = OverflowPlan::default();
    let Some(last) = visible.last() else {
        return plan;
    };
    let above = &items[..scroll.min(items.len())];
    let below = &items[(last.index + 1).min(items.len())..];

    let summary_above = EdgeSummary::of(above);
    let summary_below = EdgeSummary::of(below);

    if reserve.top > 0 {
        plan.rows.push(EdgeRow {
            edge: Edge::Above,
            rect: Rect::new(body.x, body.y, body.width, 1),
            summary: summary_above.clone(),
        });
    }
    if reserve.bottom > 0 {
        plan.rows.push(EdgeRow {
            edge: Edge::Below,
            rect: Rect::new(body.x, body.y + body.height - 1, body.width, 1),
            summary: summary_below.clone(),
        });
    }

    if fog {
        let mut add = |summary: &EdgeSummary, near: &mut dyn Iterator<Item = &VisibleItem>| {
            if summary.hidden == 0 {
                return;
            }
            let tint = summary.urgent.map(|(state, seen, _)| (state, seen));
            for (level, item) in near.take(crate::config::SIDEBAR_FOG_ROWS).enumerate() {
                if !item.exempt {
                    plan.fog.push(FogBand {
                        rect: item.rect,
                        level,
                        tint,
                    });
                }
            }
        };
        add(&summary_above, &mut visible.iter());
        add(&summary_below, &mut visible.iter().rev());
    }
    plan
}

/// `base` lifted `percent` of the way toward `target`. `None` when either is a
/// colour the terminal names rather than gives as RGB: those cannot be mixed.
pub(crate) fn lift(base: Color, target: Color, percent: u32) -> Option<Color> {
    let (Color::Rgb(br, bg, bb), Color::Rgb(tr, tg, tb)) = (base, target) else {
        return None;
    };
    let mix = |from: u8, to: u8| {
        let (from, to) = (i64::from(from), i64::from(to));
        u8::try_from((from + (to - from) * i64::from(percent) / 100).clamp(0, 255)).unwrap_or(0)
    };
    Some(Color::Rgb(mix(br, tr), mix(bg, tg), mix(bb, tb)))
}

/// The background of a fogged row: the panel background lifted toward the text
/// colour, with a faint share of `tint` in the target when a hidden entry is
/// waiting on the user.
///
/// `percent` is the row's lift and `tint_percent` the share of `tint` in the
/// target (both from `[ui] sidebar_fog` and `sidebar_fog_tint`). A row with a
/// `percent` of 0 gets no fog.
pub(crate) fn fog_color(
    base: Color,
    text: Color,
    tint: Option<Color>,
    percent: u32,
    tint_percent: u32,
) -> Option<Color> {
    if percent == 0 {
        return None;
    }
    let target = match tint {
        Some(tint) => lift(text, tint, tint_percent)?,
        None => text,
    };
    lift(base, target, percent)
}

/// The text of an edge row, clipped to `width`. Arrows come first, in the
/// colours of the hidden pins above; a dot in the colour of the most urgent
/// hidden state closes the row.
pub(crate) fn edge_row_line(
    row: &EdgeRow,
    width: u16,
    dim: Style,
    urgent_color: Option<Color>,
) -> Line<'static> {
    let summary = &row.summary;
    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ")];
    let mut text = String::new();
    match row.edge {
        Edge::Above => {
            if summary.pin_ranks.is_empty() {
                spans.push(Span::styled("↑", dim));
            } else {
                for rank in summary.pin_ranks.iter().take(4) {
                    spans.push(Span::styled(
                        "↑",
                        Style::default().fg(pin_marker_color(*rank)),
                    ));
                }
                text.push_str(&format!(" {} pinned", summary.pin_ranks.len()));
            }
            let more = summary.hidden.saturating_sub(summary.pin_ranks.len());
            if more > 0 {
                text.push_str(if text.is_empty() { " " } else { " · " });
                text.push_str(&format!("{more} more"));
            }
        }
        Edge::Below => {
            spans.push(Span::styled("↓", dim));
            text.push_str(&format!(" {} more", summary.hidden));
        }
    }
    spans.push(Span::styled(text, dim));
    if let Some((state, seen, count)) = summary.urgent {
        spans.push(Span::styled(" · ", dim));
        spans.push(Span::styled(
            "●",
            Style::default().fg(urgent_color.unwrap_or(Color::Reset)),
        ));
        spans.push(Span::styled(
            format!(" {count} {}", state_label(state, seen)),
            dim,
        ));
    }
    clip(spans, width as usize)
}

/// `spans` cut to `width` cells, the last span truncated with an ellipsis.
fn clip(spans: Vec<Span<'static>>, width: usize) -> Line<'static> {
    let mut used = 0;
    let mut out = Vec::new();
    for span in spans {
        let w = display_width(&span.content);
        if used + w <= width {
            used += w;
            out.push(span);
            continue;
        }
        let room = width.saturating_sub(used);
        if room > 0 {
            out.push(Span::styled(
                truncate_end(&span.content, room).to_string(),
                span.style,
            ));
        }
        break;
    }
    Line::from(out)
}

/// The dim style of an edge row's text.
pub(crate) fn dim_style(color: Color) -> Style {
    Style::default().fg(color).add_modifier(Modifier::DIM)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(pin: Option<usize>, state: AgentState, seen: bool) -> EdgeItem {
        EdgeItem {
            pin_rank: pin,
            state,
            seen,
        }
    }

    /// Entries one row tall, no gaps: `avail` rows hold `avail` entries from
    /// `scroll`, capped at the end of the list.
    fn fit_flat(len: usize, scroll: usize) -> impl Fn(u16) -> usize {
        move |avail| (len - scroll).min(usize::from(avail))
    }

    #[test]
    fn window_has_no_edge_rows_when_everything_fits() {
        let (reserve, count) = window(true, 10, 6, 0, fit_flat(6, 0));
        assert_eq!(reserve, EdgeReserve::default());
        assert_eq!(count, 6);
    }

    #[test]
    fn window_reserves_the_bottom_row_when_entries_are_hidden_below() {
        let (reserve, count) = window(true, 6, 20, 0, fit_flat(20, 0));
        assert_eq!(reserve, EdgeReserve { top: 0, bottom: 1 });
        assert_eq!(count, 5);
    }

    #[test]
    fn window_reserves_both_rows_in_the_middle_and_only_the_top_at_the_end() {
        let (middle, count) = window(true, 6, 20, 5, fit_flat(20, 5));
        assert_eq!(middle, EdgeReserve { top: 1, bottom: 1 });
        assert_eq!(count, 4);
        let (end, count) = window(true, 6, 20, 15, fit_flat(20, 15));
        assert_eq!(end, EdgeReserve { top: 1, bottom: 0 });
        assert_eq!(count, 5);
    }

    #[test]
    fn window_reserves_nothing_when_edge_rows_are_off_or_the_body_is_tiny() {
        let (off, count) = window(false, 6, 20, 5, fit_flat(20, 5));
        assert_eq!(off, EdgeReserve::default());
        assert_eq!(count, 6);
        let (tiny, count) = window(true, 2, 20, 5, fit_flat(20, 5));
        assert_eq!(tiny, EdgeReserve::default());
        assert_eq!(count, 2);
    }

    #[test]
    fn the_last_window_is_one_row_shorter_once_the_top_row_is_there() {
        let start = |avail: u16| 20usize.saturating_sub(usize::from(avail));
        assert_eq!(last_window_start(false, 6, start), 14);
        assert_eq!(last_window_start(true, 6, start), 15);
        // Everything fits: no scrolling, no top row.
        let fits = |avail: u16| 4usize.saturating_sub(usize::from(avail));
        assert_eq!(last_window_start(true, 6, fits), 0);
    }

    #[test]
    fn summary_counts_pins_and_picks_the_most_urgent_state() {
        let summary = EdgeSummary::of(&[
            item(Some(0), AgentState::Idle, true),
            item(Some(1), AgentState::Blocked, true),
            item(None, AgentState::Blocked, true),
            item(None, AgentState::Idle, false),
            item(None, AgentState::Working, true),
        ]);
        assert_eq!(summary.hidden, 5);
        assert_eq!(summary.pin_ranks, vec![0, 1]);
        assert_eq!(summary.urgent, Some((AgentState::Blocked, true, 2)));
    }

    #[test]
    fn summary_names_no_state_when_nothing_hidden_is_waiting() {
        let summary = EdgeSummary::of(&[
            item(None, AgentState::Idle, true),
            item(None, AgentState::Working, true),
        ]);
        assert_eq!(summary.urgent, None);
    }

    #[test]
    fn fog_lifts_the_background_toward_the_text_colour() {
        let base = Color::Rgb(10, 10, 10);
        let text = Color::Rgb(210, 210, 210);
        assert_eq!(
            fog_color(base, text, None, 17, 70),
            Some(Color::Rgb(44, 44, 44))
        );
        assert_eq!(
            fog_color(base, text, None, 7, 70),
            Some(Color::Rgb(24, 24, 24))
        );
        assert_eq!(fog_color(base, text, None, 0, 70), None, "0 is no fog");
    }

    #[test]
    fn fog_takes_a_faint_tint_of_the_urgent_state() {
        let base = Color::Rgb(10, 10, 10);
        let text = Color::Rgb(200, 200, 200);
        let red = Color::Rgb(240, 100, 100);
        let Some(Color::Rgb(r, g, b)) = fog_color(base, text, Some(red), 17, 70) else {
            panic!("expected an rgb fog");
        };
        assert!(r > g && r > b, "tinted toward red: {r} {g} {b}");
    }

    #[test]
    fn fog_needs_rgb_colours() {
        assert_eq!(
            fog_color(Color::Reset, Color::Rgb(1, 2, 3), None, 17, 70),
            None
        );
    }

    fn vis(index: usize, y: u16, exempt: bool) -> VisibleItem {
        VisibleItem {
            index,
            rect: Rect::new(0, y, 20, 1),
            exempt,
        }
    }

    #[test]
    fn plan_fogs_the_two_rows_next_to_each_hidden_edge() {
        let items: Vec<EdgeItem> = (0..10)
            .map(|_| item(None, AgentState::Idle, true))
            .collect();
        let visible: Vec<VisibleItem> = (3..7).map(|i| vis(i, (i - 3) as u16 + 1, false)).collect();
        let body = Rect::new(0, 0, 20, 6);
        let reserve = EdgeReserve { top: 1, bottom: 1 };
        let plan = plan(true, &items, 3, &visible, body, reserve);
        assert_eq!(plan.rows.len(), 2);
        assert_eq!(plan.rows[0].summary.hidden, 3);
        assert_eq!(plan.rows[1].summary.hidden, 3);
        let levels: Vec<(u16, usize)> = plan.fog.iter().map(|b| (b.rect.y, b.level)).collect();
        assert_eq!(levels, vec![(1, 0), (2, 1), (4, 0), (3, 1)]);
    }

    #[test]
    fn plan_never_fogs_an_exempt_row_and_skips_an_edge_with_nothing_hidden() {
        let items: Vec<EdgeItem> = (0..6).map(|_| item(None, AgentState::Idle, true)).collect();
        let visible: Vec<VisibleItem> = (0..5).map(|i| vis(i, i as u16, i == 4)).collect();
        let body = Rect::new(0, 0, 20, 6);
        let plan = plan(true, &items, 0, &visible, body, EdgeReserve::default());
        // Row 5 is hidden below; the nearest visible row (4) is exempt.
        let levels: Vec<(u16, usize)> = plan.fog.iter().map(|b| (b.rect.y, b.level)).collect();
        assert_eq!(levels, vec![(3, 1)]);
    }

    #[test]
    fn edge_rows_read_as_a_summary() {
        let dim = dim_style(Color::Gray);
        let above = EdgeRow {
            edge: Edge::Above,
            rect: Rect::new(0, 0, 30, 1),
            summary: EdgeSummary {
                hidden: 5,
                pin_ranks: vec![0, 1],
                urgent: None,
            },
        };
        let text: String = edge_row_line(&above, 30, dim, None)
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert_eq!(text, " ↑↑ 2 pinned · 3 more");
        let below = EdgeRow {
            edge: Edge::Below,
            rect: Rect::new(0, 5, 30, 1),
            summary: EdgeSummary {
                hidden: 5,
                pin_ranks: Vec::new(),
                urgent: Some((AgentState::Blocked, true, 1)),
            },
        };
        let text: String = edge_row_line(&below, 30, dim, Some(Color::Red))
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert_eq!(text, " ↓ 5 more · ● 1 blocked");
    }

    #[test]
    fn edge_row_clips_to_the_width() {
        let dim = dim_style(Color::Gray);
        let below = EdgeRow {
            edge: Edge::Below,
            rect: Rect::new(0, 5, 12, 1),
            summary: EdgeSummary {
                hidden: 12,
                pin_ranks: Vec::new(),
                urgent: Some((AgentState::Blocked, true, 3)),
            },
        };
        let text: String = edge_row_line(&below, 12, dim, None)
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert!(display_width(&text) <= 12, "{text:?}");
    }
}

// The client adapters use the same reserved-window arithmetic as the fork.
pub(super) fn list_metrics(
    heights: &[u16],
    gaps: &[u16],
    height: u16,
    start: usize,
    edges: bool,
) -> (crate::pane::ScrollMetrics, EdgeReserve, usize) {
    if height == 0 || heights.is_empty() {
        return (
            super::scroll::list_scroll_metrics(heights, gaps, height, start),
            EdgeReserve::default(),
            0,
        );
    }
    let max = last_window_start(edges, height, |available| {
        super::scroll::list_scroll_metrics(heights, gaps, available, usize::MAX)
            .max_offset_from_bottom
    });
    let start = start.min(max);
    let (reserve, count) = window(edges, height, heights.len(), start, |available| {
        fit(heights, gaps, start, available)
    });
    (
        crate::pane::ScrollMetrics {
            offset_from_bottom: max.saturating_sub(start),
            max_offset_from_bottom: max,
            viewport_rows: count,
        },
        reserve,
        count,
    )
}
fn fit(heights: &[u16], gaps: &[u16], start: usize, available: u16) -> usize {
    if available == 0 {
        return 0;
    }
    let mut used = 0u16;
    let mut count = 0;
    for (index, &height) in heights.iter().enumerate().skip(start) {
        let h = height.max(1).min(available);
        if used.saturating_add(h) > available {
            break;
        }
        used = used.saturating_add(h);
        count += 1;
        used = used.saturating_add(gaps.get(index).copied().unwrap_or(0));
    }
    count
}
pub(super) fn reveal_start(
    heights: &[u16],
    gaps: &[u16],
    height: u16,
    start: usize,
    target: usize,
    edges: bool,
) -> usize {
    if target < start {
        return target;
    }
    let (mut metrics, _, _) = list_metrics(heights, gaps, height, start, edges);
    let mut start = metrics
        .max_offset_from_bottom
        .saturating_sub(metrics.offset_from_bottom);
    while target >= start.saturating_add(metrics.viewport_rows)
        && start < metrics.max_offset_from_bottom
    {
        start += 1;
        metrics = list_metrics(heights, gaps, height, start, edges).0;
    }
    start
}
pub(super) fn item(status: crate::api::schema::AgentStatus, pin_rank: Option<usize>) -> EdgeItem {
    let (state, seen) = match status {
        crate::api::schema::AgentStatus::Working => (AgentState::Working, true),
        crate::api::schema::AgentStatus::Blocked => (AgentState::Blocked, true),
        crate::api::schema::AgentStatus::Done => (AgentState::Idle, false),
        crate::api::schema::AgentStatus::Idle => (AgentState::Idle, true),
        _ => (AgentState::Unknown, true),
    };
    EdgeItem {
        state,
        seen,
        pin_rank,
    }
}
fn status(state: AgentState, seen: bool) -> crate::api::schema::AgentStatus {
    match (state, seen) {
        (AgentState::Working, _) => crate::api::schema::AgentStatus::Working,
        (AgentState::Blocked, _) => crate::api::schema::AgentStatus::Blocked,
        (AgentState::Idle, false) => crate::api::schema::AgentStatus::Done,
        (AgentState::Idle, true) => crate::api::schema::AgentStatus::Idle,
        _ => crate::api::schema::AgentStatus::Unknown,
    }
}
pub(super) fn paint(
    buffer: &mut ratatui::buffer::Buffer,
    plan: &OverflowPlan,
    config: &super::ClientShellConfig,
    hits: &mut super::ShellHitMap,
    agents: bool,
    page: usize,
) {
    use ratatui::widgets::{Paragraph, Widget};
    let p = &config.palette;
    let base = if matches!(p.sidebar_bg, Color::Rgb(..)) {
        p.sidebar_bg
    } else {
        config
            .host_background
            .map(|c| Color::Rgb(c.r, c.g, c.b))
            .unwrap_or(p.panel_bg)
    };
    for band in &plan.fog {
        let tint = band
            .tint
            .map(|(state, seen)| config.state_color(status(state, seen)));
        let bg = config
            .sidebar_fog_style
            .lifts()
            .then(|| {
                fog_color(
                    base,
                    p.text,
                    tint,
                    config.sidebar_fog[band.level],
                    config.sidebar_fog_tint,
                )
            })
            .flatten();
        let fade = if config.sidebar_fog_style.dims() {
            config.sidebar_fade[band.level]
        } else {
            0
        };
        let rect = band.rect.intersection(buffer.area);
        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                let cell = &mut buffer[(x, y)];
                if let Some(bg) = bg {
                    cell.set_bg(bg);
                }
                if fade > 0 {
                    let fg = if cell.fg == Color::Reset {
                        p.text
                    } else {
                        cell.fg
                    };
                    if let Some(fg) = lift(fg, base, fade) {
                        cell.set_fg(fg);
                    }
                }
            }
        }
    }
    for row in &plan.rows {
        let color = row
            .summary
            .urgent
            .map(|(state, seen, _)| config.state_color(status(state, seen)));
        for y in row.rect.y..row.rect.bottom() {
            for x in row.rect.x..row.rect.right() {
                buffer[(x, y)]
                    .set_symbol(" ")
                    .set_style(Style::reset().bg(p.sidebar_bg));
            }
        }
        Paragraph::new(edge_row_line(
            row,
            row.rect.width,
            dim_style(p.overlay0),
            color,
        ))
        .render(row.rect, buffer);
        hits.overflow_edges
            .push((row.rect, agents, row.edge == Edge::Above, page.max(1)));
    }
}

#[cfg(test)]
mod client_adapter_tests {
    use super::*;
    use crate::client::shell::{ClientShellConfig, ShellHitMap};
    #[test]
    fn last_reserved_window_reaches_the_last_entry_with_variable_heights() {
        let heights = [2, 2, 2, 2];
        let gaps = [1, 1, 1, 0];
        let (metrics, reserve, count) = list_metrics(&heights, &gaps, 7, usize::MAX, true);
        let start = metrics.max_offset_from_bottom;
        assert_eq!(start + count, heights.len());
        assert_eq!(reserve.top, 1);
        assert_eq!(reserve.bottom, 0);
        assert_eq!(reveal_start(&heights, &gaps, 7, 0, 3, true), start);
        assert_eq!(
            list_metrics(&heights, &gaps, 7, 0, false).0,
            super::super::scroll::list_scroll_metrics(&heights, &gaps, 7, 0)
        );
    }
    #[test]
    fn zero_fog_and_fade_preserve_original_cells_and_host_background_is_cached() {
        let mut config = crate::config::Config::default();
        config.ui.sidebar_fog = vec![0, 0];
        config.ui.sidebar_fade = vec![0, 0];
        config.ui.sidebar_fog_style = "both".into();
        let mut config = ClientShellConfig::from_config(&config);
        config.palette.sidebar_bg = Color::Reset;
        config.palette.text = Color::Rgb(210, 210, 210);
        config.host_background = Some(crate::terminal_theme::RgbColor {
            r: 10,
            g: 10,
            b: 10,
        });
        let area = Rect::new(0, 0, 10, 3);
        let mut buffer = ratatui::buffer::Buffer::empty(area);
        buffer.set_style(
            area,
            Style::default()
                .fg(Color::Rgb(210, 210, 210))
                .bg(Color::Rgb(10, 10, 10)),
        );
        let before = buffer.clone();
        let mut hits = ShellHitMap::default();
        let plan = OverflowPlan {
            rows: Vec::new(),
            fog: vec![FogBand {
                rect: area,
                level: 0,
                tint: None,
            }],
        };
        paint(&mut buffer, &plan, &config, &mut hits, false, 1);
        assert_eq!(buffer, before);
        config.sidebar_fog = [17, 7];
        config.sidebar_fog_style = crate::config::SidebarFogStyle::Lift;
        paint(&mut buffer, &plan, &config, &mut hits, false, 1);
        assert_eq!(buffer[(0, 0)].bg, Color::Rgb(44, 44, 44));
    }
}
