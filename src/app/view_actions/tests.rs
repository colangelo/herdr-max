use serde_json::{json, Value};

use super::*;
use crate::api::schema::{PaneTarget, SplitDirection};

fn app() -> App {
    let mut app = App::new(
        &crate::config::Config::default(),
        crate::app::AppPolicy::TEST,
        None,
        tokio::sync::mpsc::unbounded_channel().1,
        crate::api::EventHub::default(),
    );
    app.view = Some(crate::app::view_layout::ViewApp::new(
        MachineName::parse("loop").unwrap(),
    ));
    // The far herdr's space w3 with two tabs: w3:t1 split into w3:p1 and
    // w3:p2, w3:t2 with w3:p3.
    let block = |id: &str, terminal: &str| json!({"id": id, "type": "block", "content": {"terminal": terminal}, "size": {"weight": 0.5}});
    let doc = json!({"spaces": [{"id": "w3", "name": "api", "folder": "/far/api", "herdr_max": 1, "tabs": [
        {"id": "w3:t1", "root": {"id": "w3:t1", "type": "row", "children": [block("w3:p1", "t1"), block("w3:p2", "t2")]}},
        {"id": "w3:t2", "name": "logs", "root": {"id": "w3:t2", "type": "row", "children": [block("w3:p3", "t3")]}}
    ]}]});
    app.apply_view_layout(&crate::view::doc::parse(&doc).expect("a good document"));
    app
}

fn far(app: &App, method: Method) -> Value {
    let method = app.view_far_method(&method).expect("a far request");
    let line = far_request_line(method).expect("a line");
    assert_eq!(line.last(), Some(&b'\n'), "one line");
    assert_eq!(
        line.iter().filter(|byte| **byte == b'\n').count(),
        1,
        "one line"
    );
    serde_json::from_slice(&line).expect("json")
}

// V20 (D7, D11), the request half: a workspace named `$(touch x); w` goes
// to the far herdr as one JSON string of the request, never as script
// text; the fleet half (`core_machine::fleet::view`) shows the argv.
#[test]
fn v20_a_name_goes_far_as_one_json_string() {
    let app = app();
    let name = "$(touch x); w";
    let request = far(
        &app,
        Method::WorkspaceRename(WorkspaceRenameParams {
            workspace_id: "w3".into(),
            label: name.into(),
        }),
    );
    assert_eq!(request["method"], "workspace.rename");
    assert_eq!(
        request["params"],
        json!({"workspace_id": "w3", "label": name})
    );
    assert!(request["id"]
        .as_str()
        .is_some_and(|id| id.starts_with("view:")));
}

// E.3: each action is the far herdr's own method on the far ids, with the
// person's names and nothing of this Mac: no folder, environment, machine
// or far focus.
#[test]
fn each_action_names_the_far_targets_and_nothing_of_this_mac() {
    let app = app();
    let tab = far(
        &app,
        Method::TabCreate(TabCreateParams {
            workspace_id: Some("w3".into()),
            cwd: Some("/Users/here".into()),
            focus: true,
            label: Some("build".into()),
            env: HashMap::from([("SECRET".into(), "x".into())]),
            on: Some("elsewhere".into()),
        }),
    );
    assert_eq!(tab["method"], "tab.create");
    assert_eq!(
        tab["params"],
        json!({"workspace_id": "w3", "focus": false, "label": "build"})
    );

    let split = far(
        &app,
        Method::PaneSplit(PaneSplitParams {
            workspace_id: Some("w3".into()),
            target_pane_id: Some("w3:p2".into()),
            direction: SplitDirection::Down,
            ratio: Some(0.25),
            cwd: Some("/Users/here".into()),
            focus: true,
            right_click: PaneRightClickTarget::Pane,
            env: HashMap::from([("SECRET".into(), "x".into())]),
            on: Some("elsewhere".into()),
        }),
    );
    assert_eq!(split["method"], "pane.split");
    assert_eq!(
        split["params"],
        json!({"workspace_id": "w3", "target_pane_id": "w3:p2", "direction": "down",
               "ratio": 0.25, "focus": false, "right_click": "herdr"})
    );

    let close = far(
        &app,
        Method::PaneClose(PaneCloseParams {
            pane_id: "w3:p3".into(),
            force: true,
        }),
    );
    assert_eq!(close["method"], "pane.close");
    assert_eq!(close["params"], json!({"pane_id": "w3:p3", "force": true}));

    let tab_rename = far(
        &app,
        Method::TabRename(TabRenameParams {
            tab_id: "w3:t2".into(),
            label: "tail".into(),
        }),
    );
    assert_eq!(tab_rename["method"], "tab.rename");
    assert_eq!(
        tab_rename["params"],
        json!({"tab_id": "w3:t2", "label": "tail"})
    );

    let pane_rename = far(
        &app,
        Method::PaneRename(PaneRenameParams {
            pane_id: "w3:p1".into(),
            label: Some("server".into()),
        }),
    );
    assert_eq!(pane_rename["method"], "pane.rename");
    assert_eq!(
        pane_rename["params"],
        json!({"pane_id": "w3:p1", "label": "server"})
    );
}

// A split with no target splits the far pane the view shows focused in the
// workspace's active tab.
#[test]
fn a_split_with_no_target_splits_the_focused_far_pane() {
    let app = app();
    let split = far(
        &app,
        Method::PaneSplit(PaneSplitParams {
            workspace_id: Some("w3".into()),
            target_pane_id: None,
            direction: SplitDirection::Right,
            ratio: None,
            cwd: None,
            focus: false,
            right_click: PaneRightClickTarget::Herdr,
            env: HashMap::new(),
            on: None,
        }),
    );
    assert_eq!(split["params"]["target_pane_id"], "w3:p1");
}

// Only what the far document showed can be named: an id the view does not
// show is refused here, before anything is sent, and so is a method a view
// does not send there.
#[test]
fn a_target_the_view_does_not_show_is_refused_unsent() {
    let app = app();
    for method in [
        Method::PaneClose(PaneCloseParams {
            pane_id: "w3:p9".into(),
            force: false,
        }),
        Method::TabRename(TabRenameParams {
            tab_id: "w3:t9".into(),
            label: "x".into(),
        }),
        Method::WorkspaceRename(WorkspaceRenameParams {
            workspace_id: "w9".into(),
            label: "x".into(),
        }),
    ] {
        let err = app.view_far_method(&method).expect_err("refused");
        assert!(err.contains("is not one loop's herdr shows"), "{err}");
    }
    let err = app
        .view_far_method(&Method::PaneFocus(PaneTarget {
            pane_id: "w3:p1".into(),
        }))
        .expect_err("not sent far");
    assert!(err.contains("not an action a view sends"), "{err}");
}

fn reply(text: &str) -> Value {
    serde_json::from_str(text).expect("a response")
}

// D7: a far herdr that does not know the method (an older build) is a
// missing verb: the action is refused with a notice that says so, as the
// action's own answer; nothing else changes.
#[test]
fn a_missing_far_verb_is_the_actions_refusal() {
    let machine = MachineName::parse("loop").unwrap();
    let far_reply = br#"{"id":"view:1","error":{"code":"unknown_method","message":"unknown method: tab.create"}}
"#;
    let answer = reply(&client_reply(
        &machine,
        "tab.create",
        "endpoint:1".into(),
        Ok(far_reply.to_vec()),
    ));
    assert_eq!(answer["id"], "endpoint:1");
    assert_eq!(answer["error"]["code"], "unknown_method");
    let message = answer["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("loop's herdr does not take tab.create; update Herdr there"),
        "{message}"
    );
}

// The far answers become the client's: a result it reads, a far error with
// its code (a close's confirmation is asked for here, then sent forced) and
// its text escaped, and a send that failed with why.
#[test]
fn far_answers_become_the_clients() {
    let machine = MachineName::parse("loop").unwrap();
    let ok = reply(&client_reply(
        &machine,
        "pane.rename",
        "endpoint:2".into(),
        Ok(br#"{"id":"view:2","result":{"type":"ok"}}"#.to_vec()),
    ));
    assert_eq!(ok, json!({"id": "endpoint:2", "result": {"type": "ok"}}));

    let unread = reply(&client_reply(
        &machine,
        "pane.rename",
        "endpoint:3".into(),
        Ok(br#"{"id":"view:3","result":{"type":"a_result_from_a_newer_herdr"}}"#.to_vec()),
    ));
    assert_eq!(unread["result"], json!({"type": "ok"}));

    let confirm = reply(&client_reply(
        &machine,
        "pane.close",
        "endpoint:4".into(),
        Ok(br#"{"id":"view:4","error":{"code":"confirmation_required","message":"the pane has open todos\u001b]52;c;eA==\u0007"}}"#.to_vec()),
    ));
    assert_eq!(confirm["error"]["code"], "confirmation_required");
    let message = confirm["error"]["message"].as_str().unwrap();
    assert!(
        message.starts_with("loop: the pane has open todos"),
        "{message}"
    );
    assert!(
        !message.contains('\u{1b}') && !message.contains('\u{7}'),
        "{message:?}"
    );

    let odd_code = reply(&client_reply(
        &machine,
        "pane.close",
        "endpoint:5".into(),
        Ok(br#"{"id":"view:5","error":{"code":"Odd Code\u001b","message":"no"}}"#.to_vec()),
    ));
    assert_eq!(odd_code["error"]["code"], NOT_DONE);

    let failed = reply(&client_reply(
        &machine,
        "tab.create",
        "endpoint:6".into(),
        Err("loop is dormant: press a key in one of its panes to wake it\u{1b}[2J".into()),
    ));
    assert_eq!(failed["error"]["code"], NOT_DONE);
    let message = failed["error"]["message"].as_str().unwrap();
    assert!(message.starts_with("loop is dormant"), "{message}");
    assert!(!message.contains('\u{1b}'), "{message:?}");
}

// Outside a view nothing is sent far.
#[test]
fn a_primary_server_sends_nothing_far() {
    let mut app = app();
    app.view = None;
    let err = app
        .view_far_method(&Method::PaneClose(PaneCloseParams {
            pane_id: "w3:p1".into(),
            force: false,
        }))
        .expect_err("not a view");
    assert!(err.contains("not a view"), "{err}");
}
