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

use super::sidebar::pin_marker_color;
use super::status::state_label;
use super::text::{display_width, truncate_end};
use crate::agent_priority::attention_priority;
use crate::detect::AgentState;

/// The two fog levels: how far the nearest row, and the one after it, is lifted
/// from the panel background toward the text colour.
pub(crate) const FOG_PERCENT: [u32; 2] = [17, 7];

/// How much of the most urgent hidden state's colour the fog target takes, in
/// percent. The rest is the text colour.
const FOG_TINT_PERCENT: u32 = 70;

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
    /// Rects to lighten, each with its level into [`FOG_PERCENT`] and the
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
            for (level, item) in near.take(FOG_PERCENT.len()).enumerate() {
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
pub(crate) fn fog_color(
    base: Color,
    text: Color,
    tint: Option<Color>,
    level: usize,
) -> Option<Color> {
    let percent = *FOG_PERCENT.get(level)?;
    let target = match tint {
        Some(tint) => lift(text, tint, FOG_TINT_PERCENT)?,
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
        assert_eq!(fog_color(base, text, None, 0), Some(Color::Rgb(44, 44, 44)));
        assert_eq!(fog_color(base, text, None, 1), Some(Color::Rgb(24, 24, 24)));
        assert_eq!(fog_color(base, text, None, 2), None);
    }

    #[test]
    fn fog_takes_a_faint_tint_of_the_urgent_state() {
        let base = Color::Rgb(10, 10, 10);
        let text = Color::Rgb(200, 200, 200);
        let red = Color::Rgb(240, 100, 100);
        let Some(Color::Rgb(r, g, b)) = fog_color(base, text, Some(red), 0) else {
            panic!("expected an rgb fog");
        };
        assert!(r > g && r > b, "tinted toward red: {r} {g} {b}");
    }

    #[test]
    fn fog_needs_rgb_colours() {
        assert_eq!(fog_color(Color::Reset, Color::Rgb(1, 2, 3), None, 0), None);
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
