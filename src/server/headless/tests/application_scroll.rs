use super::*;
use crate::api::schema::{
    Method, PaneApplicationScrollIntent, PaneScrollApplicationParams, Request,
};

fn public_scroll(server: &mut HeadlessServer, params: PaneScrollApplicationParams) -> String {
    let (respond_to, response_rx) = std::sync::mpsc::channel();
    server.handle_api_request_with_shutdown_check(crate::api::ApiRequestMessage {
        request: Request {
            id: "application-scroll".into(),
            method: Method::PaneScrollApplication(params),
        },
        respond_to,
        response_write_complete: None,
    });
    response_rx.recv().unwrap()
}

#[tokio::test]
async fn application_scroll_rejects_hidden_stale_inactive_and_handoff_targets() {
    for reason in ["hidden", "stale", "inactive", "handoff", "popup"] {
        let mut server = test_headless_server();
        let mut input = install_focused_test_runtime(&mut server, b"\x1b[?1049h\x1b[>3u");
        let focused = server.app.state.workspaces[0].tabs[0].root_pane;
        let hidden_tab = server.app.state.workspaces[0].test_add_tab(Some("hidden"));
        let hidden = server.app.state.workspaces[0].tabs[hidden_tab].root_pane;
        server.app.state.workspaces[0].switch_tab(0);
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
        server.clients.get_mut(&41).unwrap().shell_surface_active = reason != "inactive";
        server.handoff_in_progress = reason == "handoff";
        if reason == "popup" {
            let (runtime, _popup_input) =
                crate::terminal::TerminalRuntime::test_with_channel(40, 12);
            server.app.install_test_popup_runtime(runtime);
            server.popup_owner_tab_id = server.shell_tab_id_for_client(41);
        }
        let pane_id = server
            .app
            .public_pane_id(0, if reason == "hidden" { hidden } else { focused })
            .unwrap();
        let boot_id = if reason == "stale" {
            "previous-boot".into()
        } else {
            server.client_shell_boot_id.clone()
        };
        assert!(
            !server.handle_client_shell_endpoint_request(
                41,
                boot_id,
                Box::new(Request {
                    id: format!("scroll-{reason}"),
                    method: Method::PaneScrollApplication(PaneScrollApplicationParams {
                        pane_id,
                        intent: PaneApplicationScrollIntent::PageUp,
                        count: 1,
                    }),
                })
            ),
            "{reason}"
        );
        assert!(
            input.try_recv().is_err(),
            "{reason} must not type into a pane"
        );
        assert!(
            server.clients.contains_key(&41),
            "rejection is endpoint-local"
        );
        assert_eq!(server.foreground_client_id, None);
        shutdown_test_runtimes(&mut server);
    }
}

#[tokio::test]
async fn application_scroll_validates_count_and_drops_lost_screen() {
    let mut server = test_headless_server();
    let mut input = install_focused_test_runtime(&mut server, b"primary-screen");
    let pane_id = server.app.session_snapshot().focused_pane_id.unwrap();
    for count in [0, 65] {
        let response = public_scroll(
            &mut server,
            PaneScrollApplicationParams {
                pane_id: pane_id.clone(),
                intent: PaneApplicationScrollIntent::WheelUp,
                count,
            },
        );
        let response: crate::api::schema::ErrorResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(response.error.code, "invalid_params");
    }
    let response = public_scroll(
        &mut server,
        PaneScrollApplicationParams {
            pane_id,
            intent: PaneApplicationScrollIntent::WheelUp,
            count: 1,
        },
    );
    let _: crate::api::schema::SuccessResponse = serde_json::from_str(&response).unwrap();
    assert!(input.try_recv().is_err());
    shutdown_test_runtimes(&mut server);
}
