use crate::cli::pane::bindings as binding;
use crate::cli::projection::{cli_projection, Field, Input};

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::agents::AgentSessionInfo;
use super::common::{AgentStatus, PaneAgentState, ReadFormat, ReadSource, SplitDirection};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum PaneRightClickTarget {
    #[default]
    Herdr,
    Pane,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneSplitParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_pane_id: Option<String>,
    pub direction: SplitDirection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ratio: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default)]
    pub focus: bool,
    #[serde(default)]
    pub right_click: PaneRightClickTarget,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneInputSetParams {
    pub pane_id: String,
    pub right_click: PaneRightClickTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneLinkActivateParams {
    pub pane_id: String,
    pub viewport_row: u16,
    pub col: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset_from_bottom: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaneDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct PaneSwapParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<PaneDirection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_pane_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_pane_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneMoveParams {
    pub pane_id: String,
    pub destination: PaneMoveDestination,
    #[serde(default)]
    pub focus: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaneMoveDestination {
    Tab {
        tab_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_pane_id: Option<String>,
        split: SplitDirection,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ratio: Option<f32>,
    },
    NewTab {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    NewWorkspace {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab_label: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct PaneZoomParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    #[serde(default)]
    pub mode: PaneZoomMode,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum PaneZoomMode {
    #[default]
    Toggle,
    On,
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct PaneSyncParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    /// `on` puts the pane in the synced set, `off` takes it out. The pane's
    /// tab must already sync (`sync_not_active` otherwise).
    #[serde(default)]
    pub mode: super::SyncMode,
}

/// Start input synchronization for two explicit panes in the same tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneSyncPairParams {
    pub source_pane_id: String,
    pub target_pane_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct PaneLayoutParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct PaneProcessInfoParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct LayoutExportParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct LayoutApplyParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_label: Option<String>,
    #[serde(default)]
    pub focus: bool,
    pub root: LayoutNode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct LayoutSetSplitRatioParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    pub path: Vec<bool>,
    pub ratio: f32,
    // A mouse drag: panes on each side of the border resize proportionally.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub proportional: bool,
}

/// Rebalance every split in a layout so all panes become equally sized. Targets
/// a layout by `tab_id` or `pane_id`; with neither, the active tab is used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct LayoutBalanceParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
}

/// A tmux-style layout preset that rebuilds the pane tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LayoutPreset {
    /// One equal-width row of panes.
    EvenHorizontal,
    /// One equal-height column of panes.
    EvenVertical,
    /// A near-square grid.
    Tiled,
}

/// Rebuild a layout into a preset (`even_horizontal` / `even_vertical` /
/// `tiled`). Targets a layout by `tab_id` or `pane_id`; with neither, the
/// active tab is used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct LayoutSetPresetParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    pub preset: LayoutPreset,
}

/// Presets accepted by the additive `layout.apply_preset` method. The original
/// `LayoutPreset` remains closed to preserve the published endpoint contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AppliedLayoutPreset {
    /// One equal-width row of panes.
    EvenHorizontal,
    /// One equal-height column of panes.
    EvenVertical,
    /// A near-square grid.
    Tiled,
    /// Focused pane in the left half; others in one or two right columns.
    MainVertical,
}

/// Rebuild a tab into a preset, preserving focus. Select by `tab_id` or
/// `pane_id` (mutually exclusive); neither selects the active tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct LayoutApplyPresetParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    pub preset: AppliedLayoutPreset,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct LayoutDescription {
    pub workspace_id: String,
    pub tab_id: String,
    pub zoomed: bool,
    pub focused_pane_id: String,
    pub root: LayoutNode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayoutNode {
    Pane {
        #[serde(flatten)]
        pane: LayoutPane,
    },
    Split {
        direction: SplitDirection,
        ratio: f32,
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct LayoutPane {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneNeighborParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    pub direction: PaneDirection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct PaneEdgesParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneFocusDirectionParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    pub direction: PaneDirection,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneResizeParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    pub direction: PaneDirection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneScrollParams {
    pub pane_id: String,
    pub offset_from_bottom: u64,
}

/// Application-owned alternate-screen scroll input, distinct from host history
/// offsets and ordinary typed input that may fan out to a sync group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaneApplicationScrollIntent {
    WheelUp,
    WheelDown,
    PageUp,
    PageDown,
    Home,
    End,
    CtrlHome,
    CtrlEnd,
}

fn one_application_scroll() -> u16 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneScrollApplicationParams {
    pub pane_id: String,
    pub intent: PaneApplicationScrollIntent,
    #[serde(default = "one_application_scroll")]
    #[schemars(range(min = 1, max = 64))]
    pub count: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneTextPoint {
    pub row: u32,
    pub col: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneSelectionReadParams {
    pub pane_id: String,
    pub anchor: PaneTextPoint,
    pub cursor: PaneTextPoint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_revision: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaneCopyMotion {
    LineEnd,
    FirstNonBlank,
    NextWordStart,
    PreviousWordStart,
    NextWordEnd,
    NextBigWordStart,
    PreviousBigWordStart,
    NextBigWordEnd,
    PreviousParagraph,
    NextParagraph,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneCopyMotionParams {
    pub pane_id: String,
    pub cursor: PaneTextPoint,
    pub motion: PaneCopyMotion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_revision: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaneCopySearchDirection {
    Forward,
    Backward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneTextRange {
    pub start: PaneTextPoint,
    pub end: PaneTextPoint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneCopySearchParams {
    pub pane_id: String,
    pub query: String,
    pub direction: PaneCopySearchDirection,
    pub cursor: PaneTextPoint,
    pub content_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<PaneTextRange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct PaneListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct PaneCurrentParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_pane_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneRenameParams {
    pub pane_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneSendTextParams {
    pub pane_id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneSendKeysParams {
    pub pane_id: String,
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneSendInputParams {
    pub pane_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneReadParams {
    pub pane_id: String,
    pub source: ReadSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<u32>,
    #[serde(default)]
    pub format: ReadFormat,
    #[serde(default = "super::default_true")]
    pub strip_ansi: bool,
    /// Leave out faint (SGR 2) cells, for every source (fork issue 146).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub strip_dim: bool,
    #[serde(skip)]
    #[schemars(skip)]
    pub(crate) intent: super::common::ReadIntent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneReportAgentParams {
    pub pane_id: String,
    pub source: String,
    pub agent: String,
    pub state: PaneAgentState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session_path: Option<String>,
    /// Command that resumes this agent's session after a Herdr restart. The
    /// first element must be a plain command name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_argv: Option<Vec<String>>,
}

/// What an agent waits for, as a source reports it (fork issue 157).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentHintKind {
    /// The agent asked the user a question.
    Question,
    /// The agent waits for approval to run a tool or command.
    Permission,
}

/// A source's claim that an agent waits on the user, or the end of it. A hint
/// is evidence for the pane's state, not a state: it raises `blocked` with the
/// reason and ages out unless it is repeated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneReportHintParams {
    pub pane_id: String,
    pub source: String,
    pub agent: String,
    /// What the agent waits for. Required unless `clear` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<AgentHintKind>,
    /// The source's own id for the dialog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// How long the hint lives without being repeated, in milliseconds.
    /// Default 15000, at most 60000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_ms: Option<u64>,
    /// End the source's hint.
    #[serde(default)]
    pub clear: bool,
    /// Per-source ordering: a report with a `seq` not above the last accepted
    /// one is ignored. A wall-clock based value survives a restart of the
    /// source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneReportAgentSessionParams {
    pub pane_id: String,
    pub source: String,
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_start_source: Option<String>,
    /// Command that resumes this agent's session after a Herdr restart. The
    /// first element must be a plain command name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_argv: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneReportMetadataParams {
    pub pane_id: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applies_to_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_agent: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub state_labels: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    #[schemars(schema_with = "super::common::metadata_token_patch_schema")]
    pub tokens: HashMap<String, Option<String>>,
    #[serde(default)]
    pub clear_title: bool,
    #[serde(default)]
    pub clear_display_agent: bool,
    #[serde(default)]
    pub clear_state_labels: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 86_400_000))]
    pub ttl_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneClearAgentAuthorityParams {
    pub pane_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneReleaseAgentParams {
    pub pane_id: String,
    pub source: String,
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneInfo {
    pub pane_id: String,
    pub terminal_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub focused: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground_cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_title_stripped: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_agent: Option<String>,
    pub agent_status: AgentStatus,
    /// Pinned agents sit at the top of the agent panel in pin order.
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub pinned: bool,
    /// The pane's tab syncs input and this pane is in the synced set.
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub synced: bool,
    /// Why the agent is blocked: `question`, `permission`, `form` or
    /// `other`. Present only while `agent_status` is `blocked`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<crate::detect::BlockedReason>,
    /// When the pane entered its current blocked spell, in unix
    /// milliseconds. Present only while `agent_status` is `blocked`; it does
    /// not move while the pane stays blocked and restarts after a server
    /// restore or live handoff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_since: Option<i64>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub state_labels: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    #[schemars(schema_with = "super::common::metadata_token_values_schema")]
    pub tokens: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session: Option<AgentSessionInfo>,
    /// When input from a user or caller last reached the pane, in unix
    /// seconds. Absent when none has been recorded; treat that as unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_input_at_unix: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll: Option<PaneScrollInfo>,
    pub revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneScrollInfo {
    pub offset_from_bottom: u64,
    pub max_offset_from_bottom: u64,
    pub viewport_rows: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneProcessInfo {
    pub pane_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground_process_group_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tty: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub foreground_processes: Vec<PaneProcessInfoProcess>,
    /// The foreground job of the PTY a recognised wrapper (such as `atuin
    /// pty-proxy`) runs the pane's shell in. Absent for an unwrapped pane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nested_foreground_process_group_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nested_foreground_processes: Vec<PaneProcessInfoProcess>,
    /// The pane's shell, directly or behind a recognised wrapper, is alone at
    /// the front of its terminal: nothing is running in the pane.
    #[serde(default)]
    pub shell_at_prompt: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneProcessInfoProcess {
    pub pid: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argv0: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argv: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmdline: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneSwapResult {
    pub changed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<PaneSwapReason>,
    pub source_pane_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_pane_id: Option<String>,
    pub focused_pane_id: String,
    pub layout: PaneLayoutSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaneSwapReason {
    NoNeighbor,
    SamePane,
    NotFound,
    CrossTab,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneMoveResult {
    pub changed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<PaneMoveReason>,
    pub previous_pane_id: String,
    pub previous_workspace_id: String,
    pub previous_tab_id: String,
    pub pane: Box<PaneInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_layout: Option<Box<PaneLayoutSnapshot>>,
    pub target_layout: Box<PaneLayoutSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_workspace: Option<super::WorkspaceInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_tab: Option<super::TabInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_tab_id: Option<String>,
    pub focused_pane_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaneMoveReason {
    SameTab,
    ZoomedTab,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneZoomResult {
    pub changed: bool,
    pub zoom_changed: bool,
    pub focus_changed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<PaneZoomReason>,
    pub pane_id: String,
    pub focused_pane_id: String,
    pub zoomed: bool,
    pub layout: PaneLayoutSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaneZoomReason {
    SinglePane,
    AlreadyZoomed,
    AlreadyUnzoomed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneLayoutSnapshot {
    pub workspace_id: String,
    pub tab_id: String,
    pub zoomed: bool,
    pub area: PaneLayoutRect,
    pub focused_pane_id: String,
    pub panes: Vec<PaneLayoutPane>,
    pub splits: Vec<PaneLayoutSplit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneLayoutRect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneLayoutPane {
    pub pane_id: String,
    pub focused: bool,
    pub rect: PaneLayoutRect,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneLayoutSplit {
    pub id: String,
    pub direction: SplitDirection,
    pub ratio: f32,
    pub rect: PaneLayoutRect,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneNeighborResult {
    pub pane_id: String,
    pub direction: PaneDirection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub neighbor_pane_id: Option<String>,
    pub layout: PaneLayoutSnapshot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneEdgesResult {
    pub pane_id: String,
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
    pub layout: PaneLayoutSnapshot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneFocusDirectionResult {
    pub changed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<PaneFocusDirectionReason>,
    pub source_pane_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focused_pane_id: Option<String>,
    pub layout: PaneLayoutSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaneFocusDirectionReason {
    NoNeighbor,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneResizeResult {
    pub changed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<PaneResizeReason>,
    pub pane_id: String,
    pub focused_pane_id: String,
    pub layout: PaneLayoutSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaneResizeReason {
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneReadResult {
    pub pane_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub source: ReadSource,
    pub format: ReadFormat,
    pub text: String,
    pub revision: u64,
    pub truncated: bool,
}

/// Inclusive display-cell columns on a pane's current viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneLinkRegion {
    pub row: u16,
    pub start_col: u16,
    pub end_col: u16,
}

cli_projection! {
    pub(crate) PaneSplitProjection for PaneSplitParams {
        workspace_id => Field::new(None::<String>),
        target_pane_id => binding::target(true),
        direction => Field::new(SplitDirection::Right).input(binding::split_direction()),
        ratio => Field::new(None).input(binding::finite("ratio")),
        cwd => Field::new(None).input(binding::optional_text("cwd", "PATH").path()),
        focus => Field::new(false).input(binding::boolean("focus", true)).input(binding::boolean("no-focus", false)),
        right_click => Field::new(PaneRightClickTarget::Herdr).input(binding::right_click()),
        env => Field::new(HashMap::new()).input(binding::environment()),
        }
}

cli_projection! {
    pub(crate) PaneInputProjection for PaneInputSetParams {
        pane_id => binding::input_target(),
        right_click => Field::new(PaneRightClickTarget::Herdr).input(binding::right_click().required()),
        }
}

cli_projection! {
    pub(crate) PaneSwapProjection for PaneSwapParams {
        pane_id => binding::target(false),
        direction => Field::new(None).input(binding::optional_direction().configure(|arg| arg.conflicts_with_all(["source-pane", "target-pane"]))),
        source_pane_id => Field::new(None).input(binding::optional_text("source-pane", "ID").configure(|arg| arg.requires("target-pane"))),
        target_pane_id => Field::new(None).input(binding::optional_text("target-pane", "ID").configure(|arg| arg.requires("source-pane"))),
        }
}

cli_projection! {
    pub(crate) PaneMoveProjection for PaneMoveParams {
        pane_id => Field::new(String::new()).input(binding::pane_id()),
        destination => binding::destination(),
        focus => Field::new(true).input(binding::boolean("focus", true)).input(binding::boolean("no-focus", false)),
        }
}

cli_projection! {
    pub(crate) PaneZoomProjection for PaneZoomParams {
        pane_id => binding::target(true),
        mode => Field::new(PaneZoomMode::Toggle)
            .input(Input::flag("toggle", |slot, _| { *slot = PaneZoomMode::Toggle; Ok(()) }).configure(|arg| arg.conflicts_with_all(["on", "off"])))
            .input(Input::flag("on", |slot, _| { *slot = PaneZoomMode::On; Ok(()) }).configure(|arg| arg.conflicts_with("off")))
            .input(Input::flag("off", |slot, _| { *slot = PaneZoomMode::Off; Ok(()) })),
        }
}

cli_projection! {
    pub(crate) PaneLayoutProjection for PaneLayoutParams {
        pane_id => binding::target(false),
        }
}

cli_projection! {
    pub(crate) PaneProcessInfoProjection for PaneProcessInfoParams {
        pane_id => binding::target(false),
        }
}

cli_projection! {
    pub(crate) PaneNeighborProjection for PaneNeighborParams {
        pane_id => binding::target(false),
        direction => Field::new(PaneDirection::Left).input(binding::direction()),
        }
}

cli_projection! {
    pub(crate) PaneEdgesProjection for PaneEdgesParams {
        pane_id => binding::target(false),
        }
}

cli_projection! {
    pub(crate) PaneFocusProjection for PaneFocusDirectionParams {
        pane_id => binding::target(false),
        direction => Field::new(PaneDirection::Left).input(binding::direction()),
        }
}

cli_projection! {
    pub(crate) PaneResizeProjection for PaneResizeParams {
        pane_id => binding::target(false),
        direction => Field::new(PaneDirection::Left).input(binding::direction()),
        amount => Field::new(None).input(binding::finite("amount")),
        }
}

cli_projection! {
    pub(crate) PaneListProjection for PaneListParams {
        workspace_id => Field::new(None).input(Input::value("workspace", "WORKSPACE_ID", |slot, value| { *slot = Some(crate::cli::normalize_workspace_id(value)); Ok(()) })),
        }
}

cli_projection! {
    pub(crate) PaneCurrentProjection for PaneCurrentParams {
        caller_pane_id => binding::target(false),
        }
}

cli_projection! {
    pub(crate) PaneRenameProjection for PaneRenameParams {
        pane_id => Field::new(String::new()).input(binding::pane_id()),
        label => binding::label(),
        }
}

cli_projection! {
    pub(crate) PaneSendTextProjection for PaneSendTextParams {
        pane_id => Field::new(String::new()).input(binding::pane_id()),
        text => Field::new(String::new()).input(binding::joined("text", "TEXT")),
        }
}

cli_projection! {
    pub(crate) PaneSendKeysProjection for PaneSendKeysParams {
        pane_id => Field::new(String::new()).input(binding::pane_id()),
        keys => Field::new(Vec::new()).input(binding::keys()),
        }
}

cli_projection! {
    pub(crate) PaneRunProjection for PaneSendInputParams {
        pane_id => Field::new(String::new()).input(binding::pane_id()),
        text => Field::new(String::new()).input(binding::joined("command", "COMMAND")),
        keys => Field::new(vec![String::from("Enter")]),
        }
}

cli_projection! {
    pub(crate) PaneReadProjection for PaneReadParams {
        pane_id => Field::new(String::new()).input(binding::pane_id()),
        source => Field::new(ReadSource::Recent).input(binding::read_source(true)),
        lines => Field::new(None).input(binding::lines()),
        format => binding::read_format(),
        strip_ansi => Field::new(true).input(binding::boolean("raw", false)),

        intent => Field::new(super::common::ReadIntent::Interactive),

        strip_dim => Field::new(false).input(binding::boolean("strip-dim", true).help(crate::cli::fork_help::STRIP_DIM)),
    }
}

cli_projection! {
    pub(crate) PaneReportAgentProjection for PaneReportAgentParams {
        pane_id => Field::new(String::new()).input(binding::pane_id()),
        source => Field::new(String::new()).input(binding::source()),
        agent => Field::new(String::new()).input(binding::text("agent", "LABEL").required()),
        state => Field::new(PaneAgentState::Unknown).input(binding::state()),
        message => Field::new(None).input(binding::optional_text("message", "TEXT")),
        seq => Field::new(None).input(binding::number("seq", "N")),
        agent_session_id => Field::new(None).input(binding::optional_text("agent-session-id", "ID")),
        agent_session_path => Field::new(None).input(binding::optional_text("agent-session-path", "PATH").path()),


        resume_argv => Field::new(None).input(binding::resume()),
        }
}

cli_projection! {
    pub(crate) PaneReportSessionProjection for PaneReportAgentSessionParams {
        pane_id => Field::new(String::new()).input(binding::pane_id()),
        source => Field::new(String::new()).input(binding::source()),
        agent => Field::new(String::new()).input(binding::text("agent", "LABEL").required()),
        seq => Field::new(None).input(binding::number("seq", "N")),
        agent_session_id => Field::new(None).input(binding::optional_text("agent-session-id", "ID")),
        agent_session_path => Field::new(None).input(binding::optional_text("agent-session-path", "PATH").path()),
        session_start_source => Field::new(None).input(binding::optional_text("session-start-source", "SOURCE")),


        resume_argv => Field::new(None).input(binding::resume()),
        }
}

cli_projection! {
    pub(crate) PaneMetadataProjection for PaneReportMetadataParams {
        pane_id => Field::new(String::new()).input(binding::pane_id()),
        source => Field::new(String::new()).input(binding::source()),
        agent => Field::new(None).input(binding::optional_text("agent", "LABEL")),
        applies_to_source => Field::new(None).input(binding::optional_text("applies-to-source", "ID")),
        title => Field::new(None).input(binding::optional_text("title", "TEXT").configure(|arg| arg.conflicts_with("clear-title"))),
        display_agent => Field::new(None).input(binding::optional_text("display-agent", "TEXT").configure(|arg| arg.conflicts_with("clear-display-agent"))),
        state_labels => Field::new(HashMap::new()).input(binding::state_label().configure(|arg| arg.conflicts_with("clear-state-labels"))),

        tokens => Field::new(HashMap::new()).input(binding::token()).input(binding::clear_token()),
        clear_title => Field::new(false).input(binding::boolean("clear-title", true)),
        clear_display_agent => Field::new(false).input(binding::boolean("clear-display-agent", true)),
        clear_state_labels => Field::new(false).input(binding::boolean("clear-state-labels", true)),
        seq => Field::new(None).input(binding::number("seq", "N")),

        ttl_ms => Field::new(None).input(binding::number("ttl-ms", "N")),
        }
}

cli_projection! {
    pub(crate) PaneReleaseProjection for PaneReleaseAgentParams {
        pane_id => Field::new(String::new()).input(binding::pane_id()),
        source => Field::new(String::new()).input(binding::source()),
        agent => Field::new(String::new()).input(binding::text("agent", "LABEL").required()),
        seq => Field::new(None).input(binding::number("seq", "N")),
        }
}
