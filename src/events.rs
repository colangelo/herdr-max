//! Internal app events delivered via channel.
//!
//! Background tasks (PTY child watchers, future hook listeners, etc.) send
//! events to the main loop through this channel. No polling needed.

use std::time::Instant;

use crate::detect::{Agent, AgentState};
use crate::layout::PaneId;
use crate::workspace::{GitStatusCacheEntry, WorkspaceGitStatus};

#[derive(Debug)]
pub struct ApiWorktreeAddRequest {
    pub id: String,
    pub operation_id: u64,
    pub checkout_key: std::path::PathBuf,
    pub source_workspace_id: Option<String>,
    pub source_existing_membership: Option<crate::workspace::WorktreeSpaceMembership>,
    pub source_checkout_path: std::path::PathBuf,
    pub source_repo_root: std::path::PathBuf,
    pub repo_key: String,
    pub repo_name: String,
    pub label: Option<String>,
    pub focus: bool,
    pub respond_to: std::sync::mpsc::Sender<String>,
}

#[derive(Debug)]
pub struct WorktreeAddResult {
    pub path: std::path::PathBuf,
    pub api_request: Option<ApiWorktreeAddRequest>,
    pub result: Result<(), String>,
}

#[derive(Debug)]
pub struct ApiWorktreeRemoveRequest {
    pub id: String,
    pub operation_id: u64,
    pub checkout_key: std::path::PathBuf,
    pub shutdown_panes: Vec<crate::layout::PaneId>,
    pub respond_to: std::sync::mpsc::Sender<String>,
}

#[derive(Debug)]
pub struct WorktreeRemoveResult {
    pub workspace_id: String,
    pub path: std::path::PathBuf,
    pub workspace: Option<Box<crate::api::schema::WorkspaceInfo>>,
    pub worktree: Option<Box<crate::api::schema::WorktreeInfo>>,
    pub forced: bool,
    pub api_request: Option<ApiWorktreeRemoveRequest>,
    pub result: Result<(), String>,
}

#[derive(Debug)]
pub struct WorktreeReadResult {
    // Keep the slot until completion is consumed, including time queued on the app loop.
    pub(crate) _permit: tokio::sync::OwnedSemaphorePermit,
    pub(crate) client_local: bool,
    pub(crate) request: crate::api::schema::Request,
    pub(crate) source_workspace_id: Option<String>,
    pub(crate) source_cwd: Option<std::path::PathBuf>,
    pub(crate) result: Result<WorktreeReadData, (String, String)>,
    pub(crate) respond_to: std::sync::mpsc::Sender<String>,
}

#[derive(Debug)]
pub(crate) struct WorktreeReadData {
    pub source_checkout_path: std::path::PathBuf,
    pub source_repo_root: std::path::PathBuf,
    pub repo_key: String,
    pub repo_name: String,
    pub entries: Vec<crate::worktree::ExistingWorktree>,
}

/// An event from a background task to the main loop.
#[derive(Debug)]
pub enum AppEvent {
    /// A pane's child process exited.
    PaneDied {
        pane_id: PaneId,
        exit_reason: crate::platform::ChildExitReason,
    },
    /// A worktree-removal runtime could not be restored normally.
    WorktreeRuntimeRestoreFailed { pane_id: PaneId, operation_id: u64 },
    /// Process detection identified an agent before its screen state was confirmed.
    AgentProcessDetected {
        pane_id: PaneId,
        agent: Agent,
        observed_at: Instant,
        /// A new process of the agent the pane already had, not the first
        /// sighting of one: a restart, seen as an exit followed by the new
        /// process or as the agent's process group changing between polls.
        replaced_process: bool,
    },
    /// The command line of the agent process detection identified in a pane,
    /// read once per agent process so a restore can give it back its launch
    /// flags (fork issues 123, 127). `None` when the OS would not say.
    AgentLaunchObserved {
        pane_id: PaneId,
        agent: Agent,
        launch: Option<crate::agent_resume::AgentLaunchArgv>,
    },
    /// The current Codex input screen is visible during managed startup.
    CodexPromptObserved { pane_id: PaneId, ready: bool },
    /// Fallback detector state changed in a pane.
    StateChanged {
        pane_id: PaneId,
        agent: Option<Agent>,
        state: AgentState,
        visible_blocker: bool,
        visible_working: bool,
        /// Working because of work the agent launched, not work it is doing;
        /// see `crate::detect::AgentDetection::background_work`.
        background_work: bool,
        /// Why the state is Blocked, from the matched detection rule; see
        /// `crate::detect::AgentDetection::blocked_reason`.
        blocked_reason: Option<crate::detect::BlockedReason>,
        process_exited: bool,
        observed_at: Instant,
    },
    /// Hook-authoritative agent state was reported for a pane.
    HookStateReported {
        pane_id: PaneId,
        source: String,
        agent_label: String,
        state: AgentState,
        message: Option<String>,
        seq: Option<u64>,
        session_ref: Option<crate::agent_resume::AgentSessionRef>,
    },
    /// A Codex naming job resolved the daemon thread a pane runs (naming
    /// runs on unix only).
    #[cfg(unix)]
    CodexThreadResolved { pane_id: PaneId, thread_id: String },
    /// Agent session identity was reported without state authority.
    AgentSessionReported {
        pane_id: PaneId,
        source: String,
        agent_label: String,
        seq: Option<u64>,
        session_ref: Option<crate::agent_resume::AgentSessionRef>,
        session_start_source: Option<String>,
    },
    /// A reporter supplied the command that resumes its own session.
    AgentResumeReported {
        pane_id: PaneId,
        source: String,
        agent_label: String,
        seq: Option<u64>,
        argv: Vec<String>,
    },
    /// A pane held by a self-reported agent is back at its idle shell.
    ReportedAgentShellReturned {
        pane_id: PaneId,
        observed_at: std::time::Instant,
    },
    /// Display-only agent metadata was reported for a pane.
    HookMetadataReported {
        pane_id: PaneId,
        source: String,
        agent_label: Option<String>,
        applies_to_source: Option<String>,
        title: Option<String>,
        display_agent: Option<String>,
        state_labels: std::collections::HashMap<String, String>,
        clear_title: bool,
        clear_display_agent: bool,
        clear_state_labels: bool,
        seq: Option<u64>,
        ttl: Option<std::time::Duration>,
    },
    /// A source reported (or ended) a hint that the agent waits on the user.
    AgentHintReported {
        pane_id: PaneId,
        report: crate::terminal::AgentHintReport,
    },
    /// A pane's hint is due to be dropped (its time is up, or the screen no
    /// longer backs it).
    AgentHintExpired {
        pane_id: PaneId,
        now: std::time::Instant,
    },
    /// Hook authority was explicitly cleared for a pane.
    HookAuthorityCleared {
        pane_id: PaneId,
        source: Option<String>,
        seq: Option<u64>,
    },
    /// The current detected agent gracefully released this pane back to the shell.
    HookAgentReleased {
        pane_id: PaneId,
        source: String,
        agent_label: String,
        known_agent: Option<Agent>,
        seq: Option<u64>,
    },
    /// A new version is available through the active installation manager.
    UpdateReady {
        version: String,
        install_command: String,
    },
    /// Remote agent detection manifest update check finished.
    AgentDetectionManifestsUpdated {
        updated: Vec<crate::detect::manifest_update::ManifestUpdateCommit>,
        activated: Vec<crate::detect::Agent>,
        status: crate::detect::manifest_update::ManifestUpdateStatus,
    },
    /// A pane child emitted one or more executable BEL characters.
    /// The host-facing process forwards them to its outer terminal.
    TerminalBell { pane_id: PaneId, count: u16 },
    /// A pane child emitted a valid OSC 52 clipboard write, or herdr copied
    /// text itself. The main loop re-emits it through herdr's own clipboard
    /// writer. `source_pane` is the pane the text came from, when there is
    /// one, so the feedback can show there (fork issue 129).
    ClipboardWrite {
        content: Vec<u8>,
        source_pane: Option<PaneId>,
    },
    /// A pane child reported its shell current directory through terminal
    /// metadata such as OSC 7.
    TerminalCwdReported {
        pane_id: PaneId,
        cwd: std::path::PathBuf,
    },
    /// Background git status refresh completed for workspaces.
    GitStatusRefreshed {
        results: Vec<WorkspaceGitStatus>,
        cache_updates: Vec<(std::path::PathBuf, GitStatusCacheEntry)>,
    },
    /// Background validation of a saved membership after session restore.
    RestoredWorktreeSpaceChecked {
        workspace_id: String,
        expected: crate::workspace::WorktreeSpaceMembership,
        valid: bool,
    },
    /// A configured tab bar status command finished.
    TabBarCommandFinished {
        generation: u64,
        segment_index: usize,
        result: Result<Option<String>, String>,
    },
    /// A plugin action or event command finished.
    PluginCommandFinished {
        log_id: String,
        finished_unix_ms: u64,
        exit_code: Option<i32>,
        stdout: String,
        stderr: String,
        error: Option<String>,
    },
    /// Background `git worktree add` completed.
    WorktreeAddFinished(Box<WorktreeAddResult>),
    /// Background `git worktree remove` completed.
    WorktreeRemoveFinished(Box<WorktreeRemoveResult>),
    /// Background worktree discovery completed for an API list/open request.
    WorktreeReadFinished(Box<WorktreeReadResult>),
}
