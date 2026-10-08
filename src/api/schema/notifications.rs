use serde::{Deserialize, Serialize};

/// One entry of the server-owned notification log as exposed over the API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct NotificationInfo {
    pub id: u64,
    pub kind: NotificationKind,
    pub title: String,
    pub context: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    pub posted_at_unix: u64,
    #[serde(default)]
    pub read: bool,
}

/// Params for `notification.mark_seen`: an `id` marks that one entry read,
/// no `id` marks every entry read.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct NotificationMarkSeenParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    NeedsAttention,
    Finished,
    UpdateInstalled,
}
