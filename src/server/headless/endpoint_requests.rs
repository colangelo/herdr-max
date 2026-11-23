use super::*;

impl HeadlessServer {
    pub(super) fn handle_client_shell_endpoint_request(
        &mut self,
        client_id: u64,
        boot_id: String,
        mut request: Box<api::schema::Request>,
        view: Option<crate::protocol::endpoint::EndpointViewAction>,
    ) -> bool {
        let Some(client) = self.clients.get(&client_id) else {
            return false;
        };
        if !matches!(client.mode, ClientConnectionMode::ClientShell) {
            self.remove_client_and_resize_if_needed(client_id);
            return true;
        }
        let request_id = request.id.clone();
        if !crate::server::client_commands::supports_client_shell_method(&request.method) {
            let message = crate::server::client_commands::error_message(
                boot_id,
                request_id,
                "unsupported_endpoint_command",
                "this method is not available through the client shell command lane",
            );
            self.send_to_client(client_id, message);
            return false;
        }
        // One allowlist for a view, the one its welcome advertised
        // (core-phase5 D6, security review M2): a client that sends any
        // other method is refused before dispatch.
        #[cfg(unix)]
        if let Some(refusal) = self
            .app
            .view_method_refusal(request.method.operation().name)
        {
            let message = crate::server::client_commands::error_message(
                boot_id,
                request_id,
                "refused_in_view",
                refusal,
            );
            self.send_to_client(client_id, message);
            return false;
        }
        if boot_id != self.client_shell_boot_id {
            let message = crate::server::client_commands::error_message(
                boot_id,
                request_id,
                "stale_boot",
                "endpoint command targeted an earlier server boot",
            );
            self.send_to_client(client_id, message);
            return false;
        }
        let surface_active = client.shell_surface_active;
        if let api::schema::Method::ClientShellSurfaceSet(params) = &request.method {
            let Some((changed, projection_revision)) =
                self.set_client_shell_surface_active(client_id, params.active)
            else {
                return false;
            };
            self.send_to_client(
                client_id,
                crate::server::client_commands::success_message_with_result(
                    boot_id,
                    request_id,
                    api::schema::ResponseResult::ClientShellSurfaceSet {
                        active: params.active,
                        projection_revision,
                    },
                ),
            );
            return changed;
        }
        if client.shell_endpoint_command_in_flight {
            let message = crate::server::client_commands::error_message(
                boot_id,
                request_id,
                "endpoint_busy",
                "this endpoint is still processing another command",
            );
            self.send_to_client(client_id, message);
            return false;
        }
        if !surface_active {
            let message = crate::server::client_commands::error_message(
                boot_id,
                request_id,
                "surface_inactive",
                "this method requires an active client shell surface",
            );
            self.send_to_client(client_id, message);
            return false;
        }

        if let api::schema::Method::PaneScrollApplication(params) = &request.method {
            let viewed = self
                .app
                .parse_pane_id(&params.pane_id)
                .is_some_and(|(ws, pane)| {
                    self.shell_client_views_pane(client_id, ws, pane)
                        && self
                            .shell_target_for_client(client_id)
                            .and_then(|target| {
                                self.app
                                    .state
                                    .workspaces
                                    .get(target.workspace_index)
                                    .and_then(|workspace| workspace.tabs.get(target.tab_index))
                            })
                            .is_some_and(|tab| tab.layout.focused() == pane)
                });
            let popup_blocks = self.app.state.popup_pane.is_some()
                && self.popup_owner_tab_id == self.shell_tab_id_for_client(client_id);
            // A view's pane whose far terminal changed takes no input until
            // a person clicks or focuses it (review M3).
            #[cfg(unix)]
            let viewed = viewed
                && !self
                    .app
                    .parse_pane_id(&params.pane_id)
                    .is_some_and(|(_, pane)| self.app.view_input_held(pane, client_id));
            if self.handoff_in_progress || !viewed || popup_blocks {
                self.send_to_client(client_id, crate::server::client_commands::error_message(
                    boot_id, request_id, "input_target_unavailable",
                    "application scroll requires the focused visible pane without a blocking popup or handoff",
                ));
                return false;
            }
        }

        let api_request_id = format!(
            "endpoint:{}:{client_id}:{request_id}",
            self.client_shell_boot_id
        );
        request.id = api_request_id.clone();
        let (respond_to, response_rx) = std::sync::mpsc::channel();
        if let Err(err) = crate::server::client_commands::spawn_response_waiter(
            client_id,
            boot_id.clone(),
            request_id.clone(),
            response_rx,
            self.server_event_tx.clone(),
        ) {
            let message = crate::server::client_commands::error_message(
                boot_id,
                request_id,
                "server_unavailable",
                format!("failed to start endpoint response bridge: {err}"),
            );
            self.send_to_client(client_id, message);
            return false;
        }
        if let Some(client) = self.clients.get_mut(&client_id) {
            client.shell_endpoint_command_in_flight = true;
            // A later source restore has a new projection revision. Keep this request's lease
            // so a delayed worktree response cannot focus a pane after endpoint switching.
            client.shell_endpoint_command_surface_revision = Some(client.shell_projection_revision);
            let deferred_worktree = matches!(
                &request.method,
                api::schema::Method::WorktreeCreate(_)
                    | api::schema::Method::WorktreeRemove(_)
                    | api::schema::Method::WorktreeList(_)
                    | api::schema::Method::WorktreeOpen(_)
            );
            let deferred_navigation = match &request.method {
                api::schema::Method::WorktreeCreate(params) => params.focus,
                api::schema::Method::WorktreeOpen(params) => params.focus,
                _ => false,
            };
            client.shell_deferred_navigation_request_id =
                deferred_worktree.then(|| api_request_id.clone());
            client.shell_deferred_navigation_response = deferred_navigation.then(Vec::new);
        }
        if !super::client_list::endpoint_method_is_passive(&request.method) {
            self.record_client_input(client_id);
        }
        let foreground_changed = self.promote_client_to_foreground(client_id);
        let message = api::ApiRequestMessage {
            request: *request,
            respond_to,
            response_write_complete: None,
        };
        let Some(message) = self.take_view_action(client_id, &request_id, view, message) else {
            return foreground_changed;
        };
        foreground_changed | self.handle_client_shell_api_request(client_id, message)
    }

    /// In a view, a structural action goes to the far herdr (core-phase5
    /// D7, E.3), and its answer comes back through `respond_to`, off the
    /// loop: `None`. Any other request, and every request outside a view,
    /// is handed back for this server to do. Ids the action leaves out
    /// resolve against the sending client's own tab, never this server's
    /// focus, which another client may have moved (review L4).
    /// `client_request_id` is the client's own id of the request, and
    /// `view` what it says of its window (fork issue 317 M2, M3).
    #[cfg(unix)]
    fn take_view_action(
        &mut self,
        client_id: u64,
        client_request_id: &str,
        view: Option<crate::protocol::endpoint::EndpointViewAction>,
        message: api::ApiRequestMessage,
    ) -> Option<api::ApiRequestMessage> {
        if self.app.view.is_none()
            || !crate::view::policy::goes_far(message.request.method.operation().name)
        {
            return Some(message);
        }
        let (binding, confirms) = match view {
            Some(view) => (Some(view.binding), view.confirms),
            None => (None, None),
        };
        let sender = crate::app::ViewSender {
            client: client_id,
            tab: self.client_own_tab(client_id),
            binding,
            confirms,
            request: client_request_id.to_string(),
        };
        self.app
            .forward_view_action(message.request, message.respond_to, sender);
        None
    }

    /// A client shell connected: a view holds a pane whose far binding
    /// changes for it until it clicks or focuses the pane (fork issue 317
    /// M2).
    #[cfg(unix)]
    pub(super) fn note_view_client(&mut self, client_id: u64) {
        self.app.note_view_client(client_id);
    }

    #[cfg(not(unix))]
    pub(super) fn note_view_client(&mut self, _client_id: u64) {}

    /// A client left: a view drops its holds, its force confirmations and
    /// the request ids it took (fork issue 317 M2, M3, M5).
    #[cfg(unix)]
    pub(super) fn forget_view_client(&mut self, client_id: u64) {
        self.app.forget_view_client(client_id);
    }

    #[cfg(not(unix))]
    pub(super) fn forget_view_client(&mut self, _client_id: u64) {}

    /// The client whose API request the server does now, for a view's
    /// focus to release that client's hold only (fork issue 317 M2).
    #[cfg(unix)]
    pub(super) fn set_view_acting_client(&mut self, client_id: Option<u64>) {
        self.app.set_view_acting_client(client_id);
    }

    #[cfg(not(unix))]
    pub(super) fn set_view_acting_client(&mut self, _client_id: Option<u64>) {}

    /// The (workspace, tab) a client's own window shows, `None` when it
    /// shows none this server has.
    #[cfg(unix)]
    fn client_own_tab(&self, client_id: u64) -> Option<(usize, usize)> {
        let tab_id = self
            .clients
            .get(&client_id)?
            .shell_location
            .as_ref()?
            .focused_tab_id()?;
        self.app.parse_tab_id(tab_id)
    }

    #[cfg(not(unix))]
    fn take_view_action(
        &mut self,
        _client_id: u64,
        _client_request_id: &str,
        _view: Option<crate::protocol::endpoint::EndpointViewAction>,
        message: api::ApiRequestMessage,
    ) -> Option<api::ApiRequestMessage> {
        Some(message)
    }
}
