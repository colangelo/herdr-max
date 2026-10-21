//! Optional JSON resource facts. These types never enter a published binary codec.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientShellResourceFacts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_pins: Option<BTreeMap<String, u64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_pins: Option<BTreeMap<String, u64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_names: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_heads: Option<BTreeMap<String, ClientWorkspaceHead>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_todos: Option<BTreeMap<String, ClientPaneTodoSummary>>,
    /// The tasks of each pane that has any (fork issue 241). An endpoint
    /// from before this fact ignores it; a pane with none is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_tasks: Option<BTreeMap<String, ClientPaneTaskSummary>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_activity: Option<BTreeMap<String, bool>>,
    /// Background items (shells, monitors, agents, MCP tasks) an agent's
    /// screen lists, for panes where there are any (fork issue 172).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_count: Option<BTreeMap<String, u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_sync: Option<BTreeMap<String, ClientTabSync>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notifications: Option<ClientNotificationSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_sort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_label: Option<String>,
    /// Present (true) while the server cannot reach macOS services through its
    /// Mach bootstrap, as after a logout took its login session (fork issue 228).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub macos_services_lost: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientWorkspaceHead {
    pub label: String,
    pub short_oid: String,
    #[serde(default)]
    pub operation: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientPaneTodoSummary {
    pub total: usize,
    pub open: usize,
    /// An open set of priority names; unknown values retain the count.
    #[serde(default)]
    pub highest_priority: Option<String>,
    /// Changes whenever any todo of the pane changes, so a client holding the
    /// full list knows to fetch it again. Older endpoints send none (0).
    #[serde(default)]
    pub revision: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientPaneTaskSummary {
    /// Open tasks bound to the pane.
    #[serde(default)]
    pub open: usize,
    /// Writes for them waiting for the store ("pending sync").
    #[serde(default)]
    pub pending: usize,
    /// Writes for them held for a decision.
    #[serde(default)]
    pub conflicts: usize,
    /// Changes whenever any of the three does or a task of the pane changes,
    /// so a client holding the tasks knows to read them again.
    #[serde(default)]
    pub revision: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientTabSync {
    pub members: Vec<String>,
    pub ending: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientNotificationSummary {
    pub total: usize,
    pub unread: usize,
    /// The newest entry's id: a client holding the history fetches it again
    /// when it moves, which a full log's counts alone would miss. Older
    /// endpoints send none (0).
    #[serde(default)]
    pub latest_id: u64,
}

impl ClientShellResourceFacts {
    /// Decode independent feature maps/items so a malformed optional fact cannot
    /// discard a healthy sibling feature or reject the required core snapshot.
    pub(super) fn decode(value: &serde_json::Value) -> Option<Self> {
        let value = value.as_object()?;
        let get = |key| value.get(key).cloned();
        fn map<T: serde::de::DeserializeOwned>(
            value: Option<serde_json::Value>,
        ) -> Option<BTreeMap<String, T>> {
            let value = value?;
            Some(
                value
                    .as_object()?
                    .iter()
                    .filter_map(|(key, value)| {
                        serde_json::from_value(value.clone())
                            .ok()
                            .map(|decoded| (key.clone(), decoded))
                    })
                    .collect(),
            )
        }
        Some(Self {
            server_version: get("server_version").and_then(|v| serde_json::from_value(v).ok()),
            workspace_pins: map(get("workspace_pins")),
            pane_pins: map(get("pane_pins")),
            pane_names: map(get("pane_names")),
            workspace_heads: map(get("workspace_heads")),
            pane_todos: map(get("pane_todos")),
            pane_tasks: map(get("pane_tasks")),
            background_activity: map(get("background_activity")),
            background_count: map(get("background_count")),
            tab_sync: map(get("tab_sync")),
            notifications: get("notifications").and_then(|v| serde_json::from_value(v).ok()),
            workspace_sort: get("workspace_sort").and_then(|v| serde_json::from_value(v).ok()),
            host_label: get("host_label").and_then(|v| serde_json::from_value(v).ok()),
            macos_services_lost: get("macos_services_lost")
                .and_then(|v| serde_json::from_value(v).ok()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_optional_items_preserve_healthy_features_and_future_priority_names() {
        let facts = ClientShellResourceFacts::decode(&serde_json::json!({
            "workspace_pins": {"good": 7, "bad": "wrong type"},
            "pane_pins": false,
            "pane_todos": {"pane": {"total": 3, "open": 2, "highest_priority": "future"}},
            "notifications": "wrong type", "future_feature": true
        }))
        .unwrap();
        assert_eq!(
            facts.workspace_pins.unwrap(),
            BTreeMap::from([("good".into(), 7)])
        );
        assert!(facts.pane_pins.is_none());
        assert_eq!(
            facts.pane_todos.unwrap()["pane"]
                .highest_priority
                .as_deref(),
            Some("future")
        );
        assert!(facts.notifications.is_none());
        assert!(ClientShellResourceFacts::decode(&serde_json::json!(false)).is_none());
    }

    #[test]
    fn older_client_ignores_pane_tasks() {
        // A client from before the fact decodes what it knows and skips the
        // key it does not: modelled by the decoder meeting facts it has no
        // field for (`future_feature`), and by the new fact being absent from
        // the JSON of a server that has no tasks.
        let with_tasks = serde_json::json!({
            "pane_todos": {"pane": {"total": 1, "open": 1}},
            "pane_tasks": {"pane": {"open": 2, "pending": 1, "conflicts": 0, "revision": 5,
                                    "future_field": true}},
        });
        let facts = ClientShellResourceFacts::decode(&with_tasks).unwrap();
        assert_eq!(
            facts.pane_tasks.unwrap()["pane"],
            ClientPaneTaskSummary {
                open: 2,
                pending: 1,
                conflicts: 0,
                revision: 5
            }
        );
        assert_eq!(facts.pane_todos.unwrap()["pane"].open, 1);

        // A malformed fact costs nothing else, and a missing one is just None.
        let damaged = ClientShellResourceFacts::decode(&serde_json::json!({
            "pane_todos": {"pane": {"total": 1, "open": 1}},
            "pane_tasks": "wrong type",
        }))
        .unwrap();
        assert!(damaged.pane_tasks.is_none());
        assert_eq!(damaged.pane_todos.unwrap()["pane"].open, 1);
        let old = ClientShellResourceFacts::decode(&serde_json::json!({
            "pane_todos": {"pane": {"total": 1, "open": 1}},
        }))
        .unwrap();
        assert!(old.pane_tasks.is_none());

        // An endpoint from before the fact: a copy of the struct as it was,
        // which does not know `pane_tasks`, reads what the new server sends.
        #[derive(Debug, Default, Deserialize)]
        struct BeforeTasks {
            #[serde(default)]
            pane_todos: Option<BTreeMap<String, ClientPaneTodoSummary>>,
            #[serde(default)]
            workspace_pins: Option<BTreeMap<String, u64>>,
        }
        let new_server = serde_json::to_value(ClientShellResourceFacts {
            pane_todos: Some(
                [(
                    "pane".to_owned(),
                    ClientPaneTodoSummary {
                        total: 1,
                        open: 1,
                        ..Default::default()
                    },
                )]
                .into(),
            ),
            pane_tasks: Some(
                [(
                    "pane".to_owned(),
                    ClientPaneTaskSummary {
                        open: 3,
                        pending: 1,
                        conflicts: 0,
                        revision: 9,
                    },
                )]
                .into(),
            ),
            ..Default::default()
        })
        .unwrap();
        assert!(new_server.get("pane_tasks").is_some());
        let old: BeforeTasks = serde_json::from_value(new_server).unwrap();
        assert_eq!(old.pane_todos.unwrap()["pane"].open, 1);
        assert!(old.workspace_pins.is_none());

        // A server with no tasks writes the same JSON an older server did.
        let json = serde_json::to_value(ClientShellResourceFacts::default()).unwrap();
        assert!(json.get("pane_tasks").is_none());
    }

    #[test]
    fn background_count_decodes_and_is_optional() {
        let facts = ClientShellResourceFacts::decode(&serde_json::json!({
            "background_activity": {"pane": true},
            "background_count": {"pane": 4, "bad": "wrong type"}
        }))
        .unwrap();
        assert_eq!(
            facts.background_count.unwrap(),
            BTreeMap::from([("pane".into(), 4)])
        );
        let old = ClientShellResourceFacts::decode(&serde_json::json!({
            "background_activity": {"pane": true}
        }))
        .unwrap();
        assert!(old.background_count.is_none());
    }
}
