use super::*;
use crate::api::schema::{EmptyParams, Method, Request};

fn view_server() -> (HeadlessServer, std::sync::mpsc::Receiver<Vec<u8>>, u64) {
    let mut server = test_headless_server();
    server.app.view = Some(crate::app::view_layout::ViewApp::new(
        crate::core_machine::MachineName::parse("loop").unwrap(),
    ));
    server.app.state.workspaces = vec![crate::workspace::Workspace::test_new("w")];
    server.app.state.ensure_test_terminals();
    server.app.state.active = Some(0);
    let (writer, control_rx, _render_rx) = test_client_writer();
    let client_id = 41;
    server.handle_server_event(ServerEvent::ClientShellConnected {
        surface_reuse: false,
        surface_delta: false,
        surface_scroll: false,
        client_id,
        surface_cols: 80,
        surface_rows: 23,
        cell_width_px: 0,
        cell_height_px: 0,
        pixel_mouse: false,
        direct_graphics: false,
        endpoint_keybindings: false,
        mouse_capture: false,
        surface_active: true,
        writer,
    });
    let _initial_snapshot = client_shell_snapshot(&control_rx);
    (server, control_rx, client_id)
}

fn error_code(control_rx: &std::sync::mpsc::Receiver<Vec<u8>>) -> String {
    loop {
        let message = read_server_message(
            control_rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .expect("a response"),
        );
        if let ServerMessage::ClientShellEndpointResponseChunk { data, .. } = message {
            return serde_json::from_slice::<api::schema::ErrorResponse>(&data)
                .expect("an error response")
                .error
                .code;
        }
    }
}

// M2 (core-phase5 D6): the view's dispatch takes the one allowlist its
// welcome advertises. The review's named methods, sent anyway by a client,
// are refused before dispatch: no command is in flight, nothing reaches a
// pane, a helper or a file. A method the view takes is dispatched.
#[tokio::test]
async fn a_view_refuses_every_method_outside_its_allowlist_before_dispatch() {
    let (mut server, control_rx, client_id) = view_server();
    let pane_id = server
        .app
        .public_pane_id(0, server.app.state.workspaces[0].tabs[0].root_pane)
        .unwrap();
    let refused: Vec<Method> = [
        (
            "agent.message",
            serde_json::json!({"target": pane_id, "text": "hi"}),
        ),
        (
            "integration.install",
            serde_json::json!({"target": "claude"}),
        ),
        (
            "pane.edit_scrollback",
            serde_json::json!({"pane_id": pane_id}),
        ),
        ("command.invoke", serde_json::json!({"command_id": "x"})),
        ("worktree.list", serde_json::json!({})),
        ("worktree.open", serde_json::json!({"path": "/tmp"})),
        (
            "release_notes.dismiss",
            serde_json::json!({"version": "0.0.1"}),
        ),
        (
            "pane.swap",
            serde_json::json!({"pane_id": pane_id, "direction": "right"}),
        ),
        ("workspace.create", serde_json::json!({})),
    ]
    .into_iter()
    .map(|(method, params)| {
        serde_json::from_value(serde_json::json!({"method": method, "params": params}))
            .unwrap_or_else(|err| panic!("{method}: {err}"))
    })
    .collect();
    let boot_id = server.client_shell_boot_id.clone();
    for (index, method) in refused.into_iter().enumerate() {
        let name = method.operation().name;
        assert!(
            !server.handle_server_event(ServerEvent::ClientShellEndpointRequest {
                client_id,
                boot_id: boot_id.clone(),
                request: Box::new(Request {
                    id: format!("view:{index}"),
                    method,
                }),
            }),
            "{name}"
        );
        assert!(
            !server.clients[&client_id].shell_endpoint_command_in_flight,
            "{name} was dispatched"
        );
        assert_eq!(error_code(&control_rx), "refused_in_view", "{name}");
    }
    assert!(server.app.overlay_panes.is_empty(), "no editor pane");

    // A method the view takes goes on to dispatch.
    server.handle_server_event(ServerEvent::ClientShellEndpointRequest {
        client_id,
        boot_id,
        request: Box::new(Request {
            id: "view:list".into(),
            method: Method::NotificationList(EmptyParams::default()),
        }),
    });
    assert!(server.clients[&client_id].shell_endpoint_command_in_flight);
    shutdown_test_runtimes(&mut server);
}

// E.3 (D7): a structural action in a view is never done by the view: it
// takes the far herdr's path, off the loop, and its answer is the action's
// own. Here the target is a pane the far document did not show, so it is
// refused unsent, and nothing changed in the view's layout.
#[tokio::test]
async fn a_views_structural_action_is_never_done_here() {
    let (mut server, control_rx, client_id) = view_server();
    let pane_id = server
        .app
        .public_pane_id(0, server.app.state.workspaces[0].tabs[0].root_pane)
        .unwrap();
    let panes_before = server.app.state.workspaces[0].tabs[0].layout.pane_count();
    let method: Method = serde_json::from_value(serde_json::json!({
        "method": "pane.split",
        "params": {"target_pane_id": pane_id, "direction": "right"}
    }))
    .expect("pane.split");
    let boot_id = server.client_shell_boot_id.clone();
    server.handle_server_event(ServerEvent::ClientShellEndpointRequest {
        client_id,
        boot_id,
        request: Box::new(Request {
            id: "view:split".into(),
            method,
        }),
    });
    assert!(
        server.clients[&client_id].shell_endpoint_command_in_flight,
        "sent on, not refused as a far layout change"
    );
    let ready = loop {
        let event = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            server.server_event_rx.recv(),
        )
        .await
        .expect("an answer in time")
        .expect("the channel");
        if matches!(
            event,
            ServerEvent::ClientShellEndpointResponseChunkReady { .. }
        ) {
            break event;
        }
    };
    server.handle_server_event(ready);
    assert_eq!(error_code(&control_rx), "view_action_failed");
    assert_eq!(
        server.app.state.workspaces[0].tabs[0].layout.pane_count(),
        panes_before,
        "no split here"
    );
    shutdown_test_runtimes(&mut server);
}

// M2: a view stages no clipboard image on this Mac (the far program could
// not read its path anyway): the paste is refused and the client told.
#[tokio::test]
async fn a_view_stages_no_clipboard_image() {
    let (mut server, control_rx, client_id) = view_server();
    let pane_id = server
        .app
        .public_pane_id(0, server.app.state.workspaces[0].tabs[0].root_pane)
        .unwrap();
    assert!(
        !server.handle_server_event(ServerEvent::ClientClipboardImage {
            client_id,
            target: crate::protocol::ClientClipboardImageTarget::Pane(pane_id),
            extension: "png".into(),
            data: b"\x89PNG".to_vec(),
        })
    );
    let refusal = loop {
        if let ServerMessage::ClientShellError { message } = read_server_message(
            control_rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .expect("a message"),
        ) {
            break message;
        }
    };
    assert!(refusal.contains("view of loop"), "{refusal}");
    shutdown_test_runtimes(&mut server);
}

// Review L4: a view action that leaves its workspace out goes to the
// sending client's own workspace, not wherever this server's focus is
// (another client may have moved it); a client whose window shows no tab
// here is refused.
#[tokio::test]
async fn a_view_action_resolves_left_out_ids_against_the_clients_own_tab() {
    use crate::core_machine::fleet::view::tests::{added, far_ok, input_of};
    let core = crate::pane::FakeCore::start_with(|config| {
        config.exec_handler = Some(std::sync::Arc::new(far_ok));
    });
    core.set_machine("loop", "connected");
    let (root, _dir) = added("view-client-tab");
    let (mut server, control_rx, client_id) = view_server();
    server.app.state.workspaces = vec![
        crate::workspace::Workspace::test_new("a"),
        crate::workspace::Workspace::test_new("b"),
    ];
    server.app.state.ensure_test_terminals();
    server.app.state.active = Some(0);
    server
        .app
        .view
        .as_mut()
        .unwrap()
        .set_route(crate::core_machine::fleet::view::ViewRoute::at(
            &core.socket,
            root.clone(),
            std::sync::Arc::new(|_| Ok(serde_json::Value::Null)),
        ));
    let tab_create = |id: &str| Request {
        id: id.into(),
        method: serde_json::from_value(serde_json::json!({
            "method": "tab.create", "params": {"label": "build"}
        }))
        .expect("tab.create"),
    };
    let answer = |server: &mut HeadlessServer, request: Request| {
        let boot_id = server.client_shell_boot_id.clone();
        server.handle_server_event(ServerEvent::ClientShellEndpointRequest {
            client_id,
            boot_id,
            request: Box::new(request),
        });
    };

    // No tab of this server in the client's window: refused.
    server.clients.get_mut(&client_id).unwrap().shell_location = None;
    answer(&mut server, tab_create("view:none"));
    let ready = next_response_chunk(&mut server).await;
    server.handle_server_event(ready);
    let refused = response_value(&control_rx);
    assert_eq!(refused["error"]["code"], "view_action_failed");
    assert!(
        refused["error"]["message"]
            .as_str()
            .unwrap()
            .contains("this window shows no tab of loop"),
        "{refused}"
    );

    // The client's window shows workspace b; this server's focus is on a.
    let b = server.app.state.workspaces[1].id.clone();
    let b_tab = server.app.public_tab_id(1, 0).unwrap();
    server.clients.get_mut(&client_id).unwrap().shell_location =
        Some(crate::server::clients::ClientShellLocation {
            focused_workspace_id: Some(b.clone()),
            active_tab_ids: std::collections::HashMap::from([(b.clone(), b_tab)]),
        });
    answer(&mut server, tab_create("view:b"));
    let ready = next_response_chunk(&mut server).await;
    server.handle_server_event(ready);
    assert_eq!(response_value(&control_rx)["result"]["type"], "ok");
    let exec = core
        .requests()
        .into_iter()
        .find(|request| request["method"] == "machine.exec")
        .expect("sent");
    assert_eq!(input_of(&exec["params"])["params"]["workspace_id"], b);
    let _ = std::fs::remove_dir_all(&root);
    shutdown_test_runtimes(&mut server);
}

async fn next_response_chunk(server: &mut HeadlessServer) -> ServerEvent {
    loop {
        let event = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            server.server_event_rx.recv(),
        )
        .await
        .expect("an answer in time")
        .expect("the channel");
        if matches!(
            event,
            ServerEvent::ClientShellEndpointResponseChunkReady { .. }
        ) {
            return event;
        }
    }
}

fn response_value(control_rx: &std::sync::mpsc::Receiver<Vec<u8>>) -> serde_json::Value {
    loop {
        let message = read_server_message(
            control_rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .expect("a response"),
        );
        if let ServerMessage::ClientShellEndpointResponseChunk { data, .. } = message {
            return serde_json::from_slice(&data).expect("a response");
        }
    }
}
