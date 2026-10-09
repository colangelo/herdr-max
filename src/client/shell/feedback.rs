use super::*;

/// Client source identity uses public pane IDs and an endpoint, never server PaneId.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ClientCopyFeedback {
    pub(super) feedback: crate::app::state::CopyFeedback,
    pub(super) source_pane: Option<String>,
    pub(super) endpoint_id: Option<ClientEndpointId>,
}

impl std::ops::Deref for ClientCopyFeedback {
    type Target = crate::app::state::CopyFeedback;
    fn deref(&self) -> &Self::Target {
        &self.feedback
    }
}

impl From<crate::app::state::CopyFeedback> for ClientCopyFeedback {
    fn from(feedback: crate::app::state::CopyFeedback) -> Self {
        Self {
            feedback,
            source_pane: None,
            endpoint_id: None,
        }
    }
}

impl ClientShellState {
    pub(super) fn show_copy_feedback_for(
        &mut self,
        source_pane: Option<String>,
        endpoint_id: Option<ClientEndpointId>,
        now: std::time::Instant,
    ) -> bool {
        if !self.config.clipboard_toast_enabled {
            return false;
        }
        self.copy_feedback = Some(ClientCopyFeedback {
            feedback: crate::app::state::CopyFeedback {
                message: "copied to clipboard".to_owned(),
                source_pane: None,
            },
            source_pane,
            endpoint_id,
        });
        self.copy_feedback_deadline = Some(now + std::time::Duration::from_secs(2));
        true
    }
    pub(crate) fn cache_clipboard_origin(
        &mut self,
        endpoint: &ClientEndpointId,
        generation: Option<u64>,
        origin: crate::protocol::endpoint::EndpointClipboardOrigin,
    ) {
        self.clipboard_origins
            .insert(endpoint.clone(), (generation, origin));
    }
    pub(crate) fn show_forwarded_copy_feedback(
        &mut self,
        data: &str,
        now: std::time::Instant,
    ) -> bool {
        let origin = self
            .clipboard_origins
            .remove(&self.active_endpoint_id)
            .and_then(|(generation, origin)| {
                let current = self.snapshot.as_deref()?;
                (origin.boot_id == current.boot_id
                    && generation == self.active_snapshot_generation
                    && origin.data_digest == crate::protocol::endpoint::clipboard_data_digest(data))
                .then_some(origin)
            });
        let (source, endpoint) = origin
            .map(|origin| (Some(origin.pane_id), Some(self.active_endpoint_id.clone())))
            .unwrap_or((None, None));
        self.show_copy_feedback_for(source, endpoint, now)
    }
    pub(super) fn copy_feedback_pane(&self) -> Option<Rect> {
        let feedback = self.copy_feedback.as_ref()?;
        if feedback.endpoint_id.as_ref() != Some(&self.active_endpoint_id) {
            return None;
        }
        let source = feedback.source_pane.as_deref()?;
        self.hits
            .panes
            .iter()
            .find(|pane| pane.pane_id == source)
            .map(|pane| pane.inner_rect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn copy_feedback_keeps_source_identity_when_focus_changes() {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        state.set_snapshot(Box::new(super::super::tests::snapshot()));
        state.show_copy_feedback_for(
            Some("pane_1".into()),
            Some(ClientEndpointId::Local),
            std::time::Instant::now(),
        );
        state.snapshot.as_mut().unwrap().focused_pane_id = Some("other".into());
        let feedback = state.copy_feedback.as_ref().unwrap();
        assert_eq!(feedback.source_pane.as_deref(), Some("pane_1"));
        assert_eq!(feedback.endpoint_id, Some(ClientEndpointId::Local));
    }
    #[test]
    fn unknown_clipboard_origin_does_not_guess_the_focused_pane() {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        state.set_snapshot(Box::new(super::super::tests::snapshot()));
        state.show_copy_feedback(std::time::Instant::now());
        assert!(state.copy_feedback.as_ref().unwrap().source_pane.is_none());
    }
    #[test]
    fn clipboard_origin_is_scoped_to_boot_generation_and_exact_data() {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        state.set_snapshot(Box::new(super::super::tests::snapshot()));
        let data = "Y29waWVk";
        for (boot, generation, digest, expected) in [
            (
                "boot-1",
                None,
                crate::protocol::endpoint::clipboard_data_digest(data),
                true,
            ),
            (
                "old-boot",
                None,
                crate::protocol::endpoint::clipboard_data_digest(data),
                false,
            ),
            (
                "boot-1",
                Some(99),
                crate::protocol::endpoint::clipboard_data_digest(data),
                false,
            ),
            ("boot-1", None, "different".into(), false),
        ] {
            state.cache_clipboard_origin(
                &ClientEndpointId::Local,
                generation,
                crate::protocol::endpoint::EndpointClipboardOrigin {
                    boot_id: boot.into(),
                    pane_id: "pane_1".into(),
                    data_digest: digest,
                },
            );
            state.show_forwarded_copy_feedback(data, std::time::Instant::now());
            assert_eq!(
                state.copy_feedback.as_ref().unwrap().source_pane.is_some(),
                expected
            );
            assert!(state.clipboard_origins.is_empty());
        }
    }
}
