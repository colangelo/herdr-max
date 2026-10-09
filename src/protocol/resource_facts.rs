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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_activity: Option<BTreeMap<String, bool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_sync: Option<BTreeMap<String, ClientTabSync>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notifications: Option<ClientNotificationSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_sort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_label: Option<String>,
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
            background_activity: map(get("background_activity")),
            tab_sync: map(get("tab_sync")),
            notifications: get("notifications").and_then(|v| serde_json::from_value(v).ok()),
            workspace_sort: get("workspace_sort").and_then(|v| serde_json::from_value(v).ok()),
            host_label: get("host_label").and_then(|v| serde_json::from_value(v).ok()),
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
}
