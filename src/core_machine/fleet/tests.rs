use std::time::Duration;

use base64::Engine as _;
use serde_json::{json, Value};

use super::*;
use crate::api::client::{ApiClient, ConnectionTarget};
use crate::api::schema::{Method, PingParams, Request};
use crate::core_machine::state::{tests::scratch, DialRecord, Lease};

const MAY_RUN: &str = "it may have run there, and it is not sent again";

/// A lease that lives 2 s and is written again every 100 ms.
fn quick() -> LeaseTiming {
    LeaseTiming {
        life: 2.0,
        renew: Duration::from_millis(100),
    }
}
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
        probed_at: 1.0,
        session: "default".into(),
        path: "/opt/homebrew/bin/herdr-beta".into(),
        version: Some("0.9.3".into()),
        protocol: Some(30),
        endpoint_capabilities: Vec::new(),
        core_session: None,
        request_bridge: true,
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
    let lease = LeaseKeeper::start_with(
        dir.clone(),
        LeaseHolder::cli(std::process::id()),
        LeaseUse::Keep,
        quick(),
    )
    .expect("lease");
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

/// 3.1, 3.3: the status probe's run fits its deadline; a request with no
/// timeout of its own (a wait with no `--timeout`) gets core's longest,
/// 600 s; one with a timeout that long and 10 s more, at least 60 s; one
/// whose timeout cannot fit is refused unsent.
#[test]
fn exec_timeouts_follow_the_request_within_cores_bounds() {
    let timeout = |line: &str| exec_timeout(line.as_bytes());
    assert_eq!(
        timeout(r#"{"id":"a","method":"ping","params":{}}"#),
        Ok(PING_EXEC_SECS)
    );
    assert_eq!(timeout("not json"), Ok(600));
    assert_eq!(
        timeout(r#"{"id":"a","method":"events.wait","params":{"match":{}}}"#),
        Ok(600)
    );
    assert_eq!(
        timeout(r#"{"id":"a","method":"agent.wait","params":{"timeout_ms":5000}}"#),
        Ok(60)
    );
    assert_eq!(
        timeout(r#"{"id":"a","method":"agent.wait","params":{"timeout_ms":300001}}"#),
        Ok(311)
    );
    assert_eq!(
        timeout(r#"{"id":"a","method":"agent.wait","params":{"timeout_ms":590000}}"#),
        Ok(600)
    );
    let refused = timeout(r#"{"id":"a","method":"agent.wait","params":{"timeout_ms":590001}}"#)
        .expect_err("too long");
    assert!(
        refused.contains("rerun with --timeout 590000 or less"),
        "{refused}"
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
            "remote-api-bridge",
            "--request"
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
    // 2.3: a retry that found the far core down too is `retry_spent` with
    // `far_core_down` as its detail; it refuses as `far_core_down` does.
    let spent_down = |at| {
        Ok(Some(Gate {
            class: GateClass::RetrySpent,
            at,
            detail: Some("far_core_down".into()),
        }))
    };
    let down = dial_refusal(&machine(), spent_down(900.0), 1_000.0).expect("spent, far core down");
    assert!(
        down.contains("(retry_spent)") && down.contains("herdr machine wake loop"),
        "{down}"
    );
    assert_eq!(dial_refusal(&machine(), spent_down(100.0), 1_000.0), None);
    assert_eq!(
        dial_refusal(&machine(), gate(GateClass::DialFailed, 10.0), 1_000.0),
        None
    );
    assert_eq!(dial_refusal(&machine(), Ok(None), 1_000.0), None);
    assert!(dial_refusal(&machine(), Err(io::Error::other("bad")), 1_000.0).is_some());
    let _ = std::fs::remove_dir_all(&root);
}

/// D3, 2.1: the `cli-<pid>` lease is written at its start, written again
/// while the CLI runs, each write in force for its life only (so a killed
/// CLI's lease ends by itself within a minute), and gone with the CLI.
#[test]
fn the_cli_lease_is_renewed_while_it_runs_and_removed() {
    let (root, dir) = machine_dir("fleet-lease");
    let holder = LeaseHolder::cli(4242);
    let lease_file = dir.leases_dir().join("cli-4242");
    let keeper = LeaseKeeper::start_with(
        dir.clone(),
        holder.clone(),
        LeaseUse::Dial { act: 77.0 },
        quick(),
    )
    .expect("start");
    let leases: Vec<Lease> = dir.leases();
    assert_eq!(leases.len(), 1);
    assert_eq!(leases[0].input_at, 77.0);
    assert!(leases[0].dials);
    let first = leases[0].expires_at;
    assert!(first - now_unix() <= 2.0, "{}", first - now_unix());
    std::thread::sleep(Duration::from_millis(350));
    let renewed = dir.leases()[0].clone();
    assert!(renewed.expires_at > first, "renewed while it runs");
    assert_eq!(renewed.input_at, 77.0, "a renewal is no new act");
    // After `--dial`'s connect it only keeps the link.
    keeper.keep_only().expect("keep");
    assert!(!dir.leases()[0].dials);
    std::thread::sleep(Duration::from_millis(250));
    assert!(!dir.leases()[0].dials, "renewals keep it a keep lease");
    drop(keeper);
    assert!(!lease_file.exists(), "the lease outlived the CLI");
    std::thread::sleep(Duration::from_millis(250));
    assert!(!lease_file.exists(), "renewed after its release");
    let _ = std::fs::remove_dir_all(&root);
}

/// 2.1: a CLI killed outright (no signal handler runs) leaves a lease that
/// is in force for a minute at most, whatever its request's timeout.
#[test]
fn a_killed_clis_lease_ends_within_a_minute() {
    let (root, dir) = machine_dir("fleet-kill");
    let keeper =
        LeaseKeeper::start(dir.clone(), LeaseHolder::cli(4343), LeaseUse::Keep).expect("start");
    let left = dir.leases()[0].expires_at - now_unix();
    assert!(left <= 60.0, "{left}");
    drop(keeper);
    let _ = std::fs::remove_dir_all(&root);
}

/// 2.1: SIGTERM, SIGINT and SIGHUP remove the lease and the bridge folder
/// before the CLI ends as the signal's default does. Run in a child
/// process, which the signal ends.
#[test]
fn termination_signals_remove_the_lease_and_the_folder() {
    const CHILD: &str = "HERDR_FLEET_SIGNAL_TEST";
    if let Some(root) = std::env::var_os(CHILD) {
        let dir = MachineDir::at(Path::new(&root), &machine());
        let keeper = LeaseKeeper::start(
            dir.clone(),
            LeaseHolder::cli(std::process::id()),
            LeaseUse::Keep,
        )
        .expect("lease");
        let folder = private_folder().expect("folder");
        println!("ready {}", folder.display());
        std::thread::sleep(Duration::from_secs(30));
        drop(keeper);
        return;
    }
    use std::io::BufRead as _;
    use std::os::unix::process::ExitStatusExt as _;
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        let (root, dir) = machine_dir("fleet-signal");
        let mut child = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args([
                "--exact",
                "core_machine::fleet::tests::termination_signals_remove_the_lease_and_the_folder",
                "--nocapture",
            ])
            .env(CHILD, &root)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("child");
        let mut lines = std::io::BufReader::new(child.stdout.take().expect("stdout")).lines();
        let folder = loop {
            let line = lines.next().expect("a line").expect("read");
            if let Some(folder) = line.strip_prefix("ready ") {
                break PathBuf::from(folder);
            }
        };
        let lease_file = dir.leases_dir().join(format!("cli-{}", child.id()));
        assert!(lease_file.exists() && folder.exists());
        // SAFETY: a signal to the child this test started.
        unsafe { libc::kill(child.id() as libc::pid_t, signal) };
        let status = child.wait().expect("wait");
        assert_eq!(status.signal(), Some(signal), "{status:?}");
        assert!(!lease_file.exists(), "signal {signal} left the lease");
        assert!(!folder.exists(), "signal {signal} left the folder");
        let _ = std::fs::remove_dir_all(&root);
    }
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
    assert_eq!(params["timeout"], PING_EXEC_SECS);
    let argv: Vec<String> = serde_json::from_value(params["argv"].clone()).expect("argv");
    assert_eq!(argv, bridge_argv(&far(), "default"));
    let input = base64::engine::general_purpose::STANDARD
        .decode(params["input"].as_str().expect("input"))
        .expect("base64");
    let mut line = serde_json::to_vec(&ping()).expect("request");
    line.push(b'\n');
    assert_eq!(input, line);
    assert!(link.reported_failure().is_none());
    // The lease was written before the exec, keeps the link without
    // admitting a dial (2.2), and goes with the link.
    let lease_file = dir.leases_dir().join(format!("cli-{}", std::process::id()));
    assert!(lease_file.exists());
    assert!(!dir.leases()[0].dials);
    drop(link);
    assert!(!lease_file.exists());
    let _ = std::fs::remove_dir_all(&root);
}

/// V37 (F12): a reply cut at 8 MiB or timed out, with exit code 0, is a
/// failure; none of its output reaches the CLI.
#[test]
fn cut_and_timed_out_replies_are_failures_with_no_output() {
    for (truncated, timed_out, says) in [
        (
            true,
            false,
            "the reply from loop was cut at 8 MiB; it ran there, and it is not sent again",
        ),
        (
            false,
            true,
            "the reply from loop timed out after 9 s; it may have run there, and it is not sent again",
        ),
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
    // 3.4: exactly 8 MiB is core's own limit, and is sent.
    assert!(exec(
        &CoreAt::at(&core.socket),
        &machine(),
        &[],
        Some(&vec![b'x'; INPUT_LIMIT]),
        60
    )
    .is_ok());
    assert_eq!(execs(&core).len(), 1);
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
            && failure.ends_with(MAY_RUN),
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
        format!("herdr on loop (/opt/homebrew/bin/herdr-beta) ended with exit 127: env: herdr-beta: No such file or directory; {MAY_RUN}")
    );
    assert!(!dir.far_herdr_path().exists());
    drop(link);
    let _ = std::fs::remove_dir_all(&root);
}

/// 3.2, 7.2, #310: every far exit but 0 says the request may have run and
/// forgets the probe: a herdr with no `remote-api-bridge` (exit 2), one
/// whose `remote-api-bridge` has no `--request` (its usage, exit 1, as a
/// herdr before it prints), and any other failure.
#[test]
fn far_exits_say_it_may_have_run_and_forget_the_probe() {
    for (code, stderr) in [
        (2, &b"error: unknown command\n"[..]),
        (
            1,
            b"Error: Custom { kind: InvalidInput, error: \"usage: herdr remote-api-bridge [--check]\" }\n",
        ),
        (1, b"Error: failed to connect to remote Herdr API socket\n"),
    ] {
        let core = FakeCore::start_with(|config| {
            config.exec_reply = Some(json!({"stdout": "", "stderr": b64(stderr),
                "truncated": false, "timed_out": false, "ms": 1, "code": code}))
        });
        let (root, dir) = machine_dir("fleet-exit");
        std::fs::write(dir.far_herdr_path(), b"{}").expect("cache");
        let link = link_to(&core, &dir);
        assert!(client(&link).request_value(&ping()).is_err());
        let failure = link.reported_failure().expect("failure").to_string();
        assert!(
            failure.contains(&format!("ended with exit {code}")) && failure.ends_with(MAY_RUN),
            "{failure}"
        );
        assert!(!dir.far_herdr_path().exists(), "exit {code} kept the probe");
        drop(link);
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// 3.6: core answers `unavailable` for a link lost during the run ("lost
/// the line to <m> during the call", measured on hc 0.1.21): the request
/// may have run.
#[test]
fn unavailable_during_the_run_may_have_run() {
    let core = FakeCore::start_with(|config| {
        config.exec_error = Some(json!({"code": -32000,
            "message": "lost the line to loop during the call",
            "data": {"kind": "unavailable", "hint": "it may have happened there; check, then retry"}}))
    });
    let (root, dir) = machine_dir("fleet-lost-line");
    let link = link_to(&core, &dir);
    assert!(client(&link).request_value(&ping()).is_err());
    let failure = link.reported_failure().expect("failure").to_string();
    assert!(
        failure.contains("lost the line to loop during the call") && failure.ends_with(MAY_RUN),
        "{failure}"
    );
    assert!(!failure.contains("not sent there"), "{failure}");
    drop(link);
    let _ = std::fs::remove_dir_all(&root);
}

/// 3.5: a failure is that request's own: the next request that gets its
/// reply leaves none.
#[test]
fn a_failure_is_cleared_by_the_next_request() {
    let far_reply = b"{\"id\":\"fleet:test\",\"result\":{\"type\":\"pong\"}}\n";
    let core = FakeCore::start_with(|config| {
        config.exec_reply = Some(exec_reply(far_reply, false, false))
    });
    let (root, dir) = machine_dir("fleet-clear");
    let link = link_to(&core, &dir);
    let big = Request {
        id: "x".repeat(INPUT_LIMIT),
        method: Method::Ping(PingParams::default()),
    };
    assert!(client(&link).request_value(&big).is_err());
    assert!(link.reported_failure().is_some());
    client(&link).request_value(&ping()).expect("reply");
    assert!(link.reported_failure().is_none());
    drop(link);
    let _ = std::fs::remove_dir_all(&root);
}

/// 3.1: a CLI that gave up on a request still running there does not wait
/// for it when it ends: the lease and the folder go at once.
#[test]
fn dropping_the_bridge_does_not_wait_for_a_request_in_flight() {
    let core = FakeCore::start_with(|config| config.stall_on = vec!["machine.exec"]);
    let (root, dir) = machine_dir("fleet-stall");
    let link = link_to(&core, &dir);
    let folder = link.socket_path().parent().expect("folder").to_path_buf();
    assert!(client(&link)
        .request_value_with_timeout(&ping(), Duration::from_millis(300))
        .is_err());
    let started = std::time::Instant::now();
    drop(link);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert!(dir.leases().is_empty(), "the lease outlived the CLI");
    assert!(!folder.exists());
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
    let found = far_herdr(&at, &machine(), &dir, "default", "default").expect("probe");
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
        argv[wrapper.len()..wrapper.len() + 5],
        ["/bin/sh", "-c", PROBE_SCRIPT, "herdr-probe", "default"]
    );
    assert_eq!(argv.last().map(String::as_str), Some("herdr"));
    assert!(sent[0]["params"].get("input").is_none());

    // The same link: the kept answer, no exec.
    assert_eq!(
        far_herdr(&at, &machine(), &dir, "default", "default").expect("cached"),
        found
    );
    assert_eq!(cached_far_herdr(&dir), Some(found.clone()));
    assert_eq!(execs(&core).len(), 1);
    // A new link: probed again.
    dir.write_dial_record(&DialRecord { at: 200.0, pid: 32 })
        .expect("dial");
    assert_eq!(cached_far_herdr(&dir), None);
    let again = far_herdr(&at, &machine(), &dir, "default", "default").expect("probe again");
    assert_eq!((again.link_at, again.link_pid), (200.0, 32));
    assert_eq!(execs(&core).len(), 2);
    // 7.1: the same link 5 min later: probed again, so a far herdr
    // upgraded in place is read anew.
    let mut old = again.clone();
    old.probed_at -= 301.0;
    write_atomic(
        &dir.far_herdr_path(),
        &serde_json::to_vec(&old).expect("json"),
    )
    .expect("age it");
    far_herdr(&at, &machine(), &dir, "default", "default").expect("probe after 5 min");
    assert_eq!(execs(&core).len(), 3);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_far_side_without_herdr_is_a_failure_naming_what_was_looked_for() {
    let core = FakeCore::start_with(|config| {
        config.exec_reply = Some(
            json!({"stdout": "", "stderr": b64(b"no herdr there answers remote-api-bridge --request\n"),
            "truncated": false, "timed_out": false, "ms": 1, "code": 2}),
        )
    });
    let (root, dir) = machine_dir("fleet-none");
    let error = far_herdr(
        &CoreAt::at(&core.socket),
        &machine(),
        &dir,
        "default",
        "default",
    )
    .expect_err("none");
    assert!(
        error.starts_with("no herdr on loop answers remote-api-bridge --request"),
        "{error}"
    );
    assert!(
        error.ends_with(
            "no herdr there answers remote-api-bridge --request; install or update Herdr there"
        ),
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
    // A herdr from before the one-request bridge: skipped.
    fake(
        "herdr-stream",
        r#"case "$*" in "remote-api-bridge --check") echo herdr-api-bridge-v1 ;; *) exit 1 ;; esac"#,
    );
    fake(
        "herdr",
        r#"case "$*" in "remote-api-bridge --check") echo herdr-api-bridge-v1 ;; "remote-api-bridge --request --check") echo herdr-api-request-v1 ;; "--session work status client --json") echo '{"version":"9.9.9","core_session":"herdr-work"}' ;; *) exit 3 ;; esac"#,
    );
    let out = std::process::Command::new("/bin/sh")
        .args([
            "-c",
            PROBE_SCRIPT,
            "herdr-probe",
            "work",
            "herdr-old",
            "herdr-stream",
            "herdr",
        ])
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("HOME", &root)
        .output()
        .expect("run");
    assert!(out.status.success(), "{out:?}");
    let (path, status) = parse_probe(&out.stdout).expect("parse");
    assert_eq!(PathBuf::from(path), bin.join("herdr"));
    assert_eq!(status.version.as_deref(), Some("9.9.9"));
    assert_eq!(status.core_session.as_deref(), Some("herdr-work"));
    let none = std::process::Command::new("/bin/sh")
        .args([
            "-c",
            PROBE_SCRIPT,
            "herdr-probe",
            "work",
            "herdr-old",
            "herdr-stream",
        ])
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("HOME", &root)
        .output()
        .expect("run");
    assert_eq!(none.status.code(), Some(2));
    let _ = std::fs::remove_dir_all(&root);
}

/// 4: the far herdr must run its session in the core session the machine
/// was added with; one too old to say is taken for a release build, never
/// for a debug build's `herdrdev`.
#[test]
fn the_far_herdr_must_run_the_linked_core_session() {
    let with = |core_session: Option<&str>| FarHerdr {
        core_session: core_session.map(str::to_string),
        ..far()
    };
    assert_eq!(
        check_far_session(&machine(), &with(Some("default")), "default"),
        Ok(())
    );
    assert_eq!(
        check_far_session(&machine(), &with(Some("herdrdev-v")), "herdrdev-v"),
        Ok(())
    );
    let other =
        check_far_session(&machine(), &with(Some("default")), "herdrdev").expect_err("other");
    assert!(
        other.contains("runs session default in Herdr Core session default, not herdrdev"),
        "{other}"
    );
    assert_eq!(
        check_far_session(&machine(), &with(None), "default"),
        Ok(())
    );
    assert_eq!(
        check_far_session(&machine(), &with(None), "herdr-work"),
        Ok(())
    );
    let silent = check_far_session(&machine(), &with(None), "herdrdev-v").expect_err("debug");
    assert!(
        silent.contains("does not say which core session"),
        "{silent}"
    );
}

/// 6: with no passwd entry, a name core does not list is an unknown
/// selector (exit 2), as a typo is; one core lists is refused with the
/// reason.
#[test]
fn without_herdrs_state_an_unknown_name_stays_a_usage_error() {
    let core = FakeCore::start();
    let at = CoreAt::at(&core.socket);
    let gone = || Err(io::Error::other("uid 501 has no passwd entry"));
    assert!(matches!(
        open_at(&at, gone(), &machine(), false),
        Lookup::NotCore
    ));
    core.set_machine("loop", "connected");
    match open_at(&at, gone(), &machine(), false) {
        Lookup::Refused { code: 1, message } => {
            assert!(
                message.contains("cannot find herdr's machine state"),
                "{message}"
            )
        }
        _ => panic!("expected a refusal"),
    }
}

/// 5: a taken name is skipped for a fresh one, never used.
#[test]
fn the_bridge_folder_takes_a_fresh_name_when_one_is_taken() {
    let base = scratch("fleet-folder");
    std::fs::create_dir(base.join("taken")).expect("taken");
    let mut names = ["taken", "fresh"].into_iter().map(String::from);
    let folder = private_folder_in(&base, || names.next().unwrap_or_default()).expect("folder");
    assert_eq!(folder, base.join("fresh"));
    use std::os::unix::fs::PermissionsExt as _;
    assert_eq!(
        std::fs::metadata(&folder)
            .expect("meta")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_ne!(unguessable_leaf(), unguessable_leaf());
    let _ = std::fs::remove_dir_all(&base);
}

/// 2.1: a folder a killed CLI left is removed by the next; a live one's
/// stays.
#[test]
fn folders_of_dead_clis_are_removed() {
    let base = scratch("fleet-dead");
    let dead = base.join(format!("{FOLDER_PREFIX}999999-00"));
    let mine = base.join(format!("{FOLDER_PREFIX}{}-00", std::process::id()));
    let parent = base.join(format!(
        "{FOLDER_PREFIX}{}-00",
        crate::platform::core_link::parent_pid()
    ));
    for folder in [&dead, &mine, &parent] {
        std::fs::create_dir(folder).expect("folder");
    }
    remove_dead_folders(&base);
    assert!(!dead.exists());
    assert!(mine.exists() && parent.exists());
    let _ = std::fs::remove_dir_all(&base);
}

/// 2.2: a plain call on a connected machine holds a keep lease (it admits
/// no dial, so a drop during the call is not redialled) and connects
/// nothing.
#[test]
fn a_connected_machine_gets_a_keep_lease_and_no_connect() {
    let probe = b"herdr-probe-v1\n/Users/a/.local/bin/herdr\n{\"version\":\"0.9.3\",\"core_session\":\"default\"}\n";
    let core =
        FakeCore::start_with(|config| config.exec_reply = Some(exec_reply(probe, false, false)));
    core.set_machine("loop", "connected");
    let (root, dir) = machine_dir("fleet-connected");
    dir.write_target(&crate::core_machine::state::Target {
        target: "ac@loop".into(),
        session: "default".into(),
        ssh: "/usr/bin/ssh".into(),
        ssh_config: None,
        ssh_auth_sock: None,
    })
    .expect("target");
    let Lookup::Ready(link) = prepare(&CoreAt::at(&core.socket), &machine(), &dir, false) else {
        panic!("not ready");
    };
    let leases = dir.leases();
    assert_eq!(leases.len(), 1);
    assert!(!leases[0].dials, "a connected call's lease admits a dial");
    assert!(
        !core
            .methods()
            .iter()
            .any(|method| method == "machine.connect"),
        "{:?}",
        core.methods()
    );
    drop(link);
    assert!(dir.leases().is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

/// security review 2026-11-23 M1: far stderr reaches the message with its
/// controls escaped, in a probe's failure and in a request's, and so does
/// a far herdr's core session.
#[test]
fn far_text_in_messages_shows_its_controls_escaped() {
    let hostile = b"\x1b]52;c;aGk=\x07\x1b]0;pwned\x1b\\\x1b[2K\x1b[1Agone\rX\n";
    let shown = "\\x1b]52;c;aGk=\\x07\\x1b]0;pwned\\x1b\\\\x1b[2K\\x1b[1Agone\\x0dX";
    let core = FakeCore::start_with(|config| {
        config.exec_reply = Some(json!({"stdout": "", "stderr": b64(hostile),
            "truncated": false, "timed_out": false, "ms": 1, "code": 2}))
    });
    let (root, dir) = machine_dir("fleet-far-text");
    let probe = far_herdr(
        &CoreAt::at(&core.socket),
        &machine(),
        &dir,
        "default",
        "default",
    )
    .expect_err("probe fails");
    assert!(probe.contains(shown), "{probe}");
    assert!(!probe.chars().any(char::is_control), "{probe:?}");

    let link = link_to(&core, &dir);
    assert!(client(&link).request_value(&ping()).is_err());
    let failure = link.reported_failure().expect("failure").to_string();
    assert!(failure.contains(shown), "{failure}");
    assert!(!failure.chars().any(char::is_control), "{failure:?}");
    drop(link);

    let mut found = far();
    found.core_session = Some("herdr-x\x1b]0;t\x07".into());
    let refused = check_far_session(&machine(), &found, "default").expect_err("other core");
    assert!(refused.contains("herdr-x\\x1b]0;t\\x07"), "{refused}");
    assert!(!refused.chars().any(char::is_control), "{refused:?}");
    let _ = std::fs::remove_dir_all(&root);
}

/// #310: the probe runs before the request is sent, so no failure of it
/// says that the request may have run: a link lost, no answer, a timeout.
#[test]
fn a_failed_probe_says_the_request_was_not_sent() {
    let lost = || {
        FakeCore::start_with(|config| {
            config.exec_error = Some(json!({"code": -32000,
                "message": "lost the line to loop during the call",
                "data": {"kind": "unavailable"}}))
        })
    };
    let silent = || FakeCore::start_with(|config| config.hang_up_on = vec!["machine.exec"]);
    let late =
        || FakeCore::start_with(|config| config.exec_reply = Some(exec_reply(b"", false, true)));
    for (what, core) in [("lost", lost()), ("silent", silent()), ("late", late())] {
        let (root, dir) = machine_dir("fleet-probe-fails");
        let error = far_herdr(
            &CoreAt::at(&core.socket),
            &machine(),
            &dir,
            "default",
            "default",
        )
        .expect_err(what);
        assert!(!error.contains("may have run"), "{what}: {error}");
        assert!(
            error.starts_with("could not find the herdr on loop: ")
                && error.ends_with("; the request was not sent"),
            "{what}: {error}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// #310: a far bridge that ends well with no reply, or with no whole
/// reply line, is a failure that says the request may have run; nothing
/// is passed to the CLI.
#[test]
fn an_empty_reply_is_a_failure_that_may_have_run() {
    for stdout in [&b""[..], b"{\"id\":\"fleet:test\"", b"{}\n{}\n"] {
        let core = FakeCore::start_with(|config| {
            config.exec_reply = Some(exec_reply(stdout, false, false))
        });
        let (root, dir) = machine_dir("fleet-empty");
        let link = link_to(&core, &dir);
        assert!(client(&link).request_value(&ping()).is_err());
        let failure = link.reported_failure().expect("failure").to_string();
        assert!(
            failure.starts_with("herdr on loop (/opt/homebrew/bin/herdr-beta) answered with no")
                && failure.ends_with(MAY_RUN),
            "{stdout:?}: {failure}"
        );
        drop(link);
        let _ = std::fs::remove_dir_all(&root);
    }
}
