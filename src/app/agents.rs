use std::time::{Duration, Instant};

use bytes::Bytes;

use super::{terminal_targets::TerminalTargetError, App};
use crate::api::schema::AgentStartParams;

const DEFAULT_AGENT_START_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const MAX_AGENT_START_TIMEOUT: Duration = Duration::from_secs(300);
pub(crate) const AGENT_START_SETTLE_DELAY: Duration = Duration::from_secs(3);
const INVALID_AGENT_TIMEOUT_MESSAGE: &str =
    "agent start timeout must be greater than 3000ms and at most 300000ms";
const INVALID_AGENT_NAME_MESSAGE: &str = "agent name must start with a lowercase letter and contain only lowercase letters, digits, '-' or '_' (1-32 characters)";

fn valid_agent_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some('a'..='z'))
        && name.len() <= 32
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '-' | '_'))
}

impl App {
    pub(super) fn collect_agent_infos(&self) -> Vec<crate::api::schema::AgentInfo> {
        let title_names = self.state.agent_title_names();
        let title_names = &title_names;
        self.state
            .workspaces
            .iter()
            .enumerate()
            .flat_map(|(ws_idx, ws)| {
                ws.tabs.iter().flat_map(move |tab| {
                    tab.layout
                        .pane_ids()
                        .into_iter()
                        .filter_map(move |pane_id| {
                            self.agent_info_with_title_names(ws_idx, pane_id, title_names)
                        })
                })
            })
            .collect()
    }

    pub(super) fn reconcile_managed_agent_target(&mut self, target: &str) {
        let Ok(resolved) = self.resolve_agent_target(target) else {
            return;
        };
        let Some(terminal_id) = self
            .state
            .workspaces
            .get(resolved.ws_idx)
            .and_then(|workspace| workspace.terminal_id(resolved.pane_id))
            .cloned()
        else {
            return;
        };
        let changed = self
            .state
            .terminals
            .get_mut(&terminal_id)
            .is_some_and(|terminal| terminal.reconcile_managed_agent_at(Instant::now(), false));
        if changed {
            self.state.mark_session_dirty();
            self.schedule_session_save();
            self.emit_pane_updated(resolved.ws_idx, resolved.pane_id);
        }
    }

    pub(super) fn agent_info_for_target(
        &self,
        target: &str,
    ) -> Result<crate::api::schema::AgentInfo, TerminalTargetError> {
        let resolved = self.resolve_agent_target(target)?;
        self.agent_info(resolved.ws_idx, resolved.pane_id)
            .ok_or_else(|| TerminalTargetError::NotFound {
                target: target.to_string(),
            })
    }

    pub(super) fn focus_agent_target(
        &mut self,
        target: &str,
    ) -> Result<crate::api::schema::AgentInfo, TerminalTargetError> {
        let resolved = self.resolve_agent_target(target)?;
        self.state
            .focus_pane_in_workspace(resolved.ws_idx, resolved.pane_id);
        self.state.mark_active_tab_seen();
        self.state.mode = crate::app::Mode::Terminal;
        self.agent_info(resolved.ws_idx, resolved.pane_id)
            .ok_or_else(|| TerminalTargetError::NotFound {
                target: target.to_string(),
            })
    }

    /// Pin or unpin the agent a target names; pinned agents lead the panel.
    pub(super) fn pin_agent_target(
        &mut self,
        target: &str,
        pin: bool,
    ) -> Result<crate::api::schema::AgentInfo, TerminalTargetError> {
        let resolved = self.resolve_agent_target(target)?;
        if pin {
            self.state.pin_agent(resolved.pane_id);
        } else {
            self.state.unpin_agent(resolved.pane_id);
        }
        self.agent_info(resolved.ws_idx, resolved.pane_id)
            .ok_or_else(|| TerminalTargetError::NotFound {
                target: target.to_string(),
            })
    }

    pub(super) fn rename_agent_target(
        &mut self,
        target: &str,
        name: Option<String>,
    ) -> Result<crate::api::schema::AgentInfo, AgentRenameError> {
        let resolved = self
            .resolve_agent_target(target)
            .map_err(AgentRenameError::Target)?;
        let normalized_name = match name {
            Some(name) if valid_agent_name(&name) => Some(name),
            Some(_) => return Err(AgentRenameError::InvalidName),
            None => None,
        };

        if let Some(name) = normalized_name.as_deref() {
            let conflicts = self.agent_name_conflicts(name, &resolved.terminal_id);
            if !conflicts.is_empty() {
                return Err(AgentRenameError::DuplicateName {
                    name: name.to_string(),
                    candidates: conflicts,
                });
            }
        }

        // Only looked up when a Codex thread may need naming, and only here:
        // a rename is rare, and this reads the pane's process table.
        let codex_process = self
            .codex_app_server
            .naming_socket()
            .and_then(|_| {
                let terminal_id = self
                    .state
                    .workspaces
                    .get(resolved.ws_idx)?
                    .terminal_id(resolved.pane_id)?;
                self.terminal_runtimes.get(terminal_id)
            })
            .and_then(pane_codex_process);

        let Some(terminal) = self
            .state
            .terminals
            .values_mut()
            .find(|terminal| terminal.id.to_string() == resolved.terminal_id)
        else {
            return Err(AgentRenameError::Target(TerminalTargetError::NotFound {
                target: target.to_string(),
            }));
        };
        if terminal.managed_agent_launch_pending() {
            return Err(AgentRenameError::PendingLaunch);
        }
        if terminal.effective_agent_label().is_none() {
            return Err(AgentRenameError::NotAgent);
        }
        let codex_rename = (terminal.effective_agent_label() == Some("codex"))
            .then(|| {
                let new_name = normalized_name.clone()?;
                let thread_id = terminal
                    .persisted_agent_session
                    .as_ref()
                    .filter(|session| session.agent == "codex")
                    .map(|session| session.session_ref.value.clone())
                    .or_else(|| {
                        codex_process
                            .as_ref()
                            .and_then(|process| process.resumed_thread_id.clone())
                    });
                Some(crate::codex_app_server::NameJob::Rename {
                    thread_id,
                    cwd: terminal.cwd.clone(),
                    old_name: terminal.agent_name.clone(),
                    new_name,
                    process_started_at: codex_process
                        .as_ref()
                        .and_then(|process| process.started_at),
                })
            })
            .flatten();
        match normalized_name {
            Some(name) => terminal.set_agent_name(name),
            None => terminal.clear_agent_name(),
        }
        if let (Some(job), Some(socket)) = (codex_rename, self.codex_app_server.naming_socket()) {
            crate::codex_app_server::spawn_name_job(
                socket.to_path_buf(),
                job,
                Some(self.codex_thread_reply(resolved.pane_id)),
            );
        }
        self.state.mark_session_dirty();
        self.schedule_session_save();
        self.emit_pane_updated(resolved.ws_idx, resolved.pane_id);
        self.agent_info(resolved.ws_idx, resolved.pane_id)
            .ok_or_else(|| {
                AgentRenameError::Target(TerminalTargetError::NotFound {
                    target: target.to_string(),
                })
            })
    }

    pub(super) fn start_agent(
        &mut self,
        params: AgentStartParams,
    ) -> Result<(crate::api::schema::AgentInfo, Vec<String>), AgentStartError> {
        let name = params.name;
        if !valid_agent_name(&name) {
            return Err(AgentStartError::InvalidName);
        }
        let Some(kind) = crate::detect::parse_agent_label(&params.kind) else {
            return Err(AgentStartError::UnsupportedKind(params.kind));
        };
        if params
            .args
            .iter()
            .any(|arg| arg.chars().any(char::is_control))
        {
            return Err(AgentStartError::InvalidArgument);
        }
        let persisted_agent_session =
            crate::agent_resume::persisted_session_from_launch_args(kind, &params.args);
        let conflicts = self.agent_name_conflicts(&name, "");
        if !conflicts.is_empty() {
            return Err(AgentStartError::DuplicateName {
                name,
                candidates: conflicts,
            });
        }
        let Some((ws_idx, pane_id)) = self.parse_current_public_pane_id(&params.pane_id) else {
            return Err(AgentStartError::TargetNotFound(params.pane_id));
        };
        let terminal_id = self
            .state
            .workspaces
            .get(ws_idx)
            .and_then(|workspace| workspace.terminal_id(pane_id))
            .cloned()
            .ok_or_else(|| AgentStartError::TargetNotFound(params.pane_id.clone()))?;
        let terminal = self
            .state
            .terminals
            .get(&terminal_id)
            .ok_or_else(|| AgentStartError::TargetNotFound(params.pane_id.clone()))?;
        if terminal.is_agent_terminal() || terminal.managed_agent_kind().is_some() {
            return Err(AgentStartError::TargetBusy(params.pane_id));
        }
        let runtime = self
            .terminal_runtimes
            .get(&terminal_id)
            .ok_or_else(|| AgentStartError::TargetUnavailable(params.pane_id.clone()))?;
        let shell_name = available_shell_name(runtime)
            .ok_or_else(|| AgentStartError::TargetBusy(params.pane_id.clone()))?;

        let pane_cwd = runtime.cwd().unwrap_or_else(|| terminal.cwd.clone());
        let argv = agent_start_argv(kind, params.args, &self.codex_app_server, &pane_cwd);
        let launches_on_codex_daemon =
            kind == crate::detect::Agent::Codex && self.codex_app_server.naming_socket().is_some();
        let command = crate::platform::interactive_shell_command(&argv, &shell_name)
            .ok_or(AgentStartError::InvalidArgument)?;
        let bytes = crate::app::api_helpers::encode_api_submission(runtime, &command);
        let timeout = Duration::from_millis(
            params
                .timeout_ms
                .unwrap_or(DEFAULT_AGENT_START_TIMEOUT.as_millis() as u64),
        );
        if timeout <= AGENT_START_SETTLE_DELAY || timeout > MAX_AGENT_START_TIMEOUT {
            return Err(AgentStartError::InvalidTimeout);
        }

        let now = Instant::now();
        let terminal = self
            .state
            .terminals
            .get_mut(&terminal_id)
            .ok_or_else(|| AgentStartError::TargetUnavailable(params.pane_id.clone()))?;
        terminal.begin_managed_agent(name.clone(), kind, now, AGENT_START_SETTLE_DELAY, timeout);
        // The start cannot report ready before the settle delay; until then an identified agent is
        // re-probed fast so a process that swapped itself in place is seen in time (issue 183).
        runtime.probe_fast_until(now + AGENT_START_SETTLE_DELAY);
        if let Err(err) = runtime.try_send_bytes(Bytes::from(bytes)) {
            terminal.clear_agent_name();
            return Err(AgentStartError::InputFailed(err.to_string()));
        }
        if let Some(session) = persisted_agent_session {
            terminal.set_managed_agent_launch_session(session);
        }
        self.state.mark_session_dirty();
        self.schedule_session_save();
        if launches_on_codex_daemon {
            if let Some(socket) = self.codex_app_server.naming_socket() {
                crate::codex_app_server::spawn_name_job(
                    socket.to_path_buf(),
                    crate::codex_app_server::NameJob::Discover {
                        cwd: pane_cwd,
                        launched_at: crate::codex_app_server::unix_now_ms(),
                        name: name.clone(),
                    },
                    Some(self.codex_thread_reply(pane_id)),
                );
            }
        }

        let agent = self
            .agent_info(ws_idx, pane_id)
            .ok_or(AgentStartError::TargetUnavailable(params.pane_id))?;
        Ok((agent, argv))
    }

    pub(super) fn agent_start_error_body(
        &self,
        err: AgentStartError,
    ) -> crate::api::schema::ErrorBody {
        match err {
            AgentStartError::InvalidName => crate::api::schema::ErrorBody {
                code: "invalid_agent_name".into(),
                message: INVALID_AGENT_NAME_MESSAGE.into(),
            },
            AgentStartError::UnsupportedKind(kind) => crate::api::schema::ErrorBody {
                code: "unsupported_agent_kind".into(),
                message: format!("unsupported interactive agent kind {kind}"),
            },
            AgentStartError::InvalidArgument => crate::api::schema::ErrorBody {
                code: "invalid_agent_argument".into(),
                message: "agent arguments cannot be encoded safely for the target shell".into(),
            },
            AgentStartError::InvalidTimeout => crate::api::schema::ErrorBody {
                code: "invalid_agent_timeout".into(),
                message: INVALID_AGENT_TIMEOUT_MESSAGE.into(),
            },
            AgentStartError::TargetNotFound(target) => crate::api::schema::ErrorBody {
                code: "agent_pane_not_found".into(),
                message: format!("agent target pane {target} not found"),
            },
            AgentStartError::TargetBusy(target) => crate::api::schema::ErrorBody {
                code: "agent_pane_busy".into(),
                message: format!("agent target pane {target} is not an available shell"),
            },
            AgentStartError::TargetUnavailable(target) => crate::api::schema::ErrorBody {
                code: "agent_pane_unavailable".into(),
                message: format!("agent target pane {target} has no live terminal"),
            },
            AgentStartError::InputFailed(message) => crate::api::schema::ErrorBody {
                code: "agent_start_input_failed".into(),
                message,
            },
            AgentStartError::DuplicateName { name, candidates } => crate::api::schema::ErrorBody {
                code: "agent_name_taken".into(),
                message: format!(
                    "agent name {name} is already used; candidates: {}",
                    candidates
                        .into_iter()
                        .map(|candidate| format!(
                            "terminal_id={} pane_id={} workspace_id={} tab_id={} cwd={} status={:?}",
                            candidate.terminal_id,
                            candidate.pane_id,
                            candidate.workspace_id,
                            candidate.tab_id,
                            candidate.cwd.unwrap_or_else(|| "unknown".into()),
                            candidate.agent_status,
                        ))
                        .collect::<Vec<_>>()
                        .join("; ")
                ),
            },
        }
    }

    pub(super) fn agent_target_error_body(
        &self,
        err: TerminalTargetError,
    ) -> crate::api::schema::ErrorBody {
        match err {
            TerminalTargetError::NotFound { target } => crate::api::schema::ErrorBody {
                code: "agent_not_found".into(),
                message: format!("agent target {target} not found"),
            },
            TerminalTargetError::Ambiguous { target, candidates } => {
                crate::api::schema::ErrorBody {
                    code: "agent_target_ambiguous".into(),
                    message: format!(
                        "agent target {target} is ambiguous; candidates: {}",
                        candidates
                            .into_iter()
                            .map(|candidate| format!(
                                "terminal_id={} pane_id={} workspace_id={} tab_id={} cwd={} status={:?}",
                                candidate.terminal_id,
                                candidate.pane_id,
                                candidate.workspace_id,
                                candidate.tab_id,
                                candidate.cwd.unwrap_or_else(|| "unknown".into()),
                                candidate.agent_status,
                            ))
                            .collect::<Vec<_>>()
                            .join("; ")
                    ),
                }
            }
        }
    }

    pub(super) fn agent_rename_error_body(
        &self,
        err: AgentRenameError,
    ) -> crate::api::schema::ErrorBody {
        match err {
            AgentRenameError::Target(err) => self.agent_target_error_body(err),
            AgentRenameError::InvalidName => crate::api::schema::ErrorBody {
                code: "invalid_agent_name".into(),
                message: INVALID_AGENT_NAME_MESSAGE.into(),
            },
            AgentRenameError::NotAgent => crate::api::schema::ErrorBody {
                code: "agent_not_found".into(),
                message: "agent target does not currently host an agent".into(),
            },
            AgentRenameError::PendingLaunch => crate::api::schema::ErrorBody {
                code: "agent_launch_pending".into(),
                message: "agent name cannot change while startup is pending".into(),
            },
            AgentRenameError::DuplicateName { name, candidates } => crate::api::schema::ErrorBody {
                code: "agent_name_taken".into(),
                message: format!(
                    "agent name {name} is already used; candidates: {}",
                    candidates
                        .into_iter()
                        .map(|candidate| format!(
                            "terminal_id={} pane_id={} workspace_id={} tab_id={} cwd={} status={:?}",
                            candidate.terminal_id,
                            candidate.pane_id,
                            candidate.workspace_id,
                            candidate.tab_id,
                            candidate.cwd.unwrap_or_else(|| "unknown".into()),
                            candidate.agent_status,
                        ))
                        .collect::<Vec<_>>()
                        .join("; ")
                ),
            },
        }
    }

    pub(super) fn agent_info(
        &self,
        ws_idx: usize,
        pane_id: crate::layout::PaneId,
    ) -> Option<crate::api::schema::AgentInfo> {
        self.agent_info_with_title_names(ws_idx, pane_id, &self.state.agent_title_names())
    }

    /// `agent_info` with the title fallback names worked out once for a list.
    fn agent_info_with_title_names(
        &self,
        ws_idx: usize,
        pane_id: crate::layout::PaneId,
        title_names: &std::collections::HashMap<crate::terminal::TerminalId, String>,
    ) -> Option<crate::api::schema::AgentInfo> {
        let ws = self.state.workspaces.get(ws_idx)?;
        let pane_state = ws.pane_state(pane_id)?;
        let terminal = self.state.terminals.get(&pane_state.attached_terminal_id)?;
        if !terminal.is_agent_terminal() {
            return None;
        }
        let pane = self.pane_metadata(ws_idx, pane_id)?;
        Some(crate::api::schema::AgentInfo {
            terminal_id: pane.terminal_id,
            name: terminal
                .agent_name
                .clone()
                .or_else(|| title_names.get(&terminal.id).cloned()),
            name_source: (terminal.agent_name.is_none() && title_names.contains_key(&terminal.id))
                .then_some(crate::api::schema::AgentNameSource::Title),
            agent: pane.agent,
            title: pane.title,
            terminal_title: pane.terminal_title,
            terminal_title_stripped: pane.terminal_title_stripped,
            display_agent: pane.display_agent,
            agent_status: pane.agent_status,
            blocked_reason: pane.blocked_reason,
            blocked_since: pane.blocked_since,
            input_box: None,
            screen_detection_skipped: terminal.full_lifecycle_hook_authority_active(),
            pinned: pane.pinned,
            state_labels: pane.state_labels,
            tokens: pane.tokens,
            agent_session: pane.agent_session,
            workspace_id: pane.workspace_id,
            tab_id: pane.tab_id,
            pane_id: pane.pane_id,
            focused: pane.focused,
            launch_pending: terminal.managed_agent_launch_pending(),
            interactive_ready: terminal.managed_agent_interactive_ready(),
            state_change_seq: terminal.last_agent_state_change_seq.unwrap_or(0),
            completion_seq: terminal.last_agent_completion_seq,
            cwd: pane.cwd,
            foreground_cwd: pane.foreground_cwd,
            revision: pane.revision,
        })
    }

    fn agent_name_conflicts(
        &self,
        name: &str,
        except_terminal_id: &str,
    ) -> Vec<crate::api::schema::AgentInfo> {
        self.collect_agent_infos()
            .into_iter()
            // Only explicit names conflict: taking a name another agent has
            // as its title fallback just drops that fallback.
            .filter(|agent| {
                agent.name.as_deref() == Some(name)
                    && agent.name_source.is_none()
                    && agent.terminal_id != except_terminal_id
            })
            .collect()
    }
}

/// The argv `agent start` runs: the agent's executable, the caller's
/// arguments, and for Codex the shared-daemon arguments when enabled.
fn agent_start_argv(
    kind: crate::detect::Agent,
    args: Vec<String>,
    codex_app_server: &crate::codex_app_server::CodexAppServer,
    pane_cwd: &std::path::Path,
) -> Vec<String> {
    let codex_args = if kind == crate::detect::Agent::Codex {
        codex_app_server.launch_args(pane_cwd, &args)
    } else {
        Vec::new()
    };
    let mut argv = vec![crate::detect::interactive_agent_executable(kind).to_string()];
    argv.extend(args);
    argv.extend(codex_args);
    argv
}

impl App {
    /// Where a naming job reports the Codex thread it resolved for `pane_id`.
    fn codex_thread_reply(
        &self,
        pane_id: crate::layout::PaneId,
    ) -> crate::codex_app_server::ThreadReply {
        crate::codex_app_server::ThreadReply {
            events: self.event_tx.clone(),
            pane_id,
        }
    }
}

/// What naming needs to know about the Codex process running in a pane.
struct PaneCodexProcess {
    /// The thread it reopened with `resume <id>`.
    resumed_thread_id: Option<String>,
    /// When it started, in unix ms.
    started_at: Option<i64>,
}

/// The Codex process in the pane's foreground job, or one PTY down behind a
/// recognised wrapper.
fn pane_codex_process(runtime: &crate::terminal::TerminalRuntime) -> Option<PaneCodexProcess> {
    let job = crate::detect::foreground_job(runtime.child_pid()?)?;
    let codex = |job: &crate::platform::ForegroundJob| {
        crate::detect::agent_processes_in_job(job, crate::detect::Agent::Codex)
            .next()
            .cloned()
    };
    let process = codex(&job).or_else(|| {
        let (_, nested) = crate::detect::wrapped_shell_job(
            &job,
            crate::platform::nested_foreground_job_with_owner,
        )?;
        codex(&nested)
    })?;
    Some(PaneCodexProcess {
        resumed_thread_id: process
            .argv
            .as_deref()
            .and_then(crate::codex_app_server::resumed_thread_id),
        started_at: crate::platform::process_started_at_ms(process.pid),
    })
}

fn available_shell_name(runtime: &crate::terminal::TerminalRuntime) -> Option<String> {
    #[cfg(test)]
    if runtime.child_pid().is_none() {
        return Some("sh".into());
    }
    crate::platform::available_pane_shell(
        runtime.child_pid()?,
        crate::detect::is_nested_pty_wrapper,
    )
}

pub(super) fn runtime_hosts_agent(
    runtime: &crate::terminal::TerminalRuntime,
    expected: crate::detect::Agent,
) -> bool {
    #[cfg(test)]
    if runtime.child_pid().is_none() {
        return true;
    }
    live_runtime_agent(runtime) == Some(expected)
}

fn live_runtime_agent(runtime: &crate::terminal::TerminalRuntime) -> Option<crate::detect::Agent> {
    let job = crate::detect::foreground_job(runtime.child_pid()?)?;
    crate::detect::identify_agent_in_job(&job)
        .map(|(agent, _)| agent)
        .or_else(|| {
            job.processes
                .iter()
                .find_map(|process| crate::platform::process_agent_hint(process.pid))
        })
        .or_else(|| {
            // A PTY wrapper such as atuin's hides the agent one PTY down.
            crate::detect::nested_agent_job(&job, crate::platform::nested_foreground_job_with_owner)
                .map(|(_, agent, _)| agent)
        })
}

pub(super) enum AgentStartError {
    InvalidName,
    UnsupportedKind(String),
    InvalidArgument,
    InvalidTimeout,
    TargetNotFound(String),
    TargetBusy(String),
    TargetUnavailable(String),
    InputFailed(String),
    DuplicateName {
        name: String,
        candidates: Vec<crate::api::schema::AgentInfo>,
    },
}

pub(super) enum AgentRenameError {
    Target(TerminalTargetError),
    InvalidName,
    NotAgent,
    PendingLaunch,
    DuplicateName {
        name: String,
        candidates: Vec<crate::api::schema::AgentInfo>,
    },
}

#[cfg(test)]
mod tests {
    use super::valid_agent_name;

    #[test]
    fn agent_names_use_a_small_cli_safe_grammar() {
        for name in ["a", "reviewer-one", "reviewer_2", &"a".repeat(32)] {
            assert!(valid_agent_name(name), "expected {name:?} to be valid");
        }
        for name in [
            "",
            " reviewer",
            "reviewer ",
            "reviewer one",
            "Reviewer",
            "1reviewer",
            "reviewer.one",
            &"a".repeat(33),
        ] {
            assert!(!valid_agent_name(name), "expected {name:?} to be invalid");
        }
    }
}

#[cfg(test)]
mod codex_app_server_tests {
    use super::agent_start_argv;
    use crate::codex_app_server::CodexAppServer;
    use crate::detect::Agent;
    use std::path::Path;

    fn codex(app_server: bool) -> CodexAppServer {
        CodexAppServer::from_config(&crate::config::CodexAgentConfig {
            app_server,
            app_server_socket: "/run/codex.sock".into(),
            name_threads: false,
        })
    }

    #[test]
    fn codex_start_is_unchanged_when_the_switch_is_off() {
        assert_eq!(
            agent_start_argv(
                Agent::Codex,
                vec!["--yolo".into()],
                &codex(false),
                Path::new("/repo")
            ),
            ["codex", "--yolo"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn codex_start_goes_to_the_daemon_with_the_pane_cwd() {
        assert_eq!(
            agent_start_argv(
                Agent::Codex,
                vec!["--yolo".into()],
                &codex(true),
                Path::new("/repo")
            ),
            [
                "codex",
                "--yolo",
                "--remote",
                "unix:///run/codex.sock",
                "-C",
                "/repo"
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn other_agents_never_get_codex_arguments() {
        assert_eq!(
            agent_start_argv(Agent::Claude, Vec::new(), &codex(true), Path::new("/repo")),
            ["claude"]
        );
    }
}
