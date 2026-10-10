use super::*;
use crate::api::schema::{ClientInfo, Method, Request, ResponseResult, SuccessResponse};
use crate::protocol::{ClientMouseKind, ClientMousePosition, ClientPaneInputEvent};

struct Fixture {
    server: HeadlessServer,
    first_tab_id: String,
    first_pane_id: String,
    second_pane_id: String,
    /// Held so the test clients' writers stay open for the whole test.
    _receivers: Vec<std::sync::mpsc::Receiver<Vec<u8>>>,
}

/// One workspace with two tabs. Client 41 shows tab 1 and client 42 shows tab 2,
/// as two terminals attached to one session would.
fn two_shell_clients() -> Fixture {
    let mut server = test_headless_server();
    let mut workspace = crate::workspace::Workspace::test_new("client-list");
    let first_pane = workspace.tabs[0].root_pane;
    let second_tab = workspace.test_add_tab(Some("second"));
    let second_pane = workspace.tabs[second_tab].root_pane;
    workspace.insert_test_runtime(
        first_pane,
        crate::terminal::TerminalRuntime::test_with_screen_bytes(80, 23, b"FIRST"),
    );
    workspace.insert_test_runtime(
        second_pane,
        crate::terminal::TerminalRuntime::test_with_screen_bytes(80, 23, b"SECOND"),
    );
    server.app.state.workspaces = vec![workspace];
    server.app.state.active = Some(0);
    server.app.state.selected = 0;
    server.app.state.mode = crate::app::Mode::Terminal;
    let first_tab_id = server.app.public_tab_id(0, 0).unwrap();
    let second_tab_id = server.app.public_tab_id(0, second_tab).unwrap();
    let first_pane_id = server.app.public_pane_id(0, first_pane).unwrap();
    let second_pane_id = server.app.public_pane_id(0, second_pane).unwrap();

    let (first_control, first_render) = connect_matching_test_shell(&mut server, 41);
    let (second_control, second_render) = connect_matching_test_shell(&mut server, 42);
    let _ = first_control.recv().expect("first snapshot");
    let _ = second_control.recv().expect("second snapshot");
    assert!(server.focus_shell_client_on_tab(42, &second_tab_id));
    Fixture {
        server,
        first_tab_id,
        first_pane_id,
        second_pane_id,
        _receivers: vec![first_render, second_render],
    }
}

fn client_list(server: &mut HeadlessServer) -> Vec<ClientInfo> {
    let (respond_to, response_rx) = std::sync::mpsc::channel();
    server.handle_api_request_with_shutdown_check(crate::api::ApiRequestMessage {
        request: Request {
            id: "client-list".into(),
            method: Method::ClientList(crate::api::schema::EmptyParams::default()),
        },
        respond_to,
        response_write_complete: None,
    });
    let response = response_rx.recv().expect("client.list response");
    let success: SuccessResponse = serde_json::from_str(&response)
        .unwrap_or_else(|err| panic!("expected success, got {response}: {err}"));
    match success.result {
        ResponseResult::ClientList { clients } => clients,
        other => panic!("expected client_list, got {other:?}"),
    }
}

fn client(clients: &[ClientInfo], client_id: u64) -> &ClientInfo {
    let wanted = client_id.to_string();
    clients
        .iter()
        .find(|client| client.client_id == wanted)
        .unwrap_or_else(|| panic!("client {client_id} missing from {clients:?}"))
}

fn type_in(server: &mut HeadlessServer, client_id: u64, pane_id: &str) {
    server.handle_server_event(ServerEvent::ClientShellPaneInput {
        client_id,
        pane_id: pane_id.to_owned(),
        events: vec![ClientPaneInputEvent::TextCommit("x".into())],
    });
}

#[tokio::test]
async fn client_list_is_empty_without_shell_clients() {
    let mut server = test_headless_server();
    server.app.state.workspaces = vec![crate::workspace::Workspace::test_new("no-clients")];
    server.app.state.active = Some(0);

    assert_eq!(client_list(&mut server), Vec::<ClientInfo>::new());
}

#[tokio::test]
async fn attached_clients_are_listed_oldest_first_with_their_tabs() {
    let mut fixture = two_shell_clients();

    let clients = client_list(&mut fixture.server);

    assert_eq!(
        clients
            .iter()
            .map(|client| client.client_id.as_str())
            .collect::<Vec<_>>(),
        ["41", "42"]
    );
    assert_eq!(
        client(&clients, 41).tab_id.as_deref(),
        Some(fixture.first_tab_id.as_str())
    );
    assert_ne!(client(&clients, 42).tab_id, client(&clients, 41).tab_id);
    assert!(clients.iter().all(|client| client.workspace_id.is_some()));
    shutdown_test_runtimes(&mut fixture.server);
}

#[tokio::test]
async fn a_client_that_never_sent_input_has_no_input_age() {
    let mut fixture = two_shell_clients();

    let clients = client_list(&mut fixture.server);

    assert!(clients
        .iter()
        .all(|client| client.last_input_age_ms.is_none()));
    shutdown_test_runtimes(&mut fixture.server);
}

#[tokio::test]
async fn input_resets_that_clients_age_and_makes_it_foreground() {
    let mut fixture = two_shell_clients();
    let (first_pane, second_pane) = (
        fixture.first_pane_id.clone(),
        fixture.second_pane_id.clone(),
    );

    type_in(&mut fixture.server, 41, &first_pane);
    std::thread::sleep(Duration::from_millis(40));
    type_in(&mut fixture.server, 42, &second_pane);
    let clients = client_list(&mut fixture.server);

    // B typed last: it is foreground with the smaller age; A's age kept growing.
    let (a, b) = (client(&clients, 41), client(&clients, 42));
    assert!(b.foreground && !a.foreground, "{clients:?}");
    let (a_age, b_age) = (a.last_input_age_ms.unwrap(), b.last_input_age_ms.unwrap());
    assert!(a_age >= 40, "A's age should have grown: {a_age} ms");
    assert!(b_age < a_age, "B typed after A: {b_age} < {a_age}");

    // Typing in A again flips both.
    std::thread::sleep(Duration::from_millis(40));
    type_in(&mut fixture.server, 41, &first_pane);
    let clients = client_list(&mut fixture.server);
    let (a, b) = (client(&clients, 41), client(&clients, 42));
    assert!(a.foreground && !b.foreground, "{clients:?}");
    assert!(a.last_input_age_ms.unwrap() < b.last_input_age_ms.unwrap());
    shutdown_test_runtimes(&mut fixture.server);
}

#[tokio::test]
async fn age_is_measured_from_the_last_input_not_from_the_query() {
    let mut fixture = two_shell_clients();
    let first_pane = fixture.first_pane_id.clone();

    type_in(&mut fixture.server, 41, &first_pane);
    let later = Instant::now() + Duration::from_secs(90);
    let clients = fixture.server.client_list_infos(later);

    let age = client(&clients, 41).last_input_age_ms.unwrap();
    assert!((90_000..91_000).contains(&age), "{age} ms");
    shutdown_test_runtimes(&mut fixture.server);
}

#[tokio::test]
async fn mouse_scroll_and_paste_count_as_input() {
    for event in [
        ClientPaneInputEvent::Mouse {
            kind: ClientMouseKind::ScrollUp,
            position: ClientMousePosition::Cell { column: 1, row: 1 },
            geometry: None,
            modifiers: 0,
            lines: 1,
        },
        ClientPaneInputEvent::Mouse {
            kind: ClientMouseKind::Moved,
            position: ClientMousePosition::Cell { column: 2, row: 2 },
            geometry: None,
            modifiers: 0,
            lines: 0,
        },
        ClientPaneInputEvent::Paste("pasted".into()),
    ] {
        let mut fixture = two_shell_clients();
        fixture
            .server
            .handle_server_event(ServerEvent::ClientShellPaneInput {
                client_id: 41,
                pane_id: fixture.first_pane_id.clone(),
                events: vec![event.clone()],
            });

        let clients = client_list(&mut fixture.server);

        assert!(
            client(&clients, 41).last_input_age_ms.is_some(),
            "{event:?} should count as input"
        );
        assert!(client(&clients, 42).last_input_age_ms.is_none());
        shutdown_test_runtimes(&mut fixture.server);
    }
}

#[tokio::test]
async fn focus_reports_show_in_the_list_and_are_not_input() {
    let mut fixture = two_shell_clients();
    assert_eq!(
        client(&client_list(&mut fixture.server), 41).window_focused,
        None,
        "unknown until the terminal reports focus"
    );

    fixture
        .server
        .handle_server_event(ServerEvent::ClientShellFocus {
            client_id: 41,
            focused: true,
        });
    fixture
        .server
        .handle_server_event(ServerEvent::ClientShellFocus {
            client_id: 42,
            focused: false,
        });
    let clients = client_list(&mut fixture.server);
    assert_eq!(client(&clients, 41).window_focused, Some(true));
    assert_eq!(client(&clients, 42).window_focused, Some(false));
    assert!(
        clients
            .iter()
            .all(|client| client.last_input_age_ms.is_none()),
        "focus changes are not input: {clients:?}"
    );

    fixture
        .server
        .handle_server_event(ServerEvent::ClientShellFocus {
            client_id: 41,
            focused: false,
        });
    assert_eq!(
        client(&client_list(&mut fixture.server), 41).window_focused,
        Some(false)
    );
    shutdown_test_runtimes(&mut fixture.server);
}

#[tokio::test]
async fn a_disconnected_client_leaves_the_list() {
    let mut fixture = two_shell_clients();

    assert!(fixture
        .server
        .handle_server_event(ServerEvent::ClientDisconnected { client_id: 41 }));

    let clients = client_list(&mut fixture.server);
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0].client_id, "42");
    shutdown_test_runtimes(&mut fixture.server);
}

#[tokio::test]
async fn only_clients_on_the_popups_tab_view_it() {
    let mut fixture = two_shell_clients();
    assert!(client_list(&mut fixture.server)
        .iter()
        .all(|client| !client.views_popup));

    fixture.server.app.state.popup_pane = Some(crate::app::state::PopupPaneState {
        pane_id: crate::layout::PaneId::alloc(),
        terminal_id: crate::terminal::TerminalId::alloc(),
        width: None,
        height: None,
        plugin_id: None,
    });
    fixture.server.popup_owner_tab_id = Some(fixture.first_tab_id.clone());

    let clients = client_list(&mut fixture.server);
    assert!(client(&clients, 41).views_popup);
    assert!(!client(&clients, 42).views_popup);
    shutdown_test_runtimes(&mut fixture.server);
}

#[test]
fn only_commands_the_user_issued_count_as_input() {
    use crate::api::schema::{EmptyParams, TodoListParams};
    assert!(super::super::client_list::endpoint_method_is_passive(
        &Method::TodoList(TodoListParams { pane_id: None })
    ));
    assert!(super::super::client_list::endpoint_method_is_passive(
        &Method::NotificationList(EmptyParams::default())
    ));
    assert!(!super::super::client_list::endpoint_method_is_passive(
        &Method::NotificationClear(EmptyParams::default())
    ));
}
