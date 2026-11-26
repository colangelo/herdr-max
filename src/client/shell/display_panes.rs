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
    /// Info mode (fork issue 331): the numbered labels for `ui.display_panes_ms`, keys closing
    /// them. Resize mode is its sibling: the same labels, kept until it is left. Leaving resize
    /// for it goes back to the mode `esc` would, copy mode included.
    pub(super) fn enter_info_mode(&mut self, now: std::time::Instant) {
        self.arm_pane_labels(true, now);
        self.mode = if self.mode == ClientShellMode::Resize {
            self.copy_or_terminal_mode()
        } else {
            ClientShellMode::Terminal
        };
        self.resize_focus = None;
    }
    /// Resize mode: the same labels as info mode with no deadline; `i` goes back to info mode.
    pub(super) fn enter_resize_mode(&mut self) {
        self.close_pane_labels();
        self.mode = ClientShellMode::Resize;
        self.resize_focus = None;
    }
    /// The pane the digit `digit` names in the numbered labels.
    fn numbered_pane(&self, digit: char) -> Option<crate::api::schema::PaneTarget> {
        let index = (digit as usize).checked_sub('1' as usize)?;
        self.hits
            .panes
            .get(index)
            .map(|pane| crate::api::schema::PaneTarget {
                pane_id: pane.pane_id.clone(),
            })
    }
    /// A key in resize mode that belongs to the labels: a digit focuses that pane and stays in
    /// resize mode, `i` goes to info mode. Returns whether the key was taken.
    pub(super) fn resize_label_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) -> bool {
        match key.code {
            KeyCode::Char(digit @ '1'..='9') => {
                if let Some(target) = self.numbered_pane(digit) {
                    self.resize_focus = Some(target.pane_id.clone());
                    self.push_endpoint_method(
                        crate::api::schema::Method::PaneFocus(target),
                        outcome,
                    );
                }
                outcome.repaint = true;
                true
            }
            KeyCode::Char('i') => {
                self.enter_info_mode(std::time::Instant::now());
                outcome.repaint = true;
                true
            }
            _ => false,
        }
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
        if key.code == KeyCode::Char('r') {
            self.enter_resize_mode();
            outcome.repaint = true;
            return true;
        }
        if let KeyCode::Char(digit @ '1'..='9') = key.code {
            if let Some(target) = self.numbered_pane(digit) {
                self.push_endpoint_method(crate::api::schema::Method::PaneFocus(target), outcome);
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
    /// The herdr-max version, plain green like the bar's version chip (fork issue 331).
    Version,
}

const LABEL_GAP: &str = "  ";

/// A pane label's parts in order: [digit] name, size, version, id. When they do not fit
/// `maximum` cells the name is shortened first, then the id is dropped, then the name, then the
/// version; the size always stays.
fn label_parts(
    index: usize,
    address: &str,
    name: &str,
    version: &str,
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
    let build = |name: Option<String>, id: bool, with_version: bool| {
        let mut parts = Vec::new();
        if let Some(digit) = &digit {
            parts.push((LabelPart::Digit, digit.clone()));
        }
        if let Some(name) = name {
            parts.push((LabelPart::Name, name));
        }
        parts.push((LabelPart::Size, size.clone()));
        if with_version {
            parts.push((LabelPart::Version, version.to_owned()));
        }
        if id {
            parts.push((LabelPart::Id, address.to_owned()));
        }
        parts
    };
    let full = build(Some(name.to_owned()), true, true);
    if width(&full) <= maximum {
        return full;
    }
    for id in [true, false] {
        let without_name = width(&build(None, id, true)) + LABEL_GAP.len();
        let budget = maximum.saturating_sub(without_name);
        if budget >= 4 {
            return build(
                Some(crate::ui::truncate_end(name, budget).to_string()),
                id,
                true,
            );
        }
    }
    let bare = build(None, false, true);
    if width(&bare) <= maximum {
        bare
    } else {
        build(None, false, false)
    }
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
                .fg(p.fill_foreground())
                .bg(mode_colour)
                .add_modifier(Modifier::BOLD),
            LabelPart::Id => Style::default().fg(p.overlay1).bg(p.panel_bg),
            LabelPart::Version => Style::default().fg(p.green).bg(p.panel_bg),
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
    version: &str,
    inner: Rect,
    maximum: usize,
    numbered: bool,
) -> String {
    label_parts(index, address, name, version, inner, maximum, numbered)
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

/// `colour` mixed halfway to its own grey, like [`Palette::muted_red`]; `fallback` when the
/// terminal names the colour rather than gives it as RGB.
fn muted(colour: Color, fallback: Color) -> Color {
    match colour {
        Color::Rgb(r, g, b) => {
            let grey = (0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b)).round();
            let mix = |channel: u8| ((f32::from(channel) + grey) / 2.0).round() as u8;
            Color::Rgb(mix(r), mix(g), mix(b))
        }
        _ => fallback,
    }
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
    // Info mode and resize mode are one overlay (fork issue 331): the same number, size and
    // version labels, and only the colour tells them apart. PANES is red and RESIZE violet:
    // each mode paints its chip, its keys, its size badges and its borders in its own colour
    // (docs/ui-style.md, Mode colour).
    let numbered = state.pane_labels_explicit || resize;
    let mode_colour = if resize { p.resize_violet() } else { p.red };
    let muted_colour = if resize {
        muted(mode_colour, p.overlay1)
    } else {
        p.muted_red()
    };
    let version = connected_version(state, snapshot);
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
            version,
            inner,
            usize::from(if boxed {
                inner.width - chrome
            } else {
                inner.width
            }),
            numbered,
        );
        let line = label_spans(parts, mode_colour, p);
        let width = u16::try_from(line.width())
            .unwrap_or(u16::MAX)
            .min(inner.width);
        let active = snapshot.focused_pane_id.as_deref() == Some(pane.pane_id.as_str());
        let border = if active { mode_colour } else { muted_colour };
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
        if numbered {
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
                        .fg(p.fill_foreground())
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
    let mode = chip.bg.unwrap_or(p.red);
    let key = Style::default().fg(mode).add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(p.overlay0);
    // Fork issue 268: the size labels in the mode's colour and the version label in green, plain;
    // the sizes are chips in the mode's colour and the version a green chip (fork issue 264).
    let label = Style::default().fg(mode);
    let version_label = Style::default().fg(p.green);
    let version_chip = Style::default()
        .fg(p.fill_foreground())
        .bg(p.green)
        .add_modifier(Modifier::BOLD);
    let mut spans = vec![
        Span::styled(if resize { " RESIZE " } else { " SIZES " }, chip),
        Span::raw("  "),
        Span::styled("window ", label),
        Span::styled(format!(" {}x{} ", window.width, window.height), chip),
        Span::styled(" · ", dim),
        Span::styled("panes ", label),
        Span::styled(format!(" {}x{} ", panes.width, panes.height), chip),
    ];
    // The key hints come last, after the version, a little apart from it (fork issue 268).
    let mut hints = Vec::new();
    if resize {
        hints.extend([
            Span::styled("h/l", key),
            Span::styled(" width  ", dim),
            Span::styled("j/k", key),
            Span::styled(" height  ", dim),
        ]);
        if count > 0 {
            let last = count.min(9);
            hints.extend([
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
        hints.extend([
            Span::styled("i", key),
            Span::styled(" info  ", dim),
            Span::styled("esc", key),
            Span::styled(" done", dim),
        ]);
    } else if explicit {
        if count > 0 {
            let last = count.min(9);
            hints.extend([
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
        hints.extend([
            Span::styled("r", key),
            Span::styled(" resize  ", dim),
            Span::styled("any key", key),
            Span::styled(" close", dim),
        ]);
    }
    let version_spans = [
        Span::raw("  "),
        Span::styled("version", version_label),
        Span::raw(" "),
        Span::styled(format!(" {version} "), version_chip),
    ];
    let gap = if hints.is_empty() { "" } else { "    " };
    let total = |parts: &[Span<'_>]| parts.iter().map(Span::width).sum::<usize>();
    // Keep the hints on narrow bars, dropping the version first, as in the fork.
    if total(&spans) + total(&version_spans) + gap.len() + total(&hints) <= usize::from(width) {
        spans.extend(version_spans);
        spans.push(Span::raw(gap));
    } else if !hints.is_empty() {
        spans.push(Span::raw("  "));
    }
    spans.extend(hints);
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
                .fg(p.fill_foreground())
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
        // Fork issue 268: SIZES, then the sizes, the version, and the key hints last.
        assert!(
            text.starts_with(" SIZES   window  140x30  · panes  114x29 "),
            "{text}"
        );
        assert!(
            text.ends_with("  version  server-build     1-2 focus  r resize  any key close"),
            "{text}"
        );
        let filled: Vec<&str> = spans
            .iter()
            .filter(|s| s.style.bg == Some(p.red))
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(filled, [" SIZES ", " 140x30 ", " 114x29 "]);
        let find = |spans: &[Span<'_>], text: &str| {
            spans
                .iter()
                .find(|s| s.content == text)
                .unwrap_or_else(|| panic!("{text:?}"))
                .style
        };
        assert_eq!(find(&spans, "any key").fg, Some(p.red));
        for (text, colour) in [("window ", p.red), ("panes ", p.red), ("version", p.green)] {
            let style = find(&spans, text);
            assert_eq!(style.fg, Some(colour), "{text:?}");
            assert!(
                !style.add_modifier.contains(Modifier::BOLD),
                "{text:?} is not bold"
            );
        }
        assert_eq!(find(&spans, " server-build ").bg, Some(p.green));
        assert_eq!(find(&spans, " server-build ").fg, Some(p.fill_foreground()));
        let resize = summary_spans(
            Rect::new(0, 0, 140, 30),
            Rect::new(26, 0, 114, 29),
            2,
            false,
            true,
            "server-build",
            p,
            chip(p.resize_violet()),
            140,
        );
        assert_eq!(find(&resize, "h/l").fg, Some(p.resize_violet()));
        assert_eq!(find(&resize, "esc").fg, Some(p.resize_violet()));
        let text: String = resize.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.starts_with(" RESIZE   window "), "{text}");
        assert!(
            text.ends_with("1-2 focus  i info  esc done"),
            "the hints come last: {text}"
        );
        assert_eq!(
            resize
                .iter()
                .filter(|s| s.style.bg == Some(p.resize_violet()))
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
        let parts = label_parts(0, "w5:p16", "herdr", "0.11.0", inner, 80, false);
        assert_eq!(
            parts,
            vec![
                (LabelPart::Name, "herdr".to_owned()),
                (LabelPart::Size, " 80x20 ".to_owned()),
                (LabelPart::Version, "0.11.0".to_owned()),
                (LabelPart::Id, "w5:p16".to_owned()),
            ]
        );
        assert_eq!(
            label_text(0, "w5:p16", "herdr", "0.11.0", inner, 80, true),
            "1  herdr   80x20   0.11.0  w5:p16"
        );

        let text = label_text(0, "w5:p16", "very long name", "0.11.0", inner, 30, true);
        assert!(text.contains(" 80x20 "), "the size always stays: {text}");
        assert!(text.contains("0.11.0"), "the version stays: {text}");
        assert!(
            !text.contains("w5:p16"),
            "the id goes before the name: {text}"
        );
        assert!(super::super::render::display_width(&text) <= 30);
    }

    #[test]
    fn label_styles_name_green_size_reversed_id_plain_grey() {
        let p = Palette::catppuccin();
        let line = label_spans(
            label_parts(
                0,
                "w5:pV",
                "herdr",
                "0.11.0",
                Rect::new(0, 0, 138, 52),
                80,
                false,
            ),
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
