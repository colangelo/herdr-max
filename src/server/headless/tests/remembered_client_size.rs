//! Fork handoff/remembered-size contracts, re-expressed on server-owned runtimes.
use super::*;

fn attach_remembering_test_client(
    server: &mut HeadlessServer,
    client_id: u64,
    size: (u16, u16),
    foreground: bool,
) {
    let (writer, _control, _frames) = test_client_writer();
    server.clients.insert(
        client_id,
        ClientConnection::new_with_mode(
            ClientConnectionMode::ClientShell,
            size,
            crate::kitty_graphics::HostCellSize::default(),
            client_id,
            RenderEncoding::SemanticFrame,
            Some(writer),
        ),
    );
    if foreground {
        server.promote_client_to_foreground(client_id);
    }
}

#[test]
fn a_size_carried_over_a_handoff_is_the_no_client_size() {
    let mut server = test_headless_server();
    server.handoff_client_size = Some((200, 60));
    server.sync_foreground_client_state();
    assert_eq!(server.effective_size, (200, 60));
}
#[test]
fn a_client_attach_ends_the_carried_handoff_size() {
    let mut server = test_headless_server();
    server.handoff_client_size = Some((200, 60));
    attach_remembering_test_client(&mut server, 1, (80, 24), true);
    assert_eq!(server.handoff_client_size, None);
    server.remove_client_and_resize_if_needed(1);
    assert_eq!(
        server.effective_size,
        (80, 24),
        "the client ending the handoff size becomes remembered"
    );
}
#[test]
fn detach_keeps_the_last_client_size() {
    let mut server = test_headless_server();
    attach_remembering_test_client(&mut server, 1, (310, 56), true);
    server.remove_client_and_resize_if_needed(1);
    assert_eq!(server.foreground_client_id, None);
    assert_eq!(server.effective_size, (310, 56));
    assert_eq!(server.app.state.last_client_size, Some((310, 56)));
    assert_eq!(server.app.state.no_client_size(None), (310, 56));
    assert!(
        server.app.state.session_dirty,
        "a new remembered size is saved"
    );
}
#[test]
fn last_client_resize_wins() {
    let mut server = test_headless_server();
    attach_remembering_test_client(&mut server, 1, (200, 50), true);
    assert!(server.handle_server_event(ServerEvent::ClientShellResize {
        client_id: 1,
        surface_cols: 310,
        surface_rows: 56,
        cell_width_px: 0,
        cell_height_px: 0,
        pixel_mouse: false
    }));
    server.remove_client_and_resize_if_needed(1);
    assert_eq!(server.effective_size, (310, 56));
}
#[test]
fn tiny_client_is_not_remembered() {
    let mut server = test_headless_server();
    attach_remembering_test_client(&mut server, 1, (310, 56), true);
    server.remove_client_and_resize_if_needed(1);
    attach_remembering_test_client(&mut server, 2, (60, 20), true);
    assert_eq!(server.effective_size, (60, 20), "used while attached");
    server.remove_client_and_resize_if_needed(2);
    assert_eq!(server.effective_size, (310, 56));
}
#[test]
fn background_client_size_is_ignored() {
    let mut server = test_headless_server();
    attach_remembering_test_client(&mut server, 1, (310, 56), true);
    attach_remembering_test_client(&mut server, 2, (100, 30), false);
    assert_eq!(server.foreground_client_id, Some(1));
    server.remove_client_and_resize_if_needed(2);
    server.remove_client_and_resize_if_needed(1);
    assert_eq!(server.effective_size, (310, 56));
}
#[test]
fn handoff_size_beats_remembered_size() {
    let mut server = test_headless_server();
    server.app.state.last_client_size = Some((310, 56));
    server.handoff_client_size = Some((200, 60));
    server.sync_foreground_client_state();
    assert_eq!(server.effective_size, (200, 60));
    attach_remembering_test_client(&mut server, 1, (250, 50), true);
    server.remove_client_and_resize_if_needed(1);
    assert_eq!(server.effective_size, (250, 50), "an attach ends it");
}
#[test]
fn remember_client_size_false_keeps_todays_behaviour() {
    let mut server = test_headless_server();
    server.app.state.remember_client_size = false;
    server.app.state.last_client_size = Some((300, 50));
    attach_remembering_test_client(&mut server, 1, (310, 56), true);
    server.remove_client_and_resize_if_needed(1);
    assert_eq!(server.effective_size, server.app.state.headless_size);
    assert_eq!(
        server.app.state.last_client_size,
        Some((300, 50)),
        "nothing is recorded while off"
    );
}
#[tokio::test]
async fn cold_start_spawns_restored_panes_at_remembered_size() {
    let config = crate::config::Config::default();
    let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = crate::app::App::new(
        &config,
        crate::app::AppPolicy::TEST,
        None,
        api_rx,
        api::EventHub::default(),
    );
    app.state.last_client_size = Some((310, 56));
    let workspace = crate::workspace::Workspace::test_new("restored");
    let pane_id = workspace.tabs[0].root_pane;
    let terminal_id = workspace.terminal_id(pane_id).unwrap().clone();
    app.state.workspaces = vec![workspace];
    app.state.ensure_test_terminals();
    app.terminal_runtimes.insert(
        terminal_id.clone(),
        crate::terminal::TerminalRuntime::test_with_screen_bytes(80, 24, b""),
    );
    app.state.active = Some(0);
    app.state.selected = 0;
    app.state.mode = crate::app::Mode::Terminal;
    let mut server = test_headless_server_from_app(app);
    server.render_and_stream();
    assert_eq!(server.effective_size, (310, 56));
    let area = server.app.state.view.terminal_area;
    assert_eq!(area.width + area.x, 310);
    assert_eq!(
        server
            .app
            .terminal_runtimes
            .get(&terminal_id)
            .unwrap()
            .current_size(),
        (area.height, area.width.saturating_sub(1))
    );
    shutdown_test_runtimes(&mut server);
}
