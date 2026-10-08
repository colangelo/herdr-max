use super::harness::*;
use std::process::Command;

/// `herdr todo` with no pane argument, run from inside a pane, must land on that
/// pane. This is the feature's headline path — an agent recording its own next
/// steps as it exits — and it is the one thing unit tests cannot cover, because
/// it spans the CLI's `HERDR_PANE_ID` default, the socket round-trip, and the
/// server-side store.
/// Runs the CLI with no ambient pane identity. The harness's `run_cli` inherits
/// the caller's environment, so a developer running the suite from inside a
/// herdr pane would otherwise leak their real `HERDR_PANE_ID` into the test and
/// mask a missing-target failure.
fn run_todo_cli_without_pane(socket_path: &std::path::Path, args: &[&str]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_herdr"));
    command.env_remove("HERDR_STARTUP_CWD");
    command.args(args);
    command.env("HERDR_SOCKET_PATH", socket_path);
    command.env_remove("HERDR_PANE_ID");
    command.output().unwrap()
}

fn run_todo_cli_as_pane(
    socket_path: &std::path::Path,
    pane_id: &str,
    args: &[&str],
) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_herdr"));
    command.env_remove("HERDR_STARTUP_CWD");
    command.args(args);
    command.env("HERDR_SOCKET_PATH", socket_path);
    command.env("HERDR_PANE_ID", pane_id);
    command.output().unwrap()
}

#[test]
fn todo_cli_round_trips_through_the_server() {
    let base = unique_test_dir();
    let config_home = base.join("config");
    let runtime_dir = base.join("runtime");
    let socket_path = runtime_dir.join("herdr.sock");

    let _herdr = spawn_herdr(&config_home, &runtime_dir, &socket_path);
    wait_for_socket(&socket_path);

    let created = run_cli(
        &socket_path,
        &["workspace", "create", "--cwd", base.to_str().unwrap()],
    );
    assert!(created.status.success());
    let created_json: serde_json::Value = serde_json::from_slice(&created.stdout).unwrap();
    let pane_id = created_json["result"]["root_pane"]["pane_id"]
        .as_str()
        .expect("workspace create should report its root pane")
        .to_string();

    // No --pane, no --current: the calling pane comes from HERDR_PANE_ID.
    let added = run_todo_cli_as_pane(
        &socket_path,
        &pane_id,
        &[
            "todo",
            "add",
            "fix the flaky handoff test",
            "--priority",
            "high",
            "--json",
        ],
    );
    assert!(
        added.status.success(),
        "todo add failed: {}",
        String::from_utf8_lossy(&added.stderr)
    );
    let added_json: serde_json::Value = serde_json::from_slice(&added.stdout).unwrap();
    let todo = &added_json["result"]["todo"];
    assert_eq!(
        todo["pane_id"], pane_id,
        "the todo must land on the calling pane"
    );
    assert_eq!(todo["text"], "fix the flaky handoff test");
    assert_eq!(todo["priority"], "high");
    assert_eq!(todo["done"], false);
    let todo_id = todo["id"].as_u64().expect("todo should carry an id");

    // A second todo, so ordering and the outstanding count are observable.
    let second = run_todo_cli_as_pane(
        &socket_path,
        &pane_id,
        &["todo", "add", "rerun the deploy", "--json"],
    );
    assert!(second.status.success());

    // Listing without a target also defaults to the calling pane.
    let listed = run_todo_cli_as_pane(&socket_path, &pane_id, &["todo", "list", "--json"]);
    assert!(listed.status.success());
    let listed_json: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    let todos = listed_json["result"]["todos"].as_array().unwrap();
    assert_eq!(todos.len(), 2);
    assert_eq!(
        todos[0]["text"], "fix the flaky handoff test",
        "the high-priority todo sorts first"
    );

    // Human-readable output is what an operator actually sees.
    let plain = run_todo_cli_as_pane(&socket_path, &pane_id, &["todo", "list"]);
    assert!(plain.status.success());
    let plain_stdout = String::from_utf8_lossy(&plain.stdout);
    assert!(
        plain_stdout.contains("fix the flaky handoff test"),
        "list output should show the todo text: {plain_stdout}"
    );

    // Completing a todo moves it behind the outstanding one.
    let done = run_todo_cli_as_pane(
        &socket_path,
        &pane_id,
        &["todo", "done", &todo_id.to_string(), "--json"],
    );
    assert!(
        done.status.success(),
        "todo done failed: {}",
        String::from_utf8_lossy(&done.stderr)
    );
    let done_json: serde_json::Value = serde_json::from_slice(&done.stdout).unwrap();
    assert_eq!(done_json["result"]["todo"]["done"], true);

    let after_done = run_todo_cli_as_pane(&socket_path, &pane_id, &["todo", "list", "--json"]);
    let after_done_json: serde_json::Value = serde_json::from_slice(&after_done.stdout).unwrap();
    let todos = after_done_json["result"]["todos"].as_array().unwrap();
    assert_eq!(
        todos[0]["text"], "rerun the deploy",
        "a done high-priority todo sinks below an outstanding normal one"
    );

    // Clearing only the done ones leaves the outstanding todo behind.
    let cleared = run_todo_cli_as_pane(
        &socket_path,
        &pane_id,
        &["todo", "clear", "--done", "--json"],
    );
    assert!(cleared.status.success());
    let cleared_json: serde_json::Value = serde_json::from_slice(&cleared.stdout).unwrap();
    assert_eq!(cleared_json["result"]["removed"], 1);

    let remaining = run_todo_cli_as_pane(&socket_path, &pane_id, &["todo", "list", "--json"]);
    let remaining_json: serde_json::Value = serde_json::from_slice(&remaining.stdout).unwrap();
    assert_eq!(
        remaining_json["result"]["todos"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn todo_cli_reports_errors_from_the_server() {
    let base = unique_test_dir();
    let config_home = base.join("config");
    let runtime_dir = base.join("runtime");
    let socket_path = runtime_dir.join("herdr.sock");

    let _herdr = spawn_herdr(&config_home, &runtime_dir, &socket_path);
    wait_for_socket(&socket_path);

    let unknown_pane = run_todo_cli_without_pane(
        &socket_path,
        &["todo", "add", "nowhere", "--pane", "w9:p9", "--json"],
    );
    assert!(
        !unknown_pane.status.success(),
        "an unknown pane must be a non-zero exit"
    );
    // herdr reports API errors as JSON on stderr, leaving stdout clean for
    // machine-readable success output.
    let unknown_json: serde_json::Value =
        serde_json::from_slice(&unknown_pane.stderr).expect("error response should be json");
    assert_eq!(unknown_json["error"]["code"], "pane_not_found");
    assert!(
        unknown_pane.stdout.is_empty(),
        "a failed call must not print a success payload to stdout"
    );

    // Without a pane target and without --all there is nothing to act on, and
    // the CLI must say so rather than silently doing nothing.
    let no_target = run_todo_cli_without_pane(&socket_path, &["todo", "add", "orphan"]);
    assert!(!no_target.status.success());
    let stderr = String::from_utf8_lossy(&no_target.stderr);
    assert!(
        stderr.contains("no pane target"),
        "expected a pane-target error, got: {stderr}"
    );
}
