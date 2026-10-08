use super::*;
use ratatui::{
    style::Color,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Padding, Paragraph, Widget},
};

impl ClientShellState {
    pub(super) fn arm_pane_labels(&mut self, explicit: bool, now: std::time::Instant) {
        self.pane_labels_explicit = explicit;
        self.pane_labels_until = Some(now + self.config.display_panes_duration);
    }
    pub(super) fn pane_labels_visible(&self) -> bool {
        self.pane_labels_until.is_some()
            || self.mode == ClientShellMode::Resize
            || matches!(
                self.chrome_drag,
                Some(
                    ClientChromeDrag::PaneSplit { .. }
                        | ClientChromeDrag::SidebarWidth
                        | ClientChromeDrag::SidebarSection
                )
            )
    }
    pub(crate) fn tick_pane_labels(&mut self, now: std::time::Instant) -> bool {
        if self
            .pane_labels_until
            .is_some_and(|deadline| now >= deadline)
        {
            self.pane_labels_until = None;
            self.pane_labels_explicit = false;
            return true;
        }
        false
    }
    pub(super) fn close_pane_labels(&mut self) {
        self.pane_labels_until = None;
        self.pane_labels_explicit = false;
    }
    pub(super) fn label_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) -> bool {
        if !self.pane_labels_explicit {
            return false;
        }
        if let KeyCode::Char(digit @ '1'..='9') = key.code {
            let mut panes = self.hits.panes.iter().collect::<Vec<_>>();
            panes.sort_by_key(|pane| (pane.rect.y, pane.rect.x));
            if let Some(pane) = panes.get((digit as usize) - ('1' as usize)) {
                self.push_endpoint_method(
                    crate::api::schema::Method::PaneFocus(crate::api::schema::PaneTarget {
                        pane_id: pane.pane_id.clone(),
                    }),
                    outcome,
                );
            }
        }
        self.close_pane_labels();
        outcome.repaint = true;
        true
    }
}

fn label_text(
    index: usize,
    address: &str,
    name: &str,
    inner: Rect,
    maximum: usize,
    numbered: bool,
) -> String {
    let head = if numbered {
        format!(
            "{}  {address}",
            if index < 9 {
                (index + 1).to_string()
            } else {
                "·".into()
            }
        )
    } else {
        address.to_owned()
    };
    let size = format!("{}x{}", inner.width, inner.height);
    let full = format!("{head} · {name}  {size}");
    if usize::from(super::render::display_width(&full)) <= maximum {
        return full;
    }
    let without = usize::from(super::render::display_width(&head))
        + 3
        + 2
        + usize::from(super::render::display_width(&size));
    let budget = maximum.saturating_sub(without);
    if budget >= 4 {
        format!("{head} · {}  {size}", crate::ui::truncate_end(name, budget))
    } else {
        crate::ui::truncate_end(&format!("{head}  {size}"), maximum)
    }
}

pub(super) fn paint(
    buffer: &mut Buffer,
    layout: ClientShellLayout,
    state: &ClientShellState,
    snapshot: &ClientShellSnapshot,
) -> Vec<Rect> {
    let mut covered = Vec::new();
    let p = &state.config.palette;
    let mut panes = state.hits.panes.iter().collect::<Vec<_>>();
    panes.sort_by_key(|pane| (pane.rect.y, pane.rect.x));
    for (index, pane) in panes.iter().enumerate() {
        let inner = pane.inner_rect.intersection(buffer.area);
        if inner.is_empty() {
            continue;
        }
        let name = snapshot
            .agents
            .iter()
            .find(|agent| agent.pane_id == pane.pane_id)
            .and_then(|agent| {
                agent
                    .name
                    .as_deref()
                    .or(agent.title.as_deref())
                    .or(agent.display_agent.as_deref())
            })
            .or_else(|| {
                snapshot
                    .panes
                    .iter()
                    .find(|other| other.pane_id == pane.pane_id)
                    .and_then(|other| other.label.as_deref())
            })
            .unwrap_or("terminal");
        let boxed = inner.height >= 3 && inner.width > 4;
        let text = label_text(
            index,
            &pane.pane_id,
            name,
            inner,
            usize::from(if boxed { inner.width - 4 } else { inner.width }),
            state.pane_labels_explicit,
        );
        let width = super::render::display_width(&text);
        let active = snapshot.focused_pane_id.as_deref() == Some(pane.pane_id.as_str());
        let border = if active { p.red } else { p.muted_red() };
        let style = Style::default()
            .fg(p.text)
            .bg(p.panel_bg)
            .add_modifier(Modifier::BOLD);
        let rect = if boxed {
            Rect::new(
                inner.x + (inner.width - width - 4) / 2,
                inner.y + (inner.height - 3) / 2,
                width + 4,
                3,
            )
        } else {
            Rect::new(inner.x, inner.y + inner.height / 2, width, 1)
        };
        covered.push(rect);
        Clear.render(rect, buffer);
        if boxed {
            let block = Block::default()
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1))
                .border_style(Style::default().fg(border))
                .style(Style::default().bg(p.panel_bg));
            let text_area = block.inner(rect);
            block.render(rect, buffer);
            Paragraph::new(text).style(style).render(text_area, buffer);
        } else {
            Paragraph::new(text).style(style).render(rect, buffer);
        }
        if state.pane_labels_explicit {
            for y in pane.rect.y..pane.rect.bottom() {
                for x in pane.rect.x..pane.rect.right() {
                    if !super::contains(pane.inner_rect, (x, y))
                        && !pane
                            .scrollbar_rect
                            .is_some_and(|scroll| super::contains(scroll, (x, y)))
                    {
                        if let Some(cell) = buffer.cell_mut((x, y)) {
                            cell.set_fg(border);
                        }
                    }
                }
            }
        }
    }
    let bar = if state.config.tab_bar_position == TabBarPositionConfig::Bottom
        && !layout.tab_bar.is_empty()
    {
        layout.tab_bar
    } else {
        Rect::new(
            layout.pane_surface.x,
            layout.pane_surface.bottom().saturating_sub(1),
            layout.pane_surface.width,
            u16::from(layout.pane_surface.height > 0),
        )
    };
    if !bar.is_empty() {
        covered.push(bar);
        Clear.render(bar, buffer);
        let resize = state.mode == ClientShellMode::Resize;
        let chip = Style::default()
            .fg(if p.panel_bg == Color::Reset {
                p.surface_dim
            } else {
                p.panel_bg
            })
            .bg(if resize { p.mauve } else { p.red })
            .add_modifier(Modifier::BOLD);
        let version = state
            .endpoint_versions
            .get(&state.active_endpoint_id)
            .map(String::as_str)
            .or_else(|| {
                snapshot
                    .resource_facts
                    .as_ref()
                    .and_then(|facts| facts.server_version.as_deref())
            })
            .unwrap_or("unknown");
        let spans = summary_spans(
            buffer.area,
            layout.pane_surface,
            panes.len(),
            state.pane_labels_explicit,
            resize,
            version,
            p,
            chip,
            bar.width,
        );
        Paragraph::new(Line::from(spans))
            .style(Style::default().bg(p.panel_bg))
            .render(bar, buffer);
    }
    if !state.sidebar_collapsed {
        let (spaces, agents) =
            crate::ui::expanded_sidebar_sections(layout.sidebar, state.sidebar_section_split);
        for section in [spaces, agents] {
            let text = format!(" {}x{} ", section.width, section.height);
            let width = super::render::display_width(&text);
            if section.height > 0 && width <= section.width {
                let rect = Rect::new(section.right() - width, section.bottom() - 1, width, 1);
                covered.push(rect);
                buffer.set_stringn(
                    rect.x,
                    rect.y,
                    text,
                    usize::from(width),
                    Style::default()
                        .fg(panel_contrast_fg(p))
                        .bg(p.red)
                        .add_modifier(Modifier::BOLD),
                );
            }
        }
    }
    covered
}

fn summary_spans<'a>(
    window: Rect,
    panes: Rect,
    count: usize,
    explicit: bool,
    resize: bool,
    version: &'a str,
    p: &Palette,
    chip: Style,
    width: u16,
) -> Vec<Span<'a>> {
    let key = Style::default()
        .fg(if resize { p.accent } else { p.red })
        .add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(p.overlay0);
    let value = Style::default().fg(p.text);
    let mut spans = vec![
        Span::styled(if resize { " RESIZE " } else { " PANES " }, chip),
        Span::raw("  "),
    ];
    if resize {
        spans.extend([
            Span::styled("h/l", key),
            Span::styled(" width  ", dim),
            Span::styled("j/k", key),
            Span::styled(" height  ", dim),
            Span::styled("esc", key),
            Span::styled(" done  ", dim),
        ]);
    }
    spans.extend([
        Span::styled("window ", dim),
        Span::styled(format!("{}x{}", window.width, window.height), value),
        Span::styled(" · panes ", dim),
        Span::styled(format!("{}x{}", panes.width, panes.height), value),
    ]);
    if explicit {
        spans.push(Span::raw("  "));
        if count > 0 {
            let last = count.min(9);
            spans.extend([
                Span::styled(
                    if last == 1 {
                        "1".to_owned()
                    } else {
                        format!("1-{last}")
                    },
                    key,
                ),
                Span::styled(" focus  ", dim),
            ]);
        }
        spans.extend([Span::styled("any key", key), Span::styled(" close", dim)]);
    }
    // Keep the hints on narrow bars, dropping the version first, as in the fork.
    let version_spans = [
        Span::raw("  "),
        Span::styled(" VERSION ", chip),
        Span::raw(" "),
        Span::styled(version, value),
    ];
    if spans.iter().map(Span::width).sum::<usize>()
        + version_spans.iter().map(Span::width).sum::<usize>()
        <= usize::from(width)
    {
        spans.extend(version_spans);
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn each_resize_rearms_the_configured_linger_and_other_keys_close_explicit_labels() {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        let now = std::time::Instant::now();
        state.arm_pane_labels(false, now);
        state.arm_pane_labels(false, now + std::time::Duration::from_secs(1));
        assert!(!state.tick_pane_labels(now + std::time::Duration::from_secs(3)));
        assert!(state.tick_pane_labels(now + std::time::Duration::from_secs(4)));
        state.arm_pane_labels(true, now);
        state.close_pane_labels();
        assert!(!state.pane_labels_visible());
    }
    #[test]
    fn summary_keeps_red_version_chip_key_colors_and_narrow_bar_priority() {
        let config = ClientShellConfig::from_config(&Config::default());
        let p = &config.palette;
        let chip = Style::default()
            .fg(panel_contrast_fg(p))
            .bg(p.red)
            .add_modifier(Modifier::BOLD);
        let spans = summary_spans(
            Rect::new(0, 0, 140, 30),
            Rect::new(26, 0, 114, 29),
            2,
            true,
            false,
            "server-build",
            p,
            chip,
            140,
        );
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.ends_with("any key close   VERSION  server-build"));
        assert_eq!(
            spans
                .iter()
                .find(|s| s.content == " VERSION ")
                .unwrap()
                .style,
            chip
        );
        assert_eq!(
            spans
                .iter()
                .find(|s| s.content == "any key")
                .unwrap()
                .style
                .fg,
            Some(p.red)
        );
        let narrow = summary_spans(
            Rect::new(0, 0, 70, 30),
            Rect::new(26, 0, 44, 29),
            2,
            true,
            false,
            "server-build",
            p,
            chip,
            70,
        );
        let text: String = narrow.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.ends_with("any key close"));
        assert!(!text.contains("VERSION"));
        let passive = summary_spans(
            Rect::new(0, 0, 140, 30),
            Rect::new(26, 0, 114, 29),
            2,
            false,
            false,
            "server-build",
            p,
            chip,
            140,
        );
        assert!(!passive.iter().any(|s| s.content.contains("focus")));
    }

    #[test]
    fn long_label_drops_name_before_address_and_size() {
        let inner = Rect::new(0, 0, 80, 20);
        let text = label_text(0, "w5:p16", "very long name", inner, 20, true);
        assert!(text.contains("w5:p16"));
        assert!(text.ends_with("80x20"));
        assert!(super::super::render::display_width(&text) <= 20);
    }
}
