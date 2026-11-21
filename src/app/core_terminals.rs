//! herdr-core terminals after a restore (openspec/changes/core-backend, D10),
//! and panes that wait for a core that did not answer
//! (openspec/changes/core-phase3, D1b).

use std::time::{Duration, Instant};

use tracing::warn;

use super::state::{ToastKind, ToastNotification};
use super::App;
use crate::layout::PaneId;

/// How often panes waiting for Herdr Core look for it again (D1b).
const CORE_WAITING_RETRY: Duration = Duration::from_secs(10);

impl App {
    /// D10 "Orphans": live core terminals labelled for this session that no
    /// restored pane attached to stay running (they may hold an agent's
    /// work), each logged at warn and all named in one notice.
    pub(crate) fn report_unclaimed_core_terminals(&mut self) {
        // Claims are `(session, address)`: addresses are per core session
        // (core-phase3 D6). A pane waiting for its core claims its terminal
        // too, in case that core answers by now.
        let waiting = self
            .core_waiting_panes()
            .into_iter()
            .map(|(_, waiting)| waiting.link);
        let claimed: Vec<(String, String)> = self
            .terminal_runtimes
            .values()
            .filter_map(|runtime| runtime.core_terminal())
            .chain(waiting)
            .map(|link| (link.session, link.address))
            .collect();
        let unclaimed = crate::pane::unclaimed_core_terminals(&claimed);
        if unclaimed.is_empty() {
            return;
        }
        for terminal in &unclaimed {
            warn!(
                address = %terminal.address,
                pane_id = terminal.pane_id.as_deref().unwrap_or("-"),
                session = %terminal.session,
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
            context: unclaimed_notice_context(&unclaimed, &crate::pane::herdr_session_label()),
            position: None,
            target: None,
            anchor_pane: None,
        });
        self.sync_toast_deadline(previous);
    }
}

/// The orphan notice's look-up command, once per core session that holds
/// one (core-phase3 D5): the session to reach and the label that is ours.
fn unclaimed_notice_context(
    unclaimed: &[crate::pane::UnclaimedCoreTerminal],
    label: &str,
) -> String {
    let mut sessions: Vec<&str> = Vec::new();
    for terminal in unclaimed {
        if !sessions.contains(&terminal.session.as_str()) {
            sessions.push(&terminal.session);
        }
    }
    let commands: Vec<String> = sessions
        .iter()
        .map(|session| format!("HC_SESSION={session} hc ls --label herdr.session={label}"))
        .collect();
    format!("Left running. See them: {}", commands.join("; "))
}

impl App {
    pub(crate) fn core_waiting_deadline(&self) -> Option<Instant> {
        self.core_waiting_retry_at
    }

    /// Panes waiting for Herdr Core, with what they wait for.
    fn core_waiting_panes(&self) -> Vec<(PaneId, crate::terminal::state::CoreWaiting)> {
        self.state
            .workspaces
            .iter()
            .flat_map(|ws| ws.tabs.iter())
            .flat_map(|tab| tab.layout.pane_ids())
            .filter_map(|pane_id| {
                let waiting = self.state.pane_terminal(pane_id)?.core_waiting()?;
                Some((pane_id, waiting.clone()))
            })
            .collect()
    }

    /// Arms the next look for the cores that waiting panes need.
    pub(crate) fn schedule_core_waiting_retry(&mut self, now: Instant) {
        if self.core_waiting_retry_at.is_none() && !self.core_waiting_panes().is_empty() {
            self.core_waiting_retry_at = Some(now + CORE_WAITING_RETRY);
        }
    }

    /// When it is time, asks on a thread of its own whether the cores that
    /// waiting panes need answer, so a hung core never holds the server
    /// loop; `CoreSessionsAnswered` brings the answer back.
    #[cfg(unix)]
    pub(crate) fn retry_core_waiting_panes_if_due(&mut self, now: Instant) -> bool {
        if self.core_waiting_retry_at.is_none_or(|at| now < at) {
            return false;
        }
        self.core_waiting_retry_at = None;
        let mut sessions: Vec<String> = self
            .core_waiting_panes()
            .into_iter()
            .map(|(_, waiting)| waiting.link.session)
            .collect();
        sessions.sort_unstable();
        sessions.dedup();
        if sessions.is_empty() {
            return false;
        }
        let events = self.event_tx.clone();
        let probe = std::thread::Builder::new()
            .name("herdr-core-probe".into())
            .spawn(move || {
                let sessions = sessions
                    .into_iter()
                    .filter(|session| crate::pane::core_session_answers(session))
                    .collect();
                let _ = events
                    .blocking_send(crate::events::AppEvent::CoreSessionsAnswered { sessions });
            });
        if let Err(err) = probe {
            warn!(err = %err, "could not start the herdr-core probe; retrying later");
            self.core_waiting_retry_at = Some(now + CORE_WAITING_RETRY);
        }
        false
    }

    #[cfg(not(unix))]
    pub(crate) fn retry_core_waiting_panes_if_due(&mut self, _now: Instant) -> bool {
        false
    }

    /// Panes waiting for a core in `sessions`, which just answered, attach
    /// to their terminal, or by D1b's table start fresh or keep waiting.
    #[cfg(unix)]
    pub(crate) fn attach_waiting_panes(&mut self, sessions: &[String], now: Instant) {
        let mut changed = false;
        for (pane_id, waiting) in self.core_waiting_panes() {
            if sessions.contains(&waiting.link.session) {
                changed |= self.attach_waiting_pane(pane_id, waiting);
            }
        }
        self.schedule_core_waiting_retry(now);
        if changed {
            self.state.mark_session_dirty();
            self.render_dirty.request_generic();
            self.render_notify.notify_one();
        }
    }

    /// One waiting pane's try. Returns whether the pane changed.
    #[cfg(unix)]
    fn attach_waiting_pane(
        &mut self,
        pane_id: PaneId,
        waiting: crate::terminal::state::CoreWaiting,
    ) -> bool {
        let found = match self.attach_core_terminal(pane_id, &waiting) {
            Ok(()) => return true,
            Err(failure) => {
                let program = crate::platform::ProgramIdentity::liveness(
                    waiting.link.program,
                    self.program_start_token,
                );
                if crate::persist::after_failed_attach(failure, program)
                    == crate::persist::AfterFailedAttach::Wait
                {
                    return false;
                }
                crate::pane::find_pane_core_terminals(
                    &waiting.link.session,
                    waiting.public_pane_id.as_deref(),
                )
            }
        };
        let Some(terminal) = self.state.pane_terminal_mut(pane_id) else {
            return false;
        };
        match found.as_slice() {
            [] => {
                terminal.hold = None;
                tracing::info!(
                    pane = pane_id.raw(),
                    session = %waiting.link.session,
                    "herdr-core answered and the pane's program is gone; starting it fresh"
                );
                self.resume_suspended_pane(pane_id);
            }
            [one] => {
                let moved = crate::terminal::state::CoreWaiting {
                    link: one.clone(),
                    public_pane_id: waiting.public_pane_id.clone(),
                };
                terminal.hold = Some(crate::terminal::state::PaneHold::CoreWaiting(moved.clone()));
                if self.attach_core_terminal(pane_id, &moved).is_err() {
                    return true;
                }
            }
            _ => {
                terminal.hold = None;
                terminal.restore_error = Some(format!(
                    "More than one Herdr Core terminal is labelled for this pane ({}), so none was attached and nothing was started. Close the ones that are not this pane's, then run `herdr pane respawn` on it.",
                    found
                        .iter()
                        .map(|link| format!("{} in session {}", link.address, link.session))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                terminal.revision = terminal.revision.saturating_add(1);
            }
        }
        true
    }

    /// Attaches `pane_id` to the terminal `waiting` names and installs it.
    #[cfg(unix)]
    fn attach_core_terminal(
        &mut self,
        pane_id: PaneId,
        waiting: &crate::terminal::state::CoreWaiting,
    ) -> Result<(), crate::pane::CoreAttachFailure> {
        let Some((ws_idx, pane)) = self.find_pane(pane_id) else {
            return Err(crate::pane::CoreAttachFailure::Unreachable);
        };
        let terminal_id = pane.attached_terminal_id.clone();
        let (rows, cols) = self
            .state
            .new_pane_size(crate::ui::NewPanePlacement::Existing {
                ws_idx,
                pane: pane_id,
            });
        let runtime = crate::terminal::TerminalRuntime::reattach_core(
            pane_id,
            rows,
            cols,
            waiting.link.clone(),
            waiting.public_pane_id.clone(),
            self.state.pane_scrollback_limit_bytes,
            self.state.host_terminal_theme,
            self.pane_host_terminal_appearance(),
            self.event_tx.clone(),
            self.render_notify.clone(),
            self.render_dirty.clone(),
        )
        .map_err(|err| crate::pane::core_attach_failure(&err))?;
        tracing::info!(
            pane = pane_id.raw(),
            session = %waiting.link.session,
            address = %waiting.link.address,
            "a pane waiting for herdr-core attached to its terminal"
        );
        self.terminal_runtimes.insert(terminal_id.clone(), runtime);
        if let Some(terminal) = self.state.terminals.get_mut(&terminal_id) {
            terminal.hold = None;
            terminal.revision = terminal.revision.saturating_add(1);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::unclaimed_notice_context;
    use crate::pane::UnclaimedCoreTerminal;

    // V6 (core-phase3 D5): one look-up per session that holds an orphan,
    // each with that session and this herdr session's label.
    #[test]
    fn the_orphan_notice_names_each_session_with_our_label() {
        let orphan = |session: &str, address: &str| UnclaimedCoreTerminal {
            session: session.into(),
            address: address.into(),
            pane_id: None,
        };
        assert_eq!(
            unclaimed_notice_context(
                &[
                    orphan("herdr", "t8"),
                    orphan("herdr", "t9"),
                    orphan("default", "t7")
                ],
                "herdr"
            ),
            "Left running. See them: HC_SESSION=herdr hc ls --label herdr.session=herdr; \
             HC_SESSION=default hc ls --label herdr.session=herdr"
        );
    }
}
