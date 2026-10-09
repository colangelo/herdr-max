use super::*;
use crate::protocol::{ClientKeyCode, ClientKeyKind, ClientPaneInputEvent};

fn fixture() -> (
    HeadlessServer,
    [crate::layout::PaneId; 3],
    Vec<mpsc::Receiver<Bytes>>,
) {
    let mut server = test_headless_server();
    let mut workspace = crate::workspace::Workspace::test_new("sync-input");
    let a = workspace.tabs[0].root_pane;
    let b = workspace.test_split(ratatui::layout::Direction::Horizontal);
    let c = workspace.test_split(ratatui::layout::Direction::Vertical);
    workspace.tabs[0].layout.focus_pane(a);
    server.app.state.workspaces = vec![workspace];
    server.app.state.active = Some(0);
    server.app.state.selected = 0;
    server.app.state.mode = crate::app::Mode::Terminal;
    server.app.state.ensure_test_terminals();
    let mut receivers = Vec::new();
    for pane in [a, b, c] {
        receivers.push(install_runtime(&mut server, pane, b""));
    }
    server.clients.insert(
        41,
        ClientConnection::new(
            (80, 24),
            crate::kitty_graphics::HostCellSize::default(),
            1,
            RenderEncoding::SemanticFrame,
            None,
        ),
    );
    let workspace_id = server.app.public_workspace_id(0);
    let tab_id = server.app.public_tab_id(0, 0).unwrap();
    let client = server.clients.get_mut(&41).unwrap();
    client.shell_surface_active = true;
    client.shell_location = Some(crate::server::clients::ClientShellLocation {
        focused_workspace_id: Some(workspace_id.clone()),
        active_tab_ids: HashMap::from([(workspace_id, tab_id)]),
    });
    (server, [a, b, c], receivers)
}

fn install_runtime(
    server: &mut HeadlessServer,
    pane: crate::layout::PaneId,
    bytes: &[u8],
) -> mpsc::Receiver<Bytes> {
    let terminal_id = server.app.state.workspaces[0]
        .terminal_id(pane)
        .unwrap()
        .clone();
    let (runtime, receiver) = crate::terminal::TerminalRuntime::test_with_channel(80, 24);
    runtime.test_process_pty_bytes(bytes);
    if let Some(old) = server.app.terminal_runtimes.insert(terminal_id, runtime) {
        old.shutdown();
    }
    receiver
}

fn drain(receiver: &mut mpsc::Receiver<Bytes>) -> Vec<u8> {
    let mut bytes = Vec::new();
    while let Ok(packet) = receiver.try_recv() {
        bytes.extend_from_slice(&packet);
    }
    bytes
}

fn key(kind: ClientKeyKind, tracked: bool, physical: Option<u32>) -> ClientPaneInputEvent {
    ClientPaneInputEvent::Key {
        code: ClientKeyCode::Char('x'),
        modifiers: 0,
        kind,
        repeat_count: 1,
        shifted_codepoint: None,
        generated_text: None,
        tracks_release: tracked,
        physical_key_id: physical,
        windows_record: None,
    }
}

fn send(
    server: &mut HeadlessServer,
    pane: crate::layout::PaneId,
    events: Vec<ClientPaneInputEvent>,
) {
    let pane_id = server.app.public_pane_id(0, pane).unwrap();
    server.route_synced_pane_input(41, pane_id, events);
}

fn kitty(server: &HeadlessServer, panes: &[crate::layout::PaneId]) {
    for pane in panes {
        server
            .app
            .state
            .runtime_for_pane_in_workspace(&server.app.terminal_runtimes, 0, *pane)
            .unwrap()
            .test_process_pty_bytes(b"\x1b[>11u");
    }
}

#[tokio::test]
async fn sync_input_keys_text_paste_use_each_peer_encoding_and_exclusions() {
    let (mut server, [a, b, c], mut receivers) = fixture();
    send(&mut server, a, vec![key(ClientKeyKind::Press, false, None)]);
    assert_eq!(drain(&mut receivers[0]), b"x");
    assert!(drain(&mut receivers[1]).is_empty() && drain(&mut receivers[2]).is_empty());
    server.app.state.workspaces[0].tabs[0].set_sync(true);
    kitty(&server, &[b]);
    let runtime_c = server
        .app
        .state
        .runtime_for_pane_in_workspace(&server.app.terminal_runtimes, 0, c)
        .unwrap();
    runtime_c.test_process_pty_bytes(b"\x1b[?2004h");
    send(&mut server, a, vec![key(ClientKeyKind::Press, false, None)]);
    assert_eq!(drain(&mut receivers[0]), b"x");
    assert_eq!(drain(&mut receivers[1]), b"\x1b[120u");
    assert_eq!(drain(&mut receivers[2]), b"x");
    send(
        &mut server,
        a,
        vec![
            ClientPaneInputEvent::TextCommit("é".into()),
            ClientPaneInputEvent::Paste("paste".into()),
        ],
    );
    assert_eq!(drain(&mut receivers[0]), "épaste".as_bytes());
    assert_eq!(drain(&mut receivers[1]), "épaste".as_bytes());
    assert_eq!(
        drain(&mut receivers[2]),
        "é\x1b[200~paste\x1b[201~".as_bytes()
    );
    server.app.state.toggle_pane_sync(0, b);
    send(
        &mut server,
        a,
        vec![ClientPaneInputEvent::TextCommit("q".into())],
    );
    assert_eq!(drain(&mut receivers[0]), b"q");
    assert!(drain(&mut receivers[1]).is_empty());
    assert_eq!(drain(&mut receivers[2]), b"q");
    server.app.state.toggle_pane_sync(0, a);
    send(
        &mut server,
        a,
        vec![ClientPaneInputEvent::TextCommit("r".into())],
    );
    assert_eq!(drain(&mut receivers[0]), b"r");
    assert!(drain(&mut receivers[1]).is_empty() && drain(&mut receivers[2]).is_empty());
    server.app.state.assert_invariants_for_test();
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn sync_input_batch_and_membership_changes_keep_original_release_recipients() {
    let (mut server, [a, b, c], mut receivers) = fixture();
    server.app.state.workspaces[0].tabs[0].set_sync(true);
    server.app.state.toggle_pane_sync(0, c);
    kitty(&server, &[a, b, c]);
    send(
        &mut server,
        a,
        vec![key(ClientKeyKind::Press, true, Some(9))],
    );
    for receiver in &mut receivers[..2] {
        assert_eq!(drain(receiver), b"\x1b[120u");
    }
    server.app.state.toggle_pane_sync(0, b);
    server.app.state.toggle_pane_sync(0, c);
    let mut release = key(ClientKeyKind::Release, true, Some(9));
    if let ClientPaneInputEvent::Key { modifiers, .. } = &mut release {
        *modifiers = crossterm::event::KeyModifiers::CONTROL.bits();
    }
    send(
        &mut server,
        a,
        vec![key(ClientKeyKind::Repeat, true, Some(9)), release],
    );
    for receiver in &mut receivers[..2] {
        assert_eq!(drain(receiver), b"\x1b[120;1:2u\x1b[120;5:3u");
    }
    assert!(drain(&mut receivers[2]).is_empty());
    send(
        &mut server,
        a,
        vec![
            key(ClientKeyKind::Press, true, Some(10)),
            key(ClientKeyKind::Repeat, true, Some(10)),
            key(ClientKeyKind::Release, true, Some(10)),
        ],
    );
    assert_eq!(
        drain(&mut receivers[0]),
        b"\x1b[120u\x1b[120;1:2u\x1b[120;1:3u"
    );
    assert_eq!(
        drain(&mut receivers[2]),
        b"\x1b[120u\x1b[120;1:2u\x1b[120;1:3u"
    );
    assert!(drain(&mut receivers[1]).is_empty());
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn sync_input_respawn_and_disconnect_never_reach_replacement_runtimes() {
    let (mut server, [a, b, c], mut receivers) = fixture();
    server.app.state.workspaces[0].tabs[0].set_sync(true);
    kitty(&server, &[a, b, c]);
    send(
        &mut server,
        a,
        vec![key(ClientKeyKind::Press, true, Some(12))],
    );
    for receiver in &mut receivers {
        assert_eq!(drain(receiver), b"\x1b[120u");
    }
    let mut replacement = install_runtime(&mut server, a, b"\x1b[>11u");
    send(
        &mut server,
        a,
        vec![key(ClientKeyKind::Repeat, true, Some(12))],
    );
    assert!(drain(&mut replacement).is_empty());
    for receiver in &mut receivers[1..] {
        assert_eq!(drain(receiver), b"\x1b[120;1:2u");
    }
    server.remove_client_and_resize_if_needed(41);
    assert!(drain(&mut replacement).is_empty());
    for receiver in &mut receivers[1..] {
        assert_eq!(drain(receiver), b"\x1b[120;1:3u");
    }
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn sync_input_closed_origin_still_releases_surviving_peers() {
    let (mut server, [a, b, c], mut receivers) = fixture();
    server.app.state.workspaces[0].tabs[0].set_sync(true);
    kitty(&server, &[a, b, c]);
    let pane_id = server.app.public_pane_id(0, a).unwrap();
    send(
        &mut server,
        a,
        vec![key(ClientKeyKind::Press, true, Some(15))],
    );
    for receiver in &mut receivers {
        drain(receiver);
    }
    let terminal_id = server.app.state.workspaces[0]
        .terminal_id(a)
        .unwrap()
        .clone();
    server.app.state.workspaces[0].tabs[0].panes.remove(&a);
    server
        .app
        .terminal_runtimes
        .remove(&terminal_id)
        .unwrap()
        .shutdown();
    server.route_synced_pane_input(
        41,
        pane_id,
        vec![key(ClientKeyKind::Release, true, Some(15))],
    );
    for receiver in &mut receivers[1..] {
        assert_eq!(drain(receiver), b"\x1b[120;1:3u");
    }
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn sync_input_host_page_disposition_survives_mode_changes_and_app_pages_reach_shell_peers() {
    let (mut server, [a, b, _], mut receivers) = fixture();
    server.app.state.workspaces[0].tabs[0].set_sync(true);
    let mut page = key(ClientKeyKind::Press, true, Some(20));
    if let ClientPaneInputEvent::Key { code, .. } = &mut page {
        *code = ClientKeyCode::PageUp;
    }
    send(&mut server, a, vec![page.clone()]);
    for receiver in &mut receivers {
        assert!(drain(receiver).is_empty());
    }
    server
        .app
        .state
        .runtime_for_pane_in_workspace(&server.app.terminal_runtimes, 0, a)
        .unwrap()
        .test_process_pty_bytes(b"\x1b[?1049h");
    let mut repeat = page.clone();
    if let ClientPaneInputEvent::Key { kind, .. } = &mut repeat {
        *kind = ClientKeyKind::Repeat;
    }
    let mut release = page.clone();
    if let ClientPaneInputEvent::Key { kind, .. } = &mut release {
        *kind = ClientKeyKind::Release;
    }
    send(&mut server, a, vec![repeat, release]);
    for receiver in &mut receivers {
        assert!(drain(receiver).is_empty());
    }
    send(&mut server, a, vec![page]);
    assert_eq!(drain(&mut receivers[0]), b"\x1b[5~");
    assert_eq!(drain(&mut receivers[1]), b"\x1b[5~");
    assert!(
        server
            .app
            .state
            .runtime_for_pane_in_workspace(&server.app.terminal_runtimes, 0, b)
            .unwrap()
            .plain_page_keys_use_host_scrollback()
            == Some(true)
    );
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn sync_input_mouse_and_legacy_events_do_not_inherit_tracked_key_recipients() {
    let (mut server, [a, b, c], mut receivers) = fixture();
    server.app.state.workspaces[0].tabs[0].set_sync(true);
    for pane in [a, b, c] {
        server
            .app
            .state
            .runtime_for_pane_in_workspace(&server.app.terminal_runtimes, 0, pane)
            .unwrap()
            .test_process_pty_bytes(b"\x1b[?1000h\x1b[?1006h");
    }
    send(
        &mut server,
        a,
        vec![ClientPaneInputEvent::Mouse {
            kind: protocol::ClientMouseKind::Down(protocol::ClientMouseButton::Right),
            position: protocol::ClientMousePosition::Cell { column: 1, row: 1 },
            geometry: None,
            modifiers: 0,
            lines: 1,
        }],
    );
    assert!(!drain(&mut receivers[0]).is_empty());
    assert!(drain(&mut receivers[1]).is_empty() && drain(&mut receivers[2]).is_empty());
    send(&mut server, a, vec![key(ClientKeyKind::Press, true, None)]);
    for receiver in &mut receivers {
        drain(receiver);
    }
    server.app.state.toggle_pane_sync(0, b);
    send(
        &mut server,
        a,
        vec![
            key(ClientKeyKind::Press, false, None),
            key(ClientKeyKind::Repeat, false, None),
        ],
    );
    assert_eq!(drain(&mut receivers[0]), b"xx");
    assert!(drain(&mut receivers[1]).is_empty());
    assert_eq!(drain(&mut receivers[2]), b"xx");
    assert_eq!(
        server
            .clients
            .get_mut(&41)
            .unwrap()
            .sync_input_leases
            .drain()
            .count(),
        0
    );
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn sync_input_all_replaced_recipients_drop_continuations_without_fallback() {
    let (mut server, panes, mut receivers) = fixture();
    server.app.state.workspaces[0].tabs[0].set_sync(true);
    kitty(&server, &panes);
    send(
        &mut server,
        panes[0],
        vec![key(ClientKeyKind::Press, true, Some(25))],
    );
    for receiver in &mut receivers {
        drain(receiver);
    }
    let mut replacements = panes
        .into_iter()
        .map(|pane| install_runtime(&mut server, pane, b"\x1b[>11u"))
        .collect::<Vec<_>>();
    send(
        &mut server,
        panes[0],
        vec![
            key(ClientKeyKind::Repeat, true, Some(25)),
            key(ClientKeyKind::Release, true, Some(25)),
        ],
    );
    for receiver in &mut replacements {
        assert!(drain(receiver).is_empty());
    }
    send(
        &mut server,
        panes[0],
        vec![key(ClientKeyKind::Press, true, Some(25))],
    );
    for receiver in &mut replacements {
        assert_eq!(drain(receiver), b"\x1b[120u");
    }
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn sync_input_distinct_physical_keys_have_one_disconnect_release_each() {
    let (mut server, panes, mut receivers) = fixture();
    server.app.state.workspaces[0].tabs[0].set_sync(true);
    kitty(&server, &panes);
    send(
        &mut server,
        panes[0],
        vec![
            key(ClientKeyKind::Press, true, Some(30)),
            key(ClientKeyKind::Press, true, Some(31)),
            key(ClientKeyKind::Release, true, Some(30)),
        ],
    );
    for receiver in &mut receivers {
        assert_eq!(drain(receiver), b"\x1b[120u\x1b[120u\x1b[120;1:3u");
    }
    server.remove_client_and_resize_if_needed(41);
    for receiver in &mut receivers {
        assert_eq!(drain(receiver), b"\x1b[120;1:3u");
    }
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn sync_input_popup_blocks_new_input_and_preserves_old_peer_releases() {
    let (mut server, panes, mut receivers) = fixture();
    server.app.state.workspaces[0].tabs[0].set_sync(true);
    kitty(&server, &panes);
    send(
        &mut server,
        panes[0],
        vec![key(ClientKeyKind::Press, true, Some(35))],
    );
    for receiver in &mut receivers {
        drain(receiver);
    }
    let (popup, _popup_rx) = crate::terminal::TerminalRuntime::test_with_channel(40, 12);
    server.app.install_test_popup_runtime(popup);
    server.popup_owner_tab_id = server.shell_tab_id_for_client(41);
    send(
        &mut server,
        panes[0],
        vec![key(ClientKeyKind::Press, true, Some(36))],
    );
    for receiver in &mut receivers {
        assert!(drain(receiver).is_empty());
    }
    send(
        &mut server,
        panes[0],
        vec![key(ClientKeyKind::Release, true, Some(35))],
    );
    for receiver in &mut receivers {
        assert_eq!(drain(receiver), b"\x1b[120;1:3u");
    }
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn sync_input_moved_runtime_keeps_its_release_identity() {
    use crate::api::schema::{Method, PaneMoveDestination, PaneMoveParams, Request};
    let (mut server, panes, mut receivers) = fixture();
    server.app.state.workspaces[0].tabs[0].set_sync(true);
    kitty(&server, &panes);
    let source = server.app.public_pane_id(0, panes[0]).unwrap();
    send(
        &mut server,
        panes[0],
        vec![key(ClientKeyKind::Press, true, Some(40))],
    );
    for receiver in &mut receivers {
        drain(receiver);
    }
    server
        .app
        .state
        .workspaces
        .push(crate::workspace::Workspace::test_new("destination"));
    server.app.state.ensure_test_terminals();
    let (respond_to, response_rx) = std::sync::mpsc::channel();
    server.handle_api_request_with_shutdown_check(crate::api::ApiRequestMessage {
        request: Request {
            id: "move-held".into(),
            method: Method::PaneMove(PaneMoveParams {
                pane_id: source.clone(),
                destination: PaneMoveDestination::NewTab {
                    workspace_id: Some(server.app.public_workspace_id(1)),
                    label: None,
                },
                focus: false,
            }),
        },
        respond_to,
        response_write_complete: None,
    });
    let response: serde_json::Value = serde_json::from_str(&response_rx.recv().unwrap()).unwrap();
    assert!(response.get("error").is_none(), "{response}");
    server.route_synced_pane_input(
        41,
        source,
        vec![key(ClientKeyKind::Release, true, Some(40))],
    );
    for receiver in &mut receivers {
        assert_eq!(drain(receiver), b"\x1b[120;1:3u");
    }
    server.app.state.assert_invariants_for_test();
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn sync_input_uses_the_origin_group_instead_of_global_focused_pane_or_other_tab() {
    let (mut server, [a, b, c], mut receivers) = fixture();
    server.app.state.workspaces[0].tabs[0].start_sync_pair(a, c);
    server.app.state.workspaces[0].tabs[0].layout.focus_pane(b);
    let other = server.app.state.workspaces[0].test_add_tab(Some("other"));
    let other_pane = server.app.state.workspaces[0].tabs[other].root_pane;
    server.app.state.ensure_test_terminals();
    let mut other_rx = install_runtime(&mut server, other_pane, b"");
    server.app.state.workspaces[0].switch_tab(other);
    send(
        &mut server,
        a,
        vec![ClientPaneInputEvent::TextCommit("origin".into())],
    );
    assert_eq!(drain(&mut receivers[0]), b"origin");
    assert!(drain(&mut receivers[1]).is_empty());
    assert_eq!(drain(&mut receivers[2]), b"origin");
    assert!(drain(&mut other_rx).is_empty());
    shutdown_test_runtimes(&mut server);
}
