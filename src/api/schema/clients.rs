use serde::{Deserialize, Serialize};

/// One attached client shell as exposed by `client.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ClientInfo {
    /// Connection id; it is only valid while that client stays attached.
    pub client_id: String,
    /// Whether this is the client that API-opened popups target: the one with the latest input.
    #[serde(default)]
    pub foreground: bool,
    /// Workspace this client has focused.
    #[serde(default)]
    pub workspace_id: Option<String>,
    /// Tab this client is showing.
    #[serde(default)]
    pub tab_id: Option<String>,
    /// Milliseconds since this client last forwarded input; null when it never has.
    #[serde(default)]
    pub last_input_age_ms: Option<u64>,
    /// Whether the client's terminal window has focus; null when it does not report focus.
    #[serde(default)]
    pub window_focused: Option<bool>,
    /// Whether this client shows the open popup.
    #[serde(default)]
    pub views_popup: bool,
}
