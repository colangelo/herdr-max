use super::harness::*;
use serde_json::{json, Value};

struct Capture {
    output: std::process::Output,
    requests: Vec<Value>,
}

fn capture(args: &[&str], response: Value) -> Capture {
    let base = unique_test_dir();
    fs::create_dir_all(&base).unwrap();
    let socket = base.join("contract.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_herdr"))
        .args(args)
        .env("HERDR_SOCKET_PATH", &socket)
        .env("XDG_CONFIG_HOME", &base)
        .env("XDG_RUNTIME_DIR", &base)
        .env_remove("HERDR_CLIENT_SOCKET_PATH")
        .env_remove("HERDR_ENV")
        .env_remove("HERDR_PANE_ID")
        .env_remove("HERDR_SESSION")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut requests = Vec::new();
    loop {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            drop(listener);
            cleanup_test_base(&base);
            panic!("CLI timed out: {args:?}; output: {output:?}; requests: {requests:?}");
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                // BSD-derived platforms inherit the listener's non-blocking flag.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut line = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                if request["method"] == "ping" {
                    write_fake_pong(&mut stream, &request, "contract-baseline", CURRENT_PROTOCOL);
                } else {
                    writeln!(stream, "{response}").unwrap();
                    stream.flush().unwrap();
                }
                requests.push(request);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if child.try_wait().unwrap().is_some() {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("contract listener failed: {error}"),
        }
    }
    let output = child.wait_with_output().unwrap();
    drop(listener);
    cleanup_test_base(&base);
    Capture { output, requests }
}

#[test]
fn fork_cli_requests_match_the_unit_inventory_over_the_socket() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../../src/cli/fork_inventory.json")).unwrap();
    for case in cases {
        let argv: Vec<&str> = case["argv"]
            .as_array()
            .unwrap()
            .iter()
            .skip(1)
            .map(|value| value.as_str().unwrap())
            .collect();
        let captured = capture(&argv, json!({"id":"cli:request","result":{"type":"ok"}}));
        assert_eq!(
            captured.output.status.code(),
            Some(0),
            "{argv:?}: {:?}",
            captured.output
        );
        assert!(
            captured.output.stderr.is_empty(),
            "{argv:?}: {:?}",
            captured.output
        );
        let methods: Vec<Value> = captured
            .requests
            .iter()
            .filter(|request| request["method"] != "ping")
            .map(|request| json!({"method":request["method"], "params":request["params"]}))
            .collect();
        assert_eq!(json!(methods), case["requests"], "{argv:?}");
    }
}
