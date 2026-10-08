use super::*;
use crate::protocol::{ClientKeyCode, ClientKeyKind, ClientPaneInputEvent};
use crate::server::sync_input::{SyncInputDisposition, SyncInputRecipient};

impl HeadlessServer {
    fn captured_input_runtime(
        &self,
        recipient: &SyncInputRecipient,
    ) -> Option<&crate::terminal::TerminalRuntime> {
        let runtime = self.app.terminal_runtimes.get(&recipient.terminal_id);
        #[cfg(test)]
        let runtime = runtime.or_else(|| {
            self.app.state.workspaces.iter().find_map(|workspace| {
                workspace
                    .test_runtimes
                    .values()
                    .find(|runtime| runtime.has_input_identity(&recipient.input_identity))
            })
        });
        runtime.filter(|runtime| runtime.has_input_identity(&recipient.input_identity))
    }

    pub(super) fn deliver_captured_input(
        &self,
        recipients: &[SyncInputRecipient],
        event: &ClientPaneInputEvent,
    ) -> bool {
        let mut changed = false;
        for recipient in recipients {
            let Some(runtime) = self.captured_input_runtime(recipient) else {
                continue;
            };
            let before = runtime.scroll_metrics();
            match recipient.disposition {
                SyncInputDisposition::HostPage { up } => {
                    if !matches!(
                        event,
                        ClientPaneInputEvent::Key {
                            kind: ClientKeyKind::Release,
                            ..
                        }
                    ) {
                        let lines = runtime.current_size().0.max(1) as usize;
                        if up {
                            runtime.scroll_up(lines);
                        } else {
                            runtime.scroll_down(lines);
                        }
                    }
                }
                SyncInputDisposition::Typed => {
                    if let Err(err) = super::super::pane_input::apply_client_typed_input_events(
                        runtime,
                        std::slice::from_ref(event),
                    ) {
                        warn!(terminal_id = %recipient.terminal_id, err = %err, "synchronized client input failed");
                    }
                }
            }
            changed |= runtime.scroll_metrics() != before;
        }
        changed
    }

    fn sync_input_recipients(
        &self,
        workspace_index: usize,
        pane_id: crate::layout::PaneId,
        event: &ClientPaneInputEvent,
    ) -> Vec<SyncInputRecipient> {
        let Some(workspace) = self.app.state.workspaces.get(workspace_index) else {
            return Vec::new();
        };
        let Some(tab_index) = workspace.find_tab_index_for_pane(pane_id) else {
            return Vec::new();
        };
        let runtime = self.app.state.runtime_for_pane_in_workspace(
            &self.app.terminal_runtimes,
            workspace_index,
            pane_id,
        );
        let host_page = match event {
            ClientPaneInputEvent::Key {
                code: ClientKeyCode::PageUp,
                modifiers: 0,
                ..
            } => Some(true),
            ClientPaneInputEvent::Key {
                code: ClientKeyCode::PageDown,
                modifiers: 0,
                ..
            } => Some(false),
            _ => None,
        }
        .filter(|_| {
            runtime
                .is_some_and(|runtime| runtime.plain_page_keys_use_host_scrollback() == Some(true))
        });
        let panes = std::iter::once(pane_id).chain(
            if host_page.is_some()
                || matches!(
                    event,
                    ClientPaneInputEvent::Key {
                        kind: ClientKeyKind::Release,
                        ..
                    }
                )
            {
                Vec::new()
            } else {
                workspace.tabs[tab_index].sync_peers(pane_id)
            },
        );
        let mut seen = HashSet::new();
        panes
            .filter_map(|pane_id| {
                let terminal_id = workspace.terminal_id(pane_id)?.clone();
                if !seen.insert(terminal_id.clone()) {
                    return None;
                }
                let runtime = self.app.state.runtime_for_pane_in_workspace(
                    &self.app.terminal_runtimes,
                    workspace_index,
                    pane_id,
                )?;
                Some(SyncInputRecipient {
                    terminal_id,
                    input_identity: runtime.input_identity(),
                    disposition: host_page.map_or(SyncInputDisposition::Typed, |up| {
                        SyncInputDisposition::HostPage { up }
                    }),
                })
            })
            .collect()
    }

    pub(super) fn route_synced_pane_input(
        &mut self,
        client_id: u64,
        pane_id: String,
        events: Vec<ClientPaneInputEvent>,
    ) -> bool {
        if self.handoff_in_progress
            || !self
                .clients
                .get(&client_id)
                .is_some_and(ClientConnection::is_shell_client)
        {
            return false;
        }
        let active = self
            .clients
            .get(&client_id)
            .is_some_and(ClientConnection::is_active_shell_client);
        let pixel_mouse = self.clients.get(&client_id).is_some_and(|client| {
            client.pixel_mouse && client.host_sgr_pixels_active == Some(true)
        });
        let mut changed = false;
        let mut claimed_geometry = false;
        for mut event in events {
            let continuation = self
                .clients
                .get_mut(&client_id)
                .and_then(|client| client.sync_input_leases.continuation(&event));
            if let Some((recipients, continuation)) = continuation {
                let release = client_pane_input_releases_press(&continuation);
                let origin_visible = self.app.parse_pane_id(&pane_id).is_some_and(
                    |(workspace_index, runtime_pane_id)| {
                        self.shell_client_views_pane(client_id, workspace_index, runtime_pane_id)
                    },
                );
                let popup_blocks = self.app.state.popup_pane.is_some()
                    && self.popup_owner_tab_id == self.shell_tab_id_for_client(client_id);
                if release || (active && origin_visible && !popup_blocks) {
                    if !release
                        && !claimed_geometry
                        && client_pane_input_has_interaction(std::slice::from_ref(&continuation))
                    {
                        changed |= self.promote_client_to_foreground(client_id);
                        changed |= self.claim_shell_tab_geometry(client_id, false);
                        claimed_geometry = true;
                    }
                    changed |= self.deliver_captured_input(&recipients, &continuation);
                }
                continue;
            }
            let retired = self
                .clients
                .get_mut(&client_id)
                .and_then(|client| client.sync_input_leases.retire_before_press(&event));
            if let Some(retired) = retired {
                changed |= self.deliver_captured_input(&retired.recipients, &retired.release);
            }
            if !active
                || matches!(
                    event,
                    ClientPaneInputEvent::Key {
                        kind: ClientKeyKind::Repeat,
                        tracks_release: true,
                        ..
                    }
                )
            {
                continue;
            }
            let Some((workspace_index, runtime_pane_id)) = self.app.parse_pane_id(&pane_id) else {
                continue;
            };
            let popup_blocks = self.app.state.popup_pane.is_some()
                && self.popup_owner_tab_id == self.shell_tab_id_for_client(client_id);
            if (popup_blocks
                || !self.shell_client_views_pane(client_id, workspace_index, runtime_pane_id))
                && !client_pane_input_releases_press(&event)
            {
                continue;
            }
            if !claimed_geometry && client_pane_input_has_interaction(std::slice::from_ref(&event))
            {
                changed |= self.promote_client_to_foreground(client_id);
                changed |= self.claim_shell_tab_geometry(client_id, false);
                claimed_geometry = true;
            }
            if matches!(event, ClientPaneInputEvent::Mouse { .. }) {
                let Some(runtime) = self.app.state.runtime_for_pane_in_workspace(
                    &self.app.terminal_runtimes,
                    workspace_index,
                    runtime_pane_id,
                ) else {
                    continue;
                };
                super::super::pane_input::downgrade_ineligible_pixel_mouse(
                    std::slice::from_mut(&mut event),
                    pixel_mouse,
                    runtime.current_size(),
                    runtime.pixel_size(),
                );
                if let Some(client) = self.clients.get_mut(&client_id) {
                    client.track_shell_input(
                        ClientShellInputTarget::Pane(pane_id.clone()),
                        std::slice::from_ref(&event),
                    );
                }
                let before = runtime.scroll_metrics();
                if let Err(err) =
                    apply_client_pane_input_events(runtime, std::slice::from_ref(&event))
                {
                    warn!(client_id, pane_id, err = %err, "targeted client shell mouse input failed");
                }
                changed |= runtime.scroll_metrics() != before;
                continue;
            }
            let recipients = self.sync_input_recipients(workspace_index, runtime_pane_id, &event);
            if let Some(client) = self.clients.get_mut(&client_id) {
                let _ = client.sync_input_leases.press(recipients.clone(), &event);
            }
            changed |= self.deliver_captured_input(&recipients, &event);
        }
        changed
    }
}
