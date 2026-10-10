use super::harness::*;

#[test]
fn popup_close_reports_popup_not_open_when_no_popup_is_open() {
    let base = unique_test_dir();
    let config_home = base.join("config");
    let runtime_dir = base.join("runtime");
    let socket_path = runtime_dir.join("herdr.sock");
    let _herdr = spawn_herdr(&config_home, &runtime_dir, &socket_path);
    wait_for_socket(&socket_path);

    for args in [
        &["popup", "close"][..],
        &["popup", "close", "--plugin", "example.desk"][..],
    ] {
        let output = run_cli(&socket_path, args);
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        let error: serde_json::Value =
            serde_json::from_slice(&output.stderr).expect("error response should be json");
        assert_eq!(error["error"]["code"], "popup_not_open", "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
    }

    cleanup_test_base(&base);
}

#[test]
fn popup_close_rejects_bad_usage_before_contacting_the_server() {
    let base = unique_test_dir();
    let socket_path = base.join("missing.sock");

    for args in [
        &["popup", "close", "--plugin"][..],
        &["popup", "close", "--force"][..],
        &["popup"][..],
    ] {
        let output = run_cli(&socket_path, args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
    }
}
