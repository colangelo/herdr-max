mod tokens;

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::Span,
};

pub(crate) use self::tokens::{
    agent_rows as sidebar_agent_rows, space_rows as sidebar_space_rows, AgentTokenContext,
    ResolvedToken, ResolvedTokenKind, SpaceTokenContext,
};
use super::text::{display_width, truncate_end, truncate_start};
use crate::app::state::Palette;
use crate::app::AppState;
use crate::detect::AgentState;
use crate::terminal::TerminalRuntimeRegistry;

pub(crate) struct AgentPanelEntry {
    pub ws_idx: usize,
    pub tab_idx: usize,
    pub pane_id: crate::layout::PaneId,
    pub agent_kind_label: Option<String>,
    pub state: AgentState,
    pub seen: bool,
    pub last_agent_state_change_seq: Option<u64>,
    /// Position among pinned agents; pinned agents lead the panel in pin order.
    pub pin_order: Option<u64>,
    pub tokens: std::collections::HashMap<String, String>,
}

fn sidebar_section_heights(total_height: u16, split_ratio: f32) -> (u16, u16) {
    if total_height == 0 {
        return (0, 0);
    }
    if total_height < 6 {
        let workspace_height = total_height.div_ceil(2);
        return (
            workspace_height,
            total_height.saturating_sub(workspace_height),
        );
    }

    let workspace_height = ((total_height as f32) * split_ratio.clamp(0.1, 0.9)).round() as u16;
    let workspace_height = workspace_height.clamp(3, total_height.saturating_sub(3));
    (
        workspace_height,
        total_height.saturating_sub(workspace_height),
    )
}

pub(crate) fn expanded_sidebar_sections(area: Rect, split_ratio: f32) -> (Rect, Rect) {
    let content = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    if content.is_empty() {
        return (Rect::default(), Rect::default());
    }

    let (workspace_height, detail_height) = sidebar_section_heights(content.height, split_ratio);
    (
        Rect::new(content.x, content.y, content.width, workspace_height),
        Rect::new(
            content.x,
            content.y + workspace_height,
            content.width,
            detail_height,
        ),
    )
}

pub(crate) fn sidebar_section_divider_rect(area: Rect, split_ratio: f32) -> Rect {
    let content = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    if content.width == 0 || content.height < 6 {
        return Rect::default();
    }

    let (workspace_height, _) = sidebar_section_heights(content.height, split_ratio);
    Rect::new(content.x, content.y + workspace_height, content.width, 1)
}

pub(crate) fn agent_panel_entries_from(
    app: &AppState,
    _terminal_runtimes: &TerminalRuntimeRegistry,
) -> Vec<AgentPanelEntry> {
    let mut entries = app
        .workspaces
        .iter()
        .enumerate()
        .flat_map(|(ws_idx, workspace)| {
            workspace
                .pane_details(&app.terminals)
                .into_iter()
                .map(move |detail| AgentPanelEntry {
                    ws_idx,
                    tab_idx: detail.tab_idx,
                    pane_id: detail.pane_id,
                    agent_kind_label: detail.agent_kind_label,
                    state: detail.state,
                    seen: detail.seen,
                    last_agent_state_change_seq: detail.last_agent_state_change_seq,
                    pin_order: detail.pin_order,
                    tokens: detail.tokens,
                })
        })
        .collect();
    crate::app::agent_view::apply_agent_view(app, &mut entries);
    entries
}

pub(crate) fn resolved_token_spans(
    resolved: &[ResolvedToken],
    state_icon: (&str, Style),
    state_text_style: Style,
    workspace_style: Style,
    secondary_style: Style,
    custom_style: Style,
    palette: &Palette,
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
            | ResolvedTokenKind::Machine(text)
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
                Style::default().fg(palette.overlay0),
            ));
        }
        match &token.kind {
            ResolvedTokenKind::StateIcon => spans.push(Span::styled(
                state_icon.0.to_string(),
                apply_token_style(state_icon.1, token.style),
            )),
            ResolvedTokenKind::StateText(text) => spans.push(Span::styled(
                cut(token, text, budgets[index]),
                apply_token_style(state_text_style, token.style),
            )),
            ResolvedTokenKind::Workspace(text) => spans.push(Span::styled(
                cut(token, text, budgets[index]),
                apply_token_style(workspace_style, token.style),
            )),
            ResolvedTokenKind::Machine(text)
            | ResolvedTokenKind::Tab(text)
            | ResolvedTokenKind::Pane(text)
            | ResolvedTokenKind::Agent(text)
            | ResolvedTokenKind::Branch(text) => spans.push(Span::styled(
                cut(token, text, budgets[index]),
                apply_token_style(secondary_style, token.style),
            )),
            ResolvedTokenKind::GitStatus { ahead, behind } => {
                if *ahead > 0 {
                    spans.push(Span::styled(
                        format!("↑{ahead}"),
                        apply_token_style(Style::default().fg(palette.green), token.style),
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
                        apply_token_style(Style::default().fg(palette.red), token.style),
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
    if let Some(foreground) = patch.fg {
        style = style.fg(foreground.ratatui());
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

#[cfg(test)]
mod fitting_tests {
    use super::*;

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
    fn italic_patch_changes_only_token_content_and_preserves_explicit_false() {
        let mut row = asks_row(None, None);
        row[2].style.italic = Some(true);
        let spans = resolved_token_spans(
            &row,
            ("", Style::default()),
            Style::default(),
            Style::default(),
            Style::default(),
            Style::default(),
            &Palette::catppuccin(),
            24,
        );
        assert!(spans
            .last()
            .unwrap()
            .style
            .add_modifier
            .contains(Modifier::ITALIC));
        assert!(spans[..spans.len() - 1]
            .iter()
            .all(|span| !span.style.add_modifier.contains(Modifier::ITALIC)));
        row[2].style.italic = Some(false);
        let spans = resolved_token_spans(
            &row,
            ("", Style::default()),
            Style::default(),
            Style::default(),
            Style::default(),
            Style::default().add_modifier(Modifier::ITALIC),
            &Palette::catppuccin(),
            24,
        );
        assert!(spans
            .last()
            .unwrap()
            .style
            .sub_modifier
            .contains(Modifier::ITALIC));
    }
}
