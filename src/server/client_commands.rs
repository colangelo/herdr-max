use std::io;
use std::sync::mpsc;

use tokio::sync::mpsc as tokio_mpsc;

use crate::api::schema::{ErrorBody, ErrorResponse, Method};

use super::client_transport::ServerEvent;

pub(crate) const MAX_ENDPOINT_COMMAND_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_ENDPOINT_BOOT_ID_BYTES: usize = 128;
pub(crate) const MAX_ENDPOINT_REQUEST_ID_BYTES: usize = 128;
const ENDPOINT_RESPONSE_CHUNK_BYTES: usize = 512 * 1024;

const CLIENT_SHELL_METHODS: &[&str] = &[
    "agent.message",
    "agent.pin",
    "agent.unpin",
    "client_shell.surface.set",
    "command.invoke",
    "integration.install",
    "integration.list",
    "layout.balance",
    "layout.set_preset",
    "layout.set_split_ratio",
    "notification.clear",
    "notification.list",
    "notification.mark_seen",
    "pane.clear",
    "pane.clear_scrollback",
    "pane.close",
    "pane.copy_motion",
    "pane.copy_search",
    "pane.edit_scrollback",
    "pane.focus",
    "pane.focus_direction",
    "pane.input.set",
    "pane.link.activate",
    "pane.link.resolve",
    "pane.move",
    "pane.rename",
    "pane.resize",
    "pane.respawn",
    "pane.scroll",
    "pane.scroll_application",
    "pane.selection.read",
    "pane.split",
    "pane.swap",
    "pane.sync",
    "pane.sync_pair",
    "pane.zoom",
    "product_announcement.dismiss",
    "release_notes.dismiss",
    "server.reload_config",
    "tab.close",
    "tab.create",
    "tab.focus",
    "tab.move",
    "tab.rename",
    "tab.sync",
    "todo.add",
    "todo.clear",
    "todo.list",
    "todo.remove",
    "todo.update",
    "workspace.close",
    "workspace.create",
    "workspace.focus",
    "workspace.move",
    "workspace.move_block",
    "workspace.pin",
    "workspace.rename",
    "workspace.unpin",
    "worktree.create",
    "worktree.list",
    "worktree.open",
    "worktree.remove",
];

pub(crate) fn supported_client_shell_method_names() -> &'static [&'static str] {
    CLIENT_SHELL_METHODS
}

pub(crate) fn supports_client_shell_method_name(method: &str) -> bool {
    CLIENT_SHELL_METHODS.contains(&method)
}

pub(crate) fn supports_client_shell_method(method: &Method) -> bool {
    supports_client_shell_method_name(crate::api::api_method_name(method))
}

pub(crate) fn error_response(id: String, code: &str, message: impl Into<String>) -> String {
    serde_json::to_string(&ErrorResponse {
        id,
        error: ErrorBody {
            code: code.into(),
            message: message.into(),
        },
    })
    .unwrap_or_else(|_| {
        r#"{"id":"","error":{"code":"serialization_error","message":"failed to serialize endpoint response"}}"#.into()
    })
}

pub(crate) fn success_message_with_result(
    boot_id: String,
    request_id: String,
    result: crate::api::schema::ResponseResult,
) -> crate::protocol::ServerMessage {
    let response = serde_json::to_string(&crate::api::schema::SuccessResponse {
        id: request_id.clone(),
        result,
    })
    .unwrap_or_else(|_| {
        error_response(
            request_id.clone(),
            "serialization_error",
            "failed to serialize endpoint response",
        )
    });
    crate::protocol::ServerMessage::ClientShellEndpointResponseChunk {
        boot_id,
        request_id,
        final_chunk: true,
        data: response.into_bytes(),
    }
}

pub(crate) fn error_message(
    boot_id: String,
    request_id: String,
    code: &str,
    message: impl Into<String>,
) -> crate::protocol::ServerMessage {
    let response = error_response(request_id.clone(), code, message);
    crate::protocol::ServerMessage::ClientShellEndpointResponseChunk {
        boot_id,
        request_id,
        final_chunk: true,
        data: response.into_bytes(),
    }
}

fn correlate_response_id(response: String, request_id: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&response) else {
        return response;
    };
    let Some(id) = value.get_mut("id") else {
        return response;
    };
    if id.as_str() == Some(request_id) {
        return response;
    }
    *id = serde_json::Value::String(request_id.to_owned());
    serde_json::to_string(&value).unwrap_or(response)
}

pub(crate) fn spawn_response_waiter(
    client_id: u64,
    boot_id: String,
    request_id: String,
    response_rx: mpsc::Receiver<String>,
    server_event_tx: tokio_mpsc::Sender<ServerEvent>,
) -> io::Result<()> {
    std::thread::Builder::new()
        .name("herdr-client-endpoint-response".into())
        .spawn(move || {
            let response = response_rx.recv().unwrap_or_else(|_| {
                error_response(
                    request_id.clone(),
                    "server_unavailable",
                    "endpoint command ended without a response",
                )
            });
            let response = correlate_response_id(response, &request_id).into_bytes();
            if response.is_empty() {
                let _ = server_event_tx.blocking_send(
                    ServerEvent::ClientShellEndpointResponseChunkReady {
                        client_id,
                        boot_id,
                        request_id,
                        final_chunk: true,
                        data: Vec::new(),
                    },
                );
                return;
            }
            let chunk_count = response.len().div_ceil(ENDPOINT_RESPONSE_CHUNK_BYTES);
            for (index, chunk) in response.chunks(ENDPOINT_RESPONSE_CHUNK_BYTES).enumerate() {
                if server_event_tx
                    .blocking_send(ServerEvent::ClientShellEndpointResponseChunkReady {
                        client_id,
                        boot_id: boot_id.clone(),
                        request_id: request_id.clone(),
                        final_chunk: index + 1 == chunk_count,
                        data: chunk.to_vec(),
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use sha2::{Digest, Sha256};

    use super::*;

    fn collect_schema_refs(value: &serde_json::Value, refs: &mut BTreeSet<String>) {
        match value {
            serde_json::Value::Object(object) => {
                if let Some(reference) = object.get("$ref").and_then(serde_json::Value::as_str) {
                    if let Some(name) = reference.rsplit('/').next() {
                        refs.insert(name.to_owned());
                    }
                }
                for value in object.values() {
                    collect_schema_refs(value, refs);
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    collect_schema_refs(value, refs);
                }
            }
            _ => {}
        }
    }

    fn normalized_wire_schema(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(object) => serde_json::Value::Object(
                object
                    .iter()
                    .filter(|(key, _)| {
                        !matches!(
                            key.as_str(),
                            "description" | "examples" | "readOnly" | "title" | "writeOnly"
                        )
                    })
                    .map(|(key, value)| (key.clone(), normalized_wire_schema(value)))
                    .collect(),
            ),
            serde_json::Value::Array(values) => {
                serde_json::Value::Array(values.iter().map(normalized_wire_schema).collect())
            }
            _ => value.clone(),
        }
    }

    fn endpoint_method_shape_digests() -> BTreeMap<String, String> {
        let schema = serde_json::to_value(schemars::schema_for!(crate::api::schema::Request))
            .expect("request schema");
        let definitions = schema
            .get("$defs")
            .and_then(serde_json::Value::as_object)
            .expect("request definitions");
        let branches = schema
            .get("oneOf")
            .and_then(serde_json::Value::as_array)
            .expect("request method branches");
        let mut digests = BTreeMap::new();

        for method in CLIENT_SHELL_METHODS {
            let branch = branches
                .iter()
                .find(|branch| {
                    branch
                        .pointer("/properties/method/const")
                        .and_then(serde_json::Value::as_str)
                        == Some(method)
                })
                .unwrap_or_else(|| panic!("missing request schema branch for {method}"));
            let mut referenced_names = BTreeSet::new();
            collect_schema_refs(branch, &mut referenced_names);
            let mut visited_names = BTreeSet::new();
            let mut selected_definitions = serde_json::Map::new();
            while let Some(name) = referenced_names.pop_first() {
                if !visited_names.insert(name.clone()) {
                    continue;
                }
                let definition = definitions
                    .get(&name)
                    .unwrap_or_else(|| panic!("missing schema definition {name} for {method}"));
                collect_schema_refs(definition, &mut referenced_names);
                selected_definitions.insert(name, normalized_wire_schema(definition));
            }
            let shape = serde_json::json!({
                "request": normalized_wire_schema(branch),
                "definitions": selected_definitions,
            });
            let bytes = serde_json::to_vec(&shape).expect("method shape json");
            digests.insert(method.to_string(), format!("{:x}", Sha256::digest(bytes)));
        }

        digests
    }

    #[test]
    fn advertised_client_shell_method_shapes_stay_at_the_v1_contract() {
        let expected: BTreeMap<String, String> = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/endpoint-method-shapes-v1.json"
        )))
        .expect("endpoint method shape fixture");
        let mut actual = endpoint_method_shape_digests();
        // Freeze these first sync advertisements separately from the published fixture.
        for (method, digest) in [
            (
                "pane.sync",
                "d06e7589cfa7f46706005ce162827db7bb309c0028dc4e108a4f3a4368388cbd",
            ),
            (
                "pane.sync_pair",
                "af7e6ee03b6835ea8d3d2ae3ed4ba0b5f66b515f7294bc90c2941e09d5b331a4",
            ),
            (
                "tab.sync",
                "9caed2d73b24ee76b5feeeb5cdfc2f0b854e77b0a7030f57df05225b2ad49ed5",
            ),
        ] {
            assert_eq!(
                actual.remove(method).as_deref(),
                Some(digest),
                "{method} changed shape"
            );
        }
        assert_eq!(
            actual.remove("layout.balance").as_deref(),
            Some("2612679bd673608105f286c9b53ecd56333fbebe2c4cd4c52a4c47e2861c6246")
        );
        assert_eq!(
            actual.remove("layout.set_preset").as_deref(),
            Some("7e72c1b5a208a26bc752064997eb5a27185b31ea16ceef11dc810f5e7a50fc30")
        );
        // Freeze additive methods separately without rewriting the published fixture.
        assert_eq!(
            actual.remove("pane.clear").as_deref(),
            Some("0301d288ba198ddaa427dd7421c71911cccaf4ea03544531efa8b67ca21b08f6")
        );
        assert_eq!(
            actual.remove("pane.clear_scrollback").as_deref(),
            Some("7fae135ef10aeb8ebff20aeaedcb1fac3a560f16a277da00f2866d3ebb2877b8")
        );
        assert_eq!(
            actual.remove("pane.link.resolve").as_deref(),
            Some("f5e4a3e01453ae7b188f127ce951c12c20e0bebcc17cc364eeb6d1a01fd5bf81")
        );
        assert_eq!(
            actual.remove("pane.scroll_application").as_deref(),
            Some("2b845df515101b8252592845acb4d0147f1773559d39324574b02700d307f334")
        );
        // Fork (herdr-max): the close methods carry an optional `force` that skips the
        // open-todos refusal. Old clients omit it and get the v1 behavior; the new
        // shapes are frozen here so any further change still fails.
        for (method, digest) in [
            (
                "pane.close",
                "36b7fbb91571620bf8a5933c0e18b5ed6ecbfcaad72be91bca1773cdb9306f5e",
            ),
            (
                "tab.close",
                "4bd575541bd0b24cad7f6da0e06105ee87a0e8bbf6e189718c6437417aeae08c",
            ),
            (
                "workspace.close",
                "508d4409a530158b98432cccffb62a7ca4f7d3cb308b6db2989739f8325fdb46",
            ),
            // Fork (herdr-max): a mouse drag sends `proportional: true`. An
            // older server ignores the field and sets the one ratio, as before
            // (fork issue 175).
            (
                "layout.set_split_ratio",
                "a606d4fb0b80f1b68b282f7c342493a6adcbedd3bb19087904d10977f27554cb",
            ),
        ] {
            let actual_digest = actual.remove(method);
            assert_eq!(
                actual_digest.as_deref(),
                Some(digest),
                "{method} changed shape"
            );
        }
        // Fork (herdr-max): `pane.respawn` is additive, advertised so the
        // client shell can ask before a forced respawn (fork issue 125).
        assert_eq!(
            actual.remove("pane.respawn").as_deref(),
            Some("b261b69cb75bc97794320ac121948b1c8d8740ce4996d2bfce9f6b1c184b6897")
        );
        // Fork (herdr-max): `pane.move` is additive, advertised so the client
        // shell's pane-move picker and pane-move keys can move a pane.
        assert_eq!(
            actual.remove("pane.move").as_deref(),
            Some("eaed63cf205db2dc043ecce9e1a79cdca7f6e2521b364226bbf3121affadce7c")
        );
        // Fork (herdr-max): the notification history methods are additive,
        // advertised so the client shell's notification center can read,
        // mark and clear the server's log.
        for (method, digest) in [
            (
                "notification.clear",
                "99389f77880f4a2eb52734c68fa532dc6180a86e6a5b72dfa2097fc47f0adcda",
            ),
            (
                "notification.list",
                "80408223ff7602a272b849d37bb3ce13c5f4e63ed65215212832e2646f02b2b9",
            ),
            (
                "notification.mark_seen",
                "d16c91275200eb583eba45eb8a85230524ffeeb2cd19b30e1a395c1c1734fb79",
            ),
        ] {
            assert_eq!(
                actual.remove(method).as_deref(),
                Some(digest),
                "{method} changed shape"
            );
        }
        // Fork (herdr-max): the pane todo methods are additive, advertised so
        // the client shell's todo panel and editor can read and change todos.
        for (method, digest) in [
            (
                "todo.add",
                "4caa6b279cfe03aa4713ae1ee5798c979715d1f55fc12aa9c570151108171237",
            ),
            (
                "todo.clear",
                "dbb05704d7a588c3573d42ca86ce2f9adc30eeb314aeaf079328e4b632c05a2d",
            ),
            (
                "todo.list",
                "bdf5b96a796d079fe509d4b5dc8630e741991dabba0bed0f2b32b00c21753e5b",
            ),
            (
                "todo.remove",
                "5a2821bf16a78f3639a644f2e3eede43eff7417bd13b1e65fbfb961de2cce103",
            ),
            (
                "todo.update",
                "288f2ad9d26077dc346d8588e8e0b7d06a9af12357318d34e3a3fe3c5a88f1d6",
            ),
        ] {
            assert_eq!(
                actual.remove(method).as_deref(),
                Some(digest),
                "{method} changed shape"
            );
        }
        // Fork (herdr-max): `agent.message` is additive, advertised so the command
        // bar can send a message to a session (fork issue 182).
        assert_eq!(
            actual.remove("agent.message").as_deref(),
            Some("45849f796f0739ce5baeb3287ec246c7710368e7549836af38634c6b6a31cd91")
        );
        // Fork (herdr-max): the pin methods are additive, advertised so the client
        // shell's pin key, context menu and pin markers can pin and unpin agents
        // and workspaces (fork issue 209).
        for (method, digest) in [
            (
                "agent.pin",
                "99d8dc8995050befb79b3fef74876dbfe3f480ae7e61bf3fa437b35140e18a45",
            ),
            (
                "agent.unpin",
                "de90c44ed6c2e0880e8a35619d8a4f7eda92db520c8bd07b0e96ce961a167bfb",
            ),
            (
                "workspace.pin",
                "1ae80549033183d3d149ad420d990c80a0c6d009b3a73df39f7fb2e10402d716",
            ),
            (
                "workspace.unpin",
                "a1763d190328eb1dc5baefed70f4f05eafadb23bd3fb762900e71a86683a87f3",
            ),
        ] {
            assert_eq!(
                actual.remove(method).as_deref(),
                Some(digest),
                "{method} changed shape"
            );
        }
        let mut expected = expected;
        for method in [
            "pane.close",
            "tab.close",
            "workspace.close",
            "layout.set_split_ratio",
        ] {
            expected.remove(method);
        }

        assert_eq!(
            actual, expected,
            "an existing endpoint method changed shape; add load-bearing behavior as a new advertised method or explicitly gate new fields"
        );
    }

    #[test]
    fn advertised_client_shell_methods_are_sorted_unique_and_in_schema() {
        assert!(CLIENT_SHELL_METHODS
            .windows(2)
            .all(|pair| pair[0] < pair[1]));

        fn collect_method_constants(value: &serde_json::Value, methods: &mut Vec<String>) {
            match value {
                serde_json::Value::Object(object) => {
                    if let Some(method) = object
                        .get("const")
                        .and_then(serde_json::Value::as_str)
                        .filter(|value| value.contains('.'))
                    {
                        methods.push(method.to_owned());
                    }
                    for value in object.values() {
                        collect_method_constants(value, methods);
                    }
                }
                serde_json::Value::Array(values) => {
                    for value in values {
                        collect_method_constants(value, methods);
                    }
                }
                _ => {}
            }
        }

        let schema = serde_json::to_value(schemars::schema_for!(crate::api::schema::Request))
            .expect("request schema");
        let mut schema_methods = Vec::new();
        collect_method_constants(&schema, &mut schema_methods);
        for method in CLIENT_SHELL_METHODS {
            assert!(
                schema_methods.iter().any(|candidate| candidate == method),
                "advertised endpoint method {method:?} is absent from the request schema"
            );
        }
    }

    /// Wire names of every `Method` the client shell source constructs or
    /// matches, found by scanning `src/client` outside test code.
    ///
    /// The client refuses (`supports_endpoint_method`) and the server gate
    /// rejects any method missing from `CLIENT_SHELL_METHODS`, so a method the
    /// shell can push but the list omits fails silently in the TUI (issue 209:
    /// pin and unpin). Scanning the source keeps this from drifting: a new
    /// `Method::Foo` in the shell is picked up without editing a copied list.
    fn client_shell_source_method_names() -> BTreeSet<String> {
        fn rust_files(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).expect("read client source dir") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    if path.file_name().is_some_and(|name| name == "tests") {
                        continue;
                    }
                    rust_files(&path, files);
                } else if path.extension().is_some_and(|ext| ext == "rs")
                    && path.file_name().is_some_and(|name| name != "tests.rs")
                {
                    files.push(path);
                }
            }
        }

        fn normalize(name: &str) -> String {
            name.chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .map(|c| c.to_ascii_lowercase())
                .collect()
        }

        let schema = serde_json::to_value(schemars::schema_for!(crate::api::schema::Request))
            .expect("request schema");
        let mut wire_names = BTreeSet::new();
        fn collect(value: &serde_json::Value, names: &mut BTreeSet<String>) {
            match value {
                serde_json::Value::Object(object) => {
                    if let Some(name) = object
                        .get("const")
                        .and_then(serde_json::Value::as_str)
                        .filter(|value| value.contains('.'))
                    {
                        names.insert(name.to_owned());
                    }
                    object.values().for_each(|value| collect(value, names));
                }
                serde_json::Value::Array(values) => {
                    values.iter().for_each(|value| collect(value, names));
                }
                _ => {}
            }
        }
        collect(&schema, &mut wire_names);
        let mut by_variant = BTreeMap::new();
        for name in &wire_names {
            let previous = by_variant.insert(normalize(name), name.clone());
            assert!(previous.is_none(), "ambiguous wire name {name}");
        }

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/client");
        let mut files = Vec::new();
        rust_files(&root, &mut files);
        assert!(!files.is_empty(), "no client sources found under {root:?}");

        let mut found = BTreeSet::new();
        for file in files {
            let source = std::fs::read_to_string(&file).expect("read client source");
            let lines: Vec<&str> = source.lines().collect();
            // Drop the trailing `#[cfg(test)] mod ...` block; the shell's
            // production code never follows it.
            let end = lines
                .iter()
                .enumerate()
                .position(|(index, line)| {
                    line.trim() == "#[cfg(test)]"
                        && lines
                            .get(index + 1)
                            .is_some_and(|next| next.trim_start().starts_with("mod "))
                })
                .unwrap_or(lines.len());
            for line in &lines[..end] {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                let mut rest = *line;
                while let Some(at) = rest.find("Method::") {
                    let after = &rest[at + "Method::".len()..];
                    let variant: String = after
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric())
                        .collect();
                    let attached = rest[..at]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
                    if !variant.is_empty() && !attached {
                        let wire = by_variant.get(&normalize(&variant)).unwrap_or_else(|| {
                            panic!(
                                "{}: Method::{variant} has no request schema entry",
                                file.display()
                            )
                        });
                        found.insert(wire.clone());
                    }
                    rest = after;
                }
            }
        }
        found
    }

    #[test]
    fn every_method_the_client_shell_sends_is_advertised() {
        let sent = client_shell_source_method_names();
        assert!(
            sent.len() > 20 && sent.contains("pane.focus"),
            "client method scan looks broken: {sent:?}"
        );
        let missing: Vec<&String> = sent
            .iter()
            .filter(|method| !CLIENT_SHELL_METHODS.contains(&method.as_str()))
            .collect();
        assert!(
            missing.is_empty(),
            "the client shell sends methods missing from CLIENT_SHELL_METHODS (the client and server gate refuse them): {missing:?}"
        );
    }

    #[test]
    fn client_shell_lane_excludes_api_front_door_and_lifecycle_methods() {
        assert!(supports_client_shell_method(
            &Method::ClientShellSurfaceSet(crate::api::schema::ClientShellSurfaceSetParams {
                active: false,
            })
        ));
        assert!(supports_client_shell_method(&Method::ServerReloadConfig(
            crate::api::schema::EmptyParams::default(),
        )));
        assert!(supports_client_shell_method(&Method::PaneLinkActivate(
            crate::api::schema::PaneLinkActivateParams {
                pane_id: "w1:p1".into(),
                viewport_row: 0,
                col: 0,
                content_revision: None,
                offset_from_bottom: None,
            },
        )));
        assert!(supports_client_shell_method(&Method::PaneLinkResolve(
            crate::api::schema::PaneLinkActivateParams {
                pane_id: "w1:p1".into(),
                viewport_row: 0,
                col: 0,
                content_revision: None,
                offset_from_bottom: None,
            },
        )));
        assert!(!supports_client_shell_method(&Method::Ping(
            crate::api::schema::PingParams::default(),
        )));
        assert!(!supports_client_shell_method(&Method::ServerStop(
            crate::api::schema::EmptyParams::default(),
        )));
    }

    #[test]
    fn endpoint_response_uses_the_client_request_id() {
        let response = serde_json::json!({
            "id": "endpoint:boot-a:7:client-shell:1",
            "result": { "type": "ok" }
        })
        .to_string();

        let correlated = correlate_response_id(response, "client-shell:1");
        let decoded: serde_json::Value = serde_json::from_str(&correlated).expect("response json");

        assert_eq!(decoded["id"], "client-shell:1");
    }

    #[test]
    fn endpoint_responses_are_chunked_without_truncation() {
        let (response_tx, response_rx) = mpsc::channel();
        let (event_tx, mut event_rx) = tokio_mpsc::channel(8);
        spawn_response_waiter(
            7,
            "boot-a".into(),
            "request-a".into(),
            response_rx,
            event_tx,
        )
        .unwrap();
        let response = "x".repeat(ENDPOINT_RESPONSE_CHUNK_BYTES + 17);
        response_tx.send(response.clone()).unwrap();

        let mut received = Vec::new();
        loop {
            let ServerEvent::ClientShellEndpointResponseChunkReady {
                client_id,
                boot_id,
                request_id,
                final_chunk,
                data,
            } = event_rx.blocking_recv().expect("response chunk")
            else {
                panic!("expected response chunk");
            };
            assert_eq!(client_id, 7);
            assert_eq!(boot_id, "boot-a");
            assert_eq!(request_id, "request-a");
            received.extend(data);
            if final_chunk {
                break;
            }
        }

        assert_eq!(received, response.as_bytes());
    }
}
