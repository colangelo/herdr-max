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
    /// `pane.close_if_terminal`. The program is read from core last of
    /// all, before any layout change, on the connection the kill then goes
    /// through (fork issue 317 M1): a mismatch refuses with nothing closed,
    /// killed or swept.
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
        let terminal_id = self.state.terminal_id_for_pane(ws_idx, pane_id);
        if let Err(response) = self.program_live(
            &id,
            &params.pane_id,
            terminal_id.as_ref(),
            &params.expect,
            true,
        ) {
            return response;
        }
        let response = self.handle_pane_close(
            id,
            PaneCloseParams {
                pane_id: params.pane_id,
                force: params.force,
            },
        );
        // A close refused after the check (open todos, a group not ready)
        // leaves the runtime, and its next kill is no guarded one.
        if let Some(runtime) = terminal_id
            .as_ref()
            .and_then(|terminal_id| self.terminal_runtimes.get(terminal_id))
        {
            runtime.release_core_kill();
        }
        response
    }

    /// `pane.split_if_terminal`: the new pane starts in the target's
    /// folder, with no environment of the caller's, and takes no focus.
    pub(super) fn handle_pane_split_if_terminal(
        &mut self,
        id: String,
        params: PaneSplitIfTerminalParams,
    ) -> String {
        if let Err(response) = self.pane_showing_live(&id, &params.pane_id, &params.expect) {
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
        if let Err(response) = self.pane_showing_live(&id, &params.pane_id, &params.expect) {
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
        // Fork issue 317 M4: the pane's own id now, never one a workspace
        // left behind as an alias, and the tab the caller saw it in: a pane
        // moved to another tab since is refused, so a close cannot remove
        // the tab it moved to.
        if self.public_pane_id(ws_idx, pane_id).as_deref() != Some(pane_ref) {
            return mismatch("is not this pane's id now");
        }
        let tab = self.state.workspaces[ws_idx]
            .find_tab_index_for_pane(pane_id)
            .and_then(|tab_idx| self.public_tab_id(ws_idx, tab_idx));
        if tab.as_deref() != Some(expect.tab.as_str()) {
            return mismatch(&format!("is not in tab {}", expect.tab));
        }
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

    /// `pane_showing`, then core's live record of the pane's terminal.
    fn pane_showing_live(
        &self,
        id: &str,
        pane_ref: &str,
        expect: &PaneTerminalExpectation,
    ) -> Result<(usize, PaneId), String> {
        let (ws_idx, pane_id) = self.pane_showing(id, pane_ref, expect)?;
        let terminal_id = self.state.terminal_id_for_pane(ws_idx, pane_id);
        self.program_live(id, pane_ref, terminal_id.as_ref(), expect, false)?;
        Ok((ws_idx, pane_id))
    }

    /// Whether core shows the program `expect` names live at the pane's
    /// terminal now, read on the connection the pane's kill would use: what
    /// herdr cached at attach can be stale after core reconnected, when the
    /// address may hold another program (fork issue 317 M1). With
    /// `for_close`, that connection is kept for the close's kill.
    fn program_live(
        &self,
        id: &str,
        pane_ref: &str,
        terminal_id: Option<&crate::terminal::TerminalId>,
        expect: &PaneTerminalExpectation,
        for_close: bool,
    ) -> Result<(), String> {
        let Some(runtime) =
            terminal_id.and_then(|terminal_id| self.terminal_runtimes.get(terminal_id))
        else {
            return Err(encode_error(
                id.to_string(),
                TERMINAL_MISMATCH,
                format!("pane {pane_ref} runs no program; nothing was done"),
            ));
        };
        runtime
            .check_core_run(crate::pane::CoreRunCheck {
                terminal: &expect.terminal,
                pid: expect.pid,
                launch: &expect.launch,
                kill_through_it: for_close,
            })
            .map_err(|why| {
                encode_error(
                    id.to_string(),
                    TERMINAL_MISMATCH,
                    format!(
                        "pane {pane_ref}'s terminal {} {why}; nothing was done",
                        expect.terminal
                    ),
                )
            })
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
    use crate::app::api::RespawnTarget;
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
        let core = FakeCore::start_with(|config| config.distinct_addresses = true);
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
            json!({"terminal": format!("t{}", nth + 1), "pid": 4242,
                   "launch": run["params"]["labels"]["herdr.launch"],
                   "tab": self.tab(nth)})
        }

        /// The public id of the tab the `nth` pane is in now.
        fn tab(&self, nth: usize) -> String {
            let (ws, _) = self.app.find_pane(self.panes[nth]).expect("a pane");
            let tab = self.app.state.workspaces[ws]
                .find_tab_index_for_pane(self.panes[nth])
                .expect("a tab");
            self.app.public_tab_id(ws, tab).expect("a tab id")
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

        /// Any API call, as another caller of the far herdr would make it.
        fn raw(&mut self, method: &str, params: Value) -> Value {
            let request: Request =
                serde_json::from_value(json!({"id": "far:2", "method": method, "params": params}))
                    .expect("a request");
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

    // Fork issue 317, M1: after Herdr Core reconnects, the pane's address
    // can hold another program (a core that came back with its state wiped
    // gives `t1` again), and the pane attaches to it as its own: the pid and
    // launch label it cached at the first attach still match the request.
    // The far herdr reads the live program on the connection its kill would
    // use, so a guarded action is refused there, nothing renamed, split,
    // closed or killed; a pane whose program core still shows acts.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_reused_address_after_a_core_reconnect_is_refused() {
        let mut far = far();
        let (first, second) = (far.id(0), far.id(1));
        let aimed = far.expect(0);
        let attaches = |core: &FakeCore| {
            core.methods()
                .iter()
                .filter(|method| *method == "terminal.attach")
                .count()
        };
        let before = attaches(&far.core);
        far.core
            .add_terminal(json!({"address": "t1", "pid": 5151, "state": "live",
                                     "labels": {"herdr.launch": "another-run"}}));
        far.core.hang_up();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while attaches(&far.core) < before + far.panes.len() {
            assert!(
                std::time::Instant::now() < until,
                "not reattached: {:?}",
                far.core.methods()
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let run = far.app.state.workspaces[0]
            .terminal_id(far.panes[0])
            .and_then(|terminal| far.app.terminal_runtimes.get(terminal))
            .and_then(crate::terminal::TerminalRuntime::core_run)
            .expect("a core run");
        assert_eq!(run.pid, 4242, "the pane still names the old program");

        for (method, params) in [
            ("pane.rename_if_terminal", json!({"label": "server"})),
            ("pane.split_if_terminal", json!({"direction": "right"})),
            ("pane.close_if_terminal", json!({"force": true})),
        ] {
            let mut params = params;
            params["pane_id"] = json!(first);
            params["expect"] = aimed.clone();
            let refused = far.call(method, params);
            assert_eq!(refused["error"]["code"], "terminal_mismatch", "{refused}");
        }
        assert!(
            !far.core
                .methods()
                .iter()
                .any(|method| method == "terminal.kill"),
            "{:?}",
            far.core.methods()
        );
        assert_eq!(far.panes_of(0), 2, "nothing split or closed");
        assert_eq!(far.label(0), None, "not renamed");

        let closed = far.call(
            "pane.close_if_terminal",
            json!({"pane_id": second, "expect": far.expect(1)}),
        );
        assert!(closed["result"].is_object(), "{closed}");
        assert_eq!(far.panes_of(0), 1);
    }

    // Fork issue 317 M4: the guard names the tab the view saw the pane in.
    // A pane moved to another tab of its workspace since is refused, and so
    // its close cannot remove the tab it moved to; a pane moved to another
    // workspace leaves its old id behind as an alias, which names no pane
    // for a guarded action. Both tabs and the pane stay.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pane_moved_to_another_tab_is_refused() {
        let mut far = far();
        let second = far.id(1);
        let aimed = far.expect(1);
        let moved = far.raw(
            "pane.move",
            json!({"pane_id": second, "destination": {"type": "new_tab"}}),
        );
        assert!(moved["result"].is_object(), "{moved}");
        assert_ne!(far.tab(1), aimed["tab"].as_str().unwrap_or_default());
        let tabs = far.app.state.workspaces[0].tabs.len();
        assert_eq!(tabs, 2);
        for (method, params) in [
            ("pane.rename_if_terminal", json!({"label": "server"})),
            ("pane.split_if_terminal", json!({"direction": "right"})),
            ("pane.close_if_terminal", json!({"force": true})),
        ] {
            let mut params = params;
            params["pane_id"] = json!(far.id(1));
            params["expect"] = aimed.clone();
            let refused = far.call(method, params);
            assert_eq!(refused["error"]["code"], "terminal_mismatch", "{refused}");
            assert!(
                refused["error"]["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("is not in tab")),
                "{refused}"
            );
        }
        assert_eq!(
            far.app.state.workspaces[0].tabs.len(),
            tabs,
            "both tabs stay"
        );
        assert_eq!(far.label(1), None);

        let alone = far.app.public_workspace_id(1);
        let before = far.id(1);
        let moved = far.raw(
            "pane.move",
            json!({"pane_id": before, "destination": {"type": "new_tab", "workspace_id": alone}}),
        );
        assert!(moved["result"].is_object(), "{moved}");
        assert_ne!(far.id(1), before, "a new id in its new workspace");
        let mut aimed = far.expect(1);
        let refused = far.call(
            "pane.close_if_terminal",
            json!({"pane_id": before, "expect": aimed.clone()}),
        );
        assert_eq!(refused["error"]["code"], "terminal_mismatch", "{refused}");
        assert!(
            refused["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("not this pane's id now")),
            "{refused}"
        );
        aimed["tab"] = json!(far.tab(1));
        let closed = far.call(
            "pane.rename_if_terminal",
            json!({"pane_id": far.id(1), "label": "here", "expect": aimed}),
        );
        assert!(closed["result"].is_object(), "its own id and tab: {closed}");
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

    // Fork issue 317 M8: a guarded close of a worktree group parent's last
    // pane is refused under both `confirm_close` settings: with it off, a
    // plain close would close the whole group, the linked workspace's
    // programs included; with it on, it would ask. The group stays.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_guarded_close_never_closes_a_worktree_group() {
        let core = FakeCore::start_with(|config| config.distinct_addresses = true);
        let parent = crate::workspace::Workspace::test_new("parent");
        let linked = crate::workspace::Workspace::test_new("linked");
        let panes = vec![parent.tabs[0].root_pane, linked.tabs[0].root_pane];
        let app = super::test_app_on_fake_core(&core, vec![parent, linked], &panes);
        let mut far = Far { app, core, panes };
        for (ws, is_linked_worktree) in [(0, false), (1, true)] {
            far.app.state.workspaces[ws].worktree_space =
                Some(crate::workspace::WorktreeSpaceMembership {
                    key: "repo-key".into(),
                    label: "herdr".into(),
                    repo_root: "/repo/herdr".into(),
                    checkout_path: format!("/repo/checkout-{ws}").into(),
                    is_linked_worktree,
                });
        }
        assert!(far.app.state.workspace_close_would_close_worktree_group(0));
        for confirm_close in [false, true] {
            far.app.state.confirm_close = confirm_close;
            let refused = far.call(
                "pane.close_if_terminal",
                json!({"pane_id": far.id(0), "force": true, "expect": far.expect(0)}),
            );
            assert_eq!(
                refused["error"]["code"], "would_close_workspace",
                "confirm_close {confirm_close}: {refused}"
            );
            assert_eq!(far.app.state.workspaces.len(), 2, "the group stays");
        }
    }

    // Fork issue 317 M8: a floating pane (the popup) beside a workspace's
    // last tiled pane does not make that pane's close a close of one pane:
    // the workspace's close counts its tiled panes, so the guarded close is
    // refused, and the workspace and the floating pane stay. The floating
    // pane itself is no pane a guarded action names.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_floating_pane_beside_the_last_tiled_one_closes_nothing() {
        let mut far = far();
        let terminal_id = crate::terminal::TerminalId::alloc();
        far.app.state.terminals.insert(
            terminal_id.clone(),
            crate::terminal::TerminalState::new(terminal_id.clone(), "/popup".into()),
        );
        let floating = PaneId::alloc();
        far.app.state.popup_pane = Some(crate::app::state::PopupPaneState {
            pane_id: floating,
            terminal_id,
            width: None,
            height: None,
            plugin_id: None,
        });
        let alone = far.id(2);
        let refused = far.call(
            "pane.close_if_terminal",
            json!({"pane_id": alone, "expect": far.expect(2)}),
        );
        assert_eq!(
            refused["error"]["code"], "would_close_workspace",
            "{refused}"
        );
        assert_eq!(far.app.state.workspaces.len(), 2, "the workspace stays");
        assert!(
            far.app.state.popup_pane.is_some(),
            "the floating pane stays"
        );
        let named = far.call(
            "pane.close_if_terminal",
            json!({"pane_id": format!("p_{}", floating.raw()), "expect": far.expect(2)}),
        );
        assert_eq!(named["error"]["code"], "pane_not_found", "{named}");
        assert!(far.app.state.popup_pane.is_some());
    }

    // A pane whose program is not in herdr-core has no terminal a view could
    // have named: refused, with nothing done. The pane here is a real PTY
    // pane (a shell, its runtime present), the third pane of the fixture
    // respawned outside core, so the refusal is the `core_terminal()`
    // check's, not a missing runtime's.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pane_outside_core_is_refused() {
        let mut far = far();
        let alone = far.id(2);
        crate::pane::set_preferred_runtime(false);
        assert!(far
            .app
            .respawn_pane_runtime(far.panes[2], RespawnTarget::Shell));
        let runtime = far.app.state.workspaces[1]
            .terminal_id(far.panes[2])
            .and_then(|terminal| far.app.terminal_runtimes.get(terminal))
            .expect("a runtime");
        assert!(runtime.core_terminal().is_none(), "a PTY pane, not core");

        let expect = far.expect(2);
        for (method, params) in [
            ("pane.rename_if_terminal", json!({"label": "x"})),
            ("pane.split_if_terminal", json!({"direction": "right"})),
            ("pane.close_if_terminal", json!({})),
        ] {
            let mut params = params;
            params["pane_id"] = json!(alone);
            params["expect"] = expect.clone();
            let refused = far.call(method, params);
            assert_eq!(refused["error"]["code"], "terminal_mismatch", "{refused}");
            assert!(
                refused["error"]["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("does not run in Herdr Core")),
                "{refused}"
            );
        }
        assert_eq!(far.panes_of(1), 1, "nothing split or closed");
        assert_eq!(far.label(2), None, "not renamed");
    }
}
