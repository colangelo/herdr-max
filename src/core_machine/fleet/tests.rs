use std::time::Duration;

use base64::Engine as _;
use serde_json::{json, Value};

use super::*;
use crate::api::client::{ApiClient, ConnectionTarget};
use crate::api::schema::{Method, PingParams, Request};
use crate::core_machine::state::{tests::scratch, DialRecord, Lease};
use crate::pane::FakeCore;

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn exec_reply(stdout: &[u8], truncated: bool, timed_out: bool) -> Value {
    json!({"stdout": b64(stdout), "stderr": "", "truncated": truncated,
           "timed_out": timed_out, "ms": 3, "code": 0})
}

fn machine() -> MachineName {
    MachineName::parse("loop").expect("name")
}

fn far() -> FarHerdr {
    FarHerdr {
        link_at: 1.0,
        link_pid: 7,
        path: "/opt/homebrew/bin/herdr-beta".into(),
        version: Some("0.9.3".into()),
        protocol: Some(30),
        endpoint_capabilities: Vec::new(),
    }
}

/// A machine folder under a scratch root.
fn machine_dir(tag: &str) -> (PathBuf, MachineDir) {
    let root = scratch(tag);
    let dir = MachineDir::at(&root, &machine());
    dir.create().expect("create");
    (root, dir)
}

fn link_to(core: &FakeCore, dir: &MachineDir) -> FleetLink {
    let lease = LeaseKeeper::new(dir.clone(), LeaseHolder::cli(std::process::id()), 0.0);
    FleetLink::start(
        CoreAt::at(&core.socket),
        machine(),
        dir.clone(),
        lease,
        far(),
        "default".into(),
    )
    .expect("start")
}

fn client(link: &FleetLink) -> ApiClient {
    ApiClient::for_target(ConnectionTarget::SocketPath(link.socket_path().to_owned()))
}

fn ping() -> Request {
    Request {
        id: "fleet:test".into(),
        method: Method::Ping(PingParams::default()),
    }
}

fn execs(core: &FakeCore) -> Vec<Value> {
    core.requests()
        .into_iter()
        .filter(|request| request["method"] == "machine.exec")
        .collect()
}

#[test]
fn far_herdr_sessions_invert_herdrs_own_mapping() {
    for (core_session, herdr) in [
        ("default", Some("default")),
        ("herdr", Some("default")),
        ("herdrdev", Some("default")),
        ("herdr-work", Some("work")),
        ("herdrdev-v26", Some("v26")),
        ("work", None),
        ("herdr-", None),
    ] {
        assert_eq!(
            far_herdr_session(core_session).as_deref(),
            herdr,
            "{core_session}"
        );
    }
}

#[test]
fn exec_timeouts_follow_the_request_within_cores_bounds() {
    assert_eq!(
        exec_timeout(br#"{"id":"a","method":"ping","params":{}}"#),
        60
    );
    assert_eq!(exec_timeout(b"not json"), 60);
    assert_eq!(
        exec_timeout(br#"{"id":"a","method":"agent.wait","params":{"timeout_ms":5000}}"#),
        60
    );
    assert_eq!(
        exec_timeout(br#"{"id":"a","method":"agent.wait","params":{"timeout_ms":300001}}"#),
        311
    );
    assert_eq!(
        exec_timeout(br#"{"id":"a","method":"agent.wait","params":{"timeout_ms":9000000}}"#),
        600
    );
}

#[test]
fn the_probe_answer_is_the_mark_a_path_and_its_status() {
    let (path, status) = parse_probe(
        b"herdr-probe-v1\n/Users/a/.local/bin/herdr-beta\nnoise\n{\"version\":\"0.9.3\",\"protocol\":30,\"endpoint_capabilities\":[\"health_check\"]}\n",
    )
    .expect("parse");
    assert_eq!(path, "/Users/a/.local/bin/herdr-beta");
    assert_eq!(status.version.as_deref(), Some("0.9.3"));
    assert_eq!(status.protocol, Some(30));
    assert_eq!(status.endpoint_capabilities, ["health_check"]);
    // A herdr too old for `status client --json` still answers the bridge.
    let (_, old) = parse_probe(b"herdr-probe-v1\n/usr/local/bin/herdr\n").expect("old");
    assert_eq!(old.version, None);
    for bad in [
        &b"/usr/local/bin/herdr\n"[..],
        b"herdr-probe-v1\nherdr\n",
        b"herdr-probe-v1\n/bin/her\x07dr\n",
        b"herdr-probe-v1\n",
    ] {
        assert!(parse_probe(bad).is_none(), "{bad:?}");
    }
}

/// D7, F6: the argv that carries a request starts with D5's wrapper, then
/// names the far herdr, its session and the bridge; nothing else.
#[test]
fn the_request_argv_is_the_wrapper_and_the_far_bridge() {
    let argv = bridge_argv(&far(), "work");
    let wrapper = core::machine_env_wrapper();
    assert_eq!(argv[..wrapper.len()], wrapper[..]);
    assert_eq!(
        argv[wrapper.len()..],
        [
            "/opt/homebrew/bin/herdr-beta",
            "--session",
            "work",
            "remote-api-bridge"
        ]
    );
}

/// D7 (S4a): the refusal names the dial and the wake; a gate that only a
/// login reopens refuses `--dial`; `retry_spent` and `dial_failed` do not.
#[test]
fn dial_refusals_follow_the_gate() {
    let (root, dir) = machine_dir("fleet-gate");
    let message = not_connected_message(&machine(), &dir, "unreachable", 1_000.0);
    assert!(
        message.starts_with("loop is dormant; reaching it opens ssh"),
        "{message}"
    );
    assert!(message.contains("rerun with --dial, or run herdr machine wake loop"));
    dir.write_lease(&LeaseHolder::manual(), 2_000.0, 900.0)
        .expect("lease");
    let message = not_connected_message(&machine(), &dir, "connecting", 1_000.0);
    assert!(
        message.starts_with("loop is not connected (connecting)"),
        "{message}"
    );

    let gate = |class, at| {
        Ok(Some(Gate {
            class,
            at,
            detail: None,
        }))
    };
    for class in [
        GateClass::NeedsCheck,
        GateClass::AuthStalled,
        GateClass::AuthFailed,
    ] {
        let refusal = dial_refusal(&machine(), gate(class, 10.0), 1_000.0).expect("refused");
        assert!(refusal.contains("herdr machine login loop"), "{refusal}");
    }
    let down = dial_refusal(&machine(), gate(GateClass::FarCoreDown, 900.0), 1_000.0)
        .expect("far core down");
    assert!(down.contains("herdr machine wake loop"), "{down}");
    assert_eq!(
        dial_refusal(&machine(), gate(GateClass::FarCoreDown, 100.0), 1_000.0),
        None
    );
    assert_eq!(
        dial_refusal(&machine(), gate(GateClass::RetrySpent, 10.0), 1_000.0),
        None
    );
    assert_eq!(
        dial_refusal(&machine(), gate(GateClass::DialFailed, 10.0), 1_000.0),
        None
    );
    assert_eq!(dial_refusal(&machine(), Ok(None), 1_000.0), None);
    assert!(dial_refusal(&machine(), Err(io::Error::other("bad")), 1_000.0).is_some());
    let _ = std::fs::remove_dir_all(&root);
}

/// D3: the `cli-<pid>` lease covers each call and 120 s, is written again
/// only when a call would outrun what is left, keeps its act, and is gone
/// with the CLI.
#[test]
fn the_cli_lease_is_written_sparingly_and_removed() {
    let (root, dir) = machine_dir("fleet-lease");
    let holder = LeaseHolder::cli(4242);
    let lease_file = dir.leases_dir().join("cli-4242");
    let keeper = LeaseKeeper::new(dir.clone(), holder.clone(), 77.0);
    keeper.refresh(60.0).expect("first");
    let first = std::fs::read_to_string(&lease_file).expect("written");
    let leases: Vec<Lease> = dir.leases();
    assert_eq!(leases.len(), 1);
    assert_eq!(leases[0].input_at, 77.0);
    let left = leases[0].expires_at - now_unix();
    assert!((175.0..=181.0).contains(&left), "{left}");
    keeper.refresh(60.0).expect("again");
    keeper.refresh(30.0).expect("shorter");
    assert_eq!(std::fs::read_to_string(&lease_file).expect("same"), first);
    keeper.refresh(600.0).expect("longer");
    assert_ne!(std::fs::read_to_string(&lease_file).expect("longer"), first);
    drop(keeper);
    assert!(!lease_file.exists(), "the lease outlived the CLI");
    // A keeper that never wrote removes nothing of another holder's.
    dir.write_lease(&holder, 9_999_999_999.0, 0.0)
        .expect("other");
    drop(LeaseKeeper::new(dir.clone(), holder, 0.0));
    assert!(lease_file.exists());
    let _ = std::fs::remove_dir_all(&root);
}

/// D7: one request, one `machine.exec` on the machine with the request
/// line as its input; the reply is the far side's bytes, untouched.
#[test]
fn a_request_is_one_exec_and_its_reply_is_the_far_sides() {
    let far_reply = b"{\"id\":\"fleet:test\",\"result\":{\"type\":\"pong\"}}\n";
    let core = FakeCore::start_with(|config| {
        config.exec_reply = Some(exec_reply(far_reply, false, false))
    });
    let (root, dir) = machine_dir("fleet-ok");
    let link = link_to(&core, &dir);
    let reply = client(&link).request_value(&ping()).expect("reply");
    assert_eq!(
        reply,
        serde_json::from_slice::<Value>(far_reply).expect("json")
    );
    let sent = execs(&core);
    assert_eq!(sent.len(), 1);
    let params = &sent[0]["params"];
    assert_eq!(params["on"], "loop");
    assert_eq!(params["timeout"], 60);
    let argv: Vec<String> = serde_json::from_value(params["argv"].clone()).expect("argv");
    assert_eq!(argv, bridge_argv(&far(), "default"));
    let input = base64::engine::general_purpose::STANDARD
        .decode(params["input"].as_str().expect("input"))
        .expect("base64");
    let mut line = serde_json::to_vec(&ping()).expect("request");
    line.push(b'\n');
    assert_eq!(input, line);
    assert!(link.reported_failure().is_none());
    // The lease was written before the exec and goes with the link.
    let lease_file = dir.leases_dir().join(format!("cli-{}", std::process::id()));
    assert!(lease_file.exists());
    drop(link);
    assert!(!lease_file.exists());
    let _ = std::fs::remove_dir_all(&root);
}

/// V37 (F12): a reply cut at 8 MiB or timed out, with exit code 0, is a
/// failure; none of its output reaches the CLI.
#[test]
fn cut_and_timed_out_replies_are_failures_with_no_output() {
    for (truncated, timed_out, says) in [
        (true, false, "the reply from loop was cut at 8 MiB"),
        (false, true, "the reply from loop timed out after 60 s"),
    ] {
        let partial = b"{\"id\":\"fleet:test\",\"result\":{\"type\":\"pong\"}}\n";
        let core = FakeCore::start_with(|config| {
            config.exec_reply = Some(exec_reply(partial, truncated, timed_out))
        });
        let (root, dir) = machine_dir("fleet-cut");
        let link = link_to(&core, &dir);
        let error = client(&link).request_value(&ping()).expect_err("no reply");
        assert!(
            matches!(error, crate::api::client::ApiClientError::EmptyResponse),
            "{error:?}"
        );
        assert_eq!(link.reported_failure().expect("failure").to_string(), says);
        assert_eq!(execs(&core).len(), 1);
        drop(link);
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// V37: a request over 8 MiB raw is refused before anything is sent.
#[test]
fn a_request_over_8_mib_is_not_sent() {
    let core = FakeCore::start();
    let (root, dir) = machine_dir("fleet-big");
    let link = link_to(&core, &dir);
    let big = Request {
        id: "x".repeat(INPUT_LIMIT),
        method: Method::Ping(PingParams::default()),
    };
    assert!(client(&link).request_value(&big).is_err());
    assert_eq!(
        link.reported_failure().expect("failure").to_string(),
        "the request is over 8 MiB, so it was not sent to loop"
    );
    assert!(execs(&core).is_empty(), "{:?}", core.methods());
    assert!(exec(
        &CoreAt::at(&core.socket),
        &machine(),
        &[],
        Some(&vec![b'x'; INPUT_LIMIT + 1]),
        60
    )
    .is_err());
    assert!(execs(&core).is_empty());
    drop(link);
    let _ = std::fs::remove_dir_all(&root);
}

/// V19: the link drops after `machine.exec` is sent: one error, which says
/// the request may have run, and no second exec.
#[test]
fn a_request_lost_after_sending_is_not_sent_again() {
    let core = FakeCore::start_with(|config| config.hang_up_on = vec!["machine.exec"]);
    let (root, dir) = machine_dir("fleet-lost");
    let link = link_to(&core, &dir);
    assert!(client(&link).request_value(&ping()).is_err());
    let failure = link.reported_failure().expect("failure").to_string();
    assert!(
        failure.starts_with("no answer from loop after the request was sent")
            && failure.ends_with("it may have run there, and it is not sent again"),
        "{failure}"
    );
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(execs(&core).len(), 1, "{:?}", core.methods());
    drop(link);
    let _ = std::fs::remove_dir_all(&root);
}

/// A far herdr that no longer runs at its path (exit 127) is a failure
/// with its message, and the next call probes again.
#[test]
fn a_far_herdr_gone_from_its_path_is_probed_again() {
    let core = FakeCore::start_with(|config| {
        config.exec_reply = Some(
            json!({"stdout": "", "stderr": b64(b"env: herdr-beta: No such file or directory\n"),
            "truncated": false, "timed_out": false, "ms": 1, "code": 127}),
        )
    });
    let (root, dir) = machine_dir("fleet-gone");
    std::fs::write(dir.far_herdr_path(), b"{}").expect("cache");
    let link = link_to(&core, &dir);
    assert!(client(&link).request_value(&ping()).is_err());
    assert_eq!(
        link.reported_failure().expect("failure").to_string(),
        "herdr on loop (/opt/homebrew/bin/herdr-beta) ended with exit 127: env: herdr-beta: No such file or directory"
    );
    assert!(!dir.far_herdr_path().exists());
    drop(link);
    let _ = std::fs::remove_dir_all(&root);
}

/// E.1: the far herdr is probed through `machine.exec` (wrapped, the
/// fixed script, the names as arguments) once per link establishment.
#[test]
fn the_far_herdr_is_probed_once_per_link() {
    let probe =
        b"herdr-probe-v1\n/Users/a/.local/bin/herdr\n{\"version\":\"0.9.3\",\"protocol\":30}\n";
    let core =
        FakeCore::start_with(|config| config.exec_reply = Some(exec_reply(probe, false, false)));
    let (root, dir) = machine_dir("fleet-probe");
    let at = CoreAt::at(&core.socket);
    dir.write_dial_record(&DialRecord { at: 100.5, pid: 31 })
        .expect("dial");
    let found = far_herdr(&at, &machine(), &dir).expect("probe");
    assert_eq!(found.path, "/Users/a/.local/bin/herdr");
    assert_eq!(found.version.as_deref(), Some("0.9.3"));
    assert_eq!((found.link_at, found.link_pid), (100.5, 31));
    let sent = execs(&core);
    assert_eq!(sent.len(), 1);
    let argv: Vec<String> =
        serde_json::from_value(sent[0]["params"]["argv"].clone()).expect("argv");
    let wrapper = core::machine_env_wrapper();
    assert_eq!(argv[..wrapper.len()], wrapper[..]);
    assert_eq!(
        argv[wrapper.len()..wrapper.len() + 4],
        ["/bin/sh", "-c", PROBE_SCRIPT, "herdr-probe"]
    );
    assert_eq!(argv.last().map(String::as_str), Some("herdr"));
    assert!(sent[0]["params"].get("input").is_none());

    // The same link: the kept answer, no exec.
    assert_eq!(far_herdr(&at, &machine(), &dir).expect("cached"), found);
    assert_eq!(cached_far_herdr(&dir), Some(found.clone()));
    assert_eq!(execs(&core).len(), 1);
    // A new link: probed again.
    dir.write_dial_record(&DialRecord { at: 200.0, pid: 32 })
        .expect("dial");
    assert_eq!(cached_far_herdr(&dir), None);
    let again = far_herdr(&at, &machine(), &dir).expect("probe again");
    assert_eq!((again.link_at, again.link_pid), (200.0, 32));
    assert_eq!(execs(&core).len(), 2);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_far_side_without_herdr_is_a_failure_naming_what_was_looked_for() {
    let core = FakeCore::start_with(|config| {
        config.exec_reply = Some(
            json!({"stdout": "", "stderr": b64(b"no herdr there answers remote-api-bridge\n"),
            "truncated": false, "timed_out": false, "ms": 1, "code": 2}),
        )
    });
    let (root, dir) = machine_dir("fleet-none");
    let error = far_herdr(&CoreAt::at(&core.socket), &machine(), &dir).expect_err("none");
    assert!(
        error.starts_with("no herdr on loop answers remote-api-bridge"),
        "{error}"
    );
    assert!(
        error.ends_with("no herdr there answers remote-api-bridge; install Herdr there"),
        "{error}"
    );
    assert!(!dir.far_herdr_path().exists());
    let _ = std::fs::remove_dir_all(&root);
}

/// The script finds a herdr by the names it is given, runs nothing it is
/// not given, and stops at the first that answers the bridge check.
#[test]
fn the_probe_script_finds_the_named_herdr() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = scratch("fleet-script");
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).expect("bin");
    let fake = |name: &str, body: &str| {
        let path = bin.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).expect("mode");
    };
    fake("herdr-old", "exit 2");
    fake(
        "herdr",
        r#"case "$*" in "remote-api-bridge --check") echo herdr-api-bridge-v1 ;; "status client --json") echo '{"version":"9.9.9"}' ;; *) exit 3 ;; esac"#,
    );
    let out = std::process::Command::new("/bin/sh")
        .args(["-c", PROBE_SCRIPT, "herdr-probe", "herdr-old", "herdr"])
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("HOME", &root)
        .output()
        .expect("run");
    assert!(out.status.success(), "{out:?}");
    let (path, status) = parse_probe(&out.stdout).expect("parse");
    assert_eq!(PathBuf::from(path), bin.join("herdr"));
    assert_eq!(status.version.as_deref(), Some("9.9.9"));
    let none = std::process::Command::new("/bin/sh")
        .args(["-c", PROBE_SCRIPT, "herdr-probe", "herdr-old"])
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("HOME", &root)
        .output()
        .expect("run");
    assert_eq!(none.status.code(), Some(2));
    let _ = std::fs::remove_dir_all(&root);
}
