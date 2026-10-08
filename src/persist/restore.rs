use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use ratatui::layout::Direction;
use tokio::sync::{mpsc, Notify};
use tracing::{error, warn};

use crate::detect::AgentState;
use crate::events::AppEvent;
use crate::layout::{Node, PaneId, TileLayout};
use crate::pane::{PaneLaunchEnv, PaneState};
use crate::render_signal::RenderSignal;
use crate::terminal::{TerminalId, TerminalRuntime, TerminalState};
use crate::workspace::Workspace;

use super::snapshot::{
    PaneAgentResumeSnapshot, PaneAgentSessionSnapshot, PaneHistorySnapshot, PaneTodoSnapshot,
    TabHistorySnapshot, WorkspaceHistorySnapshot,
};
use super::{
    DirectionSnapshot, LayoutSnapshot, SessionHistorySnapshot, SessionSnapshot, TabSnapshot,
    WorkspaceSnapshot,
};

struct AgentRestoreState<'a> {
    enabled: bool,
    resumed_sessions: &'a mut HashSet<String>,
}

struct PaneRestoreStartup<'a> {
    restore_plan: Option<crate::agent_resume::AgentResumePlan>,
    initial_history_ansi: Option<&'a str>,
    duplicate_agent_session: bool,
    reserved_agent_session: Option<String>,
}

/// A saved todo link waiting for the restore-wide pane id map.
struct PendingTodoLink {
    terminal_id: TerminalId,
    todo_id: u64,
    old_raw_pane: u32,
}

/// Cross-pane todo links cannot be resolved while terminals are built: restore
/// allocates fresh pane ids tab by tab, so a link can target a pane that has not
/// been remapped yet — or one in another workspace entirely. Links are recorded
/// here as terminals are restored and resolved in a single pass afterwards,
/// once every restored tab has contributed its mapping.
#[derive(Default)]
struct TodoLinkRestore {
    id_map: HashMap<u32, PaneId>,
    pending: Vec<PendingTodoLink>,
}

impl TodoLinkRestore {
    /// Publish one tab's remap. Only panes that survived restore are published,
    /// so a link to a dropped pane stays dead instead of pointing at nothing.
    fn register_pane_ids(&mut self, id_map: &HashMap<u32, PaneId>, surviving: &HashSet<PaneId>) {
        for (old_raw, new_id) in id_map {
            if surviving.contains(new_id) {
                self.id_map.insert(*old_raw, *new_id);
            }
        }
    }

    fn resolve(self, terminals: &mut HashMap<TerminalId, TerminalState>) {
        let Self { id_map, pending } = self;
        for link in pending {
            let Some(terminal) = terminals.get_mut(&link.terminal_id) else {
                continue;
            };
            let Some(todo) = terminal
                .todos_mut()
                .iter_mut()
                .find(|todo| todo.id == link.todo_id)
            else {
                continue;
            };
            // The label was installed by `restore_pane_todos` from the
            // snapshot. `capture_pane_todos` always writes `link_pane` and
            // `link_label` together or neither, so a pending link always has a
            // label here. A hand-edited session file carrying a target with no
            // label drops the link rather than inventing one; the todo itself
            // is still kept.
            let label = todo.link.as_ref().map(|link| link.label.clone());
            todo.link = resolve_todo_link(&id_map, Some(link.old_raw_pane), label);
        }
    }
}

/// Remap a saved todo link onto the restored pane ids. A target missing from
/// the map becomes a dead link that keeps its label — never a different pane.
fn resolve_todo_link(
    id_map: &HashMap<u32, PaneId>,
    link_pane: Option<u32>,
    link_label: Option<String>,
) -> Option<crate::terminal::todo::TodoLink> {
    let label = link_label?;
    Some(crate::terminal::todo::TodoLink {
        pane: link_pane.and_then(|raw| id_map.get(&raw).copied()),
        label,
    })
}

/// Install a pane's saved todos on its restored terminal, with every link left
/// unresolved and recorded for [`TodoLinkRestore::resolve`].
fn restore_pane_todos(
    terminal: &mut TerminalState,
    saved_todos: &[PaneTodoSnapshot],
    saved_next_todo_id: u64,
    todo_links: &mut TodoLinkRestore,
) {
    if saved_todos.is_empty() && saved_next_todo_id <= 1 {
        return;
    }
    let todos = saved_todos
        .iter()
        .map(|snap| crate::terminal::todo::PaneTodo {
            id: snap.id,
            text: snap.text.clone(),
            done: snap.done,
            priority: snap.priority,
            // The label lands now so an unresolvable target degrades to a
            // labelled dead link rather than losing what the link meant.
            link: snap
                .link_label
                .clone()
                .map(|label| crate::terminal::todo::TodoLink { pane: None, label }),
            created_at_unix: snap.created_at_unix,
            updated_at_unix: snap.updated_at_unix,
        })
        .collect();
    terminal.restore_todos(todos, saved_next_todo_id);
    for snap in saved_todos {
        if let Some(old_raw_pane) = snap.link_pane {
            todo_links.pending.push(PendingTodoLink {
                terminal_id: terminal.id.clone(),
                todo_id: snap.id,
                old_raw_pane,
            });
        }
    }
}

struct RestoreRuntimeContext<'a> {
    scrollback_limit_bytes: usize,
    shell_config: crate::pane::PaneShellConfig<'a>,
    resume_agents_on_restore: bool,
    events: mpsc::Sender<AppEvent>,
    render_notify: Arc<Notify>,
    render_dirty: Arc<RenderSignal>,
}

type RestoredSession = (
    Vec<Workspace>,
    HashMap<TerminalId, TerminalState>,
    HashMap<TerminalId, TerminalRuntime>,
);
type RestoredWorkspace = (
    Workspace,
    Vec<TerminalState>,
    HashMap<TerminalId, TerminalRuntime>,
);
type RestoredTab = (
    crate::workspace::Tab,
    Vec<TerminalState>,
    HashMap<TerminalId, TerminalRuntime>,
    HashMap<PaneId, u32>,
);
type RestoreFailures<T> = (T, usize);

/// Restore workspaces from a snapshot. Each pane gets a fresh shell in its saved cwd.
pub fn restore(
    snapshot: &SessionSnapshot,
    history: Option<&SessionHistorySnapshot>,
    rows: u16,
    cols: u16,
    scrollback_limit_bytes: usize,
    default_shell: &str,
    shell_mode: crate::config::ShellModeConfig,
    resume_agents_on_restore: bool,
    events: mpsc::Sender<AppEvent>,
    render_notify: Arc<Notify>,
    render_dirty: Arc<RenderSignal>,
) -> RestoredSession {
    let mut imported_panes = HashMap::new();
    restore_with_imports(
        snapshot,
        history,
        rows,
        cols,
        scrollback_limit_bytes,
        crate::pane::PaneShellConfig::new(default_shell, shell_mode),
        resume_agents_on_restore,
        &mut imported_panes,
        events,
        render_notify,
        render_dirty,
    )
}

#[cfg(unix)]
pub fn restore_handoff(
    snapshot: &SessionSnapshot,
    scrollback_limit_bytes: usize,
    default_shell: &str,
    shell_mode: crate::config::ShellModeConfig,
    imports: &mut HashMap<u32, crate::handoff_runtime::ImportedHandoffRuntime>,
    events: mpsc::Sender<AppEvent>,
    render_notify: Arc<Notify>,
    render_dirty: Arc<RenderSignal>,
) -> std::io::Result<RestoredSession> {
    restore_with_imports_strict(
        snapshot,
        None,
        24,
        80,
        scrollback_limit_bytes,
        crate::pane::PaneShellConfig::new(default_shell, shell_mode),
        true,
        imports,
        events,
        render_notify,
        render_dirty,
    )
}

#[cfg(unix)]
pub fn handoff_pane_aliases(
    snapshot: &SessionSnapshot,
    workspaces: &[Workspace],
) -> HashMap<u32, PaneId> {
    let mut aliases = HashMap::new();
    for (ws_snap, workspace) in snapshot.workspaces.iter().zip(workspaces) {
        for (tab_snap, tab) in ws_snap.tabs.iter().zip(&workspace.tabs) {
            let old_ids = collect_snapshot_pane_ids(&tab_snap.layout);
            let new_ids = tab.layout.pane_ids();
            for (old_id, new_id) in old_ids.into_iter().zip(new_ids) {
                if old_id != new_id.raw() {
                    aliases.insert(old_id, new_id);
                }
            }
        }
    }
    aliases
}

/// The former public ids saved in `snapshot`, each pointing at its pane in
/// `workspaces` (restored from `snapshot`, so a pane's raw id may have
/// changed). Spaces are matched by id and panes by their place in the layout,
/// the way the restore built them. An id that names a live pane now is left
/// out: a live id means the pane that has it.
pub fn restored_former_public_ids(
    snapshot: &SessionSnapshot,
    workspaces: &[Workspace],
) -> HashMap<String, PaneId> {
    let live: HashSet<String> = workspaces
        .iter()
        .flat_map(|workspace| {
            workspace
                .public_pane_numbers
                .values()
                .map(|number| crate::workspace::public_pane_id_for_number(&workspace.id, *number))
        })
        .collect();
    let mut aliases = HashMap::new();
    for ws_snap in &snapshot.workspaces {
        let Some(workspace) = workspaces
            .iter()
            .find(|workspace| ws_snap.id.as_deref() == Some(workspace.id.as_str()))
        else {
            continue;
        };
        if ws_snap.tabs.len() != workspace.tabs.len() {
            continue;
        }
        for (tab_snap, tab) in ws_snap.tabs.iter().zip(&workspace.tabs) {
            let old_ids = collect_snapshot_pane_ids(&tab_snap.layout);
            let new_ids = tab.layout.pane_ids();
            if old_ids.len() != new_ids.len() {
                continue;
            }
            for (old_id, new_id) in old_ids.into_iter().zip(new_ids) {
                let Some(pane) = tab_snap.panes.get(&old_id) else {
                    continue;
                };
                for former in &pane.former_public_ids {
                    if !live.contains(former) {
                        aliases.insert(former.clone(), new_id);
                    }
                }
            }
        }
    }
    aliases
}

fn collect_snapshot_pane_ids(node: &LayoutSnapshot) -> Vec<u32> {
    let mut ids = Vec::new();
    collect_snapshot_ids_inner(node, &mut ids);
    ids
}

fn collect_snapshot_ids_inner(node: &LayoutSnapshot, ids: &mut Vec<u32>) {
    match node {
        LayoutSnapshot::Pane(id) => ids.push(*id),
        LayoutSnapshot::Split { first, second, .. } => {
            collect_snapshot_ids_inner(first, ids);
            collect_snapshot_ids_inner(second, ids);
        }
    }
}

fn migrated_public_pane_numbers_by_old_raw(
    snap: &WorkspaceSnapshot,
    next_public_pane_number: &mut usize,
) -> HashMap<u32, usize> {
    let mut public_numbers = snap.public_pane_numbers.clone();
    for tab in &snap.tabs {
        let mut pane_ids = Vec::new();
        collect_layout_snapshot_pane_ids(&tab.layout, &mut pane_ids);
        for old_raw in pane_ids {
            public_numbers.entry(old_raw).or_insert_with(|| {
                let number = *next_public_pane_number;
                *next_public_pane_number += 1;
                number
            });
        }
    }
    public_numbers
}

fn collect_layout_snapshot_pane_ids(node: &LayoutSnapshot, ids: &mut Vec<u32>) {
    match node {
        LayoutSnapshot::Pane(id) => ids.push(*id),
        LayoutSnapshot::Split { first, second, .. } => {
            collect_layout_snapshot_pane_ids(first, ids);
            collect_layout_snapshot_pane_ids(second, ids);
        }
    }
}

#[cfg(unix)]
fn restore_with_imports_strict(
    snapshot: &SessionSnapshot,
    history: Option<&SessionHistorySnapshot>,
    rows: u16,
    cols: u16,
    scrollback_limit_bytes: usize,
    shell_config: crate::pane::PaneShellConfig<'_>,
    resume_agents_on_restore: bool,
    imported_panes: &mut HashMap<u32, crate::handoff_runtime::ImportedHandoffRuntime>,
    events: mpsc::Sender<AppEvent>,
    render_notify: Arc<Notify>,
    render_dirty: Arc<RenderSignal>,
) -> std::io::Result<RestoredSession> {
    let (restored, failed_imports) = restore_with_imports_and_failures(
        snapshot,
        history,
        rows,
        cols,
        scrollback_limit_bytes,
        shell_config,
        resume_agents_on_restore,
        imported_panes,
        events,
        render_notify,
        render_dirty,
    );
    if failed_imports > 0 {
        return Err(std::io::Error::other(format!(
            "handoff failed to restore {failed_imports} imported pane runtime(s)"
        )));
    }
    if !imported_panes.is_empty() {
        return Err(std::io::Error::other(format!(
            "handoff import did not consume {} pane runtime(s)",
            imported_panes.len()
        )));
    }
    Ok(restored)
}

fn restore_with_imports(
    snapshot: &SessionSnapshot,
    history: Option<&SessionHistorySnapshot>,
    rows: u16,
    cols: u16,
    scrollback_limit_bytes: usize,
    shell_config: crate::pane::PaneShellConfig<'_>,
    resume_agents_on_restore: bool,
    imported_panes: &mut HashMap<u32, crate::handoff_runtime::ImportedHandoffRuntime>,
    events: mpsc::Sender<AppEvent>,
    render_notify: Arc<Notify>,
    render_dirty: Arc<RenderSignal>,
) -> RestoredSession {
    restore_with_imports_and_failures(
        snapshot,
        history,
        rows,
        cols,
        scrollback_limit_bytes,
        shell_config,
        resume_agents_on_restore,
        imported_panes,
        events,
        render_notify,
        render_dirty,
    )
    .0
}

fn restore_with_imports_and_failures(
    snapshot: &SessionSnapshot,
    history: Option<&SessionHistorySnapshot>,
    rows: u16,
    cols: u16,
    scrollback_limit_bytes: usize,
    shell_config: crate::pane::PaneShellConfig<'_>,
    resume_agents_on_restore: bool,
    imported_panes: &mut HashMap<u32, crate::handoff_runtime::ImportedHandoffRuntime>,
    events: mpsc::Sender<AppEvent>,
    render_notify: Arc<Notify>,
    render_dirty: Arc<RenderSignal>,
) -> RestoreFailures<RestoredSession> {
    let history = history.filter(|history| {
        let matches = history.layout_fingerprint.is_some()
            && history.layout_fingerprint == super::snapshot::layout_fingerprint(snapshot);
        if !matches {
            tracing::warn!("Ignoring pane history without a matching session layout");
        }
        matches
    });
    let mut workspaces = Vec::new();
    let mut terminals = HashMap::new();
    let mut terminal_runtimes = HashMap::new();
    let mut resumed_agent_sessions = HashSet::new();
    let mut todo_links = TodoLinkRestore::default();
    let mut failed_imports = 0;
    for (idx, ws_snap) in snapshot.workspaces.iter().enumerate() {
        let runtime_context = RestoreRuntimeContext {
            scrollback_limit_bytes,
            shell_config,
            resume_agents_on_restore,
            events: events.clone(),
            render_notify: render_notify.clone(),
            render_dirty: render_dirty.clone(),
        };
        let (restored, workspace_failed_imports) = restore_workspace(
            ws_snap,
            history.and_then(|history| history.workspaces.get(idx)),
            rows,
            cols,
            &runtime_context,
            &mut resumed_agent_sessions,
            imported_panes,
            &mut todo_links,
        );
        failed_imports += workspace_failed_imports;
        if let Some((workspace, restored_terminals, restored_runtimes)) = restored {
            for terminal in restored_terminals {
                terminals.insert(terminal.id.clone(), terminal);
            }
            terminal_runtimes.extend(restored_runtimes);
            workspaces.push(workspace);
        }
    }
    // Every tab has published its remap by now, so cross-pane todo links can
    // finally point at the panes they were saved against.
    todo_links.resolve(&mut terminals);
    crate::workspace::reserve_workspace_ids(&workspaces);
    ((workspaces, terminals, terminal_runtimes), failed_imports)
}

fn restore_workspace(
    snap: &WorkspaceSnapshot,
    history: Option<&WorkspaceHistorySnapshot>,
    rows: u16,
    cols: u16,
    runtime_context: &RestoreRuntimeContext<'_>,
    resumed_agent_sessions: &mut HashSet<String>,
    imported_panes: &mut HashMap<u32, crate::handoff_runtime::ImportedHandoffRuntime>,
    todo_links: &mut TodoLinkRestore,
) -> RestoreFailures<Option<RestoredWorkspace>> {
    let mut tabs = Vec::new();
    let mut terminals = Vec::new();
    let mut terminal_runtimes = HashMap::new();
    let workspace_id = snap
        .id
        .clone()
        .unwrap_or_else(crate::workspace::generate_workspace_id);
    let mut next_public_pane_number = snap
        .public_pane_numbers
        .values()
        .copied()
        .max()
        .and_then(|max| max.checked_add(1))
        .unwrap_or(1)
        .max(snap.next_public_pane_number);
    let public_pane_numbers_by_old_raw =
        migrated_public_pane_numbers_by_old_raw(snap, &mut next_public_pane_number);
    let public_pane_ids_by_old_raw: HashMap<u32, String> = public_pane_numbers_by_old_raw
        .iter()
        .map(|(old_raw, public_number)| {
            (
                *old_raw,
                format!(
                    "{}:p{}",
                    workspace_id,
                    crate::workspace::encode_public_number(*public_number)
                ),
            )
        })
        .collect();
    let mut public_pane_numbers = HashMap::new();
    let mut next_public_tab_number = snap
        .public_tab_numbers
        .iter()
        .copied()
        .max()
        .and_then(|max| max.checked_add(1))
        .unwrap_or(1)
        .max(snap.next_public_tab_number);
    let mut failed_imports = 0;

    for (idx, tab_snap) in snap.tabs.iter().enumerate() {
        let tab_number = snap.public_tab_numbers.get(idx).copied().unwrap_or(idx + 1);
        let (restored_tab, tab_failed_imports) = restore_tab(
            tab_snap,
            history.and_then(|history| history.tabs.get(idx)),
            tab_number,
            &workspace_id,
            rows,
            cols,
            runtime_context,
            resumed_agent_sessions,
            imported_panes,
            &public_pane_ids_by_old_raw,
            todo_links,
        );
        failed_imports += tab_failed_imports;
        let Some((mut tab, restored_terminals, restored_runtimes, reverse_id_map)) = restored_tab
        else {
            continue;
        };
        if let Some(public_tab_number) = snap.public_tab_numbers.get(idx).copied() {
            tab.number = public_tab_number;
        }
        next_public_tab_number = next_public_tab_number.max(tab.number + 1);
        for pane_id in tab.layout.pane_ids() {
            let public_number = public_pane_numbers_by_old_raw
                .get(
                    &reverse_id_map
                        .get(&pane_id)
                        .copied()
                        .unwrap_or(pane_id.raw()),
                )
                .copied()
                .unwrap_or_else(|| {
                    let number = next_public_pane_number;
                    next_public_pane_number += 1;
                    number
                });
            public_pane_numbers.insert(pane_id, public_number);
            next_public_pane_number = next_public_pane_number.max(public_number + 1);
        }
        terminals.extend(restored_terminals);
        terminal_runtimes.extend(restored_runtimes);
        tabs.push(tab);
    }

    if tabs.is_empty() {
        return (None, failed_imports);
    }

    (
        Some(Workspace {
            id: workspace_id,
            custom_name: snap.custom_name.clone(),
            identity_cwd: snap.identity_cwd.clone(),
            cached_identity_cwd: snap.identity_cwd.clone(),
            // Repository metadata is optional and may block on unavailable
            // storage. The app refreshes it after restoring the session.
            cached_auto_label: crate::workspace::fallback_label_from_cwd(&snap.identity_cwd),
            cached_git_status_key: snap.identity_cwd.clone(),
            cached_git_branch: None,
            cached_git_detached_head: None,
            cached_git_ahead_behind: None,
            cached_git_space: None,
            worktree_space: snap.worktree_space.clone(),
            pin_order: snap.pin_order,
            metadata_tokens: crate::metadata_tokens::MetadataTokens::default(),
            metadata_token_sequences: HashMap::new(),
            public_pane_numbers,
            next_public_pane_number,
            next_public_tab_number,
            active_tab: snap.active_tab.min(tabs.len().saturating_sub(1)),
            tabs,
            #[cfg(test)]
            test_runtimes: HashMap::new(),
        })
        .map(|workspace| (workspace, terminals, terminal_runtimes)),
        failed_imports,
    )
}

fn unavailable_restored_terminal(
    pane: Option<&super::snapshot::PaneSnapshot>,
    cwd: PathBuf,
    reason: String,
) -> TerminalState {
    warn!(cwd = %cwd.display(), reason = %reason, "preserving unavailable restored pane");
    let mut terminal = TerminalState::new(TerminalId::alloc(), cwd);
    terminal.restore_error = Some(reason);
    if let Some(pane) = pane {
        terminal.manual_label = pane.label.clone();
        terminal.launch_argv = pane.launch_argv.clone();
        if let Some(session) = restored_terminal_agent_session(pane.agent_session.as_ref(), false) {
            terminal.set_persisted_agent_session(session);
        }
        if let Some(resume) = saved_reported_resume(pane) {
            terminal.restore_reported_resume(reported_resume_from_snapshot(resume));
        }
        match (
            pane.agent_name.as_ref(),
            pane.managed_agent_kind
                .as_deref()
                .and_then(crate::detect::parse_canonical_agent_label),
        ) {
            (Some(name), Some(agent)) => terminal.restore_managed_agent(name.clone(), agent),
            (Some(name), None) => terminal.set_agent_name(name.clone()),
            _ => {}
        }
    }
    terminal
}

pub(crate) fn restored_worktree_space_membership(
    space: Option<crate::workspace::WorktreeSpaceMembership>,
) -> Option<crate::workspace::WorktreeSpaceMembership> {
    space.filter(|space| {
        crate::workspace::git_space_metadata(&space.checkout_path).is_none_or(|current| {
            // Discovery may find an ancestor repo, or fall back to a per-worktree
            // Git directory without objects when common-directory metadata is unreadable.
            current.checkout_key
                != crate::worktree::canonical_or_original(&space.checkout_path)
                    .display()
                    .to_string()
                || current.key == space.key
                || !std::path::Path::new(&current.key).join("objects").is_dir()
        })
    })
}

fn restore_tab(
    snap: &TabSnapshot,
    history: Option<&TabHistorySnapshot>,
    number: usize,
    workspace_id: &str,
    rows: u16,
    cols: u16,
    runtime_context: &RestoreRuntimeContext<'_>,
    resumed_agent_sessions: &mut HashSet<String>,
    imported_panes: &mut HashMap<u32, crate::handoff_runtime::ImportedHandoffRuntime>,
    public_pane_ids_by_old_raw: &HashMap<u32, String>,
    todo_links: &mut TodoLinkRestore,
) -> RestoreFailures<Option<RestoredTab>> {
    let (node, id_map) = restore_node_remapped(&snap.layout);
    let reverse_id_map: HashMap<PaneId, u32> = id_map
        .iter()
        .map(|(&old_id, &new_id)| (new_id, old_id))
        .collect();
    let pane_ids = collect_pane_ids(&node);

    let mut panes = HashMap::new();
    let mut terminals = Vec::new();
    let mut terminal_runtimes = HashMap::new();
    let mut failed_imports = 0;
    for id in &pane_ids {
        let old_id = reverse_id_map.get(id);
        let saved_pane = old_id.and_then(|old_id| snap.panes.get(old_id));
        let saved_cwd = saved_pane
            .map(|p| p.cwd.clone())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| "/".into()));

        let cwd = saved_cwd;
        let has_import = old_id.is_some_and(|old_id| imported_panes.contains_key(old_id));
        if !has_import && !cwd.is_dir() {
            let terminal = unavailable_restored_terminal(
                saved_pane,
                cwd,
                "Saved directory is unavailable. Restore the directory and restart this session."
                    .into(),
            );
            panes.insert(*id, PaneState::new(terminal.id.clone()));
            terminals.push(terminal);
            continue;
        }

        let saved_label = saved_pane.and_then(|p| p.label.clone());
        let saved_agent_name = saved_pane.and_then(|p| p.agent_name.clone());
        let saved_managed_agent = saved_pane
            .and_then(|pane| pane.managed_agent_kind.as_deref())
            .and_then(crate::detect::parse_canonical_agent_label);
        let saved_launch_argv = saved_pane.and_then(|p| p.launch_argv.clone());
        let saved_agent_session = saved_pane.and_then(|p| p.agent_session.as_ref());
        let saved_agent_resume = saved_pane.and_then(saved_reported_resume);
        let saved_todos: &[PaneTodoSnapshot] =
            saved_pane.map(|p| p.todos.as_slice()).unwrap_or_default();
        let saved_next_todo_id = saved_pane.map(|p| p.next_todo_id).unwrap_or(1);
        let saved_last_input_at_ms = saved_pane.and_then(|p| p.last_input_at_ms);
        let saved_pin_order = saved_pane.and_then(|p| p.pin_order);
        let saved_history =
            old_id.and_then(|old_id| history.and_then(|history| history.panes.get(old_id)));
        let startup = {
            let mut agent_restore = AgentRestoreState {
                enabled: runtime_context.resume_agents_on_restore,
                resumed_sessions: resumed_agent_sessions,
            };
            pane_restore_startup(
                saved_agent_session,
                saved_agent_resume,
                &cwd,
                saved_history,
                saved_pane.and_then(|p| p.agent_launch.as_ref()),
                &mut agent_restore,
            )
        };
        let restored_agent_resume = saved_agent_resume
            .filter(|_| !startup.duplicate_agent_session)
            .map(reported_resume_from_snapshot);
        let restored_agent_session =
            restored_terminal_agent_session(saved_agent_session, startup.duplicate_agent_session);
        let restored_agent_launch = saved_pane
            .and_then(|p| p.agent_launch.as_ref())
            .filter(|_| !startup.duplicate_agent_session)
            .map(|launch| crate::agent_resume::AgentLaunchFlags {
                agent: launch.agent.clone(),
                flags: launch.flags.clone(),
                started_at_ms: launch.started_at_ms,
            });
        let initial_restore_agent = startup
            .restore_plan
            .as_ref()
            .and_then(|plan| crate::detect::parse_agent_label(&plan.agent));

        let old_pane_id = reverse_id_map.get(id).copied();
        let public_pane_id = old_pane_id
            .and_then(|old_id| public_pane_ids_by_old_raw.get(&old_id))
            .map(String::as_str);
        let launch_env = public_pane_id
            .map(|pane_id| {
                PaneLaunchEnv::from_extra(Vec::new()).with_identity(
                    workspace_id.to_string(),
                    crate::workspace::public_tab_id_for_number(workspace_id, number),
                    pane_id.to_string(),
                )
            })
            .unwrap_or_default();
        let imported_runtime = old_pane_id.and_then(|old_id| imported_panes.remove(&old_id));
        let was_imported = imported_runtime.is_some();
        #[cfg(unix)]
        let imported_agent_seed = imported_runtime
            .as_ref()
            .and_then(|imported| imported.state.agent_seed());
        #[cfg(not(unix))]
        let imported_agent_seed = None;
        #[cfg(unix)]
        let handoff_agent_state = imported_runtime
            .as_ref()
            .and_then(|imported| imported.state.hook_agent_state.clone());
        #[cfg(unix)]
        let handoff_unseen = imported_runtime
            .as_ref()
            .is_some_and(|imported| imported.state.unseen);
        #[cfg(not(unix))]
        let handoff_unseen = false;
        let pending_native_agent_restore = if was_imported {
            None
        } else {
            startup.restore_plan.clone()
        };
        if let Some(plan) = pending_native_agent_restore {
            let terminal_id = TerminalId::alloc();
            let mut terminal = TerminalState::new(terminal_id.clone(), cwd.clone())
                .with_pending_agent_resume_plan(plan);
            if let Some(label) = saved_label {
                terminal.set_manual_label(label);
            }
            if let Some(session) = restored_agent_session {
                terminal.set_persisted_agent_session(session);
            }
            if let Some(resume) = restored_agent_resume {
                terminal.restore_reported_resume(resume);
            }
            if let Some(record) = restored_agent_launch.clone() {
                terminal.restore_agent_launch(record);
            }
            match (saved_agent_name, saved_managed_agent) {
                (Some(agent_name), Some(agent)) => {
                    terminal.restore_managed_agent(agent_name, agent)
                }
                // A name given to an agent herdr did not launch (`agent
                // rename`, a rename hook) survives a restore when the pane
                // resumes the agent session that holds it (fork issue 136).
                // The session owns the name, so it goes if the session does.
                (Some(agent_name), None) if saved_agent_session.is_some() => {
                    terminal.set_agent_name(agent_name)
                }
                (Some(_), None) => {}
                (None, _) => {}
            }
            if let Some(agent) = initial_restore_agent {
                let _ = terminal.set_detected_state_with_screen_signals_at(
                    Some(agent),
                    AgentState::Idle,
                    false,
                    false,
                    false,
                    false,
                    std::time::Instant::now(),
                );
            }
            restore_pane_todos(&mut terminal, saved_todos, saved_next_todo_id, todo_links);
            terminal.restored_last_input_at_ms = saved_last_input_at_ms;
            terminal.pin_order = saved_pin_order;
            panes.insert(*id, PaneState::new(terminal_id));
            terminals.push(terminal);
            continue;
        }

        #[cfg(not(unix))]
        if imported_runtime.is_some() {
            failed_imports += 1;
            continue;
        }

        let runtime_result = {
            #[cfg(unix)]
            if let Some(imported) = imported_runtime {
                TerminalRuntime::from_handoff_fd(
                    crate::handoff_runtime::ImportedHandoffRuntime {
                        master_fd: imported.master_fd,
                        state: imported.state.with_pane_id(*id),
                    },
                    runtime_context.scrollback_limit_bytes,
                    crate::terminal_theme::TerminalTheme::default(),
                    None,
                    runtime_context.events.clone(),
                    runtime_context.render_notify.clone(),
                    runtime_context.render_dirty.clone(),
                )
            } else {
                TerminalRuntime::spawn_with_initial_history(
                    *id,
                    rows,
                    cols,
                    cwd.clone(),
                    runtime_context.scrollback_limit_bytes,
                    crate::terminal_theme::TerminalTheme::default(),
                    None,
                    runtime_context.shell_config,
                    &launch_env,
                    startup.initial_history_ansi,
                    runtime_context.events.clone(),
                    runtime_context.render_notify.clone(),
                    runtime_context.render_dirty.clone(),
                )
            }

            #[cfg(not(unix))]
            {
                TerminalRuntime::spawn_with_initial_history(
                    *id,
                    rows,
                    cols,
                    cwd.clone(),
                    runtime_context.scrollback_limit_bytes,
                    crate::terminal_theme::TerminalTheme::default(),
                    None,
                    runtime_context.shell_config,
                    &launch_env,
                    startup.initial_history_ansi,
                    runtime_context.events.clone(),
                    runtime_context.render_notify.clone(),
                    runtime_context.render_dirty.clone(),
                )
            }
        };

        match runtime_result {
            Ok(runtime) => {
                let terminal_id = TerminalId::alloc();
                let mut terminal = TerminalState::new(terminal_id.clone(), cwd.clone());
                if was_imported {
                    if let Some(argv) = saved_launch_argv {
                        terminal = terminal.with_launch_argv(argv).with_respawn_shell_on_exit();
                    }
                }
                if let Some(label) = saved_label {
                    terminal.set_manual_label(label);
                }
                if let Some(session) = restored_agent_session {
                    terminal.set_persisted_agent_session(session);
                }
                if let Some(resume) = restored_agent_resume {
                    terminal.restore_reported_resume(resume);
                }
                if let Some(record) = restored_agent_launch.clone() {
                    terminal.restore_agent_launch(record);
                }
                match (saved_agent_name, saved_managed_agent) {
                    (Some(agent_name), Some(agent)) if was_imported => {
                        terminal.restore_managed_agent(agent_name, agent)
                    }
                    (Some(_), Some(_)) => {}
                    (Some(agent_name), None) if was_imported => terminal.set_agent_name(agent_name),
                    (Some(_), None) => {}
                    (None, _) => {}
                }
                let (seed_agent, seed_state) =
                    restored_agent_seed(imported_agent_seed, initial_restore_agent);
                if let Some(agent) = seed_agent {
                    let _ = terminal.set_detected_state_with_screen_signals_at(
                        Some(agent),
                        seed_state,
                        seed_state == AgentState::Blocked,
                        false,
                        false,
                        false,
                        std::time::Instant::now(),
                    );
                }
                restore_pane_todos(&mut terminal, saved_todos, saved_next_todo_id, todo_links);
                terminal.restored_last_input_at_ms = saved_last_input_at_ms;
                terminal.pin_order = saved_pin_order;
                #[cfg(unix)]
                if let Some(agent_state) = handoff_agent_state {
                    terminal.restore_handoff_agent_state(agent_state);
                }
                let mut pane = PaneState::new(terminal_id.clone());
                // Done (finished, not yet looked at) survives a live handoff.
                pane.seen = !handoff_unseen;
                panes.insert(*id, pane);
                terminal_runtimes.insert(terminal_id, runtime);
                terminals.push(terminal);
            }
            Err(e) => {
                if let Some(key) = startup.reserved_agent_session.as_deref() {
                    resumed_agent_sessions.remove(key);
                }
                if was_imported {
                    failed_imports += 1;
                    error!(
                        tab = ?snap.custom_name,
                        pane_id = id.raw(),
                        err = %e,
                        "failed to restore imported pane"
                    );
                }
                error!(
                    tab = ?snap.custom_name,
                    pane_id = id.raw(),
                    err = %e,
                    "failed to restore pane"
                );
                if !was_imported {
                    let terminal = unavailable_restored_terminal(
                        saved_pane, cwd,
                        format!("Could not start the saved shell: {e}. Fix the shell configuration and restart this session."),
                    );
                    panes.insert(*id, PaneState::new(terminal.id.clone()));
                    terminals.push(terminal);
                }
            }
        }
    }

    if panes.is_empty() {
        warn!(
            tab = ?snap.custom_name,
            "no panes could be restored for tab, dropping it"
        );
        return (None, failed_imports);
    }

    let surviving: HashSet<PaneId> = panes.keys().copied().collect();
    let Some(node) = prune_restored_node(node, &surviving) else {
        warn!(
            tab = ?snap.custom_name,
            "restored tab lost all panes after pruning missing layout nodes"
        );
        return (None, failed_imports);
    };
    let pane_ids = collect_pane_ids(&node);
    let Some(focus) = resolve_restored_pane(snap.focused, &id_map, &surviving, &pane_ids) else {
        return (None, failed_imports);
    };
    let Some(root_pane) = resolve_restored_pane(snap.root_pane, &id_map, &surviving, &pane_ids)
    else {
        return (None, failed_imports);
    };
    let layout = TileLayout::from_saved(node, focus);
    todo_links.register_pane_ids(&id_map, &surviving);

    (
        Some((
            crate::workspace::Tab {
                custom_name: snap.custom_name.clone(),
                number,
                root_pane,
                layout,
                panes,
                #[cfg(test)]
                runtimes: HashMap::new(),
                zoomed: snap.zoomed,
                // Sync mode is never saved: a restart or a live handoff turns
                // it off (fork issue 141).
                sync: None,
                events: runtime_context.events.clone(),
                render_notify: runtime_context.render_notify.clone(),
                render_dirty: runtime_context.render_dirty.clone(),
            },
            terminals,
            terminal_runtimes,
            reverse_id_map,
        )),
        failed_imports,
    )
}

fn pane_restore_startup<'a>(
    session: Option<&PaneAgentSessionSnapshot>,
    reported_resume: Option<&PaneAgentResumeSnapshot>,
    cwd: &std::path::Path,
    history: Option<&'a PaneHistorySnapshot>,
    launch: Option<&super::snapshot::PaneAgentLaunchSnapshot>,
    agent_restore: &mut AgentRestoreState<'_>,
) -> PaneRestoreStartup<'a> {
    // Native agent resume owns the conversation history. If a pane has a
    // resumable agent session and resume is enabled, do not replay saved pane
    // presentation history into that terminal, even when this pane is a
    // duplicate suppressed by session de-duplication.
    let launch = launch.map(|launch| crate::agent_resume::AgentLaunchFlags {
        agent: launch.agent.clone(),
        flags: launch.flags.clone(),
        started_at_ms: launch.started_at_ms,
    });
    let restore_plan = if !agent_restore.enabled {
        None
    } else if let Some(resume) = reported_resume {
        Some(reported_resume_from_snapshot(resume).plan(cwd))
    } else {
        session.and_then(|session| {
            restore_plan_for_snapshot(session, true)
                .map(|plan| with_session_transcript(plan, session, launch.as_ref()))
        })
    }
    .map(|plan| plan.with_launch_flags(launch.as_ref()));
    let has_native_agent_restore = restore_plan.is_some();
    // Reserve before spawning so later panes in the same restore pass cannot
    // launch the same native agent session. The caller rolls this reservation
    // back if runtime spawn fails before any agent process is started.
    let mut reserved_agent_session = None;
    let duplicate_agent_session = restore_plan.as_ref().is_some_and(|plan| {
        if agent_restore
            .resumed_sessions
            .insert(plan.dedupe_key.clone())
        {
            reserved_agent_session = Some(plan.dedupe_key.clone());
            false
        } else {
            true
        }
    });
    let restore_plan = if duplicate_agent_session {
        None
    } else {
        restore_plan
    };

    PaneRestoreStartup {
        restore_plan,
        initial_history_ansi: if has_native_agent_restore {
            None
        } else {
            history.map(|history| history.ansi.as_str())
        },
        duplicate_agent_session,
        reserved_agent_session,
    }
}

fn saved_reported_resume(pane: &super::snapshot::PaneSnapshot) -> Option<&PaneAgentResumeSnapshot> {
    pane.agent_resume
        .as_ref()
        .filter(|resume| crate::agent_resume::validate_resume_argv(&resume.argv).is_ok())
}

fn reported_resume_from_snapshot(
    resume: &PaneAgentResumeSnapshot,
) -> crate::agent_resume::ReportedAgentResume {
    crate::agent_resume::ReportedAgentResume {
        source: resume.source.clone(),
        agent: resume.agent.clone(),
        argv: resume.argv.clone(),
    }
}

/// A Claude session with no hook report restores in the model, effort and
/// mode its transcript shows (fork issue 123).
fn with_session_transcript(
    plan: crate::agent_resume::AgentResumePlan,
    session: &PaneAgentSessionSnapshot,
    launch: Option<&crate::agent_resume::AgentLaunchFlags>,
) -> crate::agent_resume::AgentResumePlan {
    if plan.agent != "claude" {
        return plan;
    }
    // Only what the process that ran at shutdown wrote: an older record is
    // an earlier run's, maybe in another mode.
    let since = launch
        .filter(|launch| launch.agent == "claude")
        .and_then(|launch| launch.started_at_ms);
    let transcript = crate::agent_resume::claude_transcript_resume(&session.value, since);
    plan.with_claude_transcript(transcript)
}

fn restore_plan_for_snapshot(
    session: &PaneAgentSessionSnapshot,
    resume_agents_on_restore: bool,
) -> Option<crate::agent_resume::AgentResumePlan> {
    if !resume_agents_on_restore {
        return None;
    }
    let persisted = persisted_agent_session_from_snapshot(session)?;
    crate::agent_resume::plan(&session.source, &session.agent, &persisted.session_ref)
}

fn persisted_agent_session_from_snapshot(
    session: &PaneAgentSessionSnapshot,
) -> Option<crate::agent_resume::PersistedAgentSession> {
    crate::agent_resume::session_ref_from_snapshot(
        &session.source,
        &session.agent,
        session.kind,
        &session.value,
    )
}

fn restored_terminal_agent_session(
    session: Option<&PaneAgentSessionSnapshot>,
    duplicate_agent_session: bool,
) -> Option<crate::agent_resume::PersistedAgentSession> {
    if duplicate_agent_session {
        return None;
    }
    session.and_then(persisted_agent_session_from_snapshot)
}

/// Agent identity and state to seed a restored pane's terminal with.
///
/// Live-handoff panes carry the pre-restart state in the handoff manifest so a
/// working agent resurfaces immediately; everything else (cold restore,
/// relaunched agents, old manifests without the field) keeps the Idle seeding.
fn restored_agent_seed(
    imported_agent_seed: Option<(crate::detect::Agent, AgentState)>,
    initial_restore_agent: Option<crate::detect::Agent>,
) -> (Option<crate::detect::Agent>, AgentState) {
    match imported_agent_seed {
        Some((agent, state)) => (Some(agent), state),
        None => (initial_restore_agent, AgentState::Idle),
    }
}

#[cfg(test)]
fn take_restore_plan_for_snapshot(
    session: &PaneAgentSessionSnapshot,
    resume_agents_on_restore: bool,
    resumed_agent_sessions: &mut HashSet<String>,
) -> Option<crate::agent_resume::AgentResumePlan> {
    restore_plan_for_snapshot(session, resume_agents_on_restore)
        .filter(|plan| resumed_agent_sessions.insert(plan.dedupe_key.clone()))
}

pub(super) fn prune_restored_node(node: Node, surviving: &HashSet<PaneId>) -> Option<Node> {
    match node {
        Node::Pane(id) => surviving.contains(&id).then_some(Node::Pane(id)),
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let first = prune_restored_node(*first, surviving);
            let second = prune_restored_node(*second, surviving);
            match (first, second) {
                (Some(first), Some(second)) => Some(Node::Split {
                    direction,
                    ratio,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (Some(remaining), None) | (None, Some(remaining)) => Some(remaining),
                (None, None) => None,
            }
        }
    }
}

pub(super) fn resolve_restored_pane(
    saved_old_id: Option<u32>,
    id_map: &HashMap<u32, PaneId>,
    surviving: &HashSet<PaneId>,
    pane_ids: &[PaneId],
) -> Option<PaneId> {
    saved_old_id
        .and_then(|old_id| id_map.get(&old_id).copied())
        .filter(|pane_id| surviving.contains(pane_id))
        .or_else(|| pane_ids.first().copied())
}

/// Restore a layout tree, remapping every pane ID to a fresh globally unique one.
/// Returns the new tree and a map of old_raw_id → new PaneId.
pub(super) fn restore_node_remapped(snap: &LayoutSnapshot) -> (Node, HashMap<u32, PaneId>) {
    let mut id_map = HashMap::new();
    let node = remap_inner(snap, &mut id_map);
    (node, id_map)
}

fn remap_inner(snap: &LayoutSnapshot, id_map: &mut HashMap<u32, PaneId>) -> Node {
    match snap {
        LayoutSnapshot::Pane(old_id) => {
            let new_id = PaneId::alloc();
            id_map.insert(*old_id, new_id);
            Node::Pane(new_id)
        }
        LayoutSnapshot::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let first_node = remap_inner(first, id_map);
            let second_node = remap_inner(second, id_map);
            let dir = match direction {
                DirectionSnapshot::Horizontal => Direction::Horizontal,
                DirectionSnapshot::Vertical => Direction::Vertical,
            };
            Node::Split {
                direction: dir,
                ratio: *ratio,
                first: Box::new(first_node),
                second: Box::new(second_node),
            }
        }
    }
}

pub(super) fn collect_pane_ids(node: &Node) -> Vec<PaneId> {
    let mut ids = Vec::new();
    collect_ids_inner(node, &mut ids);
    ids
}

fn collect_ids_inner(node: &Node, ids: &mut Vec<PaneId>) {
    match node {
        Node::Pane(id) => ids.push(*id),
        Node::Split { first, second, .. } => {
            collect_ids_inner(first, ids);
            collect_ids_inner(second, ids);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restored_agent_seed_keeps_idle_seeding_without_manifest_state() {
        use crate::detect::Agent;
        // Characterization of pre-change behavior: no manifest seed means the
        // restore-plan agent (if any) is seeded Idle.
        assert_eq!(restored_agent_seed(None, None), (None, AgentState::Idle));
        assert_eq!(
            restored_agent_seed(None, Some(Agent::Claude)),
            (Some(Agent::Claude), AgentState::Idle)
        );
    }

    #[test]
    fn seeded_working_state_is_effective_without_any_detection_event() {
        use crate::detect::Agent;
        // The sidebar spinner keys off `TerminalState.state == Working`; seeding
        // alone must produce it, with no detection task publish involved.
        let mut terminal = crate::terminal::TerminalState::new(
            crate::terminal::TerminalId::alloc(),
            std::path::PathBuf::from("/"),
        );
        let (seed_agent, seed_state) =
            restored_agent_seed(Some((Agent::Claude, AgentState::Working)), None);
        let _ = terminal.set_detected_state_with_screen_signals_at(
            seed_agent,
            seed_state,
            false,
            false,
            false,
            false,
            std::time::Instant::now(),
        );
        assert_eq!(terminal.state, AgentState::Working);
        assert_eq!(terminal.detected_agent, Some(Agent::Claude));
    }

    #[test]
    fn restored_agent_seed_prefers_manifest_state() {
        use crate::detect::Agent;
        assert_eq!(
            restored_agent_seed(Some((Agent::Claude, AgentState::Working)), None),
            (Some(Agent::Claude), AgentState::Working)
        );
        assert_eq!(
            restored_agent_seed(
                Some((Agent::Codex, AgentState::Blocked)),
                Some(Agent::Claude)
            ),
            (Some(Agent::Codex), AgentState::Blocked)
        );
    }

    fn test_session_path(name: &str) -> String {
        std::env::current_dir()
            .unwrap()
            .join(name)
            .display()
            .to_string()
    }

    #[test]
    fn restore_remaps_a_todo_link_to_the_new_pane_id() {
        // old raw 10 and 11 are two panes in the saved session; 11 is the link target
        let id_map: std::collections::HashMap<u32, PaneId> = std::collections::HashMap::from([
            (10, PaneId::from_raw(101)),
            (11, PaneId::from_raw(102)),
        ]);

        let resolved = resolve_todo_link(&id_map, Some(11), Some("infra".to_string()));

        assert_eq!(
            resolved,
            Some(crate::terminal::todo::TodoLink {
                pane: Some(PaneId::from_raw(102)),
                label: "infra".into(),
            })
        );
    }

    #[test]
    fn restore_turns_an_unmapped_todo_link_into_a_dead_link() {
        let id_map: std::collections::HashMap<u32, PaneId> =
            std::collections::HashMap::from([(10, PaneId::from_raw(101))]);

        let resolved = resolve_todo_link(&id_map, Some(999), Some("gone".to_string()));

        assert_eq!(
            resolved,
            Some(crate::terminal::todo::TodoLink {
                pane: None,
                label: "gone".into()
            }),
            "an unresolvable target must keep its label, not vanish or retarget"
        );
    }

    #[test]
    fn restore_leaves_unlinked_todos_unlinked() {
        let id_map: std::collections::HashMap<u32, PaneId> = std::collections::HashMap::new();

        assert_eq!(resolve_todo_link(&id_map, None, None), None);
    }

    #[tokio::test]
    async fn restore_rehydrates_todos_and_resolves_cross_tab_links() {
        let cwd = std::env::current_dir().unwrap();
        // Every field varies from the struct default on purpose: a mapping
        // regression that hardcoded `done: false` or `priority: default()`
        // would otherwise pass the whole suite.
        let todo = |id: u64,
                    text: &str,
                    done: bool,
                    priority: crate::terminal::todo::TodoPriority,
                    link_pane: Option<u32>,
                    link_label: Option<&str>| {
            super::super::snapshot::PaneTodoSnapshot {
                id,
                text: text.into(),
                done,
                priority,
                link_pane,
                link_label: link_label.map(str::to_string),
                created_at_unix: 100,
                updated_at_unix: 140 + id,
            }
        };
        let owner_pane = super::super::snapshot::PaneSnapshot {
            agent_resume: None,
            agent_launch: None,
            cwd: cwd.clone(),
            label: None,
            agent_name: None,
            managed_agent_kind: None,
            agent_session: None,
            launch_argv: None,
            todos: vec![
                todo(
                    4,
                    "linked",
                    true,
                    crate::terminal::todo::TodoPriority::High,
                    Some(20),
                    Some("infra"),
                ),
                todo(
                    5,
                    "dead link",
                    false,
                    crate::terminal::todo::TodoPriority::Low,
                    Some(999),
                    Some("gone"),
                ),
                todo(
                    6,
                    "unlinked",
                    false,
                    crate::terminal::todo::TodoPriority::Normal,
                    None,
                    None,
                ),
            ],
            next_todo_id: 7,
            last_input_at_ms: Some(1_790_000_000_123),
            pin_order: None,
            former_public_ids: Vec::new(),
        };
        let target_pane = super::super::snapshot::PaneSnapshot {
            agent_resume: None,
            agent_launch: None,
            cwd: cwd.clone(),
            label: None,
            agent_name: None,
            managed_agent_kind: None,
            agent_session: None,
            launch_argv: None,
            todos: Vec::new(),
            next_todo_id: 1,
            last_input_at_ms: None,
            pin_order: None,
            former_public_ids: Vec::new(),
        };
        let snapshot = SessionSnapshot {
            version: super::super::snapshot::SNAPSHOT_VERSION,
            workspaces: vec![WorkspaceSnapshot {
                id: Some("w1".into()),
                custom_name: None,
                identity_cwd: cwd.clone(),
                worktree_space: None,
                pin_order: None,
                public_pane_numbers: HashMap::from([(10, 1), (20, 2)]),
                next_public_pane_number: 3,
                public_tab_numbers: vec![1, 2],
                next_public_tab_number: 3,
                tabs: vec![
                    TabSnapshot {
                        custom_name: None,
                        layout: LayoutSnapshot::Pane(10),
                        panes: HashMap::from([(10, owner_pane)]),
                        zoomed: false,
                        focused: Some(10),
                        root_pane: Some(10),
                    },
                    TabSnapshot {
                        custom_name: None,
                        layout: LayoutSnapshot::Pane(20),
                        panes: HashMap::from([(20, target_pane)]),
                        zoomed: false,
                        focused: Some(20),
                        root_pane: Some(20),
                    },
                ],
                active_tab: 0,
            }],
            active: Some(0),
            selected: 0,
            sidebar_width: None,
            sidebar_section_split: None,
            collapsed_space_keys: Default::default(),
            last_client_size: None,
        };
        let (events, _event_rx) = mpsc::channel(4);

        let (workspaces, mut terminals, _runtimes) = restore(
            &snapshot,
            None,
            24,
            80,
            0,
            test_restore_shell(),
            crate::config::ShellModeConfig::NonLogin,
            false,
            events,
            Arc::new(Notify::new()),
            Arc::new(RenderSignal::new()),
        );

        let workspace = workspaces.first().expect("workspace should restore");
        let owner = workspace.tabs[0].root_pane;
        let target = workspace.tabs[1].root_pane;
        let owner_terminal_id = workspace.tabs[0].panes[&owner].attached_terminal_id.clone();
        let todos = terminals[&owner_terminal_id].todos();

        assert_eq!(todos.len(), 3, "no todo may be dropped by restore");
        // The last-input time rides along with the pane; a pane saved without
        // one stays unknown.
        let target_terminal_id = workspace.tabs[1].panes[&target]
            .attached_terminal_id
            .clone();
        assert_eq!(
            terminals[&owner_terminal_id].restored_last_input_at_ms,
            Some(1_790_000_000_123)
        );
        assert_eq!(
            terminals[&target_terminal_id].restored_last_input_at_ms,
            None
        );

        // Scenario "Todos survive a restart" names text, done state, priority,
        // and ids explicitly, so assert every one of them rather than trusting
        // the field-for-field copy.
        let restored: Vec<(u64, &str, bool, crate::terminal::todo::TodoPriority, u64)> = todos
            .iter()
            .map(|todo| {
                (
                    todo.id,
                    todo.text.as_str(),
                    todo.done,
                    todo.priority,
                    todo.updated_at_unix,
                )
            })
            .collect();
        assert_eq!(
            restored,
            vec![
                (
                    4,
                    "linked",
                    true,
                    crate::terminal::todo::TodoPriority::High,
                    144
                ),
                (
                    5,
                    "dead link",
                    false,
                    crate::terminal::todo::TodoPriority::Low,
                    145
                ),
                (
                    6,
                    "unlinked",
                    false,
                    crate::terminal::todo::TodoPriority::Normal,
                    146
                ),
            ],
            "ids, text, done state, priority, and timestamps must survive restore intact"
        );
        assert!(
            todos.iter().all(|todo| todo.created_at_unix == 100),
            "created_at must survive restore"
        );

        assert_eq!(
            todos[0].link,
            Some(crate::terminal::todo::TodoLink {
                pane: Some(target),
                label: "infra".into(),
            }),
            "a live link must follow its target to the new pane id"
        );
        assert_eq!(
            todos[1].link,
            Some(crate::terminal::todo::TodoLink {
                pane: None,
                label: "gone".into(),
            }),
            "an unresolvable target must keep its label as a dead link"
        );
        assert_eq!(todos[2].link, None);
        assert_eq!(
            terminals[&workspace.tabs[1].panes[&target].attached_terminal_id]
                .todos()
                .len(),
            0,
            "todos must not leak onto the linked pane"
        );

        let next = terminals
            .get_mut(&owner_terminal_id)
            .expect("restored terminal should exist")
            .add_todo(
                "after restore",
                crate::terminal::todo::TodoPriority::Normal,
                None,
                200,
            )
            .unwrap();
        assert_eq!(next.id, 7, "the saved id counter must survive restore");
    }

    #[cfg(windows)]
    fn test_restore_shell() -> &'static str {
        "C:\\Windows\\System32\\whoami.exe"
    }

    #[cfg(not(windows))]
    fn test_restore_shell() -> &'static str {
        "/bin/sh"
    }

    #[test]
    fn capture_and_restore_node_round_trip() {
        let node = Node::Split {
            direction: Direction::Horizontal,
            ratio: 0.5,
            first: Box::new(Node::Pane(PaneId::from_raw(0))),
            second: Box::new(Node::Split {
                direction: Direction::Vertical,
                ratio: 0.3,
                first: Box::new(Node::Pane(PaneId::from_raw(1))),
                second: Box::new(Node::Pane(PaneId::from_raw(2))),
            }),
        };

        let snap = super::super::snapshot::capture_node(&node);
        let (restored, id_map) = restore_node_remapped(&snap);

        assert_eq!(id_map.len(), 3);
        let ids = collect_pane_ids(&restored);
        assert_eq!(ids.len(), 3);
        let unique: std::collections::HashSet<u32> = ids.iter().map(|id| id.raw()).collect();
        assert_eq!(unique.len(), 3);
    }

    #[test]
    fn prune_restored_node_collapses_missing_branch() {
        let keep = PaneId::from_raw(11);
        let missing = PaneId::from_raw(12);
        let node = Node::Split {
            direction: Direction::Horizontal,
            ratio: 0.5,
            first: Box::new(Node::Pane(keep)),
            second: Box::new(Node::Pane(missing)),
        };
        let surviving = std::collections::HashSet::from([keep]);

        let pruned = prune_restored_node(node, &surviving).expect("remaining pane should survive");

        assert!(matches!(pruned, Node::Pane(id) if id == keep));
    }

    #[test]
    fn resolve_restored_pane_prefers_surviving_saved_id_and_falls_back_to_first_remaining() {
        let first = PaneId::from_raw(21);
        let second = PaneId::from_raw(22);
        let id_map = HashMap::from([(0_u32, first), (1_u32, second)]);
        let surviving = std::collections::HashSet::from([first]);
        let pane_ids = vec![first];

        assert_eq!(
            resolve_restored_pane(Some(0), &id_map, &surviving, &pane_ids),
            Some(first)
        );
        assert_eq!(
            resolve_restored_pane(Some(1), &id_map, &surviving, &pane_ids),
            Some(first)
        );
    }

    #[test]
    fn restored_worktree_space_membership_preserves_unavailable_checkout_and_rejects_replacement() {
        let root = std::env::temp_dir().join(format!(
            "herdr-restored-worktree-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join(".git/objects")).unwrap();
        std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        let ancestor = crate::workspace::git_space_metadata(&root).unwrap();
        let checkout = root.join("checkout");
        let membership = crate::workspace::WorktreeSpaceMembership {
            key: "original-repo-key".into(),
            label: "herdr".into(),
            repo_root: root.join("original-repo"),
            checkout_path: checkout.clone(),
            is_linked_worktree: true,
        };

        // Missing and leftover directories may discover an unrelated ancestor repo.
        assert_eq!(
            restored_worktree_space_membership(Some(membership.clone())),
            Some(membership.clone())
        );
        std::fs::create_dir_all(&checkout).unwrap();
        assert_eq!(
            restored_worktree_space_membership(Some(membership.clone())),
            Some(membership.clone())
        );

        // A real repository at the saved checkout path can prove replacement.
        std::fs::create_dir_all(checkout.join(".git/objects")).unwrap();
        std::fs::write(checkout.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        assert_eq!(
            restored_worktree_space_membership(Some(membership.clone())),
            None
        );
        let current = crate::workspace::git_space_metadata(&checkout).unwrap();
        assert_ne!(current.key, ancestor.key);
        let matching = crate::workspace::WorktreeSpaceMembership {
            key: current.key,
            ..membership.clone()
        };
        assert_eq!(
            restored_worktree_space_membership(Some(matching.clone())),
            Some(matching)
        );

        // Partial linked-worktree metadata is not evidence of replacement.
        std::fs::remove_dir_all(checkout.join(".git")).unwrap();
        let git_dir = root.join(".git/worktrees/restored");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(
            checkout.join(".git"),
            format!("gitdir: {}\n", git_dir.display()),
        )
        .unwrap();
        let linked = crate::workspace::WorktreeSpaceMembership {
            key: ancestor.key,
            ..membership.clone()
        };
        assert_eq!(
            restored_worktree_space_membership(Some(linked.clone())),
            Some(linked)
        );
        std::fs::remove_dir_all(root).unwrap();
        assert!(crate::workspace::git_space_metadata(&checkout).is_none());
        assert_eq!(
            restored_worktree_space_membership(Some(membership.clone())),
            Some(membership)
        );
    }

    #[test]
    fn restore_plan_respects_opt_in_and_allowlist() {
        let pi_session_path = test_session_path("pi-session.jsonl");
        let session = super::super::snapshot::PaneAgentSessionSnapshot {
            source: "herdr:pi".into(),
            agent: "pi".into(),
            kind: crate::agent_resume::AgentSessionRefKind::Path,
            value: pi_session_path.clone(),
        };

        assert!(restore_plan_for_snapshot(&session, false).is_none());
        assert_eq!(
            restore_plan_for_snapshot(&session, true).unwrap().argv,
            vec!["pi", "--session", pi_session_path.as_str()]
        );

        let unsupported_path = super::super::snapshot::PaneAgentSessionSnapshot {
            source: "herdr:claude".into(),
            agent: "claude".into(),
            kind: crate::agent_resume::AgentSessionRefKind::Path,
            value: test_session_path("claude-session"),
        };
        assert!(restore_plan_for_snapshot(&unsupported_path, true).is_none());
    }

    #[test]
    fn restore_plan_selection_suppresses_duplicates() {
        let pi_session_path = test_session_path("pi-session.jsonl");
        let session = super::super::snapshot::PaneAgentSessionSnapshot {
            source: "herdr:pi".into(),
            agent: "pi".into(),
            kind: crate::agent_resume::AgentSessionRefKind::Path,
            value: pi_session_path.clone(),
        };
        let mut resumed = HashSet::new();

        assert!(take_restore_plan_for_snapshot(&session, false, &mut resumed).is_none());
        assert!(resumed.is_empty());

        let first = take_restore_plan_for_snapshot(&session, true, &mut resumed)
            .expect("first restore should get a plan");
        assert_eq!(
            first.argv,
            vec!["pi", "--session", pi_session_path.as_str()]
        );
        assert!(take_restore_plan_for_snapshot(&session, true, &mut resumed).is_none());
    }

    #[test]
    fn pane_restore_startup_suppresses_history_for_native_agent_resume() {
        let session = super::super::snapshot::PaneAgentSessionSnapshot {
            source: "herdr:pi".into(),
            agent: "pi".into(),
            kind: crate::agent_resume::AgentSessionRefKind::Path,
            value: test_session_path("pi-session.jsonl"),
        };
        let history = super::super::snapshot::PaneHistorySnapshot {
            ansi: "RESTORED_HISTORY\r\n".into(),
            lines: 1,
        };
        let mut resumed = HashSet::new();
        let mut agent_restore = AgentRestoreState {
            enabled: true,
            resumed_sessions: &mut resumed,
        };

        let startup = pane_restore_startup(
            Some(&session),
            None,
            std::path::Path::new("/a"),
            Some(&history),
            None,
            &mut agent_restore,
        );

        assert!(startup.restore_plan.is_some());
        assert!(startup.initial_history_ansi.is_none());
        assert!(!startup.duplicate_agent_session);
    }

    #[test]
    fn pane_restore_startup_suppresses_history_for_duplicate_native_agent_session() {
        let session = super::super::snapshot::PaneAgentSessionSnapshot {
            source: "herdr:pi".into(),
            agent: "pi".into(),
            kind: crate::agent_resume::AgentSessionRefKind::Path,
            value: test_session_path("pi-session.jsonl"),
        };
        let history = super::super::snapshot::PaneHistorySnapshot {
            ansi: "RESTORED_HISTORY\r\n".into(),
            lines: 1,
        };
        let mut resumed = HashSet::new();
        let mut agent_restore = AgentRestoreState {
            enabled: true,
            resumed_sessions: &mut resumed,
        };

        let first = pane_restore_startup(
            Some(&session),
            None,
            std::path::Path::new("/a"),
            Some(&history),
            None,
            &mut agent_restore,
        );
        let duplicate = pane_restore_startup(
            Some(&session),
            None,
            std::path::Path::new("/a"),
            Some(&history),
            None,
            &mut agent_restore,
        );

        assert!(first.restore_plan.is_some());
        assert!(first.initial_history_ansi.is_none());
        assert!(duplicate.restore_plan.is_none());
        assert!(duplicate.initial_history_ansi.is_none());
        assert!(duplicate.duplicate_agent_session);
    }

    #[test]
    fn pane_restore_startup_prefers_reported_resume_argv() {
        let session = super::super::snapshot::PaneAgentSessionSnapshot {
            source: "herdr:pi".into(),
            agent: "pi".into(),
            kind: crate::agent_resume::AgentSessionRefKind::Path,
            value: test_session_path("pi-session.jsonl"),
        };
        let resume = super::super::snapshot::PaneAgentResumeSnapshot {
            source: "herdr:pi".into(),
            agent: "pi".into(),
            argv: vec![
                "pi".into(),
                "--continue".into(),
                "--model".into(),
                "m".into(),
            ],
        };
        let history = super::super::snapshot::PaneHistorySnapshot {
            ansi: "RESTORED_HISTORY\r\n".into(),
            lines: 1,
        };
        let mut resumed = HashSet::new();
        let mut agent_restore = AgentRestoreState {
            enabled: true,
            resumed_sessions: &mut resumed,
        };

        let project_a = std::path::Path::new("/project-a");
        let project_b = std::path::Path::new("/project-b");
        let startup = pane_restore_startup(
            Some(&session),
            Some(&resume),
            project_a,
            Some(&history),
            None,
            &mut agent_restore,
        );
        let plan = startup.restore_plan.expect("reported resume plan");
        assert_eq!(plan.agent, "pi");
        assert_eq!(plan.argv, resume.argv);
        assert!(startup.initial_history_ansi.is_none());

        let custom = super::super::snapshot::PaneAgentResumeSnapshot {
            source: "prime-agent".into(),
            agent: "prime-agent".into(),
            argv: vec!["prime-agent".into(), "--continue".into()],
        };
        let first = pane_restore_startup(
            None,
            Some(&custom),
            project_a,
            None,
            None,
            &mut agent_restore,
        );
        assert_eq!(first.restore_plan.unwrap().argv, custom.argv);
        let other_project = pane_restore_startup(
            None,
            Some(&custom),
            project_b,
            None,
            None,
            &mut agent_restore,
        );
        assert!(other_project.restore_plan.is_some());
        let duplicate = pane_restore_startup(
            None,
            Some(&custom),
            project_a,
            None,
            None,
            &mut agent_restore,
        );
        assert!(duplicate.restore_plan.is_none());
        assert!(duplicate.duplicate_agent_session);

        let mut resumed = HashSet::new();
        let mut disabled = AgentRestoreState {
            enabled: false,
            resumed_sessions: &mut resumed,
        };
        let startup = pane_restore_startup(
            None,
            Some(&custom),
            project_a,
            Some(&history),
            None,
            &mut disabled,
        );
        assert!(startup.restore_plan.is_none());
        assert_eq!(startup.initial_history_ansi, Some("RESTORED_HISTORY\r\n"));
    }

    #[test]
    fn claude_pane_restores_with_its_reported_mode_model_and_effort() {
        let session = super::super::snapshot::PaneAgentSessionSnapshot {
            source: "herdr:claude".into(),
            agent: "claude".into(),
            kind: crate::agent_resume::AgentSessionRefKind::Id,
            value: "4f1c2d3e-aaaa-bbbb-cccc-0123456789ab".into(),
        };
        let resume = super::super::snapshot::PaneAgentResumeSnapshot {
            source: "herdr:claude".into(),
            agent: "claude".into(),
            argv: [
                "claude",
                "--resume",
                "4f1c2d3e-aaaa-bbbb-cccc-0123456789ab",
                "--model",
                "claude-sonnet-5-5",
                "--effort",
                "low",
                "--allow-dangerously-skip-permissions",
                "--permission-mode",
                "auto",
            ]
            .map(String::from)
            .into(),
        };
        let mut resumed = HashSet::new();
        let mut agent_restore = AgentRestoreState {
            enabled: true,
            resumed_sessions: &mut resumed,
        };
        let startup = pane_restore_startup(
            Some(&session),
            Some(&resume),
            std::path::Path::new("/project"),
            None,
            None,
            &mut agent_restore,
        );
        let plan = startup.restore_plan.expect("reported resume plan");
        assert_eq!(plan.agent, "claude");
        assert_eq!(plan.argv, resume.argv);
    }

    fn restore_argv(
        session: Option<&super::super::snapshot::PaneAgentSessionSnapshot>,
        resume: Option<&PaneAgentResumeSnapshot>,
        launch: &[(&str, &[&str])],
    ) -> Vec<String> {
        let launch =
            launch.first().map(
                |(agent, flags)| super::super::snapshot::PaneAgentLaunchSnapshot {
                    agent: agent.to_string(),
                    flags: flags.iter().map(|flag| flag.to_string()).collect(),
                    started_at_ms: None,
                },
            );
        let mut resumed = HashSet::new();
        let mut agent_restore = AgentRestoreState {
            enabled: true,
            resumed_sessions: &mut resumed,
        };
        pane_restore_startup(
            session,
            resume,
            std::path::Path::new("/a"),
            None,
            launch.as_ref(),
            &mut agent_restore,
        )
        .restore_plan
        .expect("the pane restores an agent")
        .argv
    }

    #[test]
    fn a_gpt_claude_pane_restores_with_its_settings_file() {
        let settings = crate::agent_resume::test_settings_file("restore-gpt.json");
        let resume = PaneAgentResumeSnapshot {
            source: "herdr:claude".into(),
            agent: "claude".into(),
            argv: ["claude", "--resume", "s1", "--model", "gpt-6-astra"]
                .map(String::from)
                .to_vec(),
        };
        assert_eq!(
            restore_argv(
                None,
                Some(&resume),
                &[(
                    "claude",
                    &["--settings", &settings, "--model", "gpt-6-astra"]
                )],
            ),
            [
                "claude",
                "--resume",
                "s1",
                "--model",
                "gpt-6-astra",
                "--settings",
                &settings
            ]
        );
    }

    #[test]
    fn a_hand_started_codex_pane_restores_with_its_flags() {
        let session = super::super::snapshot::PaneAgentSessionSnapshot {
            source: "herdr:codex".into(),
            agent: "codex".into(),
            kind: crate::agent_resume::AgentSessionRefKind::Id,
            value: "t1".into(),
        };
        assert_eq!(
            restore_argv(
                Some(&session),
                None,
                &[(
                    "codex",
                    &["-m", "gpt-6-astra", "-s", "read-only", "-c", "a=1"]
                )],
            ),
            [
                "codex",
                "resume",
                "t1",
                "-m",
                "gpt-6-astra",
                "-s",
                "read-only",
                "-c",
                "a=1"
            ]
        );
    }

    #[test]
    fn launch_flags_of_another_agent_are_not_added() {
        let session = super::super::snapshot::PaneAgentSessionSnapshot {
            source: "herdr:codex".into(),
            agent: "codex".into(),
            kind: crate::agent_resume::AgentSessionRefKind::Id,
            value: "t1".into(),
        };
        assert_eq!(
            restore_argv(
                Some(&session),
                None,
                &[("claude", &["--settings", "/u/x.json"])]
            ),
            ["codex", "resume", "t1"]
        );
    }

    #[test]
    fn pane_restore_startup_keeps_history_without_native_agent_resume() {
        let session = super::super::snapshot::PaneAgentSessionSnapshot {
            source: "herdr:pi".into(),
            agent: "pi".into(),
            kind: crate::agent_resume::AgentSessionRefKind::Path,
            value: test_session_path("pi-session.jsonl"),
        };
        let history = super::super::snapshot::PaneHistorySnapshot {
            ansi: "RESTORED_HISTORY\r\n".into(),
            lines: 1,
        };
        let mut resumed = HashSet::new();
        let mut agent_restore = AgentRestoreState {
            enabled: false,
            resumed_sessions: &mut resumed,
        };

        let startup = pane_restore_startup(
            Some(&session),
            None,
            std::path::Path::new("/a"),
            Some(&history),
            None,
            &mut agent_restore,
        );

        assert!(startup.restore_plan.is_none());
        assert_eq!(startup.initial_history_ansi, Some("RESTORED_HISTORY\r\n"));
        assert!(!startup.duplicate_agent_session);
        assert!(resumed.is_empty());
    }

    #[test]
    fn restore_rehydrates_agent_session_metadata() {
        let session = super::super::snapshot::PaneAgentSessionSnapshot {
            source: "herdr:hermes".into(),
            agent: "hermes".into(),
            kind: crate::agent_resume::AgentSessionRefKind::Id,
            value: "hermes-session".into(),
        };

        let preserved = restored_terminal_agent_session(Some(&session), false)
            .expect("restore should preserve metadata");
        assert_eq!(preserved.source, "herdr:hermes");
        assert_eq!(preserved.agent, "hermes");
        assert_eq!(preserved.session_ref.value, "hermes-session");
    }

    #[test]
    fn restore_does_not_rehydrate_duplicate_agent_session_metadata() {
        let session = super::super::snapshot::PaneAgentSessionSnapshot {
            source: "herdr:pi".into(),
            agent: "pi".into(),
            kind: crate::agent_resume::AgentSessionRefKind::Path,
            value: test_session_path("pi-session.jsonl"),
        };
        let mut resumed = HashSet::new();
        assert!(take_restore_plan_for_snapshot(&session, true, &mut resumed).is_some());
        assert!(take_restore_plan_for_snapshot(&session, true, &mut resumed).is_none());

        assert!(restored_terminal_agent_session(Some(&session), true).is_none());
    }

    #[tokio::test]
    async fn failed_cold_restore_preserves_panes_and_saved_directories() {
        for missing_shell in [false, true] {
            let mut snapshot: SessionSnapshot = serde_json::from_str(include_str!(
                "../../tests/fixtures/session/current-herdr-session.json"
            ))
            .unwrap();
            let cwd = std::env::current_dir().unwrap();
            let missing = cwd.join("__herdr_missing_restore_directory__");
            assert!(!missing.exists());
            for workspace in &mut snapshot.workspaces {
                workspace.identity_cwd = cwd.clone();
                for tab in &mut workspace.tabs {
                    for pane in tab.panes.values_mut() {
                        pane.cwd = cwd.clone();
                    }
                }
            }
            let membership = crate::workspace::WorktreeSpaceMembership {
                key: "saved-repo-key".into(),
                label: "saved-repo".into(),
                repo_root: cwd.clone(),
                checkout_path: missing.clone(),
                is_linked_worktree: true,
            };
            snapshot.workspaces[0].worktree_space = Some(membership.clone());
            let failed = snapshot.workspaces[0].tabs[0].panes.get_mut(&1).unwrap();
            failed.cwd = missing.clone();
            failed.label = Some("keep my pane".into());
            failed.agent_session = Some(super::super::snapshot::PaneAgentSessionSnapshot {
                source: "herdr:opencode".into(),
                agent: "opencode".into(),
                kind: crate::agent_resume::AgentSessionRefKind::Id,
                value: "keep-my-session".into(),
            });
            let (events, _rx) = mpsc::channel(32);
            let (workspaces, terminals, runtimes) = restore(
                &snapshot,
                None,
                24,
                80,
                0,
                if missing_shell {
                    "__herdr_missing_restore_shell__"
                } else {
                    test_restore_shell()
                },
                crate::config::ShellModeConfig::NonLogin,
                false,
                events,
                Arc::new(Notify::new()),
                Arc::new(RenderSignal::new()),
            );
            let runtimes = crate::terminal::TerminalRuntimeRegistry::from(runtimes);
            let captured =
                crate::persist::capture(&workspaces, &terminals, &runtimes, Some(0), 0, None);
            assert_eq!(
                captured.workspaces.len(),
                2,
                "a launch failure must not delete a workspace"
            );
            assert_eq!(captured.workspaces[0].tabs.len(), 2);
            assert_eq!(
                captured.workspaces[0].worktree_space,
                Some(membership),
                "an unavailable checkout must retain its saved repository group across restart"
            );
            let pane = captured.workspaces[0].tabs[0]
                .panes
                .values()
                .next()
                .unwrap();
            assert_eq!(
                pane.cwd, missing,
                "fallback cwd must not replace saved intent"
            );
            assert_eq!(pane.label.as_deref(), Some("keep my pane"));
            assert_eq!(
                pane.agent_session.as_ref().unwrap().value,
                "keep-my-session"
            );
            let root = workspaces[0].tabs[0].root_pane;
            let terminal_id = workspaces[0].tabs[0].terminal_id(root).unwrap();
            assert!(
                runtimes.get(terminal_id).is_none(),
                "do not open a replacement shell elsewhere"
            );
            let healthy = workspaces[1].tabs[0]
                .terminal_id(workspaces[1].tabs[0].root_pane)
                .unwrap();
            assert_eq!(runtimes.get(healthy).is_some(), !missing_shell);
            assert!(terminals[terminal_id].restore_error.is_some());
            let mut state = crate::app::AppState::test_new();
            state.workspaces = workspaces;
            state.terminals = terminals;
            state.active = Some(0);
            state.assert_invariants_for_test();
        }
    }

    #[tokio::test]
    async fn restore_carries_persisted_agent_session_metadata() {
        let cwd = std::env::current_dir().unwrap();
        let snapshot = SessionSnapshot {
            version: super::super::snapshot::SNAPSHOT_VERSION,
            workspaces: vec![WorkspaceSnapshot {
                id: Some("workspace".into()),
                custom_name: None,
                identity_cwd: cwd.clone(),
                worktree_space: None,
                pin_order: None,
                public_pane_numbers: HashMap::new(),
                next_public_pane_number: 0,
                public_tab_numbers: Vec::new(),
                next_public_tab_number: 0,
                tabs: vec![TabSnapshot {
                    custom_name: None,
                    layout: LayoutSnapshot::Pane(0),
                    panes: HashMap::from([(
                        0,
                        super::super::snapshot::PaneSnapshot {
                            cwd,
                            label: Some("reviewer".into()),
                            agent_name: Some("reviewer".into()),
                            managed_agent_kind: Some("opencode".into()),
                            agent_session: Some(super::super::snapshot::PaneAgentSessionSnapshot {
                                source: "herdr:opencode".into(),
                                agent: "opencode".into(),
                                kind: crate::agent_resume::AgentSessionRefKind::Id,
                                value: "opencode-session".into(),
                            }),
                            agent_resume: None,
                            agent_launch: None,
                            launch_argv: None,
                            todos: Vec::new(),
                            next_todo_id: 1,
                            last_input_at_ms: None,
                            pin_order: None,
                            former_public_ids: Vec::new(),
                        },
                    )]),
                    zoomed: false,
                    focused: Some(0),
                    root_pane: Some(0),
                }],
                active_tab: 0,
            }],
            active: Some(0),
            selected: 0,
            sidebar_width: None,
            sidebar_section_split: None,
            collapsed_space_keys: Default::default(),
            last_client_size: None,
        };
        let (events, _event_rx) = mpsc::channel(4);

        let (_workspaces, terminals, _runtimes) = restore(
            &snapshot,
            None,
            24,
            80,
            0,
            test_restore_shell(),
            crate::config::ShellModeConfig::NonLogin,
            false,
            events,
            Arc::new(Notify::new()),
            Arc::new(RenderSignal::new()),
        );

        let terminal = terminals
            .values()
            .next()
            .expect("restored terminal should exist");
        assert!(
            !terminal.respawn_shell_on_exit,
            "agent sessions should not use native restore lifecycle when resume_agents_on_restore is disabled"
        );
        assert_eq!(terminal.agent_name, None);
        assert_eq!(terminal.manual_label.as_deref(), Some("reviewer"));
        let session = terminal
            .persisted_agent_session
            .as_ref()
            .expect("persisted agent session should survive restore");
        assert_eq!(session.source, "herdr:opencode");
        assert_eq!(session.agent, "opencode");
        assert_eq!(session.session_ref.value, "opencode-session");
    }

    #[tokio::test]
    async fn a_hand_started_agents_name_survives_a_restore_that_resumes_its_session() {
        let cwd = std::env::current_dir().unwrap();
        let snapshot = SessionSnapshot {
            version: super::super::snapshot::SNAPSHOT_VERSION,
            workspaces: vec![WorkspaceSnapshot {
                id: Some("workspace".into()),
                custom_name: None,
                identity_cwd: cwd.clone(),
                worktree_space: None,
                pin_order: None,
                public_pane_numbers: HashMap::new(),
                next_public_pane_number: 0,
                public_tab_numbers: Vec::new(),
                next_public_tab_number: 0,
                tabs: vec![TabSnapshot {
                    custom_name: None,
                    layout: LayoutSnapshot::Pane(0),
                    panes: HashMap::from([(
                        0,
                        super::super::snapshot::PaneSnapshot {
                            cwd,
                            label: None,
                            agent_name: Some("cchv-helper".into()),
                            managed_agent_kind: None,
                            agent_session: Some(super::super::snapshot::PaneAgentSessionSnapshot {
                                source: "herdr:claude".into(),
                                agent: "claude".into(),
                                kind: crate::agent_resume::AgentSessionRefKind::Id,
                                value: "d10d1698-e95a-45a1-a81b-c49313db53a9".into(),
                            }),
                            agent_resume: None,
                            agent_launch: None,
                            launch_argv: None,
                            todos: Vec::new(),
                            next_todo_id: 1,
                            last_input_at_ms: None,
                            pin_order: None,
                            former_public_ids: Vec::new(),
                        },
                    )]),
                    zoomed: false,
                    focused: Some(0),
                    root_pane: Some(0),
                }],
                active_tab: 0,
            }],
            active: Some(0),
            selected: 0,
            sidebar_width: None,
            sidebar_section_split: None,
            collapsed_space_keys: Default::default(),
            last_client_size: None,
        };
        let (events, _event_rx) = mpsc::channel(4);

        let (_workspaces, terminals, _runtimes) = restore(
            &snapshot,
            None,
            24,
            80,
            0,
            test_restore_shell(),
            crate::config::ShellModeConfig::NonLogin,
            true,
            events,
            Arc::new(Notify::new()),
            Arc::new(RenderSignal::new()),
        );

        let terminal = terminals
            .values()
            .next()
            .expect("restored terminal should exist");
        // Fork issue 136: the herdr name came back with the resumed session.
        assert_eq!(terminal.agent_name.as_deref(), Some("cchv-helper"));
        assert_eq!(terminal.managed_agent_kind(), None);
    }

    #[tokio::test]
    async fn restore_preserves_public_id_mapping_after_pane_id_remap() {
        let cwd = std::env::current_dir().unwrap();
        let snapshot = SessionSnapshot {
            version: super::super::snapshot::SNAPSHOT_VERSION,
            workspaces: vec![WorkspaceSnapshot {
                id: Some("w1".into()),
                custom_name: None,
                identity_cwd: cwd.clone(),
                worktree_space: None,
                pin_order: None,
                public_pane_numbers: HashMap::from([(10, 1), (20, 3)]),
                next_public_pane_number: 4,
                public_tab_numbers: vec![5],
                next_public_tab_number: 6,
                tabs: vec![TabSnapshot {
                    custom_name: None,
                    layout: LayoutSnapshot::Split {
                        direction: super::super::snapshot::DirectionSnapshot::Horizontal,
                        ratio: 0.5,
                        first: Box::new(LayoutSnapshot::Pane(10)),
                        second: Box::new(LayoutSnapshot::Pane(20)),
                    },
                    panes: HashMap::from([
                        (
                            10,
                            super::super::snapshot::PaneSnapshot {
                                cwd: cwd.clone(),
                                label: None,
                                agent_name: None,
                                managed_agent_kind: None,
                                agent_session: None,
                                agent_resume: None,
                                agent_launch: None,
                                launch_argv: None,
                                todos: Vec::new(),
                                next_todo_id: 1,
                                last_input_at_ms: None,
                                pin_order: None,
                                former_public_ids: Vec::new(),
                            },
                        ),
                        (
                            20,
                            super::super::snapshot::PaneSnapshot {
                                cwd: cwd.clone(),
                                label: None,
                                agent_name: None,
                                managed_agent_kind: None,
                                agent_session: None,
                                agent_resume: None,
                                agent_launch: None,
                                launch_argv: None,
                                todos: Vec::new(),
                                next_todo_id: 1,
                                last_input_at_ms: None,
                                pin_order: None,
                                former_public_ids: Vec::new(),
                            },
                        ),
                    ]),
                    zoomed: false,
                    focused: Some(10),
                    root_pane: Some(10),
                }],
                active_tab: 0,
            }],
            active: Some(0),
            selected: 0,
            sidebar_width: None,
            sidebar_section_split: None,
            collapsed_space_keys: Default::default(),
            last_client_size: None,
        };
        let (events, _event_rx) = mpsc::channel(4);

        let (workspaces, _terminals, _runtimes) = restore(
            &snapshot,
            None,
            24,
            80,
            0,
            test_restore_shell(),
            crate::config::ShellModeConfig::NonLogin,
            false,
            events,
            Arc::new(Notify::new()),
            Arc::new(RenderSignal::new()),
        );

        let workspace = workspaces.first().expect("workspace should restore");
        let mut public_numbers: Vec<_> = workspace.public_pane_numbers.values().copied().collect();
        public_numbers.sort_unstable();
        assert_eq!(public_numbers, vec![1, 3]);
        assert_eq!(workspace.next_public_pane_number, 4);
        assert_eq!(workspace.tabs[0].number, 5);
        assert_eq!(workspace.next_public_tab_number, 6);
    }

    #[tokio::test]
    async fn a_moved_panes_former_ids_survive_a_restore_that_remaps_pane_ids() {
        let cwd = std::env::current_dir().unwrap();
        let snapshot = SessionSnapshot {
            version: super::super::snapshot::SNAPSHOT_VERSION,
            workspaces: vec![WorkspaceSnapshot {
                id: Some("w1".into()),
                custom_name: None,
                identity_cwd: cwd.clone(),
                worktree_space: None,
                pin_order: None,
                public_pane_numbers: HashMap::from([(10, 1), (20, 3)]),
                next_public_pane_number: 4,
                public_tab_numbers: vec![5],
                next_public_tab_number: 6,
                tabs: vec![TabSnapshot {
                    custom_name: None,
                    layout: LayoutSnapshot::Split {
                        direction: super::super::snapshot::DirectionSnapshot::Horizontal,
                        ratio: 0.5,
                        first: Box::new(LayoutSnapshot::Pane(10)),
                        second: Box::new(LayoutSnapshot::Pane(20)),
                    },
                    panes: HashMap::from([
                        (
                            10,
                            super::super::snapshot::PaneSnapshot {
                                agent_resume: None,
                                agent_launch: None,
                                cwd: cwd.clone(),
                                label: None,
                                agent_name: None,
                                managed_agent_kind: None,
                                agent_session: None,
                                launch_argv: None,
                                todos: Vec::new(),
                                next_todo_id: 1,
                                last_input_at_ms: None,
                                pin_order: None,
                                former_public_ids: Vec::new(),
                            },
                        ),
                        (
                            20,
                            super::super::snapshot::PaneSnapshot {
                                agent_resume: None,
                                agent_launch: None,
                                cwd: cwd.clone(),
                                label: None,
                                agent_name: None,
                                managed_agent_kind: None,
                                agent_session: None,
                                launch_argv: None,
                                todos: Vec::new(),
                                next_todo_id: 1,
                                last_input_at_ms: None,
                                pin_order: None,
                                former_public_ids: vec!["w1:p1".into(), "w9:p7".into()],
                            },
                        ),
                    ]),
                    zoomed: false,
                    focused: Some(10),
                    root_pane: Some(10),
                }],
                active_tab: 0,
            }],
            active: Some(0),
            selected: 0,
            sidebar_width: None,
            sidebar_section_split: None,
            collapsed_space_keys: Default::default(),
            last_client_size: None,
        };
        let (events, _event_rx) = mpsc::channel(4);

        let (workspaces, _terminals, _runtimes) = restore(
            &snapshot,
            None,
            24,
            80,
            0,
            test_restore_shell(),
            crate::config::ShellModeConfig::NonLogin,
            false,
            events,
            Arc::new(Notify::new()),
            Arc::new(RenderSignal::new()),
        );

        // #111: pane 20 came from space w9 as `w9:p7`, and its shell still
        // reports under that id. After the restore renumbers the panes, the
        // id finds the pane that sits where 20 sat. `w1:p1` is the live id of
        // the other pane, so it is not kept as a former id: a live id wins.
        let aliases = restored_former_public_ids(&snapshot, &workspaces);
        let second = workspaces[0].tabs[0].layout.pane_ids()[1];
        assert_eq!(aliases.get("w9:p7"), Some(&second));
        assert_eq!(aliases.get("w1:p1"), None);
        assert_eq!(aliases.len(), 1);
    }

    #[tokio::test]
    async fn cold_restore_with_gapped_public_tab_numbers_drops_unmanaged_agent_name() {
        let cwd = std::env::current_dir().unwrap();
        let pane_snap = |id: &str| {
            (
                id.parse::<u32>().unwrap(),
                super::super::snapshot::PaneSnapshot {
                    cwd: cwd.clone(),
                    label: None,
                    agent_name: None,
                    managed_agent_kind: None,
                    agent_session: None,
                    agent_resume: None,
                    agent_launch: None,
                    launch_argv: None,
                    todos: Vec::new(),
                    next_todo_id: 1,
                    last_input_at_ms: None,
                    pin_order: None,
                    former_public_ids: Vec::new(),
                },
            )
        };
        let final_pane = super::super::snapshot::PaneSnapshot {
            cwd: cwd.clone(),
            label: Some("planner".into()),
            agent_name: Some("planner".into()),
            managed_agent_kind: None,
            agent_session: Some(super::super::snapshot::PaneAgentSessionSnapshot {
                source: "herdr:codex".into(),
                agent: "codex".into(),
                kind: crate::agent_resume::AgentSessionRefKind::Id,
                value: "codex-session".into(),
            }),
            agent_resume: None,
            agent_launch: None,
            launch_argv: None,
            todos: Vec::new(),
            next_todo_id: 1,
            last_input_at_ms: None,
            pin_order: None,
            former_public_ids: Vec::new(),
        };
        let snapshot = SessionSnapshot {
            version: super::super::snapshot::SNAPSHOT_VERSION,
            workspaces: vec![WorkspaceSnapshot {
                id: Some("w1".into()),
                custom_name: None,
                identity_cwd: cwd.clone(),
                worktree_space: None,
                pin_order: None,
                public_pane_numbers: HashMap::from([(10, 1), (11, 2), (12, 3), (13, 4)]),
                next_public_pane_number: 5,
                public_tab_numbers: vec![1, 3, 4, 5],
                next_public_tab_number: 6,
                tabs: vec![
                    TabSnapshot {
                        custom_name: None,
                        layout: LayoutSnapshot::Pane(10),
                        panes: HashMap::from([pane_snap("10")]),
                        zoomed: false,
                        focused: Some(10),
                        root_pane: Some(10),
                    },
                    TabSnapshot {
                        custom_name: None,
                        layout: LayoutSnapshot::Pane(11),
                        panes: HashMap::from([pane_snap("11")]),
                        zoomed: false,
                        focused: Some(11),
                        root_pane: Some(11),
                    },
                    TabSnapshot {
                        custom_name: None,
                        layout: LayoutSnapshot::Pane(12),
                        panes: HashMap::from([pane_snap("12")]),
                        zoomed: false,
                        focused: Some(12),
                        root_pane: Some(12),
                    },
                    TabSnapshot {
                        custom_name: None,
                        layout: LayoutSnapshot::Pane(13),
                        panes: HashMap::from([(13, final_pane)]),
                        zoomed: false,
                        focused: Some(13),
                        root_pane: Some(13),
                    },
                ],
                active_tab: 3,
            }],
            active: Some(0),
            selected: 0,
            sidebar_width: None,
            sidebar_section_split: None,
            collapsed_space_keys: Default::default(),
            last_client_size: None,
        };
        let (events, _event_rx) = mpsc::channel(4);

        let (workspaces, terminals, _runtimes) = restore(
            &snapshot,
            None,
            24,
            80,
            0,
            test_restore_shell(),
            crate::config::ShellModeConfig::NonLogin,
            false,
            events,
            Arc::new(Notify::new()),
            Arc::new(RenderSignal::new()),
        );

        let workspace = workspaces.first().expect("workspace should restore");
        assert_eq!(workspace.active_tab, 3);
        assert_eq!(workspace.tabs[3].number, 5);
        let agent_pane = workspace.tabs[3].root_pane;
        let terminal_id = &workspace.tabs[3].panes[&agent_pane].attached_terminal_id;
        assert!(terminals[terminal_id].agent_name.is_none());
        assert_eq!(terminals[terminal_id].managed_agent_kind(), None);
        assert!(workspace
            .pane_details(&terminals)
            .into_iter()
            .all(|detail| detail.pane_id != agent_pane));
    }

    #[test]
    fn legacy_restore_precomputes_missing_public_pane_numbers() {
        let cwd = std::env::current_dir().unwrap();
        let snapshot = WorkspaceSnapshot {
            id: Some("w1".into()),
            custom_name: None,
            identity_cwd: cwd,
            worktree_space: None,
            pin_order: None,
            public_pane_numbers: HashMap::new(),
            next_public_pane_number: 0,
            public_tab_numbers: Vec::new(),
            next_public_tab_number: 0,
            tabs: vec![TabSnapshot {
                custom_name: None,
                layout: LayoutSnapshot::Split {
                    direction: super::super::snapshot::DirectionSnapshot::Horizontal,
                    ratio: 0.5,
                    first: Box::new(LayoutSnapshot::Pane(10)),
                    second: Box::new(LayoutSnapshot::Pane(20)),
                },
                panes: HashMap::new(),
                zoomed: false,
                focused: Some(10),
                root_pane: Some(10),
            }],
            active_tab: 0,
        };
        let mut next_public_pane_number = 1;

        let public_numbers =
            migrated_public_pane_numbers_by_old_raw(&snapshot, &mut next_public_pane_number);

        assert_eq!(public_numbers, HashMap::from([(10, 1), (20, 2)]));
        assert_eq!(next_public_pane_number, 3);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn native_agent_restore_defers_runtime_launch() {
        let cwd = std::env::current_dir().unwrap();
        let snapshot = SessionSnapshot {
            version: super::super::snapshot::SNAPSHOT_VERSION,
            workspaces: vec![WorkspaceSnapshot {
                id: Some("workspace".into()),
                custom_name: None,
                identity_cwd: cwd.clone(),
                worktree_space: None,
                pin_order: None,
                public_pane_numbers: HashMap::new(),
                next_public_pane_number: 0,
                public_tab_numbers: Vec::new(),
                next_public_tab_number: 0,
                tabs: vec![TabSnapshot {
                    custom_name: None,
                    layout: LayoutSnapshot::Pane(0),
                    panes: HashMap::from([(
                        0,
                        super::super::snapshot::PaneSnapshot {
                            cwd,
                            label: None,
                            agent_name: None,
                            managed_agent_kind: None,
                            agent_session: Some(super::super::snapshot::PaneAgentSessionSnapshot {
                                source: "herdr:codex".into(),
                                agent: "codex".into(),
                                kind: crate::agent_resume::AgentSessionRefKind::Id,
                                value: "codex-session".into(),
                            }),
                            agent_resume: None,
                            agent_launch: None,
                            launch_argv: None,
                            todos: Vec::new(),
                            next_todo_id: 1,
                            last_input_at_ms: None,
                            pin_order: None,
                            former_public_ids: Vec::new(),
                        },
                    )]),
                    zoomed: false,
                    focused: Some(0),
                    root_pane: Some(0),
                }],
                active_tab: 0,
            }],
            active: Some(0),
            selected: 0,
            sidebar_width: None,
            sidebar_section_split: None,
            collapsed_space_keys: Default::default(),
            last_client_size: None,
        };
        let (events, _event_rx) = mpsc::channel(4);

        let (_workspaces, terminals, runtimes) = restore(
            &snapshot,
            None,
            24,
            80,
            0,
            test_restore_shell(),
            crate::config::ShellModeConfig::NonLogin,
            true,
            events,
            Arc::new(Notify::new()),
            Arc::new(RenderSignal::new()),
        );

        let terminal = terminals
            .values()
            .next()
            .expect("native agent restore should create terminal state");
        assert!(
            terminal.pending_agent_resume_plan.is_some(),
            "restored native agent panes should defer resume until client terminal context is known"
        );
        assert!(
            !terminal.respawn_shell_on_exit,
            "deferred agent resume should not use native restore lifecycle before launch"
        );
        assert!(
            runtimes.is_empty(),
            "native agent restore should not spawn a fallback-size runtime during snapshot restore"
        );
        let mut imports = HashMap::new();
        let (_handoff_workspaces, handoff_terminals, handoff_runtimes) = restore_handoff(
            &snapshot,
            0,
            test_restore_shell(),
            crate::config::ShellModeConfig::NonLogin,
            &mut imports,
            mpsc::channel(4).0,
            Arc::new(Notify::new()),
            Arc::new(RenderSignal::new()),
        )
        .expect("handoff restore should preserve pending native agent resume");
        let handoff_terminal = handoff_terminals
            .values()
            .next()
            .expect("handoff restore should create terminal state");
        assert!(
            handoff_terminal.pending_agent_resume_plan.is_some(),
            "handoff restore should preserve pending native agent resume intent"
        );
        assert!(
            handoff_runtimes.is_empty(),
            "handoff restore should not replace pending native agent resume with a shell runtime"
        );
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn live_handoff_preserves_hook_status_until_next_report() {
        for state_before_handoff in [AgentState::Working, AgentState::Blocked] {
            let (snapshot, _) = snapshot_with_saved_pane_history();
            let (events, _events_rx) = mpsc::channel(32);
            let (workspaces, mut terminals, runtimes) = restore(
                &snapshot,
                None,
                24,
                80,
                4096,
                test_restore_shell(),
                crate::config::ShellModeConfig::NonLogin,
                false,
                events.clone(),
                Arc::new(Notify::new()),
                Arc::new(RenderSignal::new()),
            );
            let terminal = terminals.values_mut().next().unwrap();
            terminal
                .set_detected_agent_process_at(crate::detect::Agent::Pi, std::time::Instant::now());
            terminal.set_persisted_agent_session(crate::agent_resume::PersistedAgentSession {
                source: "herdr:pi".into(),
                agent: "pi".into(),
                session_ref: crate::agent_resume::AgentSessionRef::path(
                    "/var/tmp/handoff-test.jsonl",
                )
                .unwrap(),
            });
            terminal.set_hook_authority_with_session_ref(
                "herdr:pi".into(),
                "pi".into(),
                state_before_handoff,
                None,
                Some(
                    crate::agent_resume::AgentSessionRef::path("/var/tmp/handoff-test.jsonl")
                        .unwrap(),
                ),
                Some(1),
            );
            assert_eq!(terminal.state, state_before_handoff);
            let runtimes = crate::terminal::TerminalRuntimeRegistry::from(runtimes);
            let snapshot =
                crate::persist::capture(&workspaces, &terminals, &runtimes, Some(0), 0, None);
            let pane_id = workspaces[0].tabs[0].panes.keys().next().copied().unwrap();
            let runtime = runtimes.values().next().unwrap();
            runtime
                .pause_handoff_reader(std::time::Duration::from_secs(2))
                .unwrap();
            let mut state = runtime.handoff_runtime_state(pane_id.raw());
            state.hook_agent_state = terminals.values().next().unwrap().handoff_agent_state();
            let state = serde_json::from_value(serde_json::to_value(state).unwrap()).unwrap();
            let mut imports = HashMap::from([(
                pane_id.raw(),
                crate::handoff_runtime::ImportedHandoffRuntime {
                    master_fd: runtime.duplicate_handoff_fd().unwrap(),
                    state,
                },
            )]);
            let (_, mut restored_terminals, restored_runtimes) = restore_handoff(
                &snapshot,
                4096,
                test_restore_shell(),
                crate::config::ShellModeConfig::NonLogin,
                &mut imports,
                events,
                Arc::new(Notify::new()),
                Arc::new(RenderSignal::new()),
            )
            .unwrap();
            drop(restored_runtimes);
            drop(runtimes);
            let idle_report = |terminal: &mut crate::terminal::TerminalState| {
                terminal.set_hook_authority_with_session_ref(
                    "herdr:pi".into(),
                    "pi".into(),
                    AgentState::Idle,
                    None,
                    Some(
                        crate::agent_resume::AgentSessionRef::path("/var/tmp/handoff-test.jsonl")
                            .unwrap(),
                    ),
                    Some(2),
                );
            };
            let terminal = restored_terminals.values_mut().next().unwrap();
            assert_eq!(terminal.state, state_before_handoff);
            terminal.set_detected_state(Some(crate::detect::Agent::Pi), AgentState::Idle);
            assert_eq!(
                terminal.state, state_before_handoff,
                "screen fallback must not erase the transferred hook status"
            );
            idle_report(terminal);
            assert_eq!(
                terminal.state,
                AgentState::Idle,
                "the next hook report must take effect immediately"
            );
            // The acquisition flag travels with the status: the restored pane
            // ends acquisition exactly as the pane that never moved does.
            let original = terminals.values_mut().next().unwrap();
            original.set_detected_state(Some(crate::detect::Agent::Pi), AgentState::Idle);
            idle_report(original);
            assert_eq!(
                terminal.finish_agent_process_acquisition(),
                original.finish_agent_process_acquisition()
            );
        }
    }

    /// Fork issue 128: a pane that was done (finished, not yet looked at)
    /// before a live handoff is still done after it.
    #[cfg(unix)]
    #[tokio::test]
    async fn live_handoff_keeps_an_unseen_pane_unseen() {
        for unseen in [true, false] {
            let (snapshot, _) = snapshot_with_saved_pane_history();
            let (events, _events_rx) = mpsc::channel(32);
            let (workspaces, terminals, runtimes) = restore(
                &snapshot,
                None,
                24,
                80,
                4096,
                test_restore_shell(),
                crate::config::ShellModeConfig::NonLogin,
                false,
                events.clone(),
                Arc::new(Notify::new()),
                Arc::new(RenderSignal::new()),
            );
            let runtimes = crate::terminal::TerminalRuntimeRegistry::from(runtimes);
            let snapshot =
                crate::persist::capture(&workspaces, &terminals, &runtimes, Some(0), 0, None);
            let pane_id = workspaces[0].tabs[0].panes.keys().next().copied().unwrap();
            let runtime = runtimes.values().next().unwrap();
            runtime
                .pause_handoff_reader(std::time::Duration::from_secs(2))
                .unwrap();
            let mut state = runtime.handoff_runtime_state(pane_id.raw());
            state.unseen = unseen;
            let state = serde_json::from_value(serde_json::to_value(state).unwrap()).unwrap();
            let mut imports = HashMap::from([(
                pane_id.raw(),
                crate::handoff_runtime::ImportedHandoffRuntime {
                    master_fd: runtime.duplicate_handoff_fd().unwrap(),
                    state,
                },
            )]);
            let (restored_workspaces, _, restored_runtimes) = restore_handoff(
                &snapshot,
                4096,
                test_restore_shell(),
                crate::config::ShellModeConfig::NonLogin,
                &mut imports,
                events,
                Arc::new(Notify::new()),
                Arc::new(RenderSignal::new()),
            )
            .unwrap();
            drop(restored_runtimes);
            drop(runtimes);
            let pane = restored_workspaces[0].tabs[0]
                .panes
                .values()
                .next()
                .unwrap();
            assert_eq!(pane.seen, !unseen, "unseen before the handoff: {unseen}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_seen_pane_writes_no_unseen_field() {
        let mut state: crate::handoff_runtime::HandoffRuntimeState =
            serde_json::from_value(serde_json::json!({
                "pane_id": 1, "child_pid": 2, "rows": 3, "cols": 4,
                "cell_width_px": 0, "cell_height_px": 0
            }))
            .unwrap();
        assert!(!state.unseen, "manifests from older servers read as seen");
        assert!(serde_json::to_value(&state)
            .unwrap()
            .get("unseen")
            .is_none());
        state.unseen = true;
        assert_eq!(serde_json::to_value(&state).unwrap()["unseen"], true);
    }

    #[tokio::test]
    async fn restore_seeds_saved_pane_history_into_runtime() {
        let (snapshot, history) = snapshot_with_saved_pane_history();
        let (events, _events_rx) = mpsc::channel(8);
        let render_notify = Arc::new(Notify::new());
        let render_dirty = Arc::new(RenderSignal::new());

        let (_workspaces, _terminals, runtimes) = restore(
            &snapshot,
            Some(&history),
            5,
            40,
            4096,
            test_restore_shell(),
            crate::config::ShellModeConfig::NonLogin,
            false,
            events,
            render_notify,
            render_dirty,
        );
        let runtime = runtimes
            .values()
            .next()
            .expect("restored runtime should exist");

        let restored_text = runtime.recent_unwrapped_text(10);
        assert!(
            restored_text.contains("RESTORED_HISTORY 👨‍👩‍👧 LINK"),
            "styled Unicode and hyperlink text should survive history replay"
        );

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while runtime.cwd().is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = runtime.try_send_bytes(bytes::Bytes::from_static(b"exit\n"));
    }

    #[tokio::test]
    async fn restore_without_history_snapshot_keeps_pane_contents_empty() {
        let (snapshot, _history) = snapshot_with_saved_pane_history();
        let (events, _events_rx) = mpsc::channel(8);
        let render_notify = Arc::new(Notify::new());
        let render_dirty = Arc::new(RenderSignal::new());

        let (_workspaces, _terminals, runtimes) = restore(
            &snapshot,
            None,
            5,
            40,
            4096,
            test_restore_shell(),
            crate::config::ShellModeConfig::NonLogin,
            false,
            events,
            render_notify,
            render_dirty,
        );
        let runtime = runtimes
            .values()
            .next()
            .expect("restored runtime should exist");

        assert!(
            !runtime
                .recent_unwrapped_text(10)
                .contains("RESTORED_HISTORY"),
            "pane history should not restore unless a history snapshot is supplied"
        );

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while runtime.cwd().is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = runtime.try_send_bytes(bytes::Bytes::from_static(b"exit\n"));
    }

    #[tokio::test]
    async fn restore_rejects_history_from_another_layout_or_without_provenance() {
        for legacy in [false, true] {
            let (mut snapshot, history) = snapshot_with_saved_pane_history();
            let mut value = serde_json::to_value(history).unwrap();
            if legacy {
                value.as_object_mut().unwrap().remove("layout_fingerprint");
            } else {
                snapshot.workspaces[0].tabs[0]
                    .panes
                    .get_mut(&0)
                    .unwrap()
                    .cwd = std::env::temp_dir();
            }
            let history = serde_json::from_value(value).unwrap();
            let (events, _rx) = mpsc::channel(8);
            let (_, _, runtimes) = restore(
                &snapshot,
                Some(&history),
                5,
                80,
                4096,
                test_restore_shell(),
                crate::config::ShellModeConfig::NonLogin,
                false,
                events,
                Arc::new(Notify::new()),
                Arc::new(RenderSignal::new()),
            );
            let runtime = runtimes.values().next().unwrap();
            assert!(
                !runtime
                    .recent_unwrapped_text(10)
                    .contains("RESTORED_HISTORY"),
                "screen history must belong to the exact saved layout"
            );
            for (_, runtime) in runtimes {
                runtime.shutdown();
            }
        }
    }

    fn snapshot_with_saved_pane_history() -> (SessionSnapshot, SessionHistorySnapshot) {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let mut panes = HashMap::new();
        panes.insert(
            0,
            super::super::snapshot::PaneSnapshot {
                cwd: cwd.clone(),
                label: None,
                agent_name: None,
                managed_agent_kind: None,
                agent_session: None,
                agent_resume: None,
                agent_launch: None,
                launch_argv: None,
                todos: Vec::new(),
                next_todo_id: 1,
                last_input_at_ms: None,
                pin_order: None,
                former_public_ids: Vec::new(),
            },
        );
        let mut history = SessionHistorySnapshot {
            version: super::super::snapshot::SNAPSHOT_VERSION,
            layout_fingerprint: None,
            workspaces: vec![WorkspaceHistorySnapshot {
                tabs: vec![super::super::snapshot::TabHistorySnapshot {
                    panes: HashMap::from([(
                        0,
                        super::super::snapshot::PaneHistorySnapshot {
                            ansi: concat!(
                                "\x1b[31mRESTORED_HISTORY 👨‍👩‍👧\x1b[0m ",
                                "\x1b]8;;https://example.com\x1b\\LINK\x1b]8;;\x1b\\\r\n"
                            )
                            .to_string(),
                            lines: 1,
                        },
                    )]),
                }],
            }],
        };
        let snapshot = SessionSnapshot {
            version: super::super::snapshot::SNAPSHOT_VERSION,
            workspaces: vec![WorkspaceSnapshot {
                id: Some("workspace".into()),
                custom_name: None,
                identity_cwd: cwd,
                worktree_space: None,
                pin_order: None,
                public_pane_numbers: HashMap::new(),
                next_public_pane_number: 0,
                public_tab_numbers: Vec::new(),
                next_public_tab_number: 0,
                tabs: vec![TabSnapshot {
                    custom_name: None,
                    layout: LayoutSnapshot::Pane(0),
                    panes,
                    zoomed: false,
                    focused: Some(0),
                    root_pane: Some(0),
                }],
                active_tab: 0,
            }],
            active: Some(0),
            selected: 0,
            sidebar_width: Some(26),
            sidebar_section_split: Some(0.5),
            collapsed_space_keys: Default::default(),
            last_client_size: None,
        };
        history.layout_fingerprint = super::super::snapshot::layout_fingerprint(&snapshot);
        (snapshot, history)
    }
}
