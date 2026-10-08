use super::copy_mode::CopyScrollAmount;
use super::*;
use crate::api::schema::{
    Method, PaneApplicationScrollIntent as Intent, PaneScrollApplicationParams,
};
use crossterm::event::{KeyCode, KeyModifiers};

const MAX_PENDING_SCROLL_BATCHES: usize = 32;

pub(super) struct ClientApplicationScroll {
    pub(super) generation: u64,
    endpoint_id: ClientEndpointId,
    endpoint_generation: Option<u64>,
    boot_id: String,
    pane_id: String,
    snapshot_index: usize,
    surface_index: usize,
    keys: HashSet<crate::input::InputLeaseKey<u8>>,
    queued: VecDeque<(Intent, u16)>,
    in_flight: Option<String>,
}

impl ClientShellState {
    fn application_scroll_valid(&self) -> bool {
        self.application_scroll.as_ref().is_some_and(|scroll| {
            self.mode == ClientShellMode::Scroll
                && self.overlay.is_none()
                && !self.popup_pending
                && self.popup_terminal_id.is_none()
                && scroll.endpoint_id == self.active_endpoint_id
                && scroll.endpoint_generation == self.active_snapshot_generation
                && self.endpoint_is_online(&scroll.endpoint_id)
                && self.snapshot.as_deref().is_some_and(|snapshot| {
                    snapshot.boot_id == scroll.boot_id
                        && snapshot.focused_pane_id.as_deref() == Some(scroll.pane_id.as_str())
                        && snapshot.panes.get(scroll.snapshot_index)
                            .is_some_and(|pane| pane.pane_id == scroll.pane_id)
                })
                // A projection gap is not an application exit. The runtime
                // checks live screen state again before encoding each tap.
                && self.pane_surface.as_ref().is_none_or(|surface| {
                    surface.panes.get(scroll.surface_index)
                        .is_some_and(|pane| pane.pane_id == scroll.pane_id && pane.alternate_screen_active)
                })
        })
    }

    // Stable metadata indices make ordinary frame updates constant-time. Only
    // topology changes need a search to relocate the pinned pane.
    fn refresh_application_scroll_indices(&mut self) {
        let Some(scroll) = self.application_scroll.as_mut() else {
            return;
        };
        if let Some(snapshot) = self.snapshot.as_deref() {
            if snapshot
                .panes
                .get(scroll.snapshot_index)
                .is_none_or(|pane| pane.pane_id != scroll.pane_id)
            {
                scroll.snapshot_index = snapshot
                    .panes
                    .iter()
                    .position(|pane| pane.pane_id == scroll.pane_id)
                    .unwrap_or(usize::MAX);
            }
        }
        if let Some(surface) = self.pane_surface.as_ref() {
            if surface
                .panes
                .get(scroll.surface_index)
                .is_none_or(|pane| pane.pane_id != scroll.pane_id)
            {
                scroll.surface_index = surface
                    .panes
                    .iter()
                    .position(|pane| pane.pane_id == scroll.pane_id)
                    .unwrap_or(usize::MAX);
            }
        }
    }

    pub(super) fn reconcile_application_scroll_projection(&mut self) {
        self.refresh_application_scroll_indices();
        if self.application_scroll.is_some() && !self.application_scroll_valid() {
            self.end_application_scroll_projection();
        }
    }

    pub(super) fn end_application_scroll_projection(&mut self) {
        let mut outcome = ClientShellInput::default();
        self.leave_application_scroll(&mut outcome);
        for action in outcome.actions {
            if let ClientShellAction::CancelQueuedEndpoint {
                endpoint_id,
                request_id,
            } = action
            {
                self.pending_application_scroll_cancellations
                    .push((endpoint_id, request_id));
            }
        }
    }

    pub(crate) fn take_application_scroll_cancellations(
        &mut self,
    ) -> Vec<(ClientEndpointId, String)> {
        std::mem::take(&mut self.pending_application_scroll_cancellations)
    }

    pub(super) fn remember_application_scroll_key(&mut self, key: crate::input::InputLeaseKey<u8>) {
        if let Some(scroll) = self.application_scroll.as_mut() {
            scroll.keys.insert(key);
        }
    }

    pub(super) fn forget_application_scroll_key(&mut self, key: &crate::input::InputLeaseKey<u8>) {
        self.retired_scroll_keys.remove(key);
        if let Some(scroll) = self.application_scroll.as_mut() {
            scroll.keys.remove(key);
        }
    }

    pub(super) fn suppress_application_scroll_key(&mut self, key: crate::input::InputLeaseKey<u8>) {
        self.retired_scroll_keys.insert(key);
        self.input_leases
            .insert_consumed(key, crate::input::ConsumedInputLease::SuppressRepeats);
    }

    pub(super) fn restore_retired_scroll_leases(&mut self) {
        for key in &self.retired_scroll_keys {
            self.input_leases
                .insert_consumed(*key, crate::input::ConsumedInputLease::SuppressRepeats);
        }
    }

    pub(super) fn reconcile_application_scroll(&mut self, outcome: &mut ClientShellInput) -> bool {
        if self.application_scroll.is_some() && !self.application_scroll_valid() {
            self.leave_application_scroll(outcome);
            return true;
        }
        false
    }

    pub(super) fn reconcile_application_scroll_focus(&mut self, outcome: &mut ClientShellInput) {
        let requested_new_focus = self.application_scroll.as_ref().is_some_and(|scroll| {
            outcome.actions.iter().any(|action| match action {
                ClientShellAction::ActivateEndpoint { .. } => true,
                ClientShellAction::Endpoint { request, .. } => match &request.method {
                    Method::PaneFocus(target) => target.pane_id != scroll.pane_id,
                    Method::PaneFocusDirection(_) => true,
                    Method::TabFocus(target) => {
                        self.snapshot
                            .as_deref()
                            .and_then(|snapshot| snapshot.focused_tab_id.as_deref())
                            != Some(target.tab_id.as_str())
                    }
                    Method::WorkspaceFocus(target) => {
                        self.snapshot
                            .as_deref()
                            .and_then(|snapshot| snapshot.focused_workspace_id.as_deref())
                            != Some(target.workspace_id.as_str())
                    }
                    Method::WorkspaceCreate(params) => params.focus,
                    Method::TabCreate(params) => params.focus,
                    Method::PaneSplit(params) => params.focus,
                    _ => false,
                },
                _ => false,
            })
        });
        if requested_new_focus {
            self.leave_application_scroll(outcome);
        }
    }

    pub(super) fn leave_application_scroll(&mut self, outcome: &mut ClientShellInput) {
        let Some(scroll) = self.application_scroll.take() else {
            return;
        };
        for key in scroll.keys {
            self.suppress_application_scroll_key(key);
        }
        if let Some(request_id) = scroll.in_flight {
            self.pending_requests.remove(&request_id);
            outcome
                .actions
                .push(ClientShellAction::CancelQueuedEndpoint {
                    endpoint_id: scroll.endpoint_id,
                    request_id,
                });
        }
        if self.mode == ClientShellMode::Scroll {
            self.mode = self.copy_or_terminal_mode();
        }
        outcome.repaint = true;
    }

    pub(super) fn try_enter_application_scroll(
        &mut self,
        direction: i8,
        amount: CopyScrollAmount,
        outcome: &mut ClientShellInput,
    ) -> bool {
        let Some(pane_id) = self.focused_pane_id() else {
            return false;
        };
        let alternate_screen = self.pane_surface.as_ref().is_some_and(|surface| {
            surface
                .panes
                .iter()
                .any(|pane| pane.pane_id == pane_id && pane.alternate_screen_active)
        });
        if !alternate_screen && !self.application_scroll_valid() {
            return false;
        }
        let intent = match (direction < 0, amount) {
            (true, CopyScrollAmount::Line) => Intent::WheelUp,
            (false, CopyScrollAmount::Line) => Intent::WheelDown,
            (true, _) => Intent::PageUp,
            (false, _) => Intent::PageDown,
        };
        let method = Method::PaneScrollApplication(PaneScrollApplicationParams {
            pane_id: pane_id.clone(),
            intent,
            count: 1,
        });
        if !self.supports_endpoint_method(&method) {
            self.push_endpoint_method(method, outcome);
            return true;
        }
        if !self.application_scroll_valid() {
            self.leave_application_scroll(outcome);
            let Some(snapshot) = self.snapshot.as_deref() else {
                return true;
            };
            self.next_application_scroll_generation =
                self.next_application_scroll_generation.saturating_add(1);
            self.application_scroll = Some(ClientApplicationScroll {
                generation: self.next_application_scroll_generation,
                endpoint_id: self.active_endpoint_id.clone(),
                endpoint_generation: self.active_snapshot_generation,
                boot_id: snapshot.boot_id.clone(),
                snapshot_index: snapshot
                    .panes
                    .iter()
                    .position(|pane| pane.pane_id == pane_id)
                    .unwrap_or(usize::MAX),
                surface_index: self
                    .pane_surface
                    .as_ref()
                    .and_then(|surface| {
                        surface
                            .panes
                            .iter()
                            .position(|pane| pane.pane_id == pane_id)
                    })
                    .unwrap_or(usize::MAX),
                keys: HashSet::new(),
                pane_id,
                queued: VecDeque::new(),
                in_flight: None,
            });
            self.copy_mode = None;
            self.selection = None;
            self.stop_selection_autoscroll();
            self.reset_copy_pipeline();
            self.mode = ClientShellMode::Scroll;
        }
        self.queue_application_scroll(intent, outcome);
        outcome.repaint = true;
        true
    }

    fn claude_application_scroll(&self) -> bool {
        self.application_scroll
            .as_ref()
            .and_then(|scroll| {
                self.snapshot
                    .as_deref()?
                    .agents
                    .iter()
                    .find(|agent| agent.pane_id == scroll.pane_id)
            })
            .and_then(|agent| {
                // The effective known runtime agent wins over a display fallback.
                agent
                    .agent
                    .as_deref()
                    .and_then(crate::detect::parse_agent_label)
                    .or_else(|| {
                        agent
                            .display_agent
                            .as_deref()
                            .and_then(crate::detect::parse_agent_label)
                    })
            })
            == Some(crate::detect::Agent::Claude)
    }

    pub(super) fn route_application_scroll_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) {
        if !self.application_scroll_valid() {
            self.leave_application_scroll(outcome);
            return;
        }
        if self.config.keybinds.matches_prefix(key) {
            self.leave_application_scroll(outcome);
            self.mode = ClientShellMode::Prefix;
            return;
        }
        if matches!(key.code, KeyCode::Esc | KeyCode::Enter)
            || key.code == KeyCode::Char('q') && key.modifiers.is_empty()
        {
            self.leave_application_scroll(outcome);
            return;
        }
        if key.modifiers.contains(KeyModifiers::ALT) {
            return;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let code = if ctrl {
            key.code
        } else {
            crate::copy_mode::copy_mode_command_char(key.clone())
                .map(KeyCode::Char)
                .unwrap_or(key.code)
        };
        let intent = match code {
            KeyCode::Char('u' | 'U') if ctrl => Some(Intent::PageUp),
            KeyCode::Char('d' | 'D') if ctrl => Some(Intent::PageDown),
            KeyCode::PageUp if !ctrl => Some(Intent::PageUp),
            KeyCode::PageDown if !ctrl => Some(Intent::PageDown),
            KeyCode::Char('k' | 'K') if ctrl => Some(Intent::WheelUp),
            KeyCode::Char('j' | 'J') if ctrl => Some(Intent::WheelDown),
            KeyCode::Char('k') | KeyCode::Up if !ctrl => Some(Intent::WheelUp),
            KeyCode::Char('j') | KeyCode::Down if !ctrl => Some(Intent::WheelDown),
            KeyCode::Char('g') | KeyCode::Home if !ctrl => {
                Some(if self.claude_application_scroll() {
                    Intent::CtrlHome
                } else {
                    Intent::Home
                })
            }
            KeyCode::Char('g') if ctrl => Some(if self.claude_application_scroll() {
                Intent::CtrlEnd
            } else {
                Intent::End
            }),
            KeyCode::Char('G') | KeyCode::End if !ctrl => {
                Some(if self.claude_application_scroll() {
                    Intent::CtrlEnd
                } else {
                    Intent::End
                })
            }
            _ => None,
        };
        if let Some(intent) = intent {
            self.queue_application_scroll(intent, outcome);
        }
    }

    fn queue_application_scroll(&mut self, intent: Intent, outcome: &mut ClientShellInput) {
        let Some(scroll) = self.application_scroll.as_mut() else {
            return;
        };
        if let Some((last_intent, count)) = scroll.queued.back_mut() {
            if *last_intent == intent && *count < 64 {
                *count += 1;
                self.dispatch_application_scroll(outcome);
                return;
            }
        }
        if scroll.queued.len() >= MAX_PENDING_SCROLL_BATCHES {
            self.set_endpoint_error("application scroll input backlog is full");
            self.leave_application_scroll(outcome);
            return;
        }
        scroll.queued.push_back((intent, 1));
        self.dispatch_application_scroll(outcome);
    }

    fn dispatch_application_scroll(&mut self, outcome: &mut ClientShellInput) {
        if !self.application_scroll_valid() {
            self.leave_application_scroll(outcome);
            return;
        }
        let Some(scroll) = self.application_scroll.as_mut() else {
            return;
        };
        if scroll.in_flight.is_some() {
            return;
        }
        let Some((intent, count)) = scroll.queued.pop_front() else {
            return;
        };
        let generation = scroll.generation;
        let method = Method::PaneScrollApplication(PaneScrollApplicationParams {
            pane_id: scroll.pane_id.clone(),
            intent,
            count,
        });
        if self.push_endpoint_method_with_kind(
            method,
            PendingEndpointKind::ApplicationScroll { generation },
            outcome,
        ) {
            if let Some(ClientShellAction::Endpoint { request, .. }) = outcome.actions.last() {
                if let Some(scroll) = self.application_scroll.as_mut() {
                    scroll.in_flight = Some(request.id.clone());
                }
            }
        } else {
            self.leave_application_scroll(outcome);
        }
    }

    pub(super) fn complete_application_scroll(
        &mut self,
        generation: u64,
        success: bool,
        outcome: &mut ClientShellInput,
    ) {
        let Some(scroll) = self.application_scroll.as_mut() else {
            return;
        };
        if scroll.generation != generation {
            return;
        }
        scroll.in_flight = None;
        if success {
            self.dispatch_application_scroll(outcome);
        } else {
            self.leave_application_scroll(outcome);
        }
    }
}
