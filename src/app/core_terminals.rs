//! herdr-core terminals after a restore (openspec/changes/core-backend, D10).

use tracing::warn;

use super::state::{ToastKind, ToastNotification};
use super::App;

impl App {
    /// D10 "Orphans": live core terminals labelled for this session that no
    /// restored pane attached to stay running (they may hold an agent's
    /// work), each logged at warn and all named in one notice.
    pub(crate) fn report_unclaimed_core_terminals(&mut self) {
        let claimed: Vec<String> = self
            .terminal_runtimes
            .iter()
            .filter_map(|(_, runtime)| runtime.core_terminal())
            .map(|link| link.address)
            .collect();
        let unclaimed = crate::pane::unclaimed_core_terminals(&claimed);
        if unclaimed.is_empty() {
            return;
        }
        let session = crate::pane::core_session_name();
        for (address, pane_id) in &unclaimed {
            warn!(
                %address,
                pane_id = pane_id.as_deref().unwrap_or("-"),
                %session,
                "herdr-core terminal of this session has no pane; left running"
            );
        }
        let previous = self.state.toast.clone();
        self.state.post_notification(ToastNotification {
            kind: ToastKind::NeedsAttention,
            title: format!(
                "{} herdr-core program{} without a pane",
                unclaimed.len(),
                if unclaimed.len() == 1 { "" } else { "s" }
            ),
            context: format!(
                "Left running. See them: HC_SESSION={session} hc ls --label herdr.session={session}"
            ),
            position: None,
            target: None,
            anchor_pane: None,
        });
        self.sync_toast_deadline(previous);
    }
}
