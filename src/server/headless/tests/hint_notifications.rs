use super::*;

fn screen(pane_id: crate::layout::PaneId, state: crate::detect::AgentState) -> AppEvent {
    AppEvent::StateChanged {
        pane_id,
        agent: Some(crate::detect::Agent::Claude),
        state,
        visible_blocker: state == crate::detect::AgentState::Blocked,
        visible_working: state == crate::detect::AgentState::Working,
        background_work: false,
        blocked_reason: (state == crate::detect::AgentState::Blocked)
            .then_some(crate::detect::BlockedReason::Question),
        process_exited: false,
        observed_at: Instant::now(),
    }
}
fn hint(
    pane_id: crate::layout::PaneId,
    kind: Option<crate::detect::BlockedReason>,
    seq: u64,
) -> AppEvent {
    AppEvent::AgentHintReported {
        pane_id,
        report: crate::terminal::AgentHintReport {
            source: "herdr:claude-mod".into(),
            agent_label: "claude".into(),
            kind,
            id: Some("question_1".into()),
            ttl: Duration::from_secs(15),
            seq: Some(seq),
        },
    }
}
fn setup() -> (
    HeadlessServer,
    crate::layout::PaneId,
    std::sync::mpsc::Receiver<Vec<u8>>,
) {
    let mut server = test_headless_server();
    let background = crate::workspace::Workspace::test_new("background");
    let pane_id = background.tabs[0].root_pane;
    server.app.state.workspaces = vec![
        background,
        crate::workspace::Workspace::test_new("foreground"),
    ];
    server.app.state.ensure_test_terminals();
    server.app.state.active = Some(1);
    server.app.state.selected = 1;
    server.app.state.toast_config.delivery = crate::config::ToastDelivery::Herdr;
    server.app.state.toast_config.delay_seconds = 0;
    server.app.state.sound.enabled = true;
    let (writer, rx, _frames) = test_client_writer();
    server.clients.insert(
        1,
        ClientConnection::new_with_mode(
            ClientConnectionMode::ClientShell,
            (80, 24),
            crate::kitty_graphics::HostCellSize::default(),
            1,
            RenderEncoding::SemanticFrame,
            Some(writer),
        ),
    );
    server.handle_internal_event_with_forwarding(AppEvent::AgentProcessDetected {
        pane_id,
        agent: crate::detect::Agent::Claude,
        observed_at: Instant::now(),
        replaced_process: false,
    });
    server
        .handle_internal_event_with_forwarding(screen(pane_id, crate::detect::AgentState::Working));
    while rx.try_recv().is_ok() {}
    (server, pane_id, rx)
}
fn sounds(rx: &std::sync::mpsc::Receiver<Vec<u8>>) -> Vec<protocol::SemanticNotificationSound> {
    let mut sounds = Vec::new();
    while let Ok(message) = rx.recv_timeout(Duration::from_millis(30)) {
        if let ServerMessage::SemanticNotification(notification) = read_server_message(message) {
            if let Some(sound) = notification.sound {
                sounds.push(sound);
            }
        }
    }
    sounds
}
#[test]
fn hint_first_and_screen_first_each_emit_exactly_one_attention_effect() {
    for hint_first in [true, false] {
        let (mut server, pane, rx) = setup();
        let question = hint(pane, Some(crate::detect::BlockedReason::Question), 10);
        let blocked = screen(pane, crate::detect::AgentState::Blocked);
        let (first, second) = if hint_first {
            (question, blocked)
        } else {
            (blocked, question)
        };
        server.handle_internal_event_with_forwarding(first);
        assert_eq!(
            sounds(&rx),
            vec![protocol::SemanticNotificationSound::Request]
        );
        server.handle_internal_event_with_forwarding(second);
        assert!(sounds(&rx).is_empty());
        server.handle_internal_event_with_forwarding(hint(
            pane,
            Some(crate::detect::BlockedReason::Question),
            9,
        ));
        assert!(sounds(&rx).is_empty(), "stale report has no effect");
    }
}
#[test]
fn hint_release_and_expiry_return_to_working_without_sound() {
    let (mut server, pane, rx) = setup();
    server.handle_internal_event_with_forwarding(hint(
        pane,
        Some(crate::detect::BlockedReason::Question),
        10,
    ));
    assert_eq!(
        sounds(&rx),
        vec![protocol::SemanticNotificationSound::Request]
    );
    server.handle_internal_event_with_forwarding(hint(pane, None, 11));
    assert!(sounds(&rx).is_empty());
    server.handle_internal_event_with_forwarding(hint(
        pane,
        Some(crate::detect::BlockedReason::Question),
        12,
    ));
    assert_eq!(
        sounds(&rx),
        vec![protocol::SemanticNotificationSound::Request]
    );
    server.handle_internal_event_with_forwarding(AppEvent::AgentHintExpired {
        pane_id: pane,
        now: Instant::now() + Duration::from_secs(60),
    });
    assert!(sounds(&rx).is_empty());
}

#[test]
fn hint_notification_uses_fork_agent_name_with_original_event_words() {
    let (mut server, pane, rx) = setup();
    let terminal_id = server.app.state.workspaces[0]
        .pane_state(pane)
        .unwrap()
        .attached_terminal_id
        .clone();
    server
        .app
        .state
        .terminals
        .get_mut(&terminal_id)
        .unwrap()
        .agent_name = Some("worker".into());
    server.handle_internal_event_with_forwarding(hint(
        pane,
        Some(crate::detect::BlockedReason::Question),
        10,
    ));
    let mut notifications = Vec::new();
    while let Ok(message) = rx.recv_timeout(Duration::from_millis(30)) {
        if let ServerMessage::SemanticNotification(notification) = read_server_message(message) {
            notifications.push(notification);
        }
    }
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].title, "worker needs attention");
    assert_eq!(
        notifications[0].sound,
        Some(protocol::SemanticNotificationSound::Request)
    );
}
