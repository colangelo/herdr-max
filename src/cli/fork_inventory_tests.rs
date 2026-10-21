//! Characterization of the fork CLI at f3fb2099, before the shared-contract port.
//! The thread-local seam runs real dispatchers/parsers without a server or env mutation.

use std::cell::RefCell;

use serde_json::{json, Value};

use crate::api::schema::Request;

thread_local! {
    static REQUESTS: RefCell<Option<Vec<Value>>> = const { RefCell::new(None) };
    static RESULT: RefCell<Value> = RefCell::new(json!({"ok": true}));
}

pub(super) fn capture_request(request: &Request) -> Option<Value> {
    REQUESTS.with_borrow_mut(|requests| {
        let requests = requests.as_mut()?;
        requests.push(serde_json::to_value(&request.method).expect("serialize captured method"));
        Some(json!({"id": request.id, "result": RESULT.with_borrow(Clone::clone)}))
    })
}

pub(super) struct Capture;

impl Capture {
    pub(super) fn begin() -> Self {
        REQUESTS.with_borrow_mut(|requests| {
            assert!(requests.is_none(), "nested CLI request capture");
            *requests = Some(Vec::new());
        });
        Self
    }

    pub(super) fn respond_with(&self, result: Value) {
        RESULT.with_borrow_mut(|current| *current = result);
    }

    fn take(&self) -> Vec<Value> {
        REQUESTS
            .with_borrow_mut(|requests| std::mem::take(requests.as_mut().expect("capture active")))
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        REQUESTS.with_borrow_mut(|requests| *requests = None);
        RESULT.with_borrow_mut(|result| *result = json!({"ok": true}));
    }
}

#[test]
fn fork_cli_requests_stay_at_the_pre_port_baseline() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fork_inventory.json")).expect("valid inventory");
    assert!(
        cases.len() >= 60,
        "inventory must retain all baseline cases"
    );
    let capture = Capture::begin();
    for case in cases {
        let argv: Vec<String> = case["argv"]
            .as_array()
            .expect("argv array")
            .iter()
            .map(|arg| arg.as_str().expect("argv string").to_owned())
            .collect();
        let result = super::maybe_run(&argv).expect("CLI handler must not need a socket");
        assert!(
            matches!(result, super::CommandOutcome::Handled(0)),
            "{argv:?}"
        );
        let requests = capture.take();
        assert_eq!(json!(requests), case["requests"], "{argv:?}");
    }
}

#[test]
fn fork_methods_are_in_the_operations_table() {
    use crate::api::schema::Method;
    for name in [
        "agent.message",
        "agent.pin",
        "agent.unpin",
        "client.list",
        "layout.apply_preset",
        "layout.balance",
        "layout.set_preset",
        "notification.clear",
        "notification.list",
        "notification.mark_seen",
        "pane.clear_scrollback",
        "pane.report_hint",
        "pane.respawn",
        "pane.respawn_agent",
        "pane.scroll_application",
        "pane.sync",
        "pane.sync_pair",
        "plugin.popup.close",
        "tab.sync",
        "task.add",
        "task.bind",
        "task.conflicts",
        "task.done",
        "task.drop",
        "task.list",
        "task.reopen",
        "task.resolve_conflict",
        "task.show",
        "task.update",
        "todo.add",
        "todo.clear",
        "todo.list",
        "todo.remove",
        "todo.update",
        "workspace.pin",
        "workspace.unpin",
    ] {
        assert!(
            Method::OPERATIONS
                .iter()
                .any(|operation| operation.name == name),
            "{name}"
        );
    }
}
