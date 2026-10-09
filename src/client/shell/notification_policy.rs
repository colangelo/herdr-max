use super::*;

const MAX_QUEUED_NOTIFICATIONS: usize = 8;
const COMPLETION_EVIDENCE_GRACE: std::time::Duration = std::time::Duration::from_secs(1);
pub(super) const COMPLETION_RECHECK_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(50);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NotificationValidation {
    Current,
    AwaitingSnapshot,
    Stale,
}

fn notification_deadline(
    config: &ClientShellConfig,
    kind: SemanticNotificationKind,
    now: std::time::Instant,
) -> Option<std::time::Instant> {
    let seconds = match kind {
        SemanticNotificationKind::NeedsAttention => config.herdr_toast.needs_attention_seconds,
        SemanticNotificationKind::Finished | SemanticNotificationKind::Custom => {
            config.herdr_toast.finished_seconds
        }
        SemanticNotificationKind::UpdateInstalled => config.herdr_toast.update_seconds,
    };
    (seconds > 0)
        .then(|| now.checked_add(std::time::Duration::from_secs(seconds)))
        .flatten()
}

impl ClientShellState {
    pub(super) fn retire_endpoint_notifications(&mut self, endpoint_id: &ClientEndpointId) {
        self.pending_notifications
            .retain(|pending| &pending.endpoint_id != endpoint_id);
        self.queued_notifications
            .retain(|queued| &queued.endpoint_id != endpoint_id);
        if self
            .visible_notification
            .as_ref()
            .is_some_and(|visible| &visible.endpoint_id == endpoint_id)
        {
            self.visible_notification = None;
            self.promote_queued_notification(std::time::Instant::now());
        }
    }

    fn queue_visible_notification(
        &mut self,
        mut notification: ClientVisibleNotification,
        now: std::time::Instant,
    ) {
        notification.deadline = notification_deadline(&self.config, notification.event.kind, now);
        if self.visible_notification.is_none()
            || notification.deadline.is_none()
            || self
                .visible_notification
                .as_ref()
                .is_some_and(|visible| visible.deadline.is_none())
        {
            if self.visible_notification.is_some() {
                self.queued_notifications.clear();
            }
            self.visible_notification = Some(notification);
            return;
        }
        if self.queued_notifications.len() == MAX_QUEUED_NOTIFICATIONS {
            self.queued_notifications.pop_front();
        }
        self.queued_notifications.push_back(notification);
    }

    /// A toast the client raises itself: the fork's action feedback (a pane
    /// move refused or failed, a refused todo save), anchored to the focused
    /// pane and never logged. Like the fork's, it replaces whatever toast is
    /// showing at once rather than waiting its turn; the queue is kept.
    /// Returns whether a repaint is due.
    pub(super) fn push_feedback_toast(&mut self, title: &str, body: impl Into<String>) -> bool {
        let snapshot = self.snapshot.as_deref();
        let event = SemanticNotification {
            kind: SemanticNotificationKind::NeedsAttention,
            title: title.to_owned(),
            body: Some(body.into()),
            sound: None,
            agent: None,
            workspace_id: snapshot.and_then(|snapshot| snapshot.focused_workspace_id.clone()),
            tab_id: snapshot.and_then(|snapshot| snapshot.focused_tab_id.clone()),
            pane_id: snapshot.and_then(|snapshot| snapshot.focused_pane_id.clone()),
            position: None,
        };
        let now = std::time::Instant::now();
        self.visible_notification = Some(ClientVisibleNotification {
            endpoint_id: self.active_endpoint_id.clone(),
            deadline: notification_deadline(&self.config, event.kind, now),
            event,
        });
        true
    }

    fn promote_queued_notification(&mut self, now: std::time::Instant) -> bool {
        let Some(mut notification) = self.queued_notifications.pop_front() else {
            return false;
        };
        notification.deadline = notification_deadline(&self.config, notification.event.kind, now);
        self.visible_notification = Some(notification);
        true
    }

    pub(super) fn focus_visible_notification(&mut self, outcome: &mut ClientShellInput) {
        let Some(notification) = self.visible_notification.as_ref() else {
            return;
        };
        if notification.event.pane_id.is_some()
            && !self.endpoint_is_online(&notification.endpoint_id)
        {
            let label = self.endpoint_label(&notification.endpoint_id).to_owned();
            self.receive_endpoint_unavailable(format!("{label} is unavailable"));
            outcome.repaint = true;
            return;
        }
        let notification = self
            .visible_notification
            .take()
            .expect("checked visible notification");
        self.promote_queued_notification(std::time::Instant::now());
        outcome.repaint = true;
        let Some(pane_id) = notification.event.pane_id else {
            return;
        };
        if notification.endpoint_id == self.active_endpoint_id {
            self.push_endpoint_method(
                crate::api::schema::Method::PaneFocus(crate::api::schema::PaneTarget { pane_id }),
                outcome,
            );
        } else if self.endpoint_is_online(&notification.endpoint_id) {
            outcome.actions.push(ClientShellAction::ActivateEndpoint {
                endpoint_id: notification.endpoint_id,
                target: Some(ClientEndpointFocusTarget::Pane(pane_id)),
            });
        }
    }

    pub(crate) fn receive_notification(
        &mut self,
        endpoint_id: &ClientEndpointId,
        event: SemanticNotification,
        now: std::time::Instant,
    ) -> (Vec<ClientShellNotificationEffect>, bool) {
        let delay = if event.kind == SemanticNotificationKind::Custom {
            0
        } else {
            self.config.toast_delay_seconds
        };
        let deadline = now
            .checked_add(std::time::Duration::from_secs(delay))
            .unwrap_or(now);
        let cleared_visible = event.pane_id.as_deref().is_some_and(|pane_id| {
            self.visible_notification.as_ref().is_some_and(|visible| {
                visible.endpoint_id == *endpoint_id
                    && visible.event.pane_id.as_deref() == Some(pane_id)
            })
        });
        if let Some(pane_id) = event.pane_id.as_deref() {
            self.pending_notifications.retain(|pending| {
                pending.endpoint_id != *endpoint_id
                    || pending.event.pane_id.as_deref() != Some(pane_id)
            });
            self.queued_notifications.retain(|queued| {
                queued.endpoint_id != *endpoint_id
                    || queued.event.pane_id.as_deref() != Some(pane_id)
            });
            if cleared_visible {
                self.visible_notification = None;
                self.promote_queued_notification(now);
            }
        }
        // Completion evidence is advisory. A Finished effect is valid only while the
        // client-projected pane remains Done, even when delivery is immediate.
        let validate_state = delay > 0 || event.kind == SemanticNotificationKind::Finished;
        self.pending_notifications.push(ClientPendingNotification {
            endpoint_id: endpoint_id.clone(),
            event,
            deadline,
            expires_at: now.checked_add(COMPLETION_EVIDENCE_GRACE).unwrap_or(now),
            validate_state,
        });
        let (effects, repaint) = self.tick_notifications(now);
        (effects, repaint || cleared_visible)
    }

    pub(crate) fn tick_notifications(
        &mut self,
        now: std::time::Instant,
    ) -> (Vec<ClientShellNotificationEffect>, bool) {
        let mut repaint = false;
        if self
            .visible_notification
            .as_ref()
            .is_some_and(|visible| visible.deadline.is_some_and(|deadline| now >= deadline))
        {
            self.visible_notification = None;
            self.promote_queued_notification(now);
            repaint = true;
        }
        if self
            .visible_endpoint_notice
            .as_ref()
            .is_some_and(|visible| now >= visible.deadline)
        {
            self.visible_endpoint_notice = None;
            repaint = true;
        }

        let pending = std::mem::take(&mut self.pending_notifications);
        let mut effects = Vec::new();
        for pending in pending {
            if pending.deadline > now {
                self.pending_notifications.push(pending);
                continue;
            }
            if pending.validate_state {
                match self.notification_validation(&pending.endpoint_id, &pending.event) {
                    NotificationValidation::Current => {}
                    NotificationValidation::AwaitingSnapshot if now < pending.expires_at => {
                        let mut pending = pending;
                        pending.deadline = now
                            .checked_add(COMPLETION_RECHECK_INTERVAL)
                            .unwrap_or(now)
                            .min(pending.expires_at);
                        self.pending_notifications.push(pending);
                        continue;
                    }
                    NotificationValidation::AwaitingSnapshot | NotificationValidation::Stale => {
                        continue;
                    }
                }
            }
            let target_active =
                self.notification_target_is_active(&pending.endpoint_id, &pending.event);
            let suppress_external = target_active && self.outer_focused != Some(false);
            if let Some(sound) = pending.event.sound {
                let suppress_sound =
                    pending.event.kind == SemanticNotificationKind::Finished && suppress_external;
                if !suppress_sound {
                    effects.push(ClientShellNotificationEffect::Sound {
                        sound: match sound {
                            SemanticNotificationSound::Done => crate::sound::Sound::Done,
                            SemanticNotificationSound::Request => crate::sound::Sound::Request,
                        },
                        agent: pending.event.agent.clone(),
                    });
                }
            }

            match self.config.toast_delivery {
                crate::config::ToastDelivery::Off => {}
                crate::config::ToastDelivery::Herdr
                    if !target_active
                        || (pending.event.kind == SemanticNotificationKind::Custom
                            && self.config.herdr_toast.pane_feedback
                                == crate::config::ToastPaneFeedback::Pane) =>
                {
                    self.queue_visible_notification(
                        ClientVisibleNotification {
                            endpoint_id: pending.endpoint_id,
                            event: pending.event,
                            deadline: Some(now),
                        },
                        now,
                    );
                    repaint = true;
                }
                crate::config::ToastDelivery::Herdr => {}
                crate::config::ToastDelivery::Terminal if !suppress_external => {
                    effects.push(ClientShellNotificationEffect::Terminal {
                        title: pending.event.title,
                        body: pending.event.body,
                    });
                }
                crate::config::ToastDelivery::System if !suppress_external => {
                    #[cfg(windows)]
                    let target = pending.event.pane_id.as_ref().and_then(|pane_id| {
                        self.endpoint_boot_id(&pending.endpoint_id).map(|boot_id| {
                            ClientSystemNotificationTarget {
                                endpoint_id: pending.endpoint_id.clone(),
                                boot_id: boot_id.to_owned(),
                                pane_id: pane_id.clone(),
                            }
                        })
                    });
                    effects.push(ClientShellNotificationEffect::System {
                        title: pending.event.title,
                        body: pending.event.body,
                        #[cfg(windows)]
                        target,
                    });
                }
                crate::config::ToastDelivery::Terminal | crate::config::ToastDelivery::System => {}
            }
        }
        (effects, repaint)
    }

    #[cfg(windows)]
    pub(crate) fn notification_target_is_current(
        &self,
        endpoint_id: &ClientEndpointId,
        target: &ClientEndpointFocusTarget,
    ) -> bool {
        let ClientEndpointFocusTarget::Notification { pane_id, boot_id } = target else {
            return true;
        };
        self.endpoint_is_online(endpoint_id)
            && self
                .endpoints
                .iter()
                .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
                .and_then(|endpoint| endpoint.snapshot.as_deref())
                .is_some_and(|snapshot| {
                    snapshot.boot_id == *boot_id
                        && snapshot.panes.iter().any(|pane| pane.pane_id == *pane_id)
                })
    }

    #[cfg(windows)]
    pub(crate) fn activate_system_notification(
        &mut self,
        target: ClientSystemNotificationTarget,
    ) -> ClientShellInput {
        let focus = ClientEndpointFocusTarget::Notification {
            pane_id: target.pane_id,
            boot_id: target.boot_id,
        };
        let mut outcome = ClientShellInput::default();
        if self.notification_target_is_current(&target.endpoint_id, &focus) {
            outcome.actions.push(ClientShellAction::ActivateEndpoint {
                endpoint_id: target.endpoint_id,
                target: Some(focus),
            });
        }
        outcome
    }

    fn notification_target_is_active(
        &self,
        endpoint_id: &ClientEndpointId,
        event: &SemanticNotification,
    ) -> bool {
        if endpoint_id != &self.active_endpoint_id {
            return false;
        }
        let Some(snapshot) = self
            .endpoints
            .iter()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
            .and_then(|endpoint| endpoint.snapshot.as_deref())
        else {
            return false;
        };
        if let Some(tab_id) = event.tab_id.as_deref() {
            return snapshot.focused_tab_id.as_deref() == Some(tab_id);
        }
        event.workspace_id.as_deref().is_some_and(|workspace_id| {
            snapshot.focused_workspace_id.as_deref() == Some(workspace_id)
        })
    }

    fn notification_validation(
        &self,
        endpoint_id: &ClientEndpointId,
        event: &SemanticNotification,
    ) -> NotificationValidation {
        let Some(pane_id) = event.pane_id.as_deref() else {
            // Finished notifications carry no independently trustworthy completion state. Without
            // a projected pane to verify as Done, do not emit completion chrome or sound.
            return if event.kind == SemanticNotificationKind::Finished {
                NotificationValidation::Stale
            } else {
                NotificationValidation::Current
            };
        };
        let Some(agent) = self
            .endpoints
            .iter()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
            .and_then(|endpoint| endpoint.snapshot.as_deref())
            .and_then(|snapshot| {
                snapshot
                    .agents
                    .iter()
                    .find(|agent| agent.pane_id == pane_id)
            })
        else {
            return NotificationValidation::AwaitingSnapshot;
        };
        match event.kind {
            SemanticNotificationKind::NeedsAttention
                if agent.agent_status == crate::api::schema::AgentStatus::Blocked =>
            {
                NotificationValidation::Current
            }
            SemanticNotificationKind::Finished
                if agent.agent_status == crate::api::schema::AgentStatus::Done =>
            {
                NotificationValidation::Current
            }
            SemanticNotificationKind::Finished
                if agent.agent_status == crate::api::schema::AgentStatus::Working =>
            {
                NotificationValidation::AwaitingSnapshot
            }
            SemanticNotificationKind::UpdateInstalled | SemanticNotificationKind::Custom => {
                NotificationValidation::Current
            }
            SemanticNotificationKind::NeedsAttention | SemanticNotificationKind::Finished => {
                NotificationValidation::Stale
            }
        }
    }
}

#[cfg(test)]
mod duration_tests {
    use super::*;
    fn note(title: &str) -> ClientVisibleNotification {
        ClientVisibleNotification {
            endpoint_id: ClientEndpointId::Local,
            event: SemanticNotification {
                kind: SemanticNotificationKind::Custom,
                title: title.into(),
                body: None,
                sound: None,
                agent: None,
                workspace_id: None,
                tab_id: None,
                pane_id: None,
                position: None,
            },
            deadline: None,
        }
    }
    #[test]
    fn zero_duration_persists_until_clicked_or_replaced_like_the_fork() {
        let mut config = Config::default();
        config.ui.toast.herdr.finished_seconds = 0;
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
        let now = std::time::Instant::now();
        state.queue_visible_notification(note("first"), now);
        assert!(state
            .visible_notification
            .as_ref()
            .unwrap()
            .deadline
            .is_none());
        state.tick_notifications(now + std::time::Duration::from_secs(60));
        assert_eq!(
            state.visible_notification.as_ref().unwrap().event.title,
            "first"
        );
        state.queue_visible_notification(note("replacement"), now);
        assert_eq!(
            state.visible_notification.as_ref().unwrap().event.title,
            "replacement"
        );
        assert!(state.queued_notifications.is_empty());
    }
    #[test]
    fn configured_positive_duration_controls_expiry_without_losing_arrival_order() {
        let mut config = Config::default();
        config.ui.toast.herdr.finished_seconds = 2;
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
        let now = std::time::Instant::now();
        state.queue_visible_notification(note("first"), now);
        state.queue_visible_notification(note("second"), now);
        state.tick_notifications(now + std::time::Duration::from_secs(1));
        assert_eq!(
            state.visible_notification.as_ref().unwrap().event.title,
            "first"
        );
        state.tick_notifications(now + std::time::Duration::from_secs(2));
        assert_eq!(
            state.visible_notification.as_ref().unwrap().event.title,
            "second"
        );
    }
}
