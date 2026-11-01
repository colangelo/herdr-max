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
                        | ClientChromeDrag::PaneLabelPeek
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
            if let Some(pane) = self.hits.panes.get((digit as usize) - ('1' as usize)) {
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

/// One piece of a pane label, styled by [`label_spans`] (fork issue 263).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LabelPart {
    /// The digit that focuses the pane in the numbered view.
    Digit,
    /// The pane's title: first, in the clipboard toast's green.
    Name,
    /// The pane's inner size, a reversed chip like the sidebar's size badges.
    Size,
    /// The pane id, plain grey.
    Id,
}

const LABEL_GAP: &str = "  ";

/// A pane label's parts in order: [digit] name, size, id. When they do not fit
/// `maximum` cells the name is shortened first, then the id is dropped, then
/// the name; the size always stays.
fn label_parts(
    index: usize,
    address: &str,
    name: &str,
    inner: Rect,
    maximum: usize,
    numbered: bool,
) -> Vec<(LabelPart, String)> {
    let digit = numbered.then(|| {
        if index < 9 {
            (index + 1).to_string()
        } else {
            "·".into()
        }
    });
    let size = format!(" {}x{} ", inner.width, inner.height);
    let width = |parts: &[(LabelPart, String)]| {
        parts
            .iter()
            .map(|(_, text)| usize::from(super::render::display_width(text)))
            .sum::<usize>()
            + LABEL_GAP.len() * parts.len().saturating_sub(1)
    };
    let build = |name: Option<String>, id: bool| {
        let mut parts = Vec::new();
        if let Some(digit) = &digit {
            parts.push((LabelPart::Digit, digit.clone()));
        }
        if let Some(name) = name {
            parts.push((LabelPart::Name, name));
        }
        parts.push((LabelPart::Size, size.clone()));
        if id {
            parts.push((LabelPart::Id, address.to_owned()));
        }
        parts
    };
    let full = build(Some(name.to_owned()), true);
    if width(&full) <= maximum {
        return full;
    }
    for id in [true, false] {
        let without_name = width(&build(None, id)) + LABEL_GAP.len();
        let budget = maximum.saturating_sub(without_name);
        if budget >= 4 {
            return build(Some(crate::ui::truncate_end(name, budget).to_string()), id);
        }
    }
    build(None, false)
}

/// The label's spans: the name bold in the toast's green, the size reversed in
/// the mode's colour, the id plain grey, the digit a key in the mode's colour.
fn label_spans(parts: Vec<(LabelPart, String)>, mode_colour: Color, p: &Palette) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, (part, text)) in parts.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(LABEL_GAP, Style::default().bg(p.panel_bg)));
        }
        let style = match part {
            LabelPart::Digit => Style::default()
                .fg(mode_colour)
                .bg(p.panel_bg)
                .add_modifier(Modifier::BOLD),
            LabelPart::Name => Style::default()
                .fg(p.green)
                .bg(p.panel_bg)
                .add_modifier(Modifier::BOLD),
            LabelPart::Size => Style::default()
                .fg(panel_contrast_fg(p))
                .bg(mode_colour)
                .add_modifier(Modifier::BOLD),
            LabelPart::Id => Style::default().fg(p.overlay1).bg(p.panel_bg),
        };
        spans.push(Span::styled(text, style));
    }
    Line::from(spans)
}

#[cfg(test)]
fn label_text(
    index: usize,
    address: &str,
    name: &str,
    inner: Rect,
    maximum: usize,
    numbered: bool,
) -> String {
    label_parts(index, address, name, inner, maximum, numbered)
        .into_iter()
        .map(|(_, text)| text)
        .collect::<Vec<_>>()
        .join(LABEL_GAP)
}

fn connected_version<'a>(
    state: &'a ClientShellState,
    snapshot: &'a ClientShellSnapshot,
) -> &'a str {
    // Welcome is the compatibility-floor source. A current, coherent optional
    // fact also covers a live handoff that preserves an older client's socket.
    snapshot
        .resource_facts
        .as_ref()
        .and_then(|facts| facts.server_version.as_deref())
        .or_else(|| {
            state
                .endpoint_versions
                .get(&state.active_endpoint_id)
                .map(String::as_str)
        })
        .unwrap_or("unknown")
}

fn pane_name<'a>(snapshot: &'a ClientShellSnapshot, pane_id: &str) -> &'a str {
    if let Some(name) = snapshot
        .resource_facts
        .as_ref()
        .and_then(|facts| facts.pane_names.as_ref())
        .and_then(|names| names.get(pane_id))
    {
        return name;
    }
    // Older endpoints retain the closest label available in their baseline fields.
    let agent = snapshot
        .agents
        .iter()
        .find(|agent| agent.pane_id == pane_id);
    agent
        .and_then(|agent| agent.title.as_deref())
        .or_else(|| {
            snapshot
                .panes
                .iter()
                .find(|pane| pane.pane_id == pane_id)
                .and_then(|pane| pane.label.as_deref())
        })
        .or_else(|| agent.and_then(|agent| agent.terminal_title_stripped.as_deref()))
        .or_else(|| agent.and_then(|agent| agent.name.as_deref()))
        .or_else(|| {
            agent.and_then(|agent| agent.display_agent.as_deref().or(agent.agent.as_deref()))
        })
        .unwrap_or("terminal")
}

pub(super) fn paint(
    buffer: &mut Buffer,
    layout: ClientShellLayout,
    state: &ClientShellState,
    snapshot: &ClientShellSnapshot,
) -> Vec<Rect> {
    let mut covered = Vec::new();
    let p = &state.config.palette;
    let resize = state.mode == ClientShellMode::Resize;
    // PANES is red and RESIZE mauve: each mode paints its chip, its keys and its size badges in its
    // own colour (docs/ui-style.md, Mode colour).
    let mode_colour = if resize { p.mauve } else { p.red };
    // The surface preserves the server tile traversal used by the fork's
    // pane_infos. Geometric sorting changes numbering for mixed column splits.
    let panes = &state.hits.panes;
    for (index, pane) in panes.iter().enumerate() {
        let inner = pane.inner_rect.intersection(buffer.area);
        if inner.is_empty() {
            continue;
        }
        let name = pane_name(snapshot, &pane.pane_id);
        let boxed = inner.height >= 3 && inner.width > 4;
        // Room for the empty ring outside the border too: a row above and below, two columns
        // on each side (fork issues 263, 264).
        let ringed = inner.height >= 5 && inner.width > 8;
        let chrome = if ringed { 8 } else { 4 };
        let parts = label_parts(
            index,
            &pane.pane_id,
            name,
            inner,
            usize::from(if boxed {
                inner.width - chrome
            } else {
                inner.width
            }),
            state.pane_labels_explicit,
        );
        let line = label_spans(parts, mode_colour, p);
        let width = u16::try_from(line.width())
            .unwrap_or(u16::MAX)
            .min(inner.width);
        let active = snapshot.focused_pane_id.as_deref() == Some(pane.pane_id.as_str());
        let border = if active { p.red } else { p.muted_red() };
        let rect = if boxed {
            let box_width = (width + 4).min(inner.width);
            Rect::new(
                inner.x + (inner.width - box_width) / 2,
                inner.y + (inner.height - 3) / 2,
                box_width,
                3,
            )
        } else {
            Rect::new(inner.x, inner.y + inner.height / 2, width, 1)
        };
        let drawn = if ringed {
            crate::ui::outer_ring(rect, inner)
        } else {
            rect
        };
        covered.push(drawn);
        crate::ui::clear_ring(buffer, drawn, p);
        if boxed {
            let block = Block::default()
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1))
                .border_style(Style::default().fg(border))
                .style(Style::default().bg(p.panel_bg));
            let text_area = block.inner(rect);
            block.render(rect, buffer);
            Paragraph::new(line).render(text_area, buffer);
        } else {
            Paragraph::new(line).render(rect, buffer);
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
        let chip = Style::default()
            .fg(if p.panel_bg == Color::Reset {
                p.surface_dim
            } else {
                p.panel_bg
            })
            .bg(mode_colour)
            .add_modifier(Modifier::BOLD);
        let version = connected_version(state, snapshot);
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
                        .bg(mode_colour)
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
    // The keys take the chip's colour: the mode's own.
    let key = Style::default()
        .fg(chip.bg.unwrap_or(p.red))
        .add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(p.overlay0);
    // The sizes are chips in the mode's colour, like the size in each pane box, and the version a
    // green chip (fork issue 264).
    let version_chip = Style::default()
        .fg(panel_contrast_fg(p))
        .bg(p.green)
        .add_modifier(Modifier::BOLD);
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
        Span::styled(format!(" {}x{} ", window.width, window.height), chip),
        Span::styled(" · panes ", dim),
        Span::styled(format!(" {}x{} ", panes.width, panes.height), chip),
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
        Span::styled("version", dim),
        Span::raw(" "),
        Span::styled(format!(" {version} "), version_chip),
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
    fn connected_version_uses_welcome_and_current_handoff_fact() {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        state.set_endpoint_server_version(&ClientEndpointId::Local, Some("welcome-build".into()));
        let mut snapshot = super::super::tests::snapshot();
        assert_eq!(connected_version(&state, &snapshot), "welcome-build");
        snapshot.resource_facts = Some(crate::protocol::ClientShellResourceFacts {
            server_version: Some("handoff-build".into()),
            ..Default::default()
        });
        assert_eq!(connected_version(&state, &snapshot), "handoff-build");
    }

    #[test]
    fn pane_labels_use_the_canonical_session_name_fact() {
        let mut snapshot = super::super::tests::snapshot();
        snapshot.resource_facts = Some(crate::protocol::ClientShellResourceFacts {
            pane_names: Some([("pane_1".into(), "reported title".into())].into()),
            ..Default::default()
        });
        assert_eq!(pane_name(&snapshot, "pane_1"), "reported title");
        snapshot.resource_facts = None;
        snapshot.panes[0].label = Some("assigned".into());
        assert_eq!(pane_name(&snapshot, "pane_1"), "assigned");
    }

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
    fn summary_has_mode_chips_for_the_mode_and_sizes_and_a_green_version_chip() {
        // Fork issue 174: keys in the mode's colour, `version` a dim label. Fork issue 264: the
        // window and panes sizes are chips in the mode's colour, the version a green chip.
        let config = ClientShellConfig::from_config(&Config::default());
        let p = &config.palette;
        let chip = |colour| {
            Style::default()
                .fg(panel_contrast_fg(p))
                .bg(colour)
                .add_modifier(Modifier::BOLD)
        };
        let spans = summary_spans(
            Rect::new(0, 0, 140, 30),
            Rect::new(26, 0, 114, 29),
            2,
            true,
            false,
            "server-build",
            p,
            chip(p.red),
            140,
        );
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.starts_with(" PANES "), "{text}");
        assert!(
            text.ends_with("any key close  version  server-build "),
            "{text}"
        );
        assert!(text.contains("window  140x30  · panes  114x29 "), "{text}");
        let filled: Vec<&str> = spans
            .iter()
            .filter(|s| s.style.bg == Some(p.red))
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(filled, [" PANES ", " 140x30 ", " 114x29 "]);
        let find = |spans: &[Span<'_>], text: &str| {
            spans
                .iter()
                .find(|s| s.content == text)
                .unwrap_or_else(|| panic!("{text:?}"))
                .style
        };
        assert_eq!(find(&spans, "any key").fg, Some(p.red));
        assert_eq!(find(&spans, "version").fg, Some(p.overlay0));
        assert_eq!(find(&spans, " server-build ").bg, Some(p.green));
        assert_eq!(
            find(&spans, " server-build ").fg,
            Some(panel_contrast_fg(p))
        );
        let resize = summary_spans(
            Rect::new(0, 0, 140, 30),
            Rect::new(26, 0, 114, 29),
            2,
            false,
            true,
            "server-build",
            p,
            chip(p.mauve),
            140,
        );
        assert_eq!(find(&resize, "h/l").fg, Some(p.mauve));
        assert_eq!(find(&resize, "esc").fg, Some(p.mauve));
        assert_eq!(
            resize
                .iter()
                .filter(|s| s.style.bg == Some(p.mauve))
                .count(),
            3,
            "the RESIZE chip and the two sizes"
        );
        let narrow = summary_spans(
            Rect::new(0, 0, 70, 30),
            Rect::new(26, 0, 44, 29),
            2,
            true,
            false,
            "server-build",
            p,
            chip(p.red),
            70,
        );
        let text: String = narrow.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.ends_with("any key close"), "{text}");
        assert!(!text.contains("version"), "the version drops first: {text}");
        let passive = summary_spans(
            Rect::new(0, 0, 140, 30),
            Rect::new(26, 0, 114, 29),
            2,
            false,
            false,
            "server-build",
            p,
            chip(p.red),
            140,
        );
        assert!(!passive.iter().any(|s| s.content.contains("focus")));
    }

    #[test]
    fn label_reads_name_size_id_and_shortens_the_name_then_drops_the_id() {
        // Fork issue 263: title first, then the size chip, then the pane id.
        let inner = Rect::new(0, 0, 80, 20);
        let parts = label_parts(0, "w5:p16", "herdr", inner, 80, false);
        assert_eq!(
            parts,
            vec![
                (LabelPart::Name, "herdr".to_owned()),
                (LabelPart::Size, " 80x20 ".to_owned()),
                (LabelPart::Id, "w5:p16".to_owned()),
            ]
        );
        assert_eq!(
            label_text(0, "w5:p16", "herdr", inner, 80, true),
            "1  herdr   80x20   w5:p16"
        );

        let text = label_text(0, "w5:p16", "very long name", inner, 20, true);
        assert!(text.contains(" 80x20 "), "the size always stays: {text}");
        assert!(
            !text.contains("w5:p16"),
            "the id goes before the name: {text}"
        );
        assert!(super::super::render::display_width(&text) <= 20);
    }

    #[test]
    fn label_styles_name_green_size_reversed_id_plain_grey() {
        let p = Palette::catppuccin();
        let line = label_spans(
            label_parts(0, "w5:pV", "herdr", Rect::new(0, 0, 138, 52), 80, false),
            p.red,
            &p,
        );
        let style_of = |text: &str| {
            line.spans
                .iter()
                .find(|span| span.content == text)
                .map(|span| span.style)
                .expect(text)
        };
        assert_eq!(style_of("herdr").fg, Some(p.green));
        assert!(style_of("herdr").add_modifier.contains(Modifier::BOLD));
        assert_eq!(style_of(" 138x52 ").bg, Some(p.red));
        assert_eq!(style_of("w5:pV").fg, Some(p.overlay1));
        assert!(!style_of("w5:pV").add_modifier.contains(Modifier::BOLD));
    }
}
