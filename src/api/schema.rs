use serde::{Deserialize, Serialize};

pub mod agents;
pub mod clients;
pub mod commands;
pub mod common;
pub mod events;
pub mod integrations;
pub mod notifications;
pub(crate) mod operations;
pub mod panes;
pub mod plugins;
pub mod response;
pub mod server;
pub mod session;
pub mod tabs;
pub mod tasks;
pub mod todos;
pub mod workspaces;
pub mod worktrees;

pub use agents::*;
pub use clients::*;
pub use commands::*;
pub use common::*;
pub use events::*;
pub use integrations::*;
pub use notifications::*;
pub use panes::*;
pub use plugins::*;
pub use response::*;
pub use server::*;
pub use session::*;
pub use tabs::*;
pub use tasks::*;
pub use todos::*;
pub use workspaces::*;
pub use worktrees::*;

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Request {
    pub id: String,
    #[serde(flatten)]
    pub method: Method,
}

// Variant order and parameter types preserve the existing wire contract.
// The two flags are independent: presentation impact, then client-shell admission.
operations::define_operations! {
    Ping(PingParams) => "ping", false, false;
    ServerStop(EmptyParams) => "server.stop", false, false;
    ServerLiveHandoff(ServerLiveHandoffParams) => "server.live_handoff", false, false;
    ServerReloadConfig(EmptyParams) => "server.reload_config", true, true;
    ServerSshAgentRegister(ServerSshAgentRegisterParams) => "server.ssh_agent.register", false, false;
    ServerAgentManifests(EmptyParams) => "server.agent_manifests", false, false;
    ServerReloadAgentManifests(EmptyParams) => "server.reload_agent_manifests", true, false;
    NotificationShow(NotificationShowParams) => "notification.show", true, false;
    ProductAnnouncementDismiss(ProductAnnouncementDismissParams) => "product_announcement.dismiss", true, true;
    ReleaseNotesDismiss(ReleaseNotesDismissParams) => "release_notes.dismiss", true, true;
    CommandInvoke(CommandInvokeParams) => "command.invoke", true, true;
    NotificationList(EmptyParams) => "notification.list", false, true;
    NotificationMarkSeen(NotificationMarkSeenParams) => "notification.mark_seen", true, true;
    NotificationClear(EmptyParams) => "notification.clear", true, true;
    TodoList(TodoListParams) => "todo.list", false, true;
    TodoAdd(TodoAddParams) => "todo.add", false, true;
    TodoUpdate(TodoUpdateParams) => "todo.update", false, true;
    TodoRemove(TodoRemoveParams) => "todo.remove", false, true;
    TodoClear(TodoClearParams) => "todo.clear", false, true;
    ClientWindowTitleSet(ClientWindowTitleSetParams) => "client.window_title.set", false, false;
    ClientWindowTitleClear(EmptyParams) => "client.window_title.clear", false, false;
    ClientList(EmptyParams) => "client.list", false, false;
    ClientShellSurfaceSet(ClientShellSurfaceSetParams) => "client_shell.surface.set", false, true;
    SessionSnapshot(EmptyParams) => "session.snapshot", false, false;
    WorkspaceCreate(WorkspaceCreateParams) => "workspace.create", true, true;
    WorkspaceList(EmptyParams) => "workspace.list", false, false;
    WorkspaceGet(WorkspaceTarget) => "workspace.get", false, false;
    WorkspaceFocus(WorkspaceTarget) => "workspace.focus", true, true;
    WorkspacePin(WorkspaceTarget) => "workspace.pin", true, true;
    WorkspaceUnpin(WorkspaceTarget) => "workspace.unpin", true, true;
    WorkspaceRename(WorkspaceRenameParams) => "workspace.rename", true, true;
    WorkspaceMove(WorkspaceMoveParams) => "workspace.move", true, true;
    WorkspaceMoveBlock(WorkspaceMoveBlockParams) => "workspace.move_block", true, true;
    WorkspaceReportMetadata(WorkspaceReportMetadataParams) => "workspace.report_metadata", true, false;
    WorkspaceClose(WorkspaceCloseParams) => "workspace.close", true, true;
    WorktreeList(WorktreeListParams) => "worktree.list", false, true;
    WorktreeCreate(WorktreeCreateParams) => "worktree.create", true, true;
    WorktreeOpen(WorktreeOpenParams) => "worktree.open", true, true;
    WorktreeRemove(WorktreeRemoveParams) => "worktree.remove", true, true;
    TabCreate(TabCreateParams) => "tab.create", true, true;
    TabList(TabListParams) => "tab.list", false, false;
    TabGet(TabTarget) => "tab.get", false, false;
    TabFocus(TabTarget) => "tab.focus", true, true;
    TabRename(TabRenameParams) => "tab.rename", true, true;
    TabSync(TabSyncParams) => "tab.sync", true, true;
    TabMove(TabMoveParams) => "tab.move", true, true;
    TabClose(TabCloseParams) => "tab.close", true, true;
    AgentList(AgentListParams) => "agent.list", false, false;
    AgentGet(AgentTarget) => "agent.get", false, false;
    AgentRead(AgentReadParams) => "agent.read", false, false;
    AgentExplain(AgentTarget) => "agent.explain", false, false;
    AgentSendKeys(AgentSendKeysParams) => "agent.send_keys", true, false;
    AgentRename(AgentRenameParams) => "agent.rename", true, false;
    AgentViewSet(AgentViewSetParams) => "agent.view.set", true, false;
    AgentViewClear(AgentViewClearParams) => "agent.view.clear", true, false;
    AgentFocus(AgentTarget) => "agent.focus", true, false;
    AgentPin(AgentTarget) => "agent.pin", true, true;
    AgentUnpin(AgentTarget) => "agent.unpin", true, true;
    AgentStart(AgentStartParams) => "agent.start", true, false;
    AgentPrompt(AgentPromptParams) => "agent.prompt", true, false;
    AgentMessage(AgentMessageParams) => "agent.message", true, true;
    AgentWait(AgentWaitParams) => "agent.wait", false, false;
    PaneSplit(PaneSplitParams) => "pane.split", true, true;
    PaneSwap(PaneSwapParams) => "pane.swap", true, true;
    PaneMove(PaneMoveParams) => "pane.move", true, true;
    PaneZoom(PaneZoomParams) => "pane.zoom", true, true;
    PaneSync(PaneSyncParams) => "pane.sync", true, true;
    PaneSyncPair(PaneSyncPairParams) => "pane.sync_pair", true, true;
    PaneLayout(PaneLayoutParams) => "pane.layout", false, false;
    PaneProcessInfo(PaneProcessInfoParams) => "pane.process_info", false, false;
    LayoutExport(LayoutExportParams) => "layout.export", false, false;
    LayoutApply(LayoutApplyParams) => "layout.apply", true, false;
    LayoutSetSplitRatio(LayoutSetSplitRatioParams) => "layout.set_split_ratio", true, true;
    LayoutBalance(LayoutBalanceParams) => "layout.balance", true, true;
    LayoutSetPreset(LayoutSetPresetParams) => "layout.set_preset", true, true;
    LayoutApplyPreset(LayoutApplyPresetParams) => "layout.apply_preset", true, true;
    PaneNeighbor(PaneNeighborParams) => "pane.neighbor", false, false;
    PaneEdges(PaneEdgesParams) => "pane.edges", false, false;
    PaneFocusDirection(PaneFocusDirectionParams) => "pane.focus_direction", true, true;
    PaneResize(PaneResizeParams) => "pane.resize", true, true;
    PaneScroll(PaneScrollParams) => "pane.scroll", true, true;
    PaneScrollApplication(PaneScrollApplicationParams) => "pane.scroll_application", true, true;
    PaneClear(PaneTarget) => "pane.clear", true, true;
    PaneEditScrollback(PaneTarget) => "pane.edit_scrollback", true, true;
    PaneSelectionRead(PaneSelectionReadParams) => "pane.selection.read", false, true;
    PaneCopyMotion(PaneCopyMotionParams) => "pane.copy_motion", false, true;
    PaneCopySearch(PaneCopySearchParams) => "pane.copy_search", false, true;
    PaneList(PaneListParams) => "pane.list", false, false;
    PaneCurrent(PaneCurrentParams) => "pane.current", false, false;
    PaneGet(PaneTarget) => "pane.get", false, false;
    PaneFocus(PaneTarget) => "pane.focus", true, true;
    PaneInputSet(PaneInputSetParams) => "pane.input.set", true, true;
    PaneLinkActivate(PaneLinkActivateParams) => "pane.link.activate", false, true;
    PaneLinkResolve(PaneLinkActivateParams) => "pane.link.resolve", false, true;
    PaneRename(PaneRenameParams) => "pane.rename", true, true;
    PaneSendText(PaneSendTextParams) => "pane.send_text", false, false;
    PaneSendKeys(PaneSendKeysParams) => "pane.send_keys", false, false;
    PaneSendInput(PaneSendInputParams) => "pane.send_input", false, false;
    PaneRead(PaneReadParams) => "pane.read", false, false;
    PaneReportAgent(PaneReportAgentParams) => "pane.report_agent", true, false;
    PaneReportAgentSession(PaneReportAgentSessionParams) => "pane.report_agent_session", true, false;
    PaneReportMetadata(PaneReportMetadataParams) => "pane.report_metadata", true, false;
    PaneReportHint(PaneReportHintParams) => "pane.report_hint", true, false;
    PaneClearAgentAuthority(PaneClearAgentAuthorityParams) => "pane.clear_agent_authority", true, false;
    PaneReleaseAgent(PaneReleaseAgentParams) => "pane.release_agent", true, false;
    PaneClearScrollback(PaneTarget) => "pane.clear_scrollback", true, true;
    PaneClose(PaneCloseParams) => "pane.close", true, true;
    PaneRespawn(PaneRespawnParams) => "pane.respawn", true, true;
    PopupClose(EmptyParams) => "popup.close", true, false;
    EventsSubscribe(EventsSubscribeParams) => "events.subscribe", false, false;
    EventsWait(EventsWaitParams) => "events.wait", false, false;
    PaneWaitForOutput(PaneWaitForOutputParams) => "pane.wait_for_output", false, false;
    IntegrationList(EmptyParams) => "integration.list", false, true;
    IntegrationInstall(IntegrationInstallParams) => "integration.install", false, true;
    IntegrationUninstall(IntegrationUninstallParams) => "integration.uninstall", false, false;
    PluginLink(PluginLinkParams) => "plugin.link", false, false;
    PluginList(PluginListParams) => "plugin.list", false, false;
    PluginUnlink(PluginUnlinkParams) => "plugin.unlink", true, false;
    PluginEnable(PluginSetEnabledParams) => "plugin.enable", false, false;
    PluginDisable(PluginSetEnabledParams) => "plugin.disable", true, false;
    PluginActionList(PluginActionListParams) => "plugin.action.list", false, false;
    PluginActionInvoke(PluginActionInvokeParams) => "plugin.action.invoke", true, false;
    PluginLogList(PluginLogListParams) => "plugin.log.list", false, false;
    PluginPaneOpen(PluginPaneOpenParams) => "plugin.pane.open", true, false;
    PluginPaneFocus(PluginPaneFocusParams) => "plugin.pane.focus", true, false;
    PluginPaneClose(PluginPaneCloseParams) => "plugin.pane.close", true, false;
    PluginPopupClose(PluginPopupCloseParams) => "plugin.popup.close", true, false;
    // Fork: a stop whose attached clients wait for the server the caller starts
    // next (https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/227).
    ServerStopForRestart(EmptyParams) => "server.stop_for_restart", false, false;
    // Fork: a respawn that resumes the pane's agent session. Its own method so
    // an older server answers "unknown method" instead of a plain respawn that
    // stops the agent (https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/238).
    PaneRespawnAgent(PaneRespawnParams) => "pane.respawn_agent", true, false;
    // Fork: the task store (https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/241).
    // New names beside `todo.*`, which keep their meaning until they retire.
    TaskList(TaskListParams) => "task.list", false, true;
    TaskShow(TaskTarget) => "task.show", false, true;
    TaskAdd(TaskAddParams) => "task.add", false, true;
    TaskUpdate(TaskUpdateParams) => "task.update", false, true;
    TaskDone(TaskMoveStatusParams) => "task.done", false, true;
    TaskDrop(TaskMoveStatusParams) => "task.drop", false, true;
    TaskReopen(TaskMoveStatusParams) => "task.reopen", false, true;
    TaskBind(TaskBindParams) => "task.bind", false, true;
    TaskConflicts(EmptyParams) => "task.conflicts", false, true;
    // Not advertised to the client shell: the CLI polls it for a write's outcome.
    TaskWriteStatus(TaskWriteStatusParams) => "task.write_status", false, false;
    TaskResolveConflict(TaskResolveConflictParams) => "task.resolve_conflict", false, true;
    // Read-only views of the buckets worklink and the gestore fill (issue 241, 2.13).
    IssueList(IssueListParams) => "issue.list", false, true;
    AskList(AskListParams) => "ask.list", false, true;
}

#[cfg(test)]
mod tests;
