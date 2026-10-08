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
        let sizes = format!(
            "  window {}x{} · panes {}x{}",
            buffer.area.width,
            buffer.area.height,
            layout.pane_surface.width,
            layout.pane_surface.height
        );
        let hint = if resize {
            "  h/l width  j/k height  esc done".to_owned()
        } else if state.pane_labels_explicit {
            format!("  1-{} focus  any key close", panes.len().min(9))
        } else {
            String::new()
        };
        let version = snapshot
            .resource_facts
            .as_ref()
            .and_then(|facts| facts.server_version.as_deref())
            .unwrap_or("unknown");
        let version = format!("  VERSION  {version}");
        let main = format!("{}{}", sizes, hint);
        let mut spans = vec![
            Span::styled(if resize { " RESIZE " } else { " PANES " }, chip),
            Span::styled(main.clone(), Style::default().fg(p.text)),
        ];
        if super::render::display_width(&main)
            .saturating_add(super::render::display_width(&version))
            .saturating_add(8)
            <= bar.width
        {
            spans.push(Span::styled(version, Style::default().fg(p.overlay0)));
        }
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
                    Style::default().fg(p.overlay0).bg(p.sidebar_bg),
                );
            }
        }
    }
    covered
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
    fn long_label_drops_name_before_address_and_size() {
        let inner = Rect::new(0, 0, 80, 20);
        let text = label_text(0, "w5:p16", "very long name", inner, 20, true);
        assert!(text.contains("w5:p16"));
        assert!(text.ends_with("80x20"));
        assert!(super::super::render::display_width(&text) <= 20);
    }
}
