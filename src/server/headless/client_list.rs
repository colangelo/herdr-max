use super::*;

impl HeadlessServer {
    /// Count one forwarded input event, or a command the client issued, against
    /// that client's idle time. Runs once per event batch in the server's event
    /// handler: nothing per frame and nothing per pane.
    pub(super) fn record_client_input(&mut self, client_id: u64) {
        if let Some(client) = self.clients.get_mut(&client_id) {
            if client.is_shell_client() {
                client.record_input(Instant::now());
            }
        }
    }

    /// The attached shell clients, oldest connection first, as `client.list`
    /// reports them. Local and remote attachments are the same kind of
    /// connection here, so both appear.
    pub(super) fn client_list_infos(&self, now: Instant) -> Vec<api::schema::ClientInfo> {
        let mut client_ids = self
            .clients
            .iter()
            .filter(|(_, client)| client.is_shell_client())
            .map(|(&client_id, _)| client_id)
            .collect::<Vec<_>>();
        client_ids.sort_unstable();
        client_ids
            .into_iter()
            .filter_map(|client_id| {
                let client = self.clients.get(&client_id)?;
                let target = self.shell_target_for_client(client_id);
                let tab_id = self.shell_tab_id_for_client(client_id);
                let views_popup = client.is_active_shell_client()
                    && self.app.state.popup_pane.is_some()
                    && self.popup_owner_tab_id.is_some()
                    && self.popup_owner_tab_id == tab_id;
                Some(api::schema::ClientInfo {
                    client_id: client_id.to_string(),
                    foreground: self.foreground_client_id == Some(client_id),
                    workspace_id: target
                        .as_ref()
                        .map(|target| self.app.public_workspace_id(target.workspace_index)),
                    tab_id,
                    last_input_age_ms: client
                        .last_input_age(now)
                        .map(|age| u64::try_from(age.as_millis()).unwrap_or(u64::MAX)),
                    window_focused: client.outer_terminal_focus,
                    views_popup,
                })
            })
            .collect()
    }

    pub(super) fn handle_client_list_api(&self, id: String) -> String {
        serde_json::to_string(&api::schema::SuccessResponse {
            id,
            result: api::schema::ResponseResult::ClientList {
                clients: self.client_list_infos(Instant::now()),
            },
        })
        .unwrap_or_else(|_| "{}".to_string())
    }
}

/// Commands a client issues over the endpoint lane that are not the user acting:
/// reads its overlays re-run on their own ticks, and bookkeeping.
pub(super) fn endpoint_method_is_passive(method: &api::schema::Method) -> bool {
    let name = api::api_method_name(method);
    name.ends_with(".list")
        || matches!(
            name,
            "pane.selection.read"
                | "pane.link.resolve"
                | "notification.mark_seen"
                | "client_shell.surface.set"
        )
}
