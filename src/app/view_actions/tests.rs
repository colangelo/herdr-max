use std::sync::Arc;

use serde_json::{json, Value};

use super::*;
use crate::api::schema::{PaneTarget, SplitDirection};
use crate::core_machine::fleet::view::tests::{added, far_ok, input_of};
use crate::pane::FakeCore;

fn block(id: &str, terminal: &str) -> Value {
    json!({"id": id, "type": "block", "content": {"terminal": terminal}, "size": {"weight": 0.5}})
}

/// The far herdr's space w3 with two tabs: w3:t1 split into w3:p1 and
/// w3:p2, w3:t2 with w3:p3.
fn far_doc() -> Value {
    json!({"spaces": [{"id": "w3", "name": "api", "folder": "/far/api", "herdr_max": 1, "tabs": [
        {"id": "w3:t1", "root": {"id": "w3:t1", "type": "row", "children": [block("w3:p1", "t1"), block("w3:p2", "t2")]}},
        {"id": "w3:t2", "name": "logs", "root": {"id": "w3:t2", "type": "row", "children": [block("w3:p3", "t3")]}}
    ]}]})
}

fn project(app: &mut App, doc: &Value) {
    app.apply_view_layout(&crate::view::doc::parse(doc).expect("a good document"));
}

/// Core's record of the far program at `terminal`, as an attach saw it.
fn record(terminal: &str) -> Value {
    json!({"address": format!("loop/{terminal}"), "state": "live", "pid": 100 + terminal.len(),
           "labels": {"herdr.launch": format!("launch-{terminal}")}})
}

/// The pane at far `terminal` attached, to the program `record` shows.
fn attached(app: &mut App, terminal: &str) {
    app.view.as_mut().unwrap().bind_instance_for_test(
        &format!("loop/{terminal}"),
        FarInstance::of(&record(terminal)),
    );
}

/// A far core that shows each terminal's program as the view attached it.
fn as_attached() -> crate::core_machine::fleet::view::TerminalReader {
    Arc::new(|address| {
        let terminal = address.rsplit('/').next().unwrap_or_default();
        Ok(record(terminal))
    })
}

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
    project(&mut app, &far_doc());
    for terminal in ["t1", "t2", "t3"] {
        attached(&mut app, terminal);
    }
    app
}

/// The client's window shows the far tab w3:t1.
const FIRST_TAB: ClientTab = Some((0, 0));

fn far(app: &App, method: Method) -> Value {
    far_in(app, method, FIRST_TAB)
}

fn far_in(app: &App, method: Method, client_tab: ClientTab) -> Value {
    let action = app
        .view_far_action(&method, client_tab)
        .expect("a far request");
    let line = far_request_line(action.method).expect("a line");
    assert_eq!(line.last(), Some(&b'\n'), "one line");
    assert_eq!(
        line.iter().filter(|byte| **byte == b'\n').count(),
        1,
        "one line"
    );
    serde_json::from_slice(&line).expect("json")
}

fn close(pane_id: &str, force: bool) -> Method {
    Method::PaneClose(PaneCloseParams {
        pane_id: pane_id.into(),
        force,
    })
}

fn split(target: Option<&str>) -> Method {
    Method::PaneSplit(PaneSplitParams {
        workspace_id: Some("w3".into()),
        target_pane_id: target.map(str::to_string),
        direction: SplitDirection::Right,
        ratio: None,
        cwd: None,
        focus: true,
        right_click: PaneRightClickTarget::Herdr,
        env: HashMap::new(),
        on: None,
    })
}

fn pane_rename(pane_id: &str) -> Method {
    Method::PaneRename(PaneRenameParams {
        pane_id: pane_id.into(),
        label: Some("server".into()),
    })
}

fn view_pane(app: &App, far: &str) -> PaneId {
    app.parse_pane_id(far).expect("a pane").1
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
            cwd: None,
            focus: true,
            label: Some("build".into()),
            env: HashMap::new(),
            on: None,
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
            cwd: None,
            focus: true,
            right_click: PaneRightClickTarget::Pane,
            env: HashMap::new(),
            on: None,
        }),
    );
    assert_eq!(split["method"], "pane.split");
    assert_eq!(
        split["params"],
        json!({"workspace_id": "w3", "target_pane_id": "w3:p2", "direction": "down",
               "ratio": 0.25, "focus": false, "right_click": "herdr"})
    );

    let close = far(&app, close("w3:p3", true));
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

    let pane_rename = far(&app, pane_rename("w3:p1"));
    assert_eq!(pane_rename["method"], "pane.rename");
    assert_eq!(
        pane_rename["params"],
        json!({"pane_id": "w3:p1", "label": "server"})
    );
}

// Review L3: a request that names a folder, an environment or a machine
// is refused, never sent with them dropped.
#[test]
fn a_folder_an_environment_or_a_machine_is_refused_not_dropped() {
    let app = app();
    let tab = |cwd: Option<&str>, env: &[(&str, &str)], on: Option<&str>| {
        Method::TabCreate(TabCreateParams {
            workspace_id: Some("w3".into()),
            cwd: cwd.map(str::to_string),
            focus: false,
            label: None,
            env: env
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                .collect(),
            on: on.map(str::to_string),
        })
    };
    let mut split_on = split(Some("w3:p1"));
    if let Method::PaneSplit(params) = &mut split_on {
        params.on = Some("elsewhere".into());
    }
    let mut split_env = split(Some("w3:p1"));
    if let Method::PaneSplit(params) = &mut split_env {
        params.env = HashMap::from([("SECRET".into(), "x".into())]);
    }
    for method in [
        tab(Some("/Users/here"), &[], None),
        tab(None, &[("SECRET", "x")], None),
        tab(None, &[], Some("elsewhere")),
        split_on,
        split_env,
    ] {
        let err = app
            .view_far_action(&method, FIRST_TAB)
            .expect_err("refused");
        assert!(
            err.contains("a view sends no folder, environment or machine"),
            "{err}"
        );
    }
}

// Review L4: a split with no target splits the focused pane of the
// sending client's own tab, and a new tab with no workspace goes to the
// client's workspace, not wherever this server's focus is; with no tab of
// the client's, it is refused.
#[test]
fn ids_left_out_resolve_against_the_clients_own_tab() {
    let mut app = app();
    // This server's focus is on w3:t1 (another client moved it there).
    app.state.workspaces[0].active_tab = 0;
    let request = far_in(&app, split(None), Some((0, 1)));
    assert_eq!(request["params"]["target_pane_id"], "w3:p3");
    let request = far_in(&app, split(None), FIRST_TAB);
    assert_eq!(request["params"]["target_pane_id"], "w3:p1");
    let tab = Method::TabCreate(TabCreateParams {
        workspace_id: None,
        cwd: None,
        focus: false,
        label: None,
        env: HashMap::new(),
        on: None,
    });
    assert_eq!(
        far_in(&app, tab.clone(), Some((0, 1)))["params"]["workspace_id"],
        "w3"
    );
    for method in [split(None), tab] {
        let err = app.view_far_action(&method, None).expect_err("no tab");
        assert!(err.contains("this window shows no tab of loop"), "{err}");
    }
}

// Only what the far document showed can be named: an id the view does not
// show is refused here, before anything is sent, and so is a method a view
// does not send there.
#[test]
fn a_target_the_view_does_not_show_is_refused_unsent() {
    let app = app();
    for method in [
        close("w3:p9", false),
        Method::TabRename(TabRenameParams {
            tab_id: "w3:t9".into(),
            label: "x".into(),
        }),
        Method::WorkspaceRename(WorkspaceRenameParams {
            workspace_id: "w9".into(),
            label: "x".into(),
        }),
    ] {
        let err = app
            .view_far_action(&method, FIRST_TAB)
            .expect_err("refused");
        assert!(err.contains("is not one loop's herdr shows"), "{err}");
    }
    let err = app
        .view_far_action(
            &Method::PaneFocus(PaneTarget {
                pane_id: "w3:p1".into(),
            }),
            FIRST_TAB,
        )
        .expect_err("not sent far");
    assert!(err.contains("not an action a view sends"), "{err}");
}

// Review M2: a close, split or rename of a pane whose input the view
// holds (the far layout put another terminal under its id) is refused.
#[test]
fn an_action_on_a_held_pane_is_refused() {
    let mut app = app();
    let doc = json!({"spaces": [{"id": "w3", "name": "api", "folder": "/far/api", "herdr_max": 1, "tabs": [
        {"id": "w3:t1", "root": {"id": "w3:t1", "type": "row", "children": [block("w3:p1", "t1"), block("w3:p2", "t9")]}},
        {"id": "w3:t2", "name": "logs", "root": {"id": "w3:t2", "type": "row", "children": [block("w3:p3", "t3")]}}
    ]}]});
    project(&mut app, &doc);
    assert!(app.view_input_held(view_pane(&app, "w3:p2")));
    for method in [
        close("w3:p2", false),
        split(Some("w3:p2")),
        pane_rename("w3:p2"),
    ] {
        let err = app.view_far_action(&method, FIRST_TAB).expect_err("held");
        assert!(
            err.contains("shows another terminal of loop than before"),
            "{err}"
        );
    }
    // A split with no target of a held focused pane, too.
    let pane = view_pane(&app, "w3:p2");
    app.state.workspaces[0].tabs[0].layout.focus_pane(pane);
    assert!(app.view_far_action(&split(None), FIRST_TAB).is_err());
    // A person's click or focus releases it: then it is that terminal.
    assert!(app.release_view_hold(pane));
    attached(&mut app, "t9");
    let action = app
        .view_far_action(&close("w3:p2", false), FIRST_TAB)
        .expect("released");
    assert_eq!(action.aimed.expect("aimed").terminal, "t9");
}

// Review M2: a close the far herdr asked to confirm goes, forced, only to
// the pane and terminal it was asked for; a confirmation opened before
// the far layout changed under it is refused, even once the pane's hold is
// released, and so is a forced close nobody asked for.
#[test]
fn a_confirmed_close_reaches_only_the_pane_it_was_asked_for() {
    let mut app = app();
    let asked = app
        .view_far_action(&close("w3:p2", false), FIRST_TAB)
        .unwrap();
    app.confirmed_close(asked).expect("asked");
    let forced = app
        .view_far_action(&close("w3:p2", true), FIRST_TAB)
        .unwrap();
    app.confirmed_close(forced).expect("the same pane: sent");

    let asked = app
        .view_far_action(&close("w3:p2", false), FIRST_TAB)
        .unwrap();
    app.confirmed_close(asked).expect("asked");
    let doc = json!({"spaces": [{"id": "w3", "name": "api", "folder": "/far/api", "herdr_max": 1, "tabs": [
        {"id": "w3:t1", "root": {"id": "w3:t1", "type": "row", "children": [block("w3:p1", "t1"), block("w3:p2", "t9")]}},
        {"id": "w3:t2", "name": "logs", "root": {"id": "w3:t2", "type": "row", "children": [block("w3:p3", "t3")]}}
    ]}]});
    project(&mut app, &doc);
    assert!(
        app.view_far_action(&close("w3:p2", true), FIRST_TAB)
            .is_err(),
        "held"
    );
    app.release_view_hold(view_pane(&app, "w3:p2"));
    attached(&mut app, "t9");
    let forced = app
        .view_far_action(&close("w3:p2", true), FIRST_TAB)
        .unwrap();
    let err = app.confirmed_close(forced).expect_err("another terminal");
    assert!(err.contains("close it again"), "{err}");

    // E.3 review, item 1: the same far pane and address, another program
    // there (a far core reset; the pane attached to it afresh).
    let asked = app
        .view_far_action(&close("w3:p2", false), FIRST_TAB)
        .unwrap();
    app.confirmed_close(asked).expect("asked");
    app.view.as_mut().unwrap().bind_instance_for_test(
        "loop/t9",
        FarInstance::of(&json!({"pid": 999, "labels": {"herdr.launch": "after-reset"}})),
    );
    let forced = app
        .view_far_action(&close("w3:p2", true), FIRST_TAB)
        .unwrap();
    let err = app.confirmed_close(forced).expect_err("another program");
    assert!(err.contains("another pane or program"), "{err}");

    let forced = app
        .view_far_action(&close("w3:p1", true), FIRST_TAB)
        .unwrap();
    assert!(app.confirmed_close(forced).is_err(), "never asked");
}

/// The public id the view gives the `nth` pane (from 0) of workspace w3
/// that the far herdr numbers none for.
fn view_numbered(nth: usize) -> String {
    crate::workspace::public_pane_id_for_number(
        "w3",
        crate::app::view_layout::VIEW_NUMBER_BASE + nth,
    )
}

// Review M4: a far pane the far herdr has no number for gets one of the
// view's, past any far number; it is not named to the far herdr. E.3
// review, item 5: the number stays with its pane, so a far pane numbered
// later moves no id and holds no pane. Item 4: a number the view gave is
// never given again, and a far pane that takes one later, even after its
// pane went projections ago, is held: a client may still hold that id.
#[test]
fn a_pane_number_the_view_gave_never_reaches_another_far_pane() {
    let mut app = app();
    let doc = |children: Vec<Value>| {
        json!({"spaces": [{"id": "w3", "name": "api", "folder": "/far/api", "herdr_max": 1, "tabs": [
            {"id": "w3:t1", "root": {"id": "w3:t1", "type": "row", "children": children}}
        ]}]})
    };
    project(
        &mut app,
        &doc(vec![block("w3:p1", "t1"), block("w3:x7", "t7")]),
    );
    attached(&mut app, "t7");
    let unnumbered = view_pane(&app, &view_numbered(0));
    assert_eq!(
        app.view.as_ref().unwrap().far_pane(unnumbered),
        Some("w3:x7")
    );
    let err = app
        .view_far_action(&close(&view_numbered(0), false), FIRST_TAB)
        .expect_err("no far number");
    assert!(err.contains("has no number in loop's herdr"), "{err}");

    // The far herdr numbers a new pane 2: no id moves, nothing is held.
    project(
        &mut app,
        &doc(vec![
            block("w3:p1", "t1"),
            block("w3:x7", "t7"),
            block("w3:p2", "t8"),
        ]),
    );
    assert_eq!(view_pane(&app, &view_numbered(0)), unnumbered, "kept");
    for far in ["w3:p1", "w3:p2"] {
        assert!(!app.view_input_held(view_pane(&app, far)), "{far}");
    }
    assert!(!app.view_input_held(unnumbered));

    // The unnumbered pane goes; two projections later a far pane has the
    // number the view gave it: held, though the view gave it in no
    // projection the one before saw.
    project(
        &mut app,
        &doc(vec![block("w3:p1", "t1"), block("w3:p2", "t8")]),
    );
    project(
        &mut app,
        &doc(vec![block("w3:p1", "t1"), block("w3:p2", "t8")]),
    );
    let taken = view_numbered(0);
    project(
        &mut app,
        &doc(vec![
            block("w3:p1", "t1"),
            block("w3:p2", "t8"),
            block(&taken, "t9"),
            block("w3:x8", "t10"),
        ]),
    );
    attached(&mut app, "t9");
    let now_taken = view_pane(&app, &taken);
    assert_ne!(now_taken, unnumbered);
    assert!(app.view_input_held(now_taken), "a number the view gave");
    let err = app
        .view_far_action(&close(&taken, false), FIRST_TAB)
        .expect_err("held");
    assert!(err.contains("than before"), "{err}");
    // A new unnumbered pane gets a number never given before.
    let fresh = view_pane(&app, &view_numbered(1));
    assert_eq!(app.view.as_ref().unwrap().far_pane(fresh), Some("w3:x8"));
    assert!(!app.view_input_held(fresh));
}

// Review M2, just before the request: the far document still shows the
// aimed-at terminal under the far pane, or nothing is sent.
#[test]
fn the_far_pane_is_checked_again_before_the_request() {
    let app = app();
    let aimed = app
        .view_far_action(&close("w3:p2", false), FIRST_TAB)
        .unwrap()
        .aimed
        .expect("aimed");
    let machine = MachineName::parse("loop").unwrap();
    let route_with = |doc: Value| {
        ViewRoute::at(
            std::path::Path::new("/nonexistent"),
            std::path::PathBuf::from("/nonexistent"),
            Arc::new(move |_| Ok(doc.clone())),
        )
        .with_terminal(as_attached())
    };
    assert_eq!(
        still_aimed(&route_with(far_doc()), &machine, Some(&aimed), true),
        Ok(())
    );
    let swapped = json!({"spaces": [{"id": "w3", "name": "api", "folder": "/far/api", "herdr_max": 1, "tabs": [
        {"id": "w3:t1", "root": {"id": "w3:t1", "type": "row", "children": [block("w3:p1", "t1"), block("w3:p2", "t5")]}}
    ]}]});
    let err =
        still_aimed(&route_with(swapped), &machine, Some(&aimed), false).expect_err("swapped");
    assert!(err.contains("shows another terminal now"), "{err}");
    let gone = json!({"spaces": [{"id": "w3", "name": "api", "folder": "/far/api", "herdr_max": 1, "tabs": [
        {"id": "w3:t1", "root": {"id": "w3:t1", "type": "row", "children": [block("w3:p1", "t1")]}}
    ]}]});
    let err = still_aimed(&route_with(gone), &machine, Some(&aimed), false).expect_err("gone");
    assert!(err.contains("no longer in loop's layout"), "{err}");
    assert_eq!(
        still_aimed(&route_with(json!(null)), &machine, None, false),
        Ok(())
    );
}

// E.3 review, item 1: a far core reset gives the address to another
// program, and the document cannot tell (the same pane id, the same `t2`).
// Core's record of the terminal is read just before the request and must
// be the program the pane attached to, and live; else nothing is sent.
#[test]
fn another_program_at_the_aimed_address_is_refused() {
    let app = app();
    let aimed = app
        .view_far_action(&pane_rename("w3:p2"), FIRST_TAB)
        .unwrap()
        .aimed
        .expect("aimed");
    let machine = MachineName::parse("loop").unwrap();
    let route_with = |terminal: Value| {
        ViewRoute::at(
            std::path::Path::new("/nonexistent"),
            std::path::PathBuf::from("/nonexistent"),
            Arc::new(|_| Ok(far_doc())),
        )
        .with_terminal(Arc::new(move |_| Ok(terminal.clone())))
    };
    assert_eq!(
        still_aimed(&route_with(record("t2")), &machine, Some(&aimed), false),
        Ok(())
    );
    let mut reset = record("t2");
    reset["pid"] = json!(4242);
    reset["labels"]["herdr.launch"] = json!("launched-after-the-reset");
    let mut ended = record("t2");
    ended["state"] = json!("exited");
    for other in [reset, ended] {
        let err = still_aimed(&route_with(other), &machine, Some(&aimed), false)
            .expect_err("another program");
        assert!(err.contains("runs another program now"), "{err}");
    }
}

// E.3 review, item 1: a pane whose terminal is not attached yet (or is
// attaching again after a rebind) has no program the view could vouch for:
// no close, split or rename goes for it.
#[test]
fn a_pane_not_attached_takes_no_action() {
    let mut app = app();
    let doc = json!({"spaces": [{"id": "w3", "name": "api", "folder": "/far/api", "herdr_max": 1, "tabs": [
        {"id": "w3:t1", "root": {"id": "w3:t1", "type": "row", "children": [block("w3:p1", "t1"), block("w3:p2", "t2"), block("w3:p4", "t4")]}},
        {"id": "w3:t2", "name": "logs", "root": {"id": "w3:t2", "type": "row", "children": [block("w3:p3", "t3")]}}
    ]}]});
    project(&mut app, &doc);
    for method in [
        close("w3:p4", false),
        split(Some("w3:p4")),
        pane_rename("w3:p4"),
    ] {
        let err = app
            .view_far_action(&method, FIRST_TAB)
            .expect_err("not attached");
        assert!(err.contains("is not attached to its program"), "{err}");
    }
    attached(&mut app, "t4");
    assert!(app
        .view_far_action(&close("w3:p4", false), FIRST_TAB)
        .is_ok());
}

// Security review of E.3, M2 P2: closing the last pane of a far workspace
// closes the workspace there, and a worktree group's parent closes its
// whole group. A view refuses it, from its own layout and again from the
// document read just before the request.
#[test]
fn the_last_pane_of_a_far_workspace_is_not_closed_from_a_view() {
    let mut app = app();
    let doc = json!({"spaces": [
        {"id": "w3", "name": "api", "folder": "/far/api", "herdr_max": 1, "tabs": [
            {"id": "w3:t1", "root": {"id": "w3:t1", "type": "row", "children": [block("w3:p1", "t1"), block("w3:p2", "t2")]}}
        ]},
        {"id": "w4", "name": "group", "folder": "/far/group", "herdr_max": 1, "tabs": [
            {"id": "w4:t1", "root": {"id": "w4:t1", "type": "row", "children": [block("w4:p1", "t6")]}}
        ]}
    ]});
    project(&mut app, &doc);
    attached(&mut app, "t6");
    let err = app
        .view_far_action(&close("w4:p1", false), Some((1, 0)))
        .expect_err("the last pane");
    assert_eq!(
        err,
        "closing the last pane of group on loop is not done from a view; close it there"
    );
    // Not the last one: sent; a rename of the last one is no close.
    assert!(app
        .view_far_action(&close("w3:p2", false), FIRST_TAB)
        .is_ok());
    assert!(app
        .view_far_action(&pane_rename("w4:p1"), Some((1, 0)))
        .is_ok());

    // The far w3:p1 was closed meanwhile: the document read before the
    // request shows w3:p2 as the last pane of api.
    let aimed = app
        .view_far_action(&close("w3:p2", false), FIRST_TAB)
        .unwrap()
        .aimed
        .expect("aimed");
    let alone = json!({"spaces": [{"id": "w3", "name": "api", "folder": "/far/api", "herdr_max": 1, "tabs": [
        {"id": "w3:t1", "root": {"id": "w3:t1", "type": "row", "children": [block("w3:p2", "t2")]}}
    ]}]});
    let route = ViewRoute::at(
        std::path::Path::new("/nonexistent"),
        std::path::PathBuf::from("/nonexistent"),
        Arc::new(move |_| Ok(alone.clone())),
    )
    .with_terminal(as_attached());
    let machine = MachineName::parse("loop").unwrap();
    let err = still_aimed(&route, &machine, Some(&aimed), true).expect_err("now the last");
    assert!(
        err.contains("closing the last pane of api on loop"),
        "{err}"
    );
    assert_eq!(
        still_aimed(&route, &machine, Some(&aimed), false),
        Ok(()),
        "a rename of it still goes"
    );
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
        .view_far_action(&close("w3:p1", false), FIRST_TAB)
        .expect_err("not a view");
    assert!(err.contains("not a view"), "{err}");
}

fn answer_of(rx: &std::sync::mpsc::Receiver<String>) -> Value {
    reply(
        &rx.recv_timeout(std::time::Duration::from_secs(20))
            .expect("an answer"),
    )
}

// Review M5, the seam: the view's close of a pane goes through its route
// (a fake core here, herdr's state in a scratch folder), exactly to the
// far pane aimed at, after the far document is read again; the far reply
// is the client's answer. A held pane sends nothing.
#[test]
fn a_views_close_reaches_the_far_pane_it_aimed_at() {
    let core = FakeCore::start_with(|config| config.exec_handler = Some(Arc::new(far_ok)));
    core.set_machine("loop", "connected");
    let (root, dir) = added("view-forward");
    let mut app = app();
    let layout_reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let reads = Arc::clone(&layout_reads);
    app.view.as_mut().unwrap().set_route(
        ViewRoute::at(
            &core.socket,
            root.clone(),
            Arc::new(move |_| {
                reads.fetch_add(1, Ordering::SeqCst);
                Ok(far_doc())
            }),
        )
        .with_terminal(as_attached()),
    );
    let (respond_to, rx) = std::sync::mpsc::channel();
    app.forward_view_action(
        Request {
            id: "endpoint:1".into(),
            method: close("w3:p2", false),
        },
        respond_to,
        FIRST_TAB,
    );
    assert_eq!(
        answer_of(&rx),
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
    assert_eq!(
        layout_reads.load(Ordering::SeqCst),
        1,
        "checked once before"
    );
    assert!(dir.leases().is_empty());

    // Held: refused on the loop, nothing sent.
    let pane = view_pane(&app, "w3:p3");
    app.hold_view_input_for_test(pane);
    let (respond_to, rx) = std::sync::mpsc::channel();
    app.forward_view_action(
        Request {
            id: "endpoint:2".into(),
            method: close("w3:p3", false),
        },
        respond_to,
        FIRST_TAB,
    );
    let refused = answer_of(&rx);
    assert_eq!(refused["error"]["code"], NOT_DONE);
    assert_eq!(
        core.requests()
            .iter()
            .filter(|request| request["method"] == "machine.exec")
            .count(),
        1,
        "nothing more sent"
    );
    let _ = std::fs::remove_dir_all(&root);
}

fn execs_of(core: &FakeCore) -> usize {
    core.requests()
        .iter()
        .filter(|request| request["method"] == "machine.exec")
        .count()
}

fn forward(app: &mut App, id: &str, method: Method) -> Value {
    let (respond_to, rx) = std::sync::mpsc::channel();
    app.forward_view_action(
        Request {
            id: id.into(),
            method,
        },
        respond_to,
        FIRST_TAB,
    );
    answer_of(&rx)
}

// E.3 review, item 6, the wiring: the view's own action runs the aim check
// through its route, so a far layout that changed since the view's last
// projection refuses it, with nothing sent.
#[test]
fn a_view_action_is_refused_when_the_far_layout_changed_under_it() {
    let core = FakeCore::start_with(|config| config.exec_handler = Some(Arc::new(far_ok)));
    core.set_machine("loop", "connected");
    let (root, dir) = added("view-forward-changed");
    let mut app = app();
    let swapped = json!({"spaces": [{"id": "w3", "name": "api", "folder": "/far/api", "herdr_max": 1, "tabs": [
        {"id": "w3:t1", "root": {"id": "w3:t1", "type": "row", "children": [block("w3:p1", "t1"), block("w3:p2", "t5")]}},
        {"id": "w3:t2", "name": "logs", "root": {"id": "w3:t2", "type": "row", "children": [block("w3:p3", "t3")]}}
    ]}]});
    app.view.as_mut().unwrap().set_route(
        ViewRoute::at(
            &core.socket,
            root.clone(),
            Arc::new(move |_| Ok(swapped.clone())),
        )
        .with_terminal(as_attached()),
    );
    let refused = forward(&mut app, "endpoint:1", pane_rename("w3:p2"));
    assert_eq!(refused["error"]["code"], NOT_DONE);
    let message = refused["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("shows another terminal now") && message.contains("nothing was sent"),
        "{message}"
    );
    assert_eq!(execs_of(&core), 0);
    assert!(dir.leases().is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

// E.3 review, item 9: a forced close refused before its request (here the
// machine is dormant) keeps the person's confirmation, so the next try
// still goes; once the far herdr answered it, the confirmation is spent.
#[test]
fn a_refused_forced_close_keeps_the_persons_confirmation() {
    let core = FakeCore::start_with(|config| config.exec_handler = Some(Arc::new(far_ok)));
    core.set_machine("loop", "dormant");
    let (root, _dir) = added("view-forward-confirm");
    let mut app = app();
    app.view.as_mut().unwrap().set_route(
        ViewRoute::at(&core.socket, root.clone(), Arc::new(|_| Ok(far_doc())))
            .with_terminal(as_attached()),
    );
    let asked = app
        .view_far_action(&close("w3:p2", false), FIRST_TAB)
        .unwrap();
    app.confirmed_close(asked).expect("asked");

    let refused = forward(&mut app, "endpoint:1", close("w3:p2", true));
    assert!(
        refused["error"]["message"]
            .as_str()
            .unwrap()
            .contains("loop is dormant"),
        "{refused}"
    );
    assert_eq!(execs_of(&core), 0);

    core.set_machine("loop", "connected");
    let done = forward(&mut app, "endpoint:2", close("w3:p2", true));
    assert_eq!(done, json!({"id": "endpoint:2", "result": {"type": "ok"}}));
    assert_eq!(execs_of(&core), 1, "the confirmed close went");
    let again = forward(&mut app, "endpoint:3", close("w3:p2", true));
    assert!(
        again["error"]["message"]
            .as_str()
            .unwrap()
            .contains("close it again"),
        "{again}"
    );
    assert_eq!(execs_of(&core), 1, "spent: nothing more sent");
    let _ = std::fs::remove_dir_all(&root);
}
