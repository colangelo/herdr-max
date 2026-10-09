use crate::api::schema::{PaneSyncPairParams, ResponseResult};
use crate::app::App;

use super::responses::{encode_error, encode_success};

impl App {
    /// Start a pair using both captured pane identities. The active workspace
    /// and currently focused pane are deliberately irrelevant to this request.
    pub(super) fn handle_pane_sync_pair(
        &mut self,
        id: String,
        params: PaneSyncPairParams,
    ) -> String {
        let Some((source_workspace, source)) = self.parse_pane_id(&params.source_pane_id) else {
            return encode_error(id, "pane_not_found", "source pane not found");
        };
        let Some((target_workspace, target)) = self.parse_pane_id(&params.target_pane_id) else {
            return encode_error(id, "pane_not_found", "target pane not found");
        };
        if source == target {
            return encode_error(
                id,
                "invalid_sync_pair",
                "a sync pair needs two different panes",
            );
        }
        let source_tab = self.state.workspaces[source_workspace].find_tab_index_for_pane(source);
        let target_tab = self.state.workspaces[target_workspace].find_tab_index_for_pane(target);
        if source_workspace != target_workspace || source_tab.is_none() || source_tab != target_tab
        {
            return encode_error(
                id,
                "sync_tab_mismatch",
                "sync pair panes must belong to the same tab",
            );
        }
        let Some(tab_index) = source_tab else {
            return encode_error(id, "tab_not_found", "source tab not found");
        };
        self.state.workspaces[source_workspace].tabs[tab_index].start_sync_pair(source, target);
        encode_success(id, ResponseResult::Ok {})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::PaneId;

    fn fixture() -> (App, [PaneId; 3]) {
        let config = crate::config::Config::default();
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &config,
            crate::app::AppPolicy::TEST,
            None,
            rx,
            crate::api::EventHub::default(),
        );
        let mut workspace = crate::workspace::Workspace::test_new("pair");
        let a = workspace.tabs[0].root_pane;
        let b = workspace.test_split(ratatui::layout::Direction::Horizontal);
        let c = workspace.test_split(ratatui::layout::Direction::Vertical);
        workspace.tabs[0].layout.focus_pane(c);
        app.state.workspaces = vec![
            workspace,
            crate::workspace::Workspace::test_new("unrelated"),
        ];
        app.state.active = Some(1);
        app.state.selected = 1;
        app.state.ensure_test_terminals();
        (app, [a, b, c])
    }

    #[tokio::test]
    async fn sync_pair_uses_captured_source_and_replaces_the_group_without_global_focus() {
        let (mut app, [a, b, c]) = fixture();
        app.state.workspaces[0].tabs[0].set_sync(true);
        let request: crate::api::schema::Request = serde_json::from_value(serde_json::json!({
            "id": "pair",
            "method": "pane.sync_pair",
            "params": {
                "source_pane_id": app.public_pane_id(0, a).unwrap(),
                "target_pane_id": app.public_pane_id(0, b).unwrap(),
            },
        }))
        .unwrap();
        assert!(crate::api::request_changes_ui(&request));
        assert_eq!(
            crate::api::api_method_name(&request.method),
            "pane.sync_pair"
        );
        let response = app.handle_api_request(request);
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert!(response.get("error").is_none(), "{response}");
        let tab = &app.state.workspaces[0].tabs[0];
        assert!(tab.pane_synced(a) && tab.pane_synced(b) && !tab.pane_synced(c));
        assert!(!tab.sync.as_ref().unwrap().whole_tab);
        assert_eq!(tab.layout.focused(), c);
        assert_eq!(app.state.active, Some(1));
        assert!(!app.state.workspaces[1].tabs[0].is_syncing());
        app.state.assert_invariants_for_test();
    }

    #[tokio::test]
    async fn sync_pair_rejects_stale_identical_and_cross_tab_targets_without_mutation() {
        for reason in ["source_stale", "stale", "identical", "workspace", "tab"] {
            let (mut app, [a, b, c]) = fixture();
            app.state.workspaces[0].tabs[0].start_sync_pair(a, b);
            let source_pane_id = if reason == "source_stale" {
                "missing".to_owned()
            } else {
                app.public_pane_id(0, a).unwrap()
            };
            let target_pane_id = match reason {
                "source_stale" => app.public_pane_id(0, b).unwrap(),
                "stale" => "missing".to_owned(),
                "identical" => source_pane_id.clone(),
                "workspace" => app
                    .public_pane_id(1, app.state.workspaces[1].tabs[0].root_pane)
                    .unwrap(),
                _ => {
                    let tab = app.state.workspaces[0].test_add_tab(Some("other"));
                    app.state.ensure_test_terminals();
                    app.public_pane_id(0, app.state.workspaces[0].tabs[tab].root_pane)
                        .unwrap()
                }
            };
            let response = app.handle_pane_sync_pair(
                "invalid".into(),
                PaneSyncPairParams {
                    source_pane_id,
                    target_pane_id,
                },
            );
            let response: serde_json::Value = serde_json::from_str(&response).unwrap();
            assert!(response.get("error").is_some(), "{reason}: {response}");
            let tab = &app.state.workspaces[0].tabs[0];
            assert!(tab.is_syncing());
            assert!(tab.pane_synced(a) && tab.pane_synced(b) && !tab.pane_synced(c));
            assert!(!tab.sync.as_ref().unwrap().whole_tab);
            app.state.assert_invariants_for_test();
        }
    }
}
