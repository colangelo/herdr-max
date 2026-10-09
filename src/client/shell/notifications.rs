#[cfg(test)]
use super::notification_policy::COMPLETION_RECHECK_INTERVAL;
use super::*;
use ratatui::{
    style::Color,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Padding, Paragraph, Widget},
};

pub(super) fn render_mobile_notice_banner(
    buffer: &mut Buffer,
    area: Rect,
    title: &str,
    body: Option<&str>,
    dot_color: Color,
    offset_for_warning: bool,
    palette: &Palette,
) -> Rect {
    if area.is_empty() {
        return Rect::default();
    }
    let warning_offset = u16::from(offset_for_warning);
    let y = area.y
        + area
            .height
            .saturating_sub(1u16.saturating_add(warning_offset));
    let rect = Rect::new(area.x, y, area.width, 1);
    let background = palette.surface0;
    Clear.render(rect, buffer);
    buffer.set_style(rect, Style::default().bg(background));
    let mut x = rect.x;
    for (text, style) in [
        (" ", Style::default().bg(background)),
        ("●", Style::default().fg(dot_color).bg(background)),
        (" ", Style::default().bg(background)),
        (
            title,
            Style::default()
                .fg(palette.text)
                .bg(background)
                .add_modifier(Modifier::BOLD),
        ),
    ] {
        x = super::render::put_segment(buffer, x, rect.y, rect.right(), text, style);
    }
    if let Some(body) = body.filter(|body| !body.is_empty()) {
        x = super::render::put_segment(
            buffer,
            x,
            rect.y,
            rect.right(),
            " · ",
            Style::default().fg(palette.overlay0).bg(background),
        );
        super::render::put_text(
            buffer,
            x,
            rect.y,
            rect.right().saturating_sub(x),
            body,
            Style::default().fg(palette.overlay0).bg(background),
        );
    }
    rect
}

pub(super) fn render_mobile_notification_banner(
    buffer: &mut Buffer,
    area: Rect,
    notification: &ClientVisibleNotification,
    offset_for_warning: bool,
    palette: &Palette,
) -> Rect {
    let event = &notification.event;
    let title = match event.kind {
        SemanticNotificationKind::NeedsAttention => event
            .title
            .strip_suffix(" needs attention")
            .map(|agent| format!("{agent} waiting"))
            .unwrap_or_else(|| event.title.clone()),
        SemanticNotificationKind::Finished => event
            .title
            .strip_suffix(" finished")
            .map(|agent| format!("{agent} done"))
            .unwrap_or_else(|| event.title.clone()),
        SemanticNotificationKind::UpdateInstalled => "update ready".to_owned(),
        SemanticNotificationKind::Custom => event.title.clone(),
    };
    let dot_color = match event.kind {
        SemanticNotificationKind::NeedsAttention => palette.red,
        SemanticNotificationKind::Finished => palette.blue,
        SemanticNotificationKind::UpdateInstalled | SemanticNotificationKind::Custom => {
            palette.accent
        }
    };
    render_mobile_notice_banner(
        buffer,
        area,
        &title,
        event.body.as_deref(),
        dot_color,
        offset_for_warning,
        palette,
    )
}

pub(super) fn render_notification_card(
    buffer: &mut Buffer,
    area: Rect,
    title: &str,
    body: &str,
    position: crate::config::ToastHerdrPosition,
    top_offset: u16,
    dot_color: Color,
    palette: &Palette,
) -> Rect {
    render_notification_card_with_size(
        buffer,
        area,
        title,
        body,
        position,
        top_offset,
        dot_color,
        palette,
        crate::config::ToastHerdrSize::Auto,
    )
}

fn render_notification_card_with_size(
    buffer: &mut Buffer,
    area: Rect,
    title: &str,
    body: &str,
    position: crate::config::ToastHerdrPosition,
    top_offset: u16,
    dot_color: Color,
    palette: &Palette,
    size: crate::config::ToastHerdrSize,
) -> Rect {
    if area.is_empty() {
        return Rect::default();
    }
    let (natural_width, natural_height) = notification_size(title, body, size, area.width);
    let width = natural_width.min(area.width);
    let height = natural_height.min(area.height);
    let x = match position {
        crate::config::ToastHerdrPosition::TopLeft
        | crate::config::ToastHerdrPosition::BottomLeft => area.x,
        crate::config::ToastHerdrPosition::TopRight
        | crate::config::ToastHerdrPosition::BottomRight => area.right().saturating_sub(width),
        crate::config::ToastHerdrPosition::Center => {
            area.x.saturating_add(area.width.saturating_sub(width) / 2)
        }
    };
    let max_y = area.bottom().saturating_sub(height).max(area.y);
    let y = match position {
        crate::config::ToastHerdrPosition::TopLeft
        | crate::config::ToastHerdrPosition::TopRight => area.y.saturating_add(top_offset),
        crate::config::ToastHerdrPosition::BottomLeft
        | crate::config::ToastHerdrPosition::BottomRight => area
            .bottom()
            .saturating_sub(height.saturating_add(top_offset)),
        crate::config::ToastHerdrPosition::Center => area
            .y
            .saturating_add(area.height.saturating_sub(height) / 2),
    }
    .clamp(area.y, max_y);
    let rect = Rect::new(x, y, width, height);
    Clear.render(rect, buffer);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.overlay0))
        .style(Style::default().bg(palette.panel_bg));
    let (pad_cols, pad_rows) = if size == crate::config::ToastHerdrSize::Auto {
        (0, 0)
    } else {
        (2, 1)
    };
    let block = block.padding(Padding::new(pad_cols, pad_cols, pad_rows, pad_rows));
    let inner = block.inner(rect);
    block.render(rect, buffer);
    Paragraph::new(Line::from(vec![
        Span::styled("●", Style::default().fg(dot_color)),
        Span::raw(" "),
        Span::styled(
            title,
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        ),
    ]))
    .render(Rect::new(inner.x, inner.y, inner.width, 1), buffer);
    if !body.is_empty() && inner.height > 1 {
        Paragraph::new(Line::from(Span::styled(
            body,
            Style::default().fg(palette.overlay0),
        )))
        .render(
            Rect::new(
                inner.x.saturating_add(2),
                inner.y + 1,
                inner.width.saturating_sub(2),
                1,
            ),
            buffer,
        );
    }
    rect
}

fn notification_size(
    title: &str,
    body: &str,
    size: crate::config::ToastHerdrSize,
    anchor: u16,
) -> (u16, u16) {
    let (cols, rows) = if size == crate::config::ToastHerdrSize::Auto {
        (0, 0)
    } else {
        (2, 1)
    };
    let content = unicode_width::UnicodeWidthStr::width(title)
        .max(unicode_width::UnicodeWidthStr::width(body))
        .saturating_add(6 + 2 * cols);
    let minimum = match size {
        crate::config::ToastHerdrSize::Auto => 0,
        crate::config::ToastHerdrSize::Medium => u32::from(anchor) * 2 / 5,
        crate::config::ToastHerdrSize::Large => u32::from(anchor) * 3 / 5,
    } as u16;
    (
        u16::try_from(content).unwrap_or(u16::MAX).max(minimum),
        if body.is_empty() {
            3 + 2 * rows as u16
        } else {
            4 + 2 * rows as u16
        },
    )
}

pub(super) fn render_visible_notification_for_source(
    buffer: &mut Buffer,
    frame: Rect,
    pane_area: Rect,
    source: Option<Rect>,
    notification: &ClientVisibleNotification,
    config: &ClientShellConfig,
    offset: u16,
) -> Rect {
    let event = &notification.event;
    let position = event.position.unwrap_or(config.toast_position);
    let source = if event.kind == SemanticNotificationKind::Custom
        && config.herdr_toast.pane_feedback == crate::config::ToastPaneFeedback::Pane
    {
        source.filter(|pane| {
            let (width, height) = notification_size(
                &event.title,
                event.body.as_deref().unwrap_or_default(),
                config.herdr_toast.size,
                pane.width,
            );
            width <= pane.width && height <= pane.height
        })
    } else {
        None
    };
    let (anchor, position) = if let Some(source) = source {
        (source, crate::config::ToastHerdrPosition::Center)
    } else if position == crate::config::ToastHerdrPosition::Center && !pane_area.is_empty() {
        (pane_area, position)
    } else {
        (frame, position)
    };
    let dot = match event.kind {
        SemanticNotificationKind::NeedsAttention => config.palette.red,
        SemanticNotificationKind::Finished => config.palette.blue,
        _ => config.palette.accent,
    };
    render_notification_card_with_size(
        buffer,
        anchor,
        &event.title,
        event.body.as_deref().unwrap_or_default(),
        position,
        offset,
        dot,
        &config.palette,
        config.herdr_toast.size,
    )
}

#[cfg(test)]
pub(super) fn render_visible_notification(
    buffer: &mut Buffer,
    area: Rect,
    notification: &ClientVisibleNotification,
    default_position: crate::config::ToastHerdrPosition,
    top_offset: u16,
    palette: &Palette,
) -> Rect {
    let event = &notification.event;
    let dot_color = match event.kind {
        SemanticNotificationKind::NeedsAttention => palette.red,
        SemanticNotificationKind::Finished => palette.blue,
        SemanticNotificationKind::UpdateInstalled | SemanticNotificationKind::Custom => {
            palette.accent
        }
    };
    render_notification_card(
        buffer,
        area,
        &event.title,
        event.body.as_deref().unwrap_or_default(),
        event.position.unwrap_or(default_position),
        top_offset,
        dot_color,
        palette,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notification() -> ClientVisibleNotification {
        ClientVisibleNotification {
            endpoint_id: ClientEndpointId::Local,
            event: SemanticNotification {
                kind: SemanticNotificationKind::Custom,
                title: "notice".into(),
                body: None,
                sound: None,
                agent: None,
                workspace_id: None,
                tab_id: None,
                pane_id: None,
                position: None,
            },
            deadline: Some(std::time::Instant::now()),
        }
    }

    #[test]
    fn mobile_notification_is_a_bottom_banner_with_released_title() {
        let palette = crate::app::client_palette_from_config(&Config::default());
        let mut notification = notification();
        notification.event.kind = SemanticNotificationKind::NeedsAttention;
        notification.event.title = "pi needs attention".into();
        notification.event.body = Some("workspace · tab 1".into());
        let area = Rect::new(0, 0, 44, 20);
        let mut buffer = Buffer::empty(area);
        for cell in &mut buffer.content {
            cell.set_symbol("X");
        }
        let rect =
            render_mobile_notification_banner(&mut buffer, area, &notification, true, &palette);
        assert_eq!(rect, Rect::new(0, 18, 44, 1));
        let text = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("pi waiting"));
        assert!(text.contains("workspace · tab 1"));
        assert!(buffer.content[18 * 44..19 * 44]
            .iter()
            .all(|cell| cell.symbol() != "X"));
    }

    #[test]
    fn unavailable_notification_target_stays_visible_and_reports_the_machine() {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        state.visible_notification = Some(ClientVisibleNotification {
            endpoint_id: ClientEndpointId::Local,
            event: SemanticNotification {
                kind: SemanticNotificationKind::NeedsAttention,
                title: "agent needs attention".into(),
                body: None,
                sound: None,
                agent: Some("agent".into()),
                workspace_id: Some("workspace".into()),
                tab_id: Some("tab".into()),
                pane_id: Some("pane".into()),
                position: None,
            },
            deadline: Some(std::time::Instant::now() + std::time::Duration::from_secs(5)),
        });
        let mut outcome = ClientShellInput::default();

        state.focus_visible_notification(&mut outcome);

        assert!(state.visible_notification.is_some());
        assert!(state
            .visible_endpoint_notice
            .as_ref()
            .is_some_and(|notice| notice.body.contains("Local is unavailable")));
        assert!(outcome.repaint);
    }

    #[test]
    fn immediate_finished_evidence_for_acknowledged_idle_never_toasts_or_sounds() {
        let mut config = ClientShellConfig::from_config(&Config::default());
        config.toast_delivery = crate::config::ToastDelivery::Herdr;
        config.toast_delay_seconds = 0;
        let mut state = ClientShellState::new(config);
        let mut snapshot = super::super::tests::snapshot();
        snapshot.agents.push(crate::protocol::ClientShellAgent {
            pane_id: "pane_1".into(),
            workspace_id: "ws_1".into(),
            tab_id: "tab_1".into(),
            name: Some("agent".into()),
            display_agent: None,
            agent: Some("agent".into()),
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            agent_status: crate::api::schema::AgentStatus::Idle,
            state_change_seq: 2,
            state_labels: Vec::new(),
            tokens: Vec::new(),
            focused: true,
        });
        state.set_snapshot(Box::new(snapshot));

        let (effects, repaint) = state.receive_notification(
            &ClientEndpointId::Local,
            SemanticNotification {
                kind: SemanticNotificationKind::Finished,
                title: "agent finished".into(),
                body: None,
                sound: Some(SemanticNotificationSound::Done),
                agent: Some("agent".into()),
                workspace_id: Some("ws_1".into()),
                tab_id: Some("tab_1".into()),
                pane_id: Some("pane_1".into()),
                position: None,
            },
            std::time::Instant::now(),
        );

        assert!(effects.is_empty());
        assert!(!repaint);
        assert!(state.visible_notification.is_none());
    }

    #[test]
    fn finished_hint_waits_for_the_projected_completion_snapshot() {
        let mut config = ClientShellConfig::from_config(&Config::default());
        config.toast_delivery = crate::config::ToastDelivery::Terminal;
        config.toast_delay_seconds = 0;
        let mut state = ClientShellState::new(config);
        let mut snapshot = super::super::tests::snapshot();
        snapshot.agents.push(crate::protocol::ClientShellAgent {
            pane_id: "pane_1".into(),
            workspace_id: "ws_1".into(),
            tab_id: "tab_1".into(),
            name: Some("agent".into()),
            display_agent: None,
            agent: Some("agent".into()),
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            agent_status: crate::api::schema::AgentStatus::Working,
            state_change_seq: 1,
            state_labels: Vec::new(),
            tokens: Vec::new(),
            focused: false,
        });
        state.set_snapshot(Box::new(snapshot.clone()));
        let now = std::time::Instant::now();
        let event = SemanticNotification {
            kind: SemanticNotificationKind::Finished,
            title: "agent finished".into(),
            body: None,
            sound: None,
            agent: Some("agent".into()),
            workspace_id: Some("ws_1".into()),
            tab_id: Some("background-tab".into()),
            pane_id: Some("pane_1".into()),
            position: None,
        };

        let (effects, _) = state.receive_notification(&ClientEndpointId::Local, event, now);
        assert!(effects.is_empty());
        assert_eq!(state.pending_notifications.len(), 1);

        snapshot.revision = snapshot.revision.saturating_add(1);
        snapshot.agents[0].agent_status = crate::api::schema::AgentStatus::Idle;
        snapshot.agents[0].state_change_seq = 2;
        state.set_snapshot(Box::new(snapshot));
        let (effects, _) = state.tick_notifications(now + COMPLETION_RECHECK_INTERVAL);
        assert!(matches!(
            effects.as_slice(),
            [ClientShellNotificationEffect::Terminal { title, .. }] if title == "agent finished"
        ));
        assert!(state.pending_notifications.is_empty());
    }

    #[test]
    fn retiring_one_endpoint_preserves_other_endpoint_notifications() {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        let mut local = notification();
        local.event.title = "local".into();
        let remote_id = ClientEndpointId::Ssh(
            crate::client::endpoint::ProfileId::parse("0123456789abcdef0123456789abcdef").unwrap(),
        );
        let mut remote = notification();
        remote.endpoint_id = remote_id.clone();
        remote.event.title = "remote".into();
        state.visible_notification = Some(local);
        state.queued_notifications.push_back(remote);

        state.retire_endpoint_notifications(&ClientEndpointId::Local);

        assert_eq!(
            state
                .visible_notification
                .as_ref()
                .map(|notification| (&notification.endpoint_id, notification.event.title.as_str())),
            Some((&remote_id, "remote"))
        );
    }

    #[test]
    fn concurrent_endpoint_notifications_are_queued_in_arrival_order() {
        let mut config = Config::default();
        config.ui.toast.delivery = crate::config::ToastDelivery::Herdr;
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
        let now = std::time::Instant::now();
        for title in ["first", "second"] {
            state.receive_notification(
                &ClientEndpointId::Local,
                SemanticNotification {
                    kind: SemanticNotificationKind::Custom,
                    title: title.into(),
                    body: None,
                    sound: None,
                    agent: None,
                    workspace_id: None,
                    tab_id: None,
                    pane_id: Some(title.into()),
                    position: None,
                },
                now,
            );
        }

        assert_eq!(
            state
                .visible_notification
                .as_ref()
                .map(|notification| notification.event.title.as_str()),
            Some("first")
        );
        assert_eq!(state.queued_notifications.len(), 1);

        state.tick_notifications(now + std::time::Duration::from_secs(5));

        assert_eq!(
            state
                .visible_notification
                .as_ref()
                .map(|notification| notification.event.title.as_str()),
            Some("second")
        );
        assert!(state.queued_notifications.is_empty());
    }

    #[test]
    fn notification_rect_stays_inside_short_nonzero_area() {
        let palette = crate::app::client_palette_from_config(&Config::default());
        for height in [1, 2] {
            let area = Rect::new(3, 4, 8, height);
            for position in [
                crate::config::ToastHerdrPosition::TopRight,
                crate::config::ToastHerdrPosition::BottomRight,
            ] {
                let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 10));
                let rect = render_visible_notification(
                    &mut buffer,
                    area,
                    &notification(),
                    position,
                    1,
                    &palette,
                );
                assert!(rect.y >= area.y);
                assert!(rect.bottom() <= area.bottom());
            }
        }
    }
    #[test]
    fn centered_toast_uses_pane_area_and_size_presets() {
        let mut config = Config::default();
        config.ui.toast.herdr.position = crate::config::ToastHerdrPosition::Center;
        let frame = Rect::new(0, 0, 100, 30);
        let panes = Rect::new(30, 1, 70, 29);
        let mut buffer = Buffer::empty(frame);
        let note = notification();
        let config = ClientShellConfig::from_config(&config);
        let auto = render_visible_notification_for_source(
            &mut buffer,
            frame,
            panes,
            None,
            &note,
            &config,
            1,
        );
        assert_eq!(auto.x, panes.x + (panes.width - auto.width) / 2);
        assert_eq!(auto.y, panes.y + (panes.height - auto.height) / 2);
        let mut wider = Config::default();
        wider.ui.toast.herdr.position = crate::config::ToastHerdrPosition::Center;
        wider.ui.toast.herdr.size = crate::config::ToastHerdrSize::Large;
        let config = ClientShellConfig::from_config(&wider);
        let large = render_visible_notification_for_source(
            &mut buffer,
            frame,
            panes,
            None,
            &note,
            &config,
            1,
        );
        assert!(large.width >= panes.width * 3 / 5);
        assert!(large.height > auto.height);
    }
    #[test]
    fn pane_action_toast_centers_only_when_the_whole_box_fits() {
        let mut config = Config::default();
        config.ui.toast.herdr.pane_feedback = crate::config::ToastPaneFeedback::Pane;
        let config = ClientShellConfig::from_config(&config);
        let note = notification();
        let frame = Rect::new(0, 0, 100, 30);
        let pane = Rect::new(35, 4, 60, 20);
        let mut buffer = Buffer::empty(frame);
        let rect = render_visible_notification_for_source(
            &mut buffer,
            frame,
            frame,
            Some(pane),
            &note,
            &config,
            0,
        );
        assert_eq!(rect.x, pane.x + (pane.width - rect.width) / 2);
        assert_eq!(rect.y, pane.y + (pane.height - rect.height) / 2);
        let small = render_visible_notification_for_source(
            &mut buffer,
            frame,
            frame,
            Some(Rect::new(0, 0, 2, 2)),
            &note,
            &config,
            0,
        );
        assert_eq!(small.x, frame.right() - small.width);
    }
}
