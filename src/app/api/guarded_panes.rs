//! A pane's close, split and rename bound to the program the caller saw
//! there (core-phase5 D7, fork issue 316). A view of this herdr on another
//! Mac names the pane by its public id (`w3:p4`), and that id can move to
//! another pane, or the pane's terminal can hold another program, between
//! what the view showed and the moment this server acts. So the request
//! names the program too, as the view attached to it: the pane's terminal
//! in the primary herdr-core session (`t7`, as the `herdr-app` document
//! names it), its pid and the `herdr.launch` label of its run. This server
//! compares all three with the pane the id resolves to, in the same turn of
//! its loop as the action, and refuses a mismatch with `terminal_mismatch`.
//!
//! A guarded close is a close of that one pane only: one that would close
//! its workspace (the pane is the workspace's last), and with a worktree
//! group's parent the whole group, is refused with `would_close_workspace`.

use std::collections::HashMap;

use crate::api::schema::{
    PaneCloseIfTerminalParams, PaneCloseParams, PaneRenameIfTerminalParams, PaneRenameParams,
    PaneRightClickTarget, PaneSplitIfTerminalParams, PaneSplitParams, PaneTerminalExpectation,
};
use crate::app::App;
use crate::layout::PaneId;

use super::responses::encode_error;

/// The code of a guarded action refused because the pane shows another
/// program than the one the request names.
pub(crate) const TERMINAL_MISMATCH: &str = "terminal_mismatch";
/// The code of a guarded close refused because it would close the pane's
/// workspace.
pub(crate) const WOULD_CLOSE_WORKSPACE: &str = "would_close_workspace";

impl App {
    /// `pane.close_if_terminal`.
    pub(super) fn handle_pane_close_if_terminal(
        &mut self,
        id: String,
        params: PaneCloseIfTerminalParams,
    ) -> String {
        let (ws_idx, pane_id) = match self.pane_showing(&id, &params.pane_id, &params.expect) {
            Ok(pane) => pane,
            Err(response) => return response,
        };
        if self.state.close_pane_would_close_workspace(ws_idx, pane_id) {
            return encode_error(
                id,
                WOULD_CLOSE_WORKSPACE,
                format!(
                    "pane {} is the last of its workspace, so closing it would close the workspace (and a worktree group with its parent); nothing was closed",
                    params.pane_id
                ),
            );
        }
        self.handle_pane_close(
            id,
            PaneCloseParams {
                pane_id: params.pane_id,
                force: params.force,
            },
        )
    }

    /// `pane.split_if_terminal`: the new pane starts in the target's
    /// folder, with no environment of the caller's, and takes no focus.
    pub(super) fn handle_pane_split_if_terminal(
        &mut self,
        id: String,
        params: PaneSplitIfTerminalParams,
    ) -> String {
        if let Err(response) = self.pane_showing(&id, &params.pane_id, &params.expect) {
            return response;
        }
        self.handle_pane_split(
            id,
            PaneSplitParams {
                workspace_id: None,
                target_pane_id: Some(params.pane_id),
                direction: params.direction,
                ratio: params.ratio,
                cwd: None,
                focus: false,
                right_click: PaneRightClickTarget::default(),
                env: HashMap::new(),
                on: None,
            },
        )
    }

    /// `pane.rename_if_terminal`.
    pub(super) fn handle_pane_rename_if_terminal(
        &mut self,
        id: String,
        params: PaneRenameIfTerminalParams,
    ) -> String {
        if let Err(response) = self.pane_showing(&id, &params.pane_id, &params.expect) {
            return response;
        }
        self.handle_pane_rename(
            id,
            PaneRenameParams {
                pane_id: params.pane_id,
                label: params.label,
            },
        )
    }

    /// The pane `pane_ref` names, if it shows the program `expect` names:
    /// its terminal is `expect.terminal` in the primary herdr-core session
    /// (the one the `herdr-app` document is written to), with that pid and
    /// launch label, all known. `Err` is the encoded refusal.
    fn pane_showing(
        &self,
        id: &str,
        pane_ref: &str,
        expect: &PaneTerminalExpectation,
    ) -> Result<(usize, PaneId), String> {
        let Some((ws_idx, pane_id)) = self.parse_pane_id(pane_ref) else {
            return Err(encode_error(
                id.to_string(),
                "pane_not_found",
                format!("pane {pane_ref} not found"),
            ));
        };
        let mismatch = |what: &str| {
            Err(encode_error(
                id.to_string(),
                TERMINAL_MISMATCH,
                format!("pane {pane_ref} {what}; nothing was done"),
            ))
        };
        let Some(runtime) = self
            .state
            .workspaces
            .get(ws_idx)
            .and_then(|ws| ws.terminal_id(pane_id))
            .and_then(|terminal_id| self.terminal_runtimes.get(terminal_id))
        else {
            return mismatch("runs no program");
        };
        let Some(link) = runtime.core_terminal() else {
            return mismatch("does not run in Herdr Core");
        };
        if link.session != crate::pane::core_session_name() || link.address != expect.terminal {
            return mismatch(&format!("shows another terminal than {}", expect.terminal));
        }
        let run = runtime.core_run().unwrap_or_default();
        if run.pid == 0 || run.pid != expect.pid || run.launch.as_deref() != Some(&expect.launch) {
            return mismatch(&format!(
                "runs another program at {} than the one asked for",
                expect.terminal
            ));
        }
        Ok((ws_idx, pane_id))
    }
}

/// An app whose `workspaces` have each of `panes` run a shell in the fake
/// herdr-core `core`: the far herdr of the guarded-action tests.
#[cfg(all(test, unix))]
pub(crate) fn test_app_on_fake_core(
    core: &crate::pane::FakeCore,
    workspaces: Vec<crate::workspace::Workspace>,
    panes: &[PaneId],
) -> App {
    std::env::set_var("HC_SOCKET", &core.socket);
    let mut app = App::new(
        &crate::config::Config::default(),
        crate::app::AppPolicy::TEST,
        None,
        tokio::sync::mpsc::unbounded_channel().1,
        crate::api::EventHub::default(),
    );
    app.state.workspaces = workspaces;
    app.state.active = Some(0);
    app.state.ensure_test_terminals();
    app.state.default_shell = "/bin/cat".into();
    app.program_start_token = |_| crate::platform::ProcessStart::table(None);
    crate::pane::set_preferred_runtime(true);
    for pane in panes {
        assert!(app.respawn_pane_runtime(*pane, super::RespawnTarget::Shell));
    }
    app
}

#[cfg(all(test, unix))]
mod tests {
    use serde_json::{json, Value};

    use crate::api::schema::{Method, Request};
    use crate::app::App;
    use crate::layout::PaneId;
    use crate::pane::FakeCore;

    /// A far herdr: workspace `far` split into two panes and workspace
    /// `alone` with one, each pane's program in the primary herdr-core
    /// session (a fake core here).
    struct Far {
        app: App,
        core: FakeCore,
        panes: Vec<PaneId>,
    }

    impl Drop for Far {
        fn drop(&mut self) {
            for (_, runtime) in self.app.terminal_runtimes.drain() {
                runtime.shutdown();
            }
        }
    }

    fn far() -> Far {
        let core = FakeCore::start();
        let mut split = crate::workspace::Workspace::test_new("far");
        let first = split.tabs[0].root_pane;
        let second = split.test_split(ratatui::layout::Direction::Horizontal);
        let alone = crate::workspace::Workspace::test_new("alone");
        let third = alone.tabs[0].root_pane;
        let panes = vec![first, second, third];
        let app = super::test_app_on_fake_core(&core, vec![split, alone], &panes);
        Far { app, core, panes }
    }

    impl Far {
        fn id(&self, nth: usize) -> String {
            let (ws, _) = self.app.find_pane(self.panes[nth]).expect("a pane");
            self.app
                .public_pane_id(ws, self.panes[nth])
                .expect("a public id")
        }

        /// What a view names for the `nth` pane's program: as the fake
        /// core started it (each run its own launch label).
        fn expect(&self, nth: usize) -> Value {
            let run = self
                .core
                .requests()
                .into_iter()
                .filter(|request| request["method"] == "terminal.run")
                .nth(nth)
                .expect("a run");
            json!({"terminal": "t1", "pid": 4242,
                   "launch": run["params"]["labels"]["herdr.launch"]})
        }

        fn call(&mut self, method: &str, params: Value) -> Value {
            let request: Request =
                serde_json::from_value(json!({"id": "far:1", "method": method, "params": params}))
                    .expect("a request");
            assert!(matches!(
                request.method,
                Method::PaneCloseIfTerminal(_)
                    | Method::PaneSplitIfTerminal(_)
                    | Method::PaneRenameIfTerminal(_)
                    | Method::PaneClose(_)
            ));
            serde_json::from_str(&self.app.handle_api_request(request)).expect("json")
        }

        fn panes_of(&self, ws: usize) -> usize {
            self.app.state.workspaces[ws]
                .tabs
                .iter()
                .map(|tab| tab.layout.pane_count())
                .sum()
        }

        fn label(&self, nth: usize) -> Option<String> {
            let terminal = self.app.state.pane_terminal(self.panes[nth])?;
            terminal.manual_label.clone()
        }
    }

    // Fork issue 316, M1: the far herdr acts only on the program the view
    // aimed at. The public id names the pane; the terminal, pid and launch
    // label name its program, and any of them differing (the id moved to
    // another pane, or the pane shows another program) refuses the action,
    // with nothing done.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_guarded_action_acts_only_on_the_program_it_names() {
        let mut far = far();
        let (first, second) = (far.id(0), far.id(1));
        let mut wrong_pid = far.expect(0);
        wrong_pid["pid"] = json!(4243);
        let mut wrong_terminal = far.expect(0);
        wrong_terminal["terminal"] = json!("t9");
        for expect in [far.expect(1), wrong_pid, wrong_terminal] {
            let refused = far.call(
                "pane.rename_if_terminal",
                json!({"pane_id": first, "label": "server", "expect": expect}),
            );
            assert_eq!(refused["error"]["code"], "terminal_mismatch", "{refused}");
            assert_eq!(far.label(0), None, "not renamed");
            let refused = far.call(
                "pane.split_if_terminal",
                json!({"pane_id": first, "direction": "right", "expect": expect}),
            );
            assert_eq!(refused["error"]["code"], "terminal_mismatch", "{refused}");
            let refused = far.call(
                "pane.close_if_terminal",
                json!({"pane_id": first, "expect": expect}),
            );
            assert_eq!(refused["error"]["code"], "terminal_mismatch", "{refused}");
            assert_eq!(far.panes_of(0), 2, "nothing split or closed");
        }

        let renamed = far.call(
            "pane.rename_if_terminal",
            json!({"pane_id": first, "label": "server", "expect": far.expect(0)}),
        );
        assert!(renamed["result"].is_object(), "{renamed}");
        assert_eq!(far.label(0).as_deref(), Some("server"));
        let split = far.call(
            "pane.split_if_terminal",
            json!({"pane_id": first, "direction": "right", "expect": far.expect(0)}),
        );
        assert!(split["result"].is_object(), "{split}");
        assert_eq!(far.panes_of(0), 3);
        let closed = far.call(
            "pane.close_if_terminal",
            json!({"pane_id": second, "expect": far.expect(1)}),
        );
        assert!(closed["result"].is_object(), "{closed}");
        assert_eq!(far.panes_of(0), 2);
    }

    // Fork issue 316, M2: a guarded close is a close of one pane. The last
    // pane of a workspace is refused, so the workspace (and a worktree
    // group with its parent) stays; a plain `pane.close` would close it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_guarded_close_never_closes_a_workspace() {
        let mut far = far();
        let alone = far.id(2);
        let refused = far.call(
            "pane.close_if_terminal",
            json!({"pane_id": alone, "force": true, "expect": far.expect(2)}),
        );
        assert_eq!(
            refused["error"]["code"], "would_close_workspace",
            "{refused}"
        );
        assert_eq!(far.app.state.workspaces.len(), 2, "the workspace stays");
        let plain = far.call("pane.close", json!({"pane_id": alone}));
        assert!(plain["result"].is_object(), "{plain}");
        assert_eq!(far.app.state.workspaces.len(), 1, "a plain close closes it");
    }

    // A pane whose program is not in herdr-core has no terminal a view could
    // have named: refused.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pane_outside_core_is_refused() {
        let mut far = far();
        let first = far.id(0);
        let expect = far.expect(0);
        let terminal_id = far.app.state.workspaces[0].tabs[0]
            .terminal_id(far.panes[0])
            .cloned()
            .expect("a terminal");
        if let Some(runtime) = far.app.terminal_runtimes.remove(&terminal_id) {
            runtime.shutdown();
        }
        let refused = far.call(
            "pane.rename_if_terminal",
            json!({"pane_id": first, "label": "x", "expect": expect}),
        );
        assert_eq!(refused["error"]["code"], "terminal_mismatch", "{refused}");
    }
}
