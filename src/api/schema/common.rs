use serde::{Deserialize, Serialize};

pub(super) fn metadata_token_patch_schema(
    _generator: &mut schemars::SchemaGenerator,
) -> schemars::Schema {
    schemars::json_schema!({
        "type": "object",
        "maxProperties": 16,
        "propertyNames": { "pattern": "^[A-Za-z0-9_-]{1,32}$" },
        "additionalProperties": { "type": ["string", "null"] }
    })
}

pub(super) fn metadata_token_values_schema(
    _generator: &mut schemars::SchemaGenerator,
) -> schemars::Schema {
    schemars::json_schema!({
        "type": "object",
        "maxProperties": 32,
        "propertyNames": { "pattern": "^[A-Za-z0-9_-]{1,32}$" },
        "additionalProperties": { "type": "string" }
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct EmptyParams {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct WorkspaceTarget {
    pub workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneTarget {
    pub pane_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProductAnnouncementDismissParams {
    pub version: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ReleaseNotesDismissParams {
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TabTarget {
    pub tab_id: String,
}

/// `pane.close`. Without `force`, a close that would drop open todos or a
/// worktree group is refused with `confirmation_required`; with it, the pane
/// closes and the result lists the todos it dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneCloseParams {
    pub pane_id: String,
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub force: bool,
}

/// `pane.respawn` and `pane.respawn_agent`. Without `force`, a respawn that
/// would stop a running process or that leaves open todos behind is refused
/// with `confirmation_required`; with it, the pane respawns and the result says
/// what it went ahead over. `pane.respawn_agent` starts the pane's agent again
/// from its saved session, as a restore does, and is refused with
/// `no_agent_session` when the pane has none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneRespawnParams {
    pub pane_id: String,
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub force: bool,
}

/// `pane.respawn_if_idle`: moves a pane's program onto the primary
/// herdr-core session only while the pane is idle (core-phase3 D9), deciding
/// from what the server sees in one turn. With `check_only`, it only says
/// whether the pane would move.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneRespawnIfIdleParams {
    pub pane_id: String,
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub check_only: bool,
}

/// The program a guarded pane action is for (core-phase5 D7, fork issue
/// 316): the pane's terminal in the server's primary herdr-core session, as
/// the `herdr-app` document names it (`t7`), the pid core reported for its
/// program, and the `herdr.launch` label of the run that started it. The
/// server compares all three with the pane it resolves, in the same turn as
/// the action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneTerminalExpectation {
    pub terminal: String,
    pub pid: u32,
    pub launch: String,
}

/// `pane.close_if_terminal`: `pane.close` of a pane that still shows the
/// program `expect` names, and only of that one pane. A close that would
/// close its workspace (the pane is the workspace's last), and with it a
/// worktree group, is refused with `would_close_workspace`; a pane that
/// shows another program is refused with `terminal_mismatch`. Its own
/// method, so an older server answers that it does not know it instead of
/// closing whatever pane has the id now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneCloseIfTerminalParams {
    pub pane_id: String,
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub force: bool,
    pub expect: PaneTerminalExpectation,
}

/// `pane.split_if_terminal`: `pane.split` of the pane `pane_id` while it
/// still shows the program `expect` names. The new pane starts in the
/// target pane's folder and is not focused.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneSplitIfTerminalParams {
    pub pane_id: String,
    pub direction: SplitDirection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ratio: Option<f32>,
    pub expect: PaneTerminalExpectation,
}

/// `pane.rename_if_terminal`: `pane.rename` of a pane that still shows the
/// program `expect` names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PaneRenameIfTerminalParams {
    pub pane_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub expect: PaneTerminalExpectation,
}

/// Why `pane.respawn_if_idle` did not move a pane (core-phase3 D9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaneMoveRefusal {
    /// The pane already runs on the primary herdr-core session.
    AlreadyOnPrimary,
    /// A replacement would not start on herdr-core: `runtime` is not
    /// `core`, or herdr gave up on core.
    CoreOff,
    /// The pane matches `[experimental] core_pty_panes`.
    Excluded,
    /// Suspended, waiting for herdr-core, an overlay, starting again, or
    /// being stopped by a worktree removal.
    NotMovable,
    /// Input reached the pane in the last 60 s, or a Submit is still queued.
    RecentInput,
    /// The pane's agent has no manifest move facts.
    UnsupportedAgent,
    /// The agent has not been idle for 60 s.
    AgentBusy,
    /// The agent shows background work.
    BackgroundWork,
    /// The agent's input box is not empty, or not visible.
    DraftInBox,
    /// herdr has no resume plan for the agent.
    NoResumePlan,
    /// The terminal's foreground process is not the agent, or a process in
    /// the pane's session is outside the agent's process tree.
    ForeignProcess,
    /// The shell is not at its prompt, or runs more than itself.
    ShellBusy,
    /// Input reached the shell pane in the last 10 minutes.
    ShellRecent,
    /// herdr-core did not confirm the old program ended; nothing changed.
    CoreUnreachable,
    /// The directory the replacement would start in no longer exists. The
    /// check runs before any signal, so the program keeps running.
    CwdMissing,
    /// The pane's program runs on a herdr-core machine: a move would end
    /// it there and start its replacement on this host (core-phase5 D5).
    OnMachine,
    /// A reason a newer server gives that this client does not know.
    #[serde(other)]
    Unknown,
}

/// `tab.close`, with the same `force` as `pane.close`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TabCloseParams {
    pub tab_id: String,
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentTarget {
    pub target: String,
}

crate::cli::projection::cli_params! {
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
    pub struct ClientWindowTitleSetParams => ClientWindowTitleSetProjection {
        pub title: String => crate::cli::projection::Field::new(String::new()).input(
            crate::cli::projection::Input::positional("title", 0, |title, value| {
                *title = value.to_owned();
                Ok(())
            }).value_name("TITLE").verbatim()
        ),
    }
}

/// Updates whether the requesting client shell receives and controls pane presentation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ClientShellSurfaceSetParams {
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SplitDirection {
    Right,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReadSource {
    Visible,
    Recent,
    RecentUnwrapped,
    Detection,
}

pub(crate) const READ_SOURCES: &[crate::cli::projection::Choice<ReadSource>] = &[
    crate::cli::projection::Choice::new("visible", ReadSource::Visible),
    crate::cli::projection::Choice::new("recent", ReadSource::Recent),
    crate::cli::projection::Choice::new("recent-unwrapped", ReadSource::RecentUnwrapped),
    crate::cli::projection::Choice::hidden("recent_unwrapped", ReadSource::RecentUnwrapped),
    crate::cli::projection::Choice::new("detection", ReadSource::Detection),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ReadIntent {
    #[default]
    Interactive,
    Passive,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum ReadFormat {
    #[default]
    Text,
    Ansi,
}

pub(crate) const READ_FORMATS: &[crate::cli::projection::Choice<ReadFormat>] = &[
    crate::cli::projection::Choice::new("text", ReadFormat::Text),
    crate::cli::projection::Choice::new("ansi", ReadFormat::Ansi),
];

crate::cli::projection::cli_params! {
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
    pub struct NotificationShowParams => NotificationShowProjection {
        pub title: String => notification_cli::title(),
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub body: Option<String> => notification_cli::body(),
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub position: Option<crate::config::ToastHerdrPosition> => notification_cli::position(),
        #[serde(default, skip_serializing_if = "NotificationShowSound::is_none")]
        pub sound: NotificationShowSound => notification_cli::sound(),
    }
}

mod notification_cli {
    use super::NotificationShowSound;
    use crate::cli::projection::{Field, Input, ParseError};
    use crate::config::ToastHerdrPosition;

    const POSITIONS: [(&str, ToastHerdrPosition); 5] = [
        ("top-left", ToastHerdrPosition::TopLeft),
        ("top-right", ToastHerdrPosition::TopRight),
        ("bottom-left", ToastHerdrPosition::BottomLeft),
        ("bottom-right", ToastHerdrPosition::BottomRight),
        ("center", ToastHerdrPosition::Center),
    ];
    const SOUNDS: [(&str, NotificationShowSound); 3] = [
        ("none", NotificationShowSound::None),
        ("done", NotificationShowSound::Done),
        ("request", NotificationShowSound::Request),
    ];

    pub(super) fn title() -> Field<String> {
        Field::new(String::new()).input(
            Input::positional("title", 0, |slot, value| {
                *slot = value.to_owned();
                Ok(())
            })
            .value_name("TITLE")
            .verbatim(),
        )
    }

    pub(super) fn body() -> Field<Option<String>> {
        Field::new(None)
            .input(crate::cli::workspace_bindings::optional_text("body", "TEXT").order(0))
    }

    pub(super) fn position() -> Field<Option<ToastHerdrPosition>> {
        Field::new(None).input(Input::value("position", "POSITION", |slot, value| {
            *slot = Some(POSITIONS.iter().find(|(name, _)| *name == value).map(|(_, value)| *value)
                .ok_or_else(|| ParseError::Usage(format!("invalid position: {value} (expected top-left, top-right, bottom-left, or bottom-right)")))?);
            Ok(())
        }).configure(|arg| arg.value_parser(clap::builder::PossibleValuesParser::new(POSITIONS.iter().map(|(name, _)| *name)))).order(1))
    }

    pub(super) fn sound() -> Field<NotificationShowSound> {
        Field::new(NotificationShowSound::None).input(
            Input::value("sound", "SOUND", |slot, value| {
                *slot = SOUNDS
                    .iter()
                    .find(|(name, _)| *name == value)
                    .map(|(_, value)| *value)
                    .ok_or_else(|| {
                        ParseError::Usage(format!(
                            "invalid sound: {value} (expected none, done, or request)"
                        ))
                    })?;
                Ok(())
            })
            .configure(|arg| {
                arg.value_parser(clap::builder::PossibleValuesParser::new(
                    SOUNDS.iter().map(|(name, _)| *name),
                ))
            })
            .order(2),
        )
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum NotificationShowSound {
    #[default]
    None,
    Done,
    Request,
}

impl NotificationShowSound {
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotificationShowReason {
    Shown,
    Disabled,
    RateLimited,
    NoForegroundClient,
    Busy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClientWindowTitleReason {
    Set,
    Cleared,
    NoForegroundClient,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaneAgentState {
    Idle,
    Working,
    Blocked,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    Unknown,
}

pub(crate) fn default_true() -> bool {
    true
}
