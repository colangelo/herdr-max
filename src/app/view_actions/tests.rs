use std::sync::Arc;

use serde_json::{json, Value};

use super::*;
use crate::api::schema::{PaneTarget, SplitDirection};
use crate::core_machine::fleet::view::tests::{added, far_ok, input_of};
use crate::core_machine::fleet::view::ViewRoute;
use crate::pane::FakeCore;

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

/// What this build's API answers a request for `method`, which it does
/// not know: the reply an older far herdr gives a method it lacks.
fn real_reply_to_an_unknown(method: &str) -> Vec<u8> {
    let line = format!(
        "{}",
        json!({"id": "view:1", "method": method, "params": {}})
    );
    let err = serde_json::from_str::<Request>(&line).expect_err("an unknown method");
    let mut reply =
        serde_json::to_vec(&crate::api::unparsed_request_response(&line, &err)).expect("json");
    reply.push(b'\n');
    reply
}

// Review M1, D7: a far herdr that does not know the method (an older
// build) answers `invalid_request` naming it (serde's "unknown variant"),
// or `unknown_method` for a retired one. Either is a missing verb: the
// action is refused with the update notice, never with the far list of
// methods.
#[test]
fn a_missing_far_verb_is_the_actions_refusal() {
    let machine = MachineName::parse("loop").unwrap();
    let real = real_reply_to_an_unknown("pane.rename_v9");
    let as_sent: Value = serde_json::from_slice(&real).unwrap();
    assert_eq!(
        as_sent["error"]["code"], "invalid_request",
        "the real shape"
    );
    let answer = reply(&client_reply(
        &machine,
        "pane.rename_v9",
        "endpoint:1".into(),
        Ok(real),
    ));
    assert_eq!(answer["id"], "endpoint:1");
    assert_eq!(answer["error"]["code"], "unknown_method");
    let message = answer["error"]["message"].as_str().unwrap();
    assert_eq!(
        message,
        "loop's herdr does not take pane.rename_v9; update Herdr there to do this from a view"
    );

    let retired = real_reply_to_an_unknown("pane.graphics.clear");
    let answer = reply(&client_reply(
        &machine,
        "pane.graphics.clear",
        "endpoint:2".into(),
        Ok(retired),
    ));
    assert_eq!(answer["error"]["code"], "unknown_method");

    // Another method's invalid request is no missing verb, and its list of
    // methods is not shown.
    let other = real_reply_to_an_unknown("tab.create_v9");
    let answer = reply(&client_reply(
        &machine,
        "pane.rename",
        "endpoint:3".into(),
        Ok(other),
    ));
    assert_eq!(answer["error"]["code"], NOT_DONE);
    let message = answer["error"]["message"].as_str().unwrap();
    assert!(
        !message.contains("expected one of") && !message.contains("ping"),
        "{message}"
    );
}

// The far answers become the client's: a result it reads, a far error with
// its text escaped, and a send that failed with why. Only a code whose
// client handling this build knows is passed on (review L2); a reply that
// is neither a result nor an error, or an error that is no object, may
// have run (review L1).
#[test]
fn far_answers_become_the_clients() {
    let machine = MachineName::parse("loop").unwrap();
    let answer = |text: &[u8]| {
        reply(&client_reply(
            &machine,
            "pane.close",
            "endpoint:2".into(),
            Ok(text.to_vec()),
        ))
    };
    assert_eq!(
        answer(br#"{"id":"view:2","result":{"type":"ok"}}"#),
        json!({"id": "endpoint:2", "result": {"type": "ok"}})
    );
    assert_eq!(
        answer(br#"{"id":"view:3","result":{"type":"a_result_from_a_newer_herdr"}}"#)["result"],
        json!({"type": "ok"})
    );

    let confirm = answer(br#"{"id":"view:4","error":{"code":"confirmation_required","message":"this pane still has 2 open todos\u001b]52;c;eA==\u0007"}}"#);
    assert_eq!(confirm["error"]["code"], "confirmation_required");
    let message = confirm["error"]["message"].as_str().unwrap();
    assert!(
        message.starts_with("loop: this pane still has 2 open todos"),
        "{message}"
    );
    assert!(
        !message.contains('\u{1b}') && !message.contains('\u{7}'),
        "{message:?}"
    );

    for code in [
        "stale_target",
        "stale_content",
        "endpoint_timeout",
        "endpoint_cancelled",
        "server_unavailable",
        "pane_not_found",
        "Odd Code\u{1b}",
    ] {
        let far = json!({"id": "view:5", "error": {"code": code, "message": "no"}});
        let got = answer(far.to_string().as_bytes());
        assert_eq!(got["error"]["code"], NOT_DONE, "{code}");
        let message = got["error"]["message"].as_str().unwrap();
        assert!(
            message.starts_with("loop's herdr refused pane.close"),
            "{message}"
        );
        assert!(!message.contains('\u{1b}'), "{message:?}");
    }

    for odd in [
        br#"{"id":"view:6"}"#.as_slice(),
        br#"{"id":"view:6","result":null}"#,
        br#"{"id":"view:6","error":"no"}"#,
        br#"{"id":"view:6","error":null}"#,
        br#"[1]"#,
    ] {
        let got = answer(odd);
        assert_eq!(
            got["error"]["code"],
            NOT_DONE,
            "{}",
            String::from_utf8_lossy(odd)
        );
        assert!(
            got["error"]["message"]
                .as_str()
                .unwrap()
                .contains("it may have run there"),
            "{got}"
        );
    }

    let failed = reply(&client_reply(
        &machine,
        "tab.create",
        "endpoint:7".into(),
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

// Review M5, the seam: the view's close of a pane goes through its route
// (a fake core here, herdr's state in a scratch folder), exactly to the
// far pane named; the far reply is the client's answer.
#[test]
fn a_views_close_reaches_the_far_pane_it_aimed_at() {
    let core = FakeCore::start_with(|config| config.exec_handler = Some(Arc::new(far_ok)));
    core.set_machine("loop", "connected");
    let (root, dir) = added("view-forward");
    let mut app = app();
    app.view
        .as_mut()
        .unwrap()
        .set_route(ViewRoute::at(&core.socket, root.clone()));
    let (respond_to, rx) = std::sync::mpsc::channel();
    app.forward_view_action(
        Request {
            id: "endpoint:1".into(),
            method: Method::PaneClose(PaneCloseParams {
                pane_id: "w3:p2".into(),
                force: false,
            }),
        },
        respond_to,
    );
    let answer = reply(
        &rx.recv_timeout(std::time::Duration::from_secs(20))
            .expect("an answer"),
    );
    assert_eq!(
        answer,
        json!({"id": "endpoint:1", "result": {"type": "ok"}})
    );
    let execs: Vec<Value> = core
        .requests()
        .into_iter()
        .filter(|request| request["method"] == "machine.exec")
        .collect();
    assert_eq!(execs.len(), 1);
    let sent = input_of(&execs[0]["params"]);
    assert_eq!(sent["method"], "pane.close");
    assert_eq!(sent["params"]["pane_id"], "w3:p2");
    assert_ne!(sent["params"]["force"], true);
    assert!(dir.leases().is_empty());
    let _ = std::fs::remove_dir_all(&root);
}
