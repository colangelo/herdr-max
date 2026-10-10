use super::*;
use crate::api::schema::{Method, Request};

struct TwoClients {
    server: HeadlessServer,
    first_tab_id: String,
    second_tab_id: String,
    first_pane_id: String,
    /// Held so the test clients' writers stay open for the whole test.
    _render_receivers: Vec<std::sync::mpsc::Receiver<Vec<u8>>>,
}

/// One workspace with two tabs; client 31 (A) sits on tab 1 and client 32 (B) on
/// tab 2. Client 32 attached last, so it starts as the foreground client.
fn two_clients_on_different_tabs() -> TwoClients {
    let mut server = test_headless_server();
    let mut workspace = crate::workspace::Workspace::test_new("popup-owner");
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

    let (first_control, first_render) = connect_matching_test_shell(&mut server, 31);
    let (second_control, second_render) = connect_matching_test_shell(&mut server, 32);
    let _ = first_control.recv().expect("first snapshot");
    let _ = second_control.recv().expect("second snapshot");
    assert!(server.focus_shell_client_on_tab(32, &second_tab_id));
    TwoClients {
        server,
        first_tab_id,
        second_tab_id,
        first_pane_id,
        _render_receivers: vec![first_render, second_render],
    }
}

fn client_request(server: &mut HeadlessServer, client_id: u64, method: Method) -> String {
    let (respond_to, response_rx) = std::sync::mpsc::channel();
    server.handle_client_shell_api_request(
        client_id,
        crate::api::ApiRequestMessage {
            request: Request {
                id: "client-request".into(),
                method,
            },
            respond_to,
            response_write_complete: None,
        },
    );
    response_rx.recv().expect("client request response")
}

#[cfg(unix)]
fn public_request(server: &mut HeadlessServer, method: Method) -> String {
    let (respond_to, response_rx) = std::sync::mpsc::channel();
    server.handle_api_request_with_shutdown_check(crate::api::ApiRequestMessage {
        request: Request {
            id: "public-request".into(),
            method,
        },
        respond_to,
        response_write_complete: None,
    });
    response_rx.recv().expect("public request response")
}

/// Client B navigates last (so the server's focused tab is B's), then the user
/// types in client A, which is therefore the foreground client.
fn navigate_in_b_then_type_in_a(fixture: &mut TwoClients) {
    let second_tab_id = fixture.second_tab_id.clone();
    let response = client_request(
        &mut fixture.server,
        32,
        Method::TabFocus(crate::api::schema::TabTarget {
            tab_id: second_tab_id.clone(),
        }),
    );
    assert!(response.contains("\"result\""), "{response}");
    fixture
        .server
        .handle_server_event(ServerEvent::ClientShellPaneInput {
            client_id: 31,
            pane_id: fixture.first_pane_id.clone(),
            events: vec![crate::protocol::ClientPaneInputEvent::TextCommit(
                "typed-in-A".into(),
            )],
        });
}

#[tokio::test]
async fn typing_in_a_client_makes_it_foreground_while_the_server_focus_stays_elsewhere() {
    let mut fixture = two_clients_on_different_tabs();

    navigate_in_b_then_type_in_a(&mut fixture);

    // The facts the issue relies on: A is foreground, tab 2 is still the
    // server-wide focused tab.
    assert_eq!(fixture.server.foreground_client_id, Some(31));
    assert_eq!(
        fixture
            .server
            .default_shell_target()
            .and_then(|target| fixture
                .server
                .app
                .public_tab_id(target.workspace_index, target.tab_index)),
        Some(fixture.second_tab_id.clone())
    );
    assert_eq!(
        fixture.server.api_popup_owner_tab_id(),
        Some(fixture.first_tab_id.clone())
    );
    shutdown_test_runtimes(&mut fixture.server);
}

#[tokio::test]
async fn api_popup_owner_is_none_without_a_shell_client() {
    let mut server = test_headless_server();
    server.app.state.workspaces = vec![crate::workspace::Workspace::test_new("no-clients")];
    server.app.state.active = Some(0);

    assert_eq!(server.api_popup_owner_tab_id(), None);
}

#[cfg(unix)]
mod end_to_end {
    use super::*;
    use crate::api::schema::{
        ErrorResponse, PluginLinkParams, PluginPaneOpenParams, PluginPopupCloseParams,
        ResponseResult, SuccessResponse,
    };

    fn link_popup_plugin(server: &mut HeadlessServer, name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "herdr-popup-owner-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("herdr-plugin.toml"),
            format!(
                r#"
id = "example.{name}"
name = "Popup Owner Plugin"
version = "0.1.0"
min_herdr_version = "0.6.10"
platforms = ["linux", "macos"]

[[panes]]
id = "desk"
title = "Desk"
placement = "popup"
command = ["sh", "-c", "sleep 30"]
"#
            ),
        )
        .unwrap();
        let linked = server.app.handle_api_request(Request {
            id: "link".into(),
            method: Method::PluginLink(PluginLinkParams {
                path: root.display().to_string(),
                enabled: true,
                source: None,
            }),
        });
        assert!(linked.contains("plugin_linked"), "{linked}");
        root
    }

    fn open_popup(plugin: &str) -> Method {
        Method::PluginPaneOpen(PluginPaneOpenParams {
            plugin_id: format!("example.{plugin}"),
            entrypoint: "desk".into(),
            placement: None,
            width: None,
            height: None,
            workspace_id: None,
            target_pane_id: None,
            direction: None,
            cwd: None,
            focus: false,
            env: std::collections::HashMap::new(),
        })
    }

    fn assert_ok(response: &str) {
        let success: SuccessResponse = serde_json::from_str(response)
            .unwrap_or_else(|err| panic!("expected success, got {response}: {err}"));
        assert_eq!(success.result, ResponseResult::Ok {});
    }

    fn error_of(response: &str) -> ErrorResponse {
        serde_json::from_str(response)
            .unwrap_or_else(|err| panic!("expected error, got {response}: {err}"))
    }

    #[tokio::test]
    async fn api_opened_popup_lands_on_the_tab_of_the_client_being_typed_in() {
        let mut fixture = two_clients_on_different_tabs();
        let root = link_popup_plugin(&mut fixture.server, "owner-api");
        navigate_in_b_then_type_in_a(&mut fixture);

        let response = public_request(&mut fixture.server, open_popup("owner-api"));

        assert_ok(&response);
        assert_eq!(
            fixture.server.popup_owner_tab_id,
            Some(fixture.first_tab_id.clone())
        );
        shutdown_test_runtimes(&mut fixture.server);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn api_opened_popup_without_a_shell_client_falls_back_to_the_focused_tab() {
        let mut fixture = two_clients_on_different_tabs();
        let root = link_popup_plugin(&mut fixture.server, "owner-fallback");
        fixture.server.app.state.workspaces[0].switch_tab(1);
        assert!(fixture
            .server
            .handle_server_event(ServerEvent::ClientDisconnected { client_id: 31 }));
        assert!(fixture
            .server
            .handle_server_event(ServerEvent::ClientDisconnected { client_id: 32 }));
        assert_eq!(fixture.server.foreground_client_id, None);

        let response = public_request(&mut fixture.server, open_popup("owner-fallback"));

        assert_ok(&response);
        assert_eq!(
            fixture.server.popup_owner_tab_id,
            Some(fixture.second_tab_id.clone())
        );
        shutdown_test_runtimes(&mut fixture.server);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn popup_opened_from_a_clients_own_request_keeps_that_clients_tab() {
        let mut fixture = two_clients_on_different_tabs();
        let root = link_popup_plugin(&mut fixture.server, "owner-client");
        navigate_in_b_then_type_in_a(&mut fixture);
        assert_eq!(fixture.server.foreground_client_id, Some(31));

        // Client B asks for the popup itself, as its popup key would.
        let response = client_request(&mut fixture.server, 32, open_popup("owner-client"));

        assert_ok(&response);
        assert_eq!(
            fixture.server.popup_owner_tab_id,
            Some(fixture.second_tab_id.clone())
        );
        shutdown_test_runtimes(&mut fixture.server);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn closing_by_plugin_only_closes_the_popup_that_plugin_opened() {
        let mut fixture = two_clients_on_different_tabs();
        let root = link_popup_plugin(&mut fixture.server, "owner-close");
        navigate_in_b_then_type_in_a(&mut fixture);
        assert_ok(&public_request(
            &mut fixture.server,
            open_popup("owner-close"),
        ));
        assert_eq!(
            fixture
                .server
                .app
                .state
                .popup_pane
                .as_ref()
                .and_then(|popup| popup.plugin_id.as_deref()),
            Some("example.owner-close")
        );

        let refused = error_of(&public_request(
            &mut fixture.server,
            Method::PluginPopupClose(PluginPopupCloseParams {
                plugin_id: "example.somebody-else".into(),
            }),
        ));

        assert_eq!(refused.error.code, "popup_not_owned");
        assert!(fixture.server.app.state.popup_pane.is_some());
        assert!(fixture.server.popup_owner_tab_id.is_some());

        assert_ok(&public_request(
            &mut fixture.server,
            Method::PluginPopupClose(PluginPopupCloseParams {
                plugin_id: "example.owner-close".into(),
            }),
        ));

        assert!(fixture.server.app.state.popup_pane.is_none());
        assert_eq!(fixture.server.popup_owner_tab_id, None);
        let none_open = error_of(&public_request(
            &mut fixture.server,
            Method::PopupClose(crate::api::schema::EmptyParams::default()),
        ));
        assert_eq!(none_open.error.code, "popup_not_open");
        shutdown_test_runtimes(&mut fixture.server);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn popup_busy_refusal_names_the_popups_tab_and_the_close_command() {
        let mut fixture = two_clients_on_different_tabs();
        let root = link_popup_plugin(&mut fixture.server, "owner-busy");
        navigate_in_b_then_type_in_a(&mut fixture);
        assert_ok(&public_request(
            &mut fixture.server,
            open_popup("owner-busy"),
        ));

        // The next open, from client B's key, hits the single popup slot.
        let busy = error_of(&client_request(
            &mut fixture.server,
            32,
            open_popup("owner-busy"),
        ));

        assert_eq!(busy.error.code, "ui_busy");
        assert!(
            busy.error.message.contains("tab \"1\""),
            "{}",
            busy.error.message
        );
        assert!(
            busy.error.message.contains("herdr popup close"),
            "{}",
            busy.error.message
        );
        shutdown_test_runtimes(&mut fixture.server);
        let _ = std::fs::remove_dir_all(root);
    }
}
