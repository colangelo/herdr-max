use std::sync::Mutex;

use base64::Engine as _;
use serde_json::{json, Value};

use super::*;
use crate::core_machine::fleet::FarHerdr;
use crate::core_machine::state::{tests::scratch, write_atomic, DialRecord, Lease, Target};
use crate::pane::core_machines as core;
use crate::pane::FakeCore;

pub(crate) fn machine() -> MachineName {
    MachineName::parse("loop").expect("name")
}

fn far() -> FarHerdr {
    FarHerdr {
        link_at: 100.5,
        link_pid: 31,
        probed_at: now_unix(),
        session: "default".into(),
        path: "/opt/homebrew/bin/herdr".into(),
        version: Some("0.9.3".into()),
        protocol: Some(30),
        endpoint_capabilities: Vec::new(),
        core_session: Some("default".into()),
        request_bridge: true,
    }
}

/// A machine herdr added, linked now, whose far herdr was probed on this
/// link (so the action is the only exec).
pub(crate) fn added(tag: &str) -> (PathBuf, MachineDir) {
    let root = scratch(tag);
    let dir = MachineDir::at(&root, &machine());
    dir.create().expect("create");
    dir.write_target(&Target {
        target: "ac@loop".into(),
        session: "default".into(),
        ssh: "/usr/bin/ssh".into(),
        ssh_config: None,
        ssh_auth_sock: None,
        via: None,
    })
    .expect("target");
    dir.write_dial_record(&DialRecord { at: 100.5, pid: 31 })
        .expect("dial");
    write_atomic(
        &dir.far_herdr_path(),
        &serde_json::to_vec(&far()).expect("json"),
    )
    .expect("probe");
    (root, dir)
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub(crate) fn ran(stdout: &[u8]) -> Value {
    json!({"stdout": b64(stdout), "stderr": "", "truncated": false, "timed_out": false,
           "ms": 3, "code": 0})
}

/// The request line a `machine.exec` carried as its input.
pub(crate) fn input_of(params: &Value) -> Value {
    let input = base64::engine::general_purpose::STANDARD
        .decode(params["input"].as_str().unwrap_or_default())
        .unwrap_or_default();
    serde_json::from_slice(&input).unwrap_or(Value::Null)
}

/// A far herdr that does every request: `ok`, with the request's id.
pub(crate) fn far_ok(params: &Value) -> Value {
    let id = input_of(params)["id"].clone();
    ran(format!("{}\n", json!({"id": id, "result": {"type": "ok"}})).as_bytes())
}

fn execs(core: &FakeCore) -> Vec<Value> {
    core.requests()
        .into_iter()
        .filter(|request| request["method"] == "machine.exec")
        .collect()
}

fn no_layout() -> LayoutReader {
    Arc::new(|_| Ok(Value::Null))
}

fn route(core: &FakeCore, root: &std::path::Path) -> ViewRoute {
    ViewRoute::at(&core.socket, root.to_path_buf(), no_layout())
}

fn soon() -> Instant {
    Instant::now() + VIEW_ACTION_BUDGET
}

fn fine() -> AimCheck {
    Box::new(|| Ok(()))
}

/// A commit with nothing to spend.
fn spent() -> Commit {
    Box::new(|| Ok(()))
}

/// A commit that counts its calls.
fn counted(calls: &Arc<std::sync::atomic::AtomicUsize>) -> Commit {
    let calls = Arc::clone(calls);
    Box::new(move || {
        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    })
}

fn tab_create(id: &str) -> Vec<u8> {
    format!(
        "{}\n",
        json!({"id": id, "method": "tab.create", "params": {"focus": false}})
    )
    .into_bytes()
}

// V20 (D7, D11): a workspace named `$(touch x); w`, renamed from a view.
// The argv is the fixed wrapper and far bridge, with no shell in it and no
// element built from the name; the name is one JSON string of the request
// the bridge reads as its input, so nothing there parses it as script or
// as an option. The request runs within the action's budget (review M3).
#[test]
fn v20_a_name_reaches_the_far_herdr_as_data_never_a_shell_line() {
    let name = "$(touch x); w";
    let request = json!({"id": "view:1", "method": "workspace.rename",
                         "params": {"workspace_id": "w3", "label": name}});
    let line = format!("{request}\n").into_bytes();
    let far_reply = b"{\"id\":\"view:1\",\"result\":{\"type\":\"ok\"}}\n";
    let core = FakeCore::start_with(|config| config.exec_reply = Some(ran(far_reply)));
    core.set_machine("loop", "connected");
    let (root, dir) = added("view-v20");

    let reply = send(
        &route(&core, &root),
        &machine(),
        &line,
        soon(),
        fine(),
        spent(),
    )
    .expect("sent");
    assert_eq!(reply, far_reply);

    let sent = execs(&core);
    assert_eq!(sent.len(), 1, "one exec, the action");
    let params = &sent[0]["params"];
    assert_eq!(params["on"], "loop");
    assert_eq!(params["timeout"], VIEW_REQUEST_SECS, "within the budget");
    let argv: Vec<String> = serde_json::from_value(params["argv"].clone()).expect("argv");
    let mut expected = core::machine_env_wrapper();
    expected.extend(
        [
            "/opt/homebrew/bin/herdr",
            "--session",
            "default",
            "remote-api-bridge",
            "--request",
        ]
        .map(String::from),
    );
    assert_eq!(argv, expected);
    assert!(
        !argv
            .iter()
            .any(|arg| arg == "sh" || arg.ends_with("/sh") || arg == "-c" || arg.contains("touch")),
        "no shell and no name in the argv: {argv:?}"
    );
    let input = base64::engine::general_purpose::STANDARD
        .decode(params["input"].as_str().expect("input"))
        .expect("base64");
    assert_eq!(input, line, "the request line, byte for byte");
    let sent_request: Value = serde_json::from_slice(&input).expect("json");
    assert_eq!(sent_request["params"]["label"], name, "one string");

    assert!(
        !core
            .methods()
            .iter()
            .any(|method| method == "machine.connect"),
        "an action never dials"
    );
    assert!(dir.leases().is_empty(), "the action's lease is gone");
    let _ = std::fs::remove_dir_all(&root);
}

// D7, D3: an action on a machine that is not connected is refused at
// once: no lease, no exec, no dial; a key in a pane is what wakes it.
#[test]
fn an_action_on_a_dormant_machine_refuses_and_dials_nothing() {
    let core = FakeCore::start();
    core.set_machine("loop", "dormant");
    let (root, dir) = added("view-dormant");
    let err = send(
        &route(&core, &root),
        &machine(),
        &tab_create("view:2"),
        soon(),
        fine(),
        spent(),
    )
    .expect_err("refused");
    assert!(err.contains("loop is dormant: press a key"), "{err}");
    assert!(
        err.contains("this action sent nothing and dialled nothing")
            && err.contains("keys in this window may still let Herdr Core's own retry connect it"),
        "{err}"
    );
    assert!(execs(&core).is_empty());
    assert!(!core
        .methods()
        .iter()
        .any(|method| method == "machine.connect"));
    assert!(dir.leases().is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

// A far error is a reply like any other (the far herdr ran and said no):
// passed on whole, for the view to make it the action's notice.
#[test]
fn a_far_refusal_comes_back_as_the_reply() {
    let far_reply =
        b"{\"id\":\"view:3\",\"error\":{\"code\":\"pane_not_found\",\"message\":\"pane w3:p9 not found\"}}\n";
    let core = FakeCore::start_with(|config| config.exec_reply = Some(ran(far_reply)));
    core.set_machine("loop", "connected");
    let (root, _dir) = added("view-far-error");
    let reply = send(
        &route(&core, &root),
        &machine(),
        &tab_create("view:3"),
        soon(),
        fine(),
        spent(),
    )
    .expect("a reply");
    assert_eq!(reply, far_reply);
    let _ = std::fs::remove_dir_all(&root);
}

/// The leases in force when the request ran, and the time then.
type Seen = Arc<Mutex<Option<(Vec<Lease>, f64)>>>;

// L5: what the action writes during the call. The far herdr's probe is
// stale, and the lease it wrote first is gone by the time the probe ends
// (a slow probe ate it); the request still runs under the view's own keep
// lease, written again before it: `cli-<pid>`, no dial, in force for about
// a minute. After the call no lease is left.
#[test]
fn the_request_runs_under_a_keep_lease_written_again_after_the_probe() {
    let (root, dir) = added("view-lease");
    std::fs::remove_file(dir.far_herdr_path()).expect("a stale probe");
    let at_request: Seen = Arc::default();
    let seen = Arc::clone(&at_request);
    let probe_dir = MachineDir::at(&root, &machine());
    let core = FakeCore::start_with(move |config| {
        config.exec_handler = Some(Arc::new(move |params: &Value| {
            if params["input"].is_null() {
                // The probe: it ends after the first write's life.
                for lease in probe_dir.leases() {
                    let _ = probe_dir.remove_lease(&lease.holder);
                }
                return ran(b"herdr-probe-v1\n/opt/homebrew/bin/herdr\n{\"version\":\"0.9.3\",\"core_session\":\"default\"}\n");
            }
            *seen.lock().expect("seen") = Some((probe_dir.leases(), now_unix()));
            far_ok(params)
        }));
    });
    core.set_machine("loop", "connected");
    send(
        &route(&core, &root),
        &machine(),
        &tab_create("view:5"),
        soon(),
        fine(),
        spent(),
    )
    .expect("sent");
    assert_eq!(execs(&core).len(), 2, "the probe and the request");
    let (leases, now) = at_request
        .lock()
        .expect("seen")
        .take()
        .expect("the request ran");
    assert_eq!(
        leases.len(),
        1,
        "the view's lease during the request: {leases:?}"
    );
    let lease = &leases[0];
    assert_eq!(lease.holder, LeaseHolder::cli(std::process::id()));
    assert!(!lease.dials, "a view action's lease admits a dial");
    assert!(
        lease.expires_at > now + 50.0 && lease.expires_at <= now + 61.0,
        "about a minute: {}",
        lease.expires_at - now
    );
    assert!(dir.leases().is_empty(), "the lease is gone after the call");
    let _ = std::fs::remove_dir_all(&root);
}

// A name herdr did not add, or a missing state folder, sends nothing.
#[test]
fn a_machine_herdr_did_not_add_gets_nothing() {
    let core = FakeCore::start();
    core.set_machine("loop", "connected");
    let root = scratch("view-not-added");
    let err = send(
        &route(&core, &root),
        &machine(),
        &tab_create("view:4"),
        soon(),
        fine(),
        spent(),
    )
    .expect_err("refused");
    assert!(err.contains("herdr here did not add loop"), "{err}");
    assert!(execs(&core).is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

// Review M3: a second action while one runs is refused at once, with
// nothing written or sent, never left to wait past the client's patience.
#[test]
fn a_second_action_while_one_runs_is_refused_at_once() {
    let core = FakeCore::start_with(|config| config.exec_handler = Some(Arc::new(far_ok)));
    core.set_machine("loop", "connected");
    let (root, dir) = added("view-busy");
    let route = route(&core, &root);
    let running = route.busy.lock().expect("the first action");
    let started = Instant::now();
    let err = send(
        &route,
        &machine(),
        &tab_create("view:6"),
        soon(),
        fine(),
        spent(),
    )
    .expect_err("refused");
    assert!(started.elapsed() < Duration::from_secs(2), "no wait");
    assert!(
        err.contains("another action on loop from this view is still running")
            && err.contains("nothing was sent to loop"),
        "{err}"
    );
    assert!(execs(&core).is_empty() && dir.leases().is_empty());
    drop(running);
    let _ = std::fs::remove_dir_all(&root);
}

// Review M3: the whole action has one deadline. Too little of it left for
// the probe, the check and the shortest request: nothing is written or
// sent. The request's run is what is left, at most 15 s, never under 5 s.
#[test]
fn an_action_ends_within_its_deadline() {
    let core = FakeCore::start_with(|config| config.exec_handler = Some(Arc::new(far_ok)));
    core.set_machine("loop", "connected");
    let (root, dir) = added("view-deadline");
    let err = send(
        &route(&core, &root),
        &machine(),
        &tab_create("view:7"),
        Instant::now() + Duration::from_secs(20),
        fine(),
        spent(),
    )
    .expect_err("out of time");
    assert!(
        err.contains("took too long") && err.contains("nothing was sent"),
        "{err}"
    );
    assert!(execs(&core).is_empty() && dir.leases().is_empty());

    assert_eq!(
        request_secs(Duration::from_secs(40)),
        Some(VIEW_REQUEST_SECS)
    );
    assert_eq!(request_secs(Duration::from_secs(12)), Some(7));
    assert_eq!(request_secs(Duration::from_secs(9)), None);
    // The budget's parts: list, probe, check and the longest request.
    let worst = 5 + (VIEW_PROBE_SECS + CALL_MARGIN_SECS) + VIEW_CHECK_SECS;
    assert!(worst + VIEW_REQUEST_MIN_SECS + CALL_MARGIN_SECS <= VIEW_ACTION_BUDGET.as_secs());
    let _ = std::fs::remove_dir_all(&root);
}

// Review M2: the check of the aimed-at far pane runs before the request;
// when it fails, nothing is sent and the lease is gone.
#[test]
fn a_failed_aim_check_sends_nothing() {
    let core = FakeCore::start_with(|config| config.exec_handler = Some(Arc::new(far_ok)));
    core.set_machine("loop", "connected");
    let (root, dir) = added("view-aim");
    let err = send(
        &route(&core, &root),
        &machine(),
        &tab_create("view:8"),
        soon(),
        Box::new(|| Err("pane w3:p2 shows another terminal there now".into())),
        spent(),
    )
    .expect_err("refused");
    assert!(
        err.contains("another terminal there now") && err.contains("nothing was sent to loop"),
        "{err}"
    );
    assert!(execs(&core).is_empty() && dir.leases().is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

// E.3 review, item 8: the one-at-a-time lock is the view's own (its
// route's, shared by its clones), not the process's: another view's
// action, or another test's, is not refused for it.
#[test]
fn one_views_running_action_does_not_refuse_anothers() {
    let core = FakeCore::start_with(|config| config.exec_handler = Some(Arc::new(far_ok)));
    core.set_machine("loop", "connected");
    let (root, _dir) = added("view-busy-other");
    let busy = route(&core, &root);
    let _running = busy.busy.lock().expect("the first action");
    send(
        &route(&core, &root),
        &machine(),
        &tab_create("view:9"),
        soon(),
        fine(),
        spent(),
    )
    .expect("another view's route is free");
    let err = send(
        &busy,
        &machine(),
        &tab_create("view:10"),
        soon(),
        fine(),
        spent(),
    )
    .expect_err("the same view's is not");
    assert!(err.contains("is still running"), "{err}");
    let _ = std::fs::remove_dir_all(&root);
}

// E.3 review, item 6: a stale probe runs within the action's budget, 12 s,
// not the fleet CLI's 30 s.
#[test]
fn the_probe_of_a_view_action_runs_within_its_budget() {
    let (root, dir) = added("view-probe-budget");
    std::fs::remove_file(dir.far_herdr_path()).expect("a stale probe");
    let core = FakeCore::start_with(|config| {
        config.exec_handler = Some(Arc::new(|params: &Value| {
            if params["input"].is_null() {
                return ran(b"herdr-probe-v1\n/opt/homebrew/bin/herdr\n{\"version\":\"0.9.3\",\"core_session\":\"default\"}\n");
            }
            far_ok(params)
        }));
    });
    core.set_machine("loop", "connected");
    send(
        &route(&core, &root),
        &machine(),
        &tab_create("view:11"),
        soon(),
        fine(),
        spent(),
    )
    .expect("sent");
    let sent = execs(&core);
    assert_eq!(sent.len(), 2, "the probe and the request");
    assert!(sent[0]["params"]["input"].is_null(), "the probe first");
    assert_eq!(sent[0]["params"]["timeout"], VIEW_PROBE_SECS);
    let _ = std::fs::remove_dir_all(&root);
}

// E.3 review, item 7: an aim check that does not answer (its connection
// to core waits on another, say) gets its own 10 s and no more: nothing is
// sent, the lease is gone, and the action answers within its deadline.
#[test]
fn an_aim_check_that_does_not_answer_sends_nothing() {
    let core = FakeCore::start_with(|config| config.exec_handler = Some(Arc::new(far_ok)));
    core.set_machine("loop", "connected");
    let (root, dir) = added("view-aim-slow");
    let started = Instant::now();
    let err = send(
        &route(&core, &root),
        &machine(),
        &tab_create("view:12"),
        soon(),
        Box::new(|| {
            std::thread::sleep(Duration::from_secs(40));
            Ok(())
        }),
        spent(),
    )
    .expect_err("out of time");
    let took = started.elapsed();
    assert!(
        took >= Duration::from_secs(VIEW_CHECK_SECS)
            && took < Duration::from_secs(VIEW_CHECK_SECS + 3),
        "{took:?}"
    );
    assert!(
        err.contains("took too long to show the pane's terminal")
            && err.contains("nothing was sent to loop"),
        "{err}"
    );
    assert!(execs(&core).is_empty() && dir.leases().is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

// Fork issue 316, M7: every call to core ends by the action's deadline,
// its connect and wait included: a core that takes a call and never
// answers holds the action no longer than what is left of it, not a
// call's own 5 s.
#[test]
fn a_core_that_does_not_answer_holds_no_call_past_the_deadline() {
    let core = FakeCore::start_with(|config| config.stall_on.push("machine.list"));
    let (root, dir) = added("view-stalled-core");
    let started = Instant::now();
    let err = send(
        &route(&core, &root),
        &machine(),
        &tab_create("view:20"),
        Instant::now() + Duration::from_secs(1),
        fine(),
        spent(),
    )
    .expect_err("no answer");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "waited {:?}",
        started.elapsed()
    );
    assert!(err.contains("nothing was sent to loop"), "{err}");
    assert!(execs(&core).is_empty() && dir.leases().is_empty());

    // A call with no time left is not sent at all.
    let before = core.requests().len();
    let refused = core::call_running_by(
        &core.socket,
        "machine.list",
        json!({}),
        Duration::from_secs(5),
        Some(Instant::now()),
    );
    assert!(
        matches!(refused, Err(core::CallError::NotSent(_))),
        "{refused:?}"
    );
    assert_eq!(core.requests().len(), before, "nothing reached core");
    let _ = std::fs::remove_dir_all(&root);
}

// Fork issue 316, M7: the view server's graceful end removes the lease of
// an action still running, whose thread will not outlive the process to
// remove its own. The list is the route's own, so tests in one process do
// not share it.
#[test]
fn the_view_servers_end_removes_a_running_actions_lease() {
    let core = FakeCore::start_with(|config| config.exec_handler = Some(Arc::new(far_ok)));
    core.set_machine("loop", "connected");
    let (root, dir) = added("view-end-lease");
    let (release, held) = std::sync::mpsc::channel::<()>();
    let route = route(&core, &root);
    let running_route = route.clone();
    let running = std::thread::spawn(move || {
        send(
            &running_route,
            &machine(),
            &tab_create("view:21"),
            soon(),
            Box::new(move || {
                let _ = held.recv();
                Err("the view ended".into())
            }),
            spent(),
        )
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while dir.leases().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(dir.leases().len(), 1, "the running action's lease");
    route.end();
    assert!(dir.leases().is_empty(), "removed at the server's end");
    let _ = release.send(());
    let _ = running.join();
    assert!(execs(&core).is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

// Fork issue 316, review item 5: an action thread still running after the
// server's end must not write its lease again: nothing is sent, no lease is
// left, and nothing is spent.
#[test]
fn an_action_running_past_the_servers_end_writes_no_lease_and_sends_nothing() {
    let core = FakeCore::start_with(|config| config.exec_handler = Some(Arc::new(far_ok)));
    core.set_machine("loop", "connected");
    let (root, dir) = added("view-end-after");
    let (release, held) = std::sync::mpsc::channel::<()>();
    let route = route(&core, &root);
    let running_route = route.clone();
    let commits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = counted(&commits);
    let running = std::thread::spawn(move || {
        send(
            &running_route,
            &machine(),
            &tab_create("view:22"),
            soon(),
            Box::new(move || {
                let _ = held.recv();
                Ok(())
            }),
            counted,
        )
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while dir.leases().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    route.end();
    let _ = release.send(());
    let err = running
        .join()
        .expect("joined")
        .expect_err("the lease cannot be written again");
    assert!(err.contains("could not write the lease"), "{err}");
    assert!(err.contains("nothing was sent to loop"), "{err}");
    assert!(execs(&core).is_empty(), "nothing was sent");
    assert!(dir.leases().is_empty(), "no lease written after the end");
    assert_eq!(commits.load(std::sync::atomic::Ordering::SeqCst), 0);
    let _ = std::fs::remove_dir_all(&root);
}

// Fork issue 316, review item 2: what the request spends (the person's
// confirmation) is spent once, where the request goes, and by nothing that
// refuses before it.
#[test]
fn the_commit_runs_only_where_the_request_is_sent() {
    let core = FakeCore::start_with(|config| config.exec_handler = Some(Arc::new(far_ok)));
    core.set_machine("loop", "connected");
    let (root, _dir) = added("view-commit");
    let commits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let route = route(&core, &root);

    let err = send(
        &route,
        &machine(),
        &tab_create("view:23"),
        soon(),
        Box::new(|| Err("aimed elsewhere".into())),
        counted(&commits),
    )
    .expect_err("refused by the check");
    assert!(err.contains("aimed elsewhere"), "{err}");
    assert_eq!(commits.load(std::sync::atomic::Ordering::SeqCst), 0);

    let err = send(
        &route,
        &machine(),
        &tab_create("view:24"),
        soon(),
        fine(),
        Box::new(|| Err("the confirmation is gone".into())),
    )
    .expect_err("refused by the commit");
    assert!(err.contains("the confirmation is gone"), "{err}");
    assert!(err.contains("nothing was sent to loop"), "{err}");
    assert!(execs(&core).is_empty(), "a refused commit sends nothing");

    send(
        &route,
        &machine(),
        &tab_create("view:25"),
        soon(),
        fine(),
        counted(&commits),
    )
    .expect("sent");
    assert_eq!(commits.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(execs(&core).len(), 1);
    let _ = std::fs::remove_dir_all(&root);
}
