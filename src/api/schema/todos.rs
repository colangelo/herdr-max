use serde::{Deserialize, Serialize};

use crate::terminal::todo::TodoPriority;

/// One pane todo as exposed over the API. `pane_id` is the owning pane, so a
/// whole-session `todo.list` stays unambiguous.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TodoInfo {
    pub pane_id: String,
    pub id: u64,
    pub text: String,
    pub done: bool,
    pub priority: TodoPriority,
    /// Public id of the linked pane while it still resolves. Absent once the
    /// target is gone, which is what makes the link dead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_pane_id: Option<String>,
    /// Label captured when the link was made, so a dead link can still say what
    /// it meant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_label: Option<String>,
    /// Always serialized: a client rendering a dead link needs to tell "no
    /// link" from "link whose target is gone", and both carry no `link_pane_id`.
    #[serde(default)]
    pub link_alive: bool,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

/// The open todos a forced close dropped with their pane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ClosedPaneTodos {
    pub pane_id: String,
    pub todos: Vec<TodoInfo>,
}

/// The program a forced respawn stopped: the pane's foreground program (looked
/// for behind a wrapper such as atuin's pty-proxy), or the pane shell's pid
/// with no name when the platform cannot tell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct StoppedProcess {
    pub pid: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Params for `todo.list`: a `pane_id` scopes the result to one pane, no
/// `pane_id` returns every pane's todos.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TodoListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TodoAddParams {
    pub pane_id: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<TodoPriority>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_pane_id: Option<String>,
}

/// Params for `todo.update`. Every optional field left out keeps its current
/// value; `clear_link` removes the link and wins over `link_pane_id`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TodoUpdateParams {
    pub pane_id: String,
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<TodoPriority>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_pane_id: Option<String>,
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub clear_link: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TodoRemoveParams {
    pub pane_id: String,
    pub id: u64,
}

/// Params for `todo.clear`: `done_only` keeps the outstanding todos.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TodoClearParams {
    pub pane_id: String,
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub done_only: bool,
}
