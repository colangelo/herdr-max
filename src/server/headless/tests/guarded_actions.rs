//! A view's guarded pane actions (`pane.close_if_terminal`,
//! `pane.split_if_terminal`; fork issue 316) change the tabs and the layout
//! of the far herdr like their plain twins, so the far TUI clients must see
//! it: their shell locations are reconciled, and the controlled tab
//! geometry is applied again.

use super::*;
use crate::api::schema::Request;
use crate::pane::FakeCore;
use serde_json::{json, Value};

/// A far herdr whose panes run in a fake herdr-core: workspace `w` with one
/// pane in each of `tabs` tabs, and one shell client looking at the last.
struct GuardedFar {
    server: HeadlessServer,
    core: FakeCore,
    panes: Vec<crate::layout::PaneId>,
    tab_ids: Vec<String>,
}

impl Drop for GuardedFar {
    fn drop(&mut self) {
        for (_, runtime) in self.server.app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
    }
}

fn guarded_far(tabs: usize) -> (GuardedFar, std::sync::mpsc::Receiver<Vec<u8>>) {
    let core = FakeCore::start_with(|config| config.distinct_addresses = true);
    let mut workspace = crate::workspace::Workspace::test_new("w");
    let mut panes = vec![workspace.tabs[0].root_pane];
    for nth in 1..tabs {
        let tab = workspace.test_add_tab(Some(&format!("t{nth}")));
        panes.push(workspace.tabs[tab].root_pane);
    }
    let app = crate::app::test_app_on_fake_core(&core, vec![workspace], &panes);
    let mut server = test_headless_server_from_app(app);
    server.app.state.selected = 0;
    server.app.state.mode = crate::app::Mode::Terminal;
    let tab_ids = (0..tabs)
        .map(|tab| server.app.public_tab_id(0, tab).expect("tab id"))
        .collect();
    let (control, _render) = connect_test_shell(&mut server, 71, 100, 30);
    let _ = control.recv().expect("snapshot");
    (
        GuardedFar {
            server,
            core,
            panes,
            tab_ids,
        },
        control,
    )
}

impl GuardedFar {
    fn pane_id(&self, nth: usize) -> String {
        self.server
            .app
            .public_pane_id(0, self.panes[nth])
            .expect("a public id")
    }

    /// What a view names for the `nth` pane's program.
    fn expect(&self, nth: usize) -> Value {
        let run = self
            .core
            .requests()
            .into_iter()
            .filter(|request| request["method"] == "terminal.run")
            .nth(nth)
            .expect("a run");
        json!({"terminal": format!("t{}", nth + 1), "pid": 4242,
               "launch": run["params"]["labels"]["herdr.launch"], "tab": self.tab_ids[nth]})
    }

    fn public(&mut self, method: &str, params: Value) -> Value {
        let request: Request =
            serde_json::from_value(json!({"id": "far:1", "method": method, "params": params}))
                .expect("a request");
        let (respond_to, response) = std::sync::mpsc::channel();
        self.server
            .handle_api_request_with_shutdown_check(api::ApiRequestMessage {
                request,
                respond_to,
                response_write_complete: None,
            });
        serde_json::from_str(&response.recv().expect("a response")).expect("json")
    }

    fn size_of(&self, pane: crate::layout::PaneId) -> (u16, u16) {
        let terminal = self.server.app.state.workspaces[0]
            .terminal_id(pane)
            .cloned()
            .expect("a terminal");
        self.server
            .app
            .terminal_runtimes
            .get(&terminal)
            .expect("a runtime")
            .current_size()
    }

    fn client_sees_tab(&self, tab_id: &str) -> bool {
        self.server.clients[&71]
            .shell_location
            .as_ref()
            .expect("a location")
            .active_tab_ids
            .values()
            .any(|active| active == tab_id)
    }
}

// A guarded close of the last pane of a tab that is not the workspace's last
// closes the tab, like a plain close: the far TUI client that looked at it is
// moved off it, and the geometry claim on it is dropped.
#[tokio::test(flavor = "multi_thread")]
async fn a_guarded_close_of_a_tabs_last_pane_reconciles_the_far_clients() {
    let (mut far, _control) = guarded_far(2);
    let gone = far.tab_ids[1].clone();
    assert!(far.server.focus_shell_client_on_tab(71, &gone));
    assert!(far.server.claim_shell_tab_geometry(71, false));
    assert!(far.client_sees_tab(&gone));
    assert!(far.server.tab_geometry_controllers.contains_key(&gone));

    let (pane, expect) = (far.pane_id(1), far.expect(1));
    let closed = far.public(
        "pane.close_if_terminal",
        json!({"pane_id": pane, "expect": expect}),
    );
    assert!(closed["result"].is_object(), "{closed}");

    assert_eq!(far.server.app.state.workspaces[0].tabs.len(), 1);
    assert!(!far.client_sees_tab(&gone), "the client keeps a stale tab");
    assert!(!far.server.tab_geometry_controllers.contains_key(&gone));
}

// A guarded split changes the layout of a tab a far client controls: the
// geometry is applied again, so the panes have the size the client's surface
// gives them (the same as after a plain split).
#[tokio::test(flavor = "multi_thread")]
async fn a_guarded_split_applies_the_controlled_geometry_again() {
    let (mut far, _control) = guarded_far(1);
    assert_eq!(
        far.server.tab_geometry_controllers.get(&far.tab_ids[0]),
        Some(&71),
        "the client controls the tab"
    );
    let first = far.panes[0];
    let before = far.size_of(first);

    let (pane, expect) = (far.pane_id(0), far.expect(0));
    let split = far.public(
        "pane.split_if_terminal",
        json!({"pane_id": pane, "direction": "right", "expect": expect}),
    );
    assert!(split["result"].is_object(), "{split}");

    let after = far.size_of(first);
    assert_ne!(after, before, "the first pane was not resized");
}
