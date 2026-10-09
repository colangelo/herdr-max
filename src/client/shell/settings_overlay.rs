use super::*;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};

const SETTINGS_WIDTH: u16 = 76;
const SETTINGS_BASE_HEIGHT: u16 = 22;
/// Most rows the integrations footer may take.
const INTEGRATIONS_FOOTER_MAX_ROWS: u16 = 6;
/// Rows the integrations section spends outside its list and footer.
const INTEGRATIONS_CHROME_ROWS: u16 = 14;

/// The lines under the integrations list: the install messages, else the
/// hint for the list's state.
fn integrations_footer_lines(
    settings: &ClientSettingsOverlay,
    palette: &Palette,
) -> Vec<Line<'static>> {
    let style = Style::default().fg(palette.overlay1).bg(palette.panel_bg);
    let mut lines: Vec<Line<'static>> = settings
        .integration_messages
        .iter()
        .map(|message| Line::from(Span::styled(format!(" {message}"), style)))
        .collect();
    if settings.installing_integrations {
        lines.push(Line::from(Span::styled(" installing…", style)));
    }
    if !lines.is_empty() || settings.loading_integrations {
        return lines;
    }
    let found_any = settings.integrations.iter().any(|integration| {
        integration.available
            || integration.state != crate::api::schema::IntegrationState::NotInstalled
    });
    let hint = if settings
        .integrations
        .iter()
        .any(super::super::settings::integration_needs_install)
    {
        " press install to add available or outdated integrations"
    } else if found_any {
        " all detected integrations are installed"
    } else {
        " no supported agent CLIs found on PATH"
    };
    vec![Line::from(Span::styled(hint, style))]
}

fn integrations_footer_height(
    settings: &ClientSettingsOverlay,
    width: u16,
    palette: &Palette,
) -> u16 {
    let lines = integrations_footer_lines(settings, palette);
    if lines.is_empty() {
        return 0;
    }
    (Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .line_count(width.max(1)) as u16)
        .min(INTEGRATIONS_FOOTER_MAX_ROWS)
}

/// The popup grows with the integrations list and its footer.
fn settings_popup_height(settings: &ClientSettingsOverlay, palette: &Palette) -> u16 {
    if settings.section != ClientSettingsSection::Integrations {
        return SETTINGS_BASE_HEIGHT;
    }
    let list_rows = settings.integrations.len().max(1) as u16;
    let footer_rows = integrations_footer_height(settings, SETTINGS_WIDTH - 2, palette);
    (INTEGRATIONS_CHROME_ROWS + list_rows + footer_rows).max(SETTINGS_BASE_HEIGHT)
}

pub(super) fn render_settings_overlay(
    buffer: &mut Buffer,
    settings: &ClientSettingsOverlay,
    integration_updates_available: bool,
    config: &ClientShellConfig,
    palette: &Palette,
) -> Option<OverlayRender> {
    let popup = popup(
        buffer.area,
        SETTINGS_WIDTH,
        settings_popup_height(settings, palette),
    )?;
    let inner = panel(buffer, popup, palette.accent, palette.panel_bg)?;
    if inner.width < 10 || inner.height < 4 {
        return Some(OverlayRender {
            area: popup,
            ..OverlayRender::default()
        });
    }
    // Title, tabs and rule; a gap; the content; a gap; the hint and buttons.
    let stack = crate::ui::modal_stack_areas(inner, 3, 2, 0, 1);

    put_text(
        buffer,
        stack.header.x,
        stack.header.y,
        stack.header.width,
        " settings",
        Style::default()
            .fg(palette.text)
            .bg(palette.panel_bg)
            .add_modifier(Modifier::BOLD),
    );

    let integration_badge = integration_updates_available
        || settings
            .integrations
            .iter()
            .any(|integration| integration.state == crate::api::schema::IntegrationState::Outdated);
    // The fork's Tabs: a space either side of each label and one between
    // tabs; only the label, with its badge, takes the highlight. The hit rect
    // keeps the padding.
    let tabs_y = stack.header.y + 1;
    let mut tab_x = inner.x;
    let mut tab_hits = Vec::new();
    for section in ClientSettingsSection::ALL {
        let badge = *section == ClientSettingsSection::Integrations && integration_badge;
        let label = section.label();
        let title_width = display_width(label) + if badge { 2 } else { 0 };
        let rect_width = (title_width + 2).min(inner.right().saturating_sub(tab_x));
        if rect_width == 0 {
            break;
        }
        let rect = Rect::new(tab_x, tabs_y, rect_width, 1);
        let active = *section == settings.section;
        let base = Style::default().fg(palette.overlay1).bg(palette.panel_bg);
        buffer.set_style(rect, base);
        let title_rect = Rect::new(
            tab_x + 1,
            tabs_y,
            title_width.min(rect.right().saturating_sub(tab_x + 1)),
            1,
        );
        if badge {
            put_text(
                buffer,
                title_rect.x,
                tabs_y,
                title_rect.width,
                "● ",
                Style::default()
                    .fg(palette.accent)
                    .bg(palette.panel_bg)
                    .add_modifier(Modifier::BOLD),
            );
            put_text(
                buffer,
                title_rect.x + 2,
                tabs_y,
                title_rect.width.saturating_sub(2),
                label,
                base,
            );
        } else {
            put_text(buffer, title_rect.x, tabs_y, title_rect.width, label, base);
        }
        if active {
            buffer.set_style(
                title_rect,
                Style::default()
                    .fg(contrast(palette))
                    .bg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            );
        }
        tab_hits.push((rect, *section));
        tab_x = rect.right().saturating_add(1);
        if tab_x >= inner.right() {
            break;
        }
    }
    put_text(
        buffer,
        stack.header.x,
        stack.header.y + 2,
        stack.header.width,
        &"─".repeat(inner.width as usize),
        Style::default().fg(palette.surface0).bg(palette.panel_bg),
    );

    let content = stack.content;
    let mut choice_hits = Vec::new();
    match settings.section {
        ClientSettingsSection::Theme => {
            render_theme_list(buffer, content, settings, palette, &mut choice_hits);
        }
        ClientSettingsSection::Indicators => {
            render_choice_list(
                buffer,
                content,
                ChoiceList {
                    title: "agent status indicators",
                    description: "choose color dots or distinct symbols for each state",
                    options: &["color dots  ● ● ● ○ ·", "distinct symbols  × ◐ □ ✓ ·"],
                    current: super::super::settings::indicator_index(config.status_indicators),
                    selected: settings.selected,
                    row_height: 1,
                },
                palette,
                &mut choice_hits,
            );
        }
        ClientSettingsSection::Sound => {
            render_choice_list(
                buffer,
                content,
                ChoiceList {
                    title: "sound alerts",
                    description: "play sounds when agents change state in background",
                    options: &["on", "off"],
                    current: usize::from(!config.sound_enabled),
                    selected: settings.selected,
                    row_height: 1,
                },
                palette,
                &mut choice_hits,
            );
        }
        ClientSettingsSection::Toast => {
            render_choice_list(
                buffer,
                content,
                ChoiceList {
                    title: "notification popups",
                    description: "choose where background popup notifications should appear",
                    options: &["off", "inside herdr", "via terminal", "via system"],
                    current: super::super::settings::toast_index(config.toast_delivery),
                    selected: settings.selected,
                    row_height: 2,
                },
                palette,
                &mut choice_hits,
            );
        }
        ClientSettingsSection::Integrations => {
            render_integrations(buffer, content, settings, palette);
        }
    }

    let installable = settings
        .integrations
        .iter()
        .any(super::super::settings::integration_needs_install);
    let show_primary = settings.section != ClientSettingsSection::Integrations || installable;
    let mut buttons = Vec::new();
    if show_primary {
        buttons.push(ModalButton::primary(
            "↵",
            if settings.section == ClientSettingsSection::Integrations {
                "install"
            } else {
                "apply"
            },
            palette.accent,
        ));
    }
    buttons.push(ModalButton::secondary("esc", "close"));
    let rects = render_button_row(
        buffer,
        inner,
        &buttons,
        BUTTON_GAP,
        inner.height.saturating_sub(1),
        palette,
    );
    let (primary, close) = if show_primary {
        (rects[0], rects[1])
    } else {
        (Rect::default(), rects[0])
    };
    if let Some(footer) = stack.footer {
        let hint = Style::default().bg(palette.panel_bg);
        Widget::render(
            Paragraph::new(Line::from(vec![
                Span::styled(" ↑↓", hint.fg(palette.overlay0)),
                Span::styled(" select  ", hint.fg(palette.overlay1)),
                Span::styled("tab", hint.fg(palette.overlay0)),
                Span::styled(" section", hint.fg(palette.overlay1)),
            ])),
            Rect::new(footer.x, footer.y, footer.width, 1),
            buffer,
        );
    }

    Some(OverlayRender {
        area: popup,
        primary,
        cancel: close,
        settings_popup: popup,
        settings_tabs: tab_hits,
        settings_choices: choice_hits,
        ..OverlayRender::default()
    })
}

/// Every theme, the selection marked `▸` and highlighted across the row, the
/// saved one marked `✓` in green. The list scrolls so the selection is its
/// last visible row once it passes the bottom.
fn render_theme_list(
    buffer: &mut Buffer,
    area: Rect,
    settings: &ClientSettingsOverlay,
    palette: &Palette,
    hits: &mut Vec<(Rect, usize)>,
) {
    let visible = usize::from(area.height);
    let scroll = settings.selected.saturating_sub(visible.saturating_sub(1));
    let saved = super::super::settings::normalized_theme_name(&settings.original_theme_name);
    for (visible_index, (index, name)) in crate::config::THEME_NAMES
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible)
        .enumerate()
    {
        let rect = Rect::new(area.x, area.y + visible_index as u16, area.width, 1);
        let selected = index == settings.selected;
        let current = super::super::settings::normalized_theme_name(name) == saved;
        let name_style = Style::default().fg(palette.subtext0).bg(palette.panel_bg);
        if selected {
            // The highlight overrides the ✓'s green too.
            let style = Style::default()
                .fg(palette.text)
                .bg(palette.surface0)
                .add_modifier(Modifier::BOLD);
            buffer.set_style(rect, style);
            let marker = if current { " ✓" } else { "" };
            put_text(
                buffer,
                rect.x,
                rect.y,
                rect.width,
                &format!(" ▸ {name}{marker}"),
                style,
            );
        } else {
            let x = put_segment(
                buffer,
                rect.x,
                rect.y,
                rect.right(),
                &format!("   {name}"),
                name_style,
            );
            if current {
                put_segment(
                    buffer,
                    x,
                    rect.y,
                    rect.right(),
                    " ✓",
                    Style::default().fg(palette.green).bg(palette.panel_bg),
                );
            }
        }
        hits.push((rect, index));
    }
}

struct ChoiceList<'a> {
    title: &'a str,
    description: &'a str,
    options: &'a [&'a str],
    /// The saved option, marked `✓`.
    current: usize,
    selected: usize,
    /// 1, or 2 for a spaced-out list; the label sits on the first row and the
    /// highlight covers both.
    row_height: u16,
}

/// The fork's choice list: the description over two rows, a blank row, then
/// one row per option as ` {title}: {option}`, `✓` on the saved one.
fn render_choice_list(
    buffer: &mut Buffer,
    area: Rect,
    list: ChoiceList<'_>,
    palette: &Palette,
    hits: &mut Vec<(Rect, usize)>,
) {
    Widget::render(
        Paragraph::new(format!(" {}", list.description))
            .style(Style::default().fg(palette.overlay1).bg(palette.panel_bg))
            .wrap(Wrap { trim: false }),
        Rect::new(area.x, area.y, area.width, area.height.min(2)),
        buffer,
    );
    let mut y = area.y.saturating_add(3);
    for (index, option) in list.options.iter().enumerate() {
        if y >= area.bottom() {
            break;
        }
        let height = list.row_height.min(area.bottom() - y);
        let rect = Rect::new(area.x, y, area.width, height);
        let style = if index == list.selected {
            Style::default()
                .fg(palette.text)
                .bg(palette.surface0)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.subtext0).bg(palette.panel_bg)
        };
        buffer.set_style(rect, style);
        let marker = if index == list.current { " ✓" } else { "" };
        put_text(
            buffer,
            rect.x,
            rect.y,
            rect.width,
            &format!(" {}: {option}{marker}", list.title),
            style,
        );
        hits.push((rect, index));
        y = y.saturating_add(list.row_height);
    }
}

fn render_integrations(
    buffer: &mut Buffer,
    area: Rect,
    settings: &ClientSettingsOverlay,
    palette: &Palette,
) {
    let footer_height = integrations_footer_height(settings, area.width, palette);
    put_text(
        buffer,
        area.x,
        area.y,
        area.width,
        "agent integrations",
        Style::default()
            .fg(palette.text)
            .bg(palette.panel_bg)
            .add_modifier(Modifier::BOLD),
    );
    Widget::render(
        Paragraph::new("enable session restore and, where supported, direct status updates")
            .style(Style::default().fg(palette.overlay1).bg(palette.panel_bg))
            .wrap(Wrap { trim: false }),
        Rect::new(
            area.x,
            area.y + 1,
            area.width,
            area.height.saturating_sub(1).min(2),
        ),
        buffer,
    );
    // Title 1, description 2, blank 1; then the list; a blank and the footer.
    let list_y = area.y.saturating_add(4);
    let footer_y = area.bottom().saturating_sub(footer_height);
    let list_bottom = footer_y.saturating_sub(1).max(list_y).min(area.bottom());
    let muted = Style::default().fg(palette.overlay1).bg(palette.panel_bg);
    if settings.integrations.is_empty() {
        put_text(
            buffer,
            area.x,
            list_y,
            area.width,
            if settings.loading_integrations {
                " loading integrations…"
            } else {
                " no integration targets available"
            },
            muted,
        );
    }
    for (index, integration) in settings.integrations.iter().enumerate() {
        let y = list_y + index as u16;
        if y >= list_bottom {
            break;
        }
        let (marker, color, status) = match integration.state {
            crate::api::schema::IntegrationState::Current => ("✓", palette.green, "installed"),
            crate::api::schema::IntegrationState::Outdated => {
                ("↻", palette.yellow, "update available")
            }
            crate::api::schema::IntegrationState::NotInstalled if integration.available => {
                ("+", palette.accent, "available")
            }
            crate::api::schema::IntegrationState::NotInstalled => {
                ("–", palette.overlay0, "not found")
            }
        };
        let x = put_segment(
            buffer,
            area.x,
            y,
            area.right(),
            &format!(" {marker} "),
            Style::default().fg(color).bg(palette.panel_bg),
        );
        let x = put_segment(
            buffer,
            x,
            y,
            area.right(),
            &format!("{:<9}", integration.label),
            Style::default().fg(palette.subtext0).bg(palette.panel_bg),
        );
        put_segment(buffer, x, y, area.right(), status, muted);
    }
    if footer_height > 0 {
        Widget::render(
            Paragraph::new(integrations_footer_lines(settings, palette)).wrap(Wrap { trim: false }),
            Rect::new(area.x, footer_y, area.width, footer_height),
            buffer,
        );
    }
}
