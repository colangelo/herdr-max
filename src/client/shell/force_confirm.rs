//! Confirmations for closes and respawns the server refused without `force`
//! (fork c7500330, 82897fd1, de8f3926). The server answers
//! `confirmation_required` naming what would be lost; the client shows the
//! fork's confirmation with the todo count taken from that refusal, never from
//! its own snapshot, so a stale client cannot understate the loss. Accepting
//! resends the same target with `force`.

use super::{
    ClientConfirmCloseOverlay, ClientForceConfirmation, ClientForceTarget, ClientShellInput,
    ClientShellOverlay, ClientShellState,
};
use crate::api::schema::{
    Method, PaneCloseParams, PaneRespawnParams, TabCloseParams, WorkspaceCloseParams,
};

/// The target a refused request names, when the request can be forced and was
/// not forced already.
pub(super) fn force_target_for(method: &Method) -> Option<ClientForceTarget> {
    match method {
        Method::PaneClose(params) if !params.force => Some(ClientForceTarget::PaneClose {
            pane_id: params.pane_id.clone(),
        }),
        Method::TabClose(params) if !params.force => Some(ClientForceTarget::TabClose {
            tab_id: params.tab_id.clone(),
        }),
        Method::WorkspaceClose(params) if !params.force => {
            Some(ClientForceTarget::WorkspaceClose {
                workspace_id: params.workspace_id.clone(),
                close_group: params.close_group,
            })
        }
        Method::PaneRespawn(params) if !params.force => Some(ClientForceTarget::PaneRespawn {
            pane_id: params.pane_id.clone(),
        }),
        _ => None,
    }
}

/// The open-todo count a refusal leads with ("has 2 open todos (...)").
/// `None` when the refusal is about something else, such as a worktree group.
pub(super) fn refused_open_todo_count(message: &str) -> Option<usize> {
    let marker = message.find(" open todo")?;
    message[..marker]
        .rsplit(' ')
        .next()
        .and_then(|count| count.parse().ok())
}

fn outstanding(count: usize) -> String {
    if count == 1 {
        "1 outstanding todo".to_owned()
    } else {
        format!("{count} outstanding todos")
    }
}

impl ClientShellState {
    /// The name the fork's confirmation gives a pane: its title, manual label,
    /// or agent, else "this pane".
    fn force_pane_label(&self, pane_id: &str) -> String {
        let snapshot = self.snapshot.as_deref();
        let agent = snapshot.and_then(|snapshot| {
            snapshot
                .agents
                .iter()
                .find(|agent| agent.pane_id == pane_id)
        });
        let pane = snapshot
            .and_then(|snapshot| snapshot.panes.iter().find(|pane| pane.pane_id == pane_id));
        agent
            .and_then(|agent| agent.title.clone())
            .or_else(|| pane.and_then(|pane| pane.label.clone()))
            .or_else(|| agent.and_then(|agent| agent.display_agent.clone()))
            .or_else(|| agent.and_then(|agent| agent.agent.clone()))
            .filter(|label| !label.trim().is_empty())
            .unwrap_or_else(|| "this pane".to_owned())
    }

    fn force_target_workspace(&self, target: &ClientForceTarget) -> Option<String> {
        let snapshot = self.snapshot.as_deref()?;
        match target {
            ClientForceTarget::PaneClose { pane_id }
            | ClientForceTarget::PaneRespawn { pane_id } => snapshot
                .panes
                .iter()
                .find(|pane| &pane.pane_id == pane_id)
                .map(|pane| pane.workspace_id.clone()),
            ClientForceTarget::TabClose { tab_id } => snapshot
                .tabs
                .iter()
                .find(|tab| &tab.tab_id == tab_id)
                .map(|tab| tab.workspace_id.clone()),
            ClientForceTarget::WorkspaceClose { workspace_id, .. } => snapshot
                .workspaces
                .iter()
                .any(|workspace| &workspace.workspace_id == workspace_id)
                .then(|| workspace_id.clone()),
        }
    }

    /// Answer a refused close or respawn: a pane close or respawn opens the
    /// fork's confirmation, a tab or workspace close is forced at once.
    /// Returns false when
    /// the refusal is not one a forced resend answers, so the caller can treat
    /// it as before (a worktree-group close keeps its own confirmation).
    pub(super) fn open_force_confirmation(
        &mut self,
        target: &ClientForceTarget,
        boot_id: &str,
        message: &str,
        outcome: &mut ClientShellInput,
    ) -> bool {
        let todos = refused_open_todo_count(message);
        let (title, detail) = match target {
            ClientForceTarget::PaneRespawn { pane_id } => {
                let label = self.force_pane_label(pane_id);
                let detail = match todos {
                    Some(count) if count > 0 => {
                        format!("{label} - restarts the process, {}", outstanding(count))
                    }
                    _ => format!("{label} - restarts the process"),
                };
                ("Respawn pane and kill what is running?", detail)
            }
            ClientForceTarget::PaneClose { pane_id } => {
                let Some(count) = todos else {
                    return false;
                };
                let label = self.force_pane_label(pane_id);
                (
                    "Close pane with unfinished todos?",
                    format!("{label} - {}", outstanding(count)),
                )
            }
            // The fork never asked about todos on a tab or workspace close:
            // the user already chose to close it (after the ordinary
            // confirm_close dialog where that applies), so a refusal that is
            // only about todos is answered with the forced close at once.
            ClientForceTarget::TabClose { .. } | ClientForceTarget::WorkspaceClose { .. } => {
                if todos.is_none() {
                    return false;
                }
                self.accept_force_confirmation(
                    ClientForceConfirmation {
                        target: target.clone(),
                        boot_id: boot_id.to_owned(),
                    },
                    outcome,
                );
                return true;
            }
        };
        let Some(workspace_id) = self.force_target_workspace(target) else {
            return false;
        };
        let close_group = matches!(
            target,
            ClientForceTarget::WorkspaceClose {
                close_group: true,
                ..
            }
        );
        // One confirmation at a time: this replaces any other overlay, so a
        // confirmation dismissed this way can never be accepted later.
        self.overlay = Some(ClientShellOverlay::ConfirmClose(
            ClientConfirmCloseOverlay {
                workspace_id,
                close_group,
                tab_target: None,
                force: Some(ClientForceConfirmation {
                    target: target.clone(),
                    boot_id: boot_id.to_owned(),
                }),
                title: title.to_owned(),
                detail,
            },
        ));
        true
    }

    /// Resend a confirmed target with `force`, unless it changed underneath.
    pub(super) fn accept_force_confirmation(
        &mut self,
        force: ClientForceConfirmation,
        outcome: &mut ClientShellInput,
    ) {
        outcome.repaint = true;
        let same_boot = self
            .snapshot
            .as_deref()
            .is_some_and(|snapshot| snapshot.boot_id == force.boot_id);
        if !same_boot || self.force_target_workspace(&force.target).is_none() {
            self.receive_endpoint_unavailable("Close target changed; try closing it again".into());
            return;
        }
        let method = match force.target {
            ClientForceTarget::PaneClose { pane_id } => Method::PaneClose(PaneCloseParams {
                pane_id,
                force: true,
            }),
            ClientForceTarget::TabClose { tab_id } => Method::TabClose(TabCloseParams {
                tab_id,
                force: true,
            }),
            ClientForceTarget::WorkspaceClose {
                workspace_id,
                close_group,
            } => Method::WorkspaceClose(WorkspaceCloseParams {
                workspace_id,
                close_group,
                force: true,
            }),
            ClientForceTarget::PaneRespawn { pane_id } => Method::PaneRespawn(PaneRespawnParams {
                pane_id,
                force: true,
            }),
        };
        self.push_endpoint_method(method, outcome);
    }
}

#[cfg(test)]
mod tests {
    use super::refused_open_todo_count;

    #[test]
    fn the_todo_count_comes_from_the_refusal() {
        assert_eq!(
            refused_open_todo_count(
                "this pane still has 2 open todos (w1:p1: a; w1:p1: b); pass --force (force=true) to close it anyway"
            ),
            Some(2)
        );
        assert_eq!(
            refused_open_todo_count(
                "this pane is still running claude (pid 1) and has 1 open todo (w1:p1: ship 3 open todos)"
            ),
            Some(1)
        );
        assert_eq!(
            refused_open_todo_count("closing this pane would close a worktree group"),
            None
        );
    }
}
