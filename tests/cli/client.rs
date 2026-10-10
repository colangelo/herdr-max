use super::harness::*;

#[test]
fn client_list_is_empty_when_no_client_is_attached() {
    let base = unique_test_dir();
    let config_home = base.join("config");
    let runtime_dir = base.join("runtime");
    let socket_path = runtime_dir.join("herdr.sock");
    let _herdr = spawn_herdr(&config_home, &runtime_dir, &socket_path);
    wait_for_socket(&socket_path);

    let output = run_cli(&socket_path, &["client", "list"]);

    assert_eq!(output.status.code(), Some(0));
    let response: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("response should be json");
    assert_eq!(response["result"]["type"], "client_list");
    assert_eq!(response["result"]["clients"], serde_json::json!([]));

    cleanup_test_base(&base);
}

#[test]
fn client_list_rejects_bad_usage_before_contacting_the_server() {
    let base = unique_test_dir();
    let socket_path = base.join("missing.sock");

    let output = run_cli(&socket_path, &["client", "list", "--all"]);

    assert_eq!(output.status.code(), Some(2));
}
