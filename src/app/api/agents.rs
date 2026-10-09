use std::time::Duration;

use bytes::Bytes;

use crate::api::schema::{
    AgentMessageBusy, AgentMessageMode, AgentMessageNote, AgentMessageParams, AgentPromptParams,
    AgentRenameParams, AgentSendKeysParams, AgentStartParams, AgentStatus, AgentTarget,
    PaneReadResult, ResponseResult,
};
use crate::app::App;

use super::responses::{encode_error, encode_error_body, encode_success};

const AGENT_PROMPT_SUBMIT_DELAY: Duration = Duration::from_millis(300);

/// `agent.message` writes typed text in pieces of at most this many bytes, with
/// a short gap between them: a TUI that classifies input by the size of a
/// burst (Claude Code collapses ~1000 bytes into `[Pasted text]`) then sees
/// typing, not a paste. The same sizes as `herdr pane send-text --chunk 300`.
const AGENT_SEND_PIECE_BYTES: usize = 300;
const AGENT_SEND_PIECE_GAP: Duration = Duration::from_millis(20);
/// One line; a longer message belongs in a file the agent can read.
const AGENT_SEND_MAX_BYTES: usize = 4000;
const AGENT_NOTE_TIMEOUT: Duration = Duration::from_secs(10);
const AGENT_NOTE_SENDER: &str = "herdr-palette";

// Codex's Windows input reader does not surface bracketed paste. It detects the prompt as a
// "paste burst" and, while that burst is buffered, rewrites a following Enter into a newline
// instead of submitting. The burst only flushes after an idle timeout, so any size-based delay is
// a timing guess that fails when ConPTY delivery lags it. Codex flushes a buffered burst
// synchronously when it receives a non-character key, so appending one after the paste gives the
// submission a deterministic paste boundary regardless of prompt size or delivery speed.
#[cfg(windows)]
fn append_codex_paste_boundary(runtime: &crate::terminal::TerminalRuntime, text: &mut Vec<u8>) {
    let keys = match crate::app::api_helpers::encode_api_keys(runtime, &["right".to_string()]) {
        Ok(keys) => keys,
        Err(key) => {
            tracing::warn!(key = %key, "failed to encode Codex paste boundary key");
            return;
        }
    };
    if let Some(key) = keys.into_iter().find(|bytes| !bytes.is_empty()) {
        text.extend_from_slice(&key);
    }
}

impl App {
    pub(super) fn handle_agent_list(
        &mut self,
        id: String,
        params: crate::api::schema::AgentListParams,
    ) -> String {
        // The input box reads each Claude pane's screen: only when asked.
        let agents = self.collect_agent_infos();
        encode_success(
            id,
            ResponseResult::AgentList {
                agents: if params.input_box {
                    agents
                        .into_iter()
                        .map(|agent| self.with_input_box(agent))
                        .collect()
                } else {
                    agents
                },
            },
        )
    }

    pub(super) fn handle_agent_get(&mut self, id: String, target: AgentTarget) -> String {
        self.reconcile_managed_agent_target(&target.target);
        let agent = match self.agent_info_for_target(&target.target) {
            Ok(agent) => self.with_input_box(agent),
            Err(err) => return encode_error_body(id, self.agent_target_error_body(err)),
        };

        encode_success(id, ResponseResult::AgentInfo { agent })
    }

    /// The agent with its input box draft (fork issue 146), read from the
    /// bottom of the pane's buffer. Only a Claude Code agent has one, and only
    /// these two calls ask: detection, rendering and the other agent answers
    /// never do.
    fn with_input_box(
        &self,
        mut agent: crate::api::schema::AgentInfo,
    ) -> crate::api::schema::AgentInfo {
        if agent.agent.as_deref() != Some("claude") {
            return agent;
        }
        agent.input_box = self
            .parse_pane_id(&agent.pane_id)
            .and_then(|(ws_idx, pane_id)| self.lookup_runtime(ws_idx, pane_id))
            .and_then(|(runtime, _)| {
                crate::detect::manifest::claude_input_box_text(&runtime.detection_ansi())
            })
            .map(|text| crate::api::schema::AgentInputBox { text });
        agent
    }

    pub(super) fn handle_agent_focus(&mut self, id: String, target: AgentTarget) -> String {
        let agent = match self.focus_agent_target(&target.target) {
            Ok(agent) => agent,
            Err(err) => return encode_error_body(id, self.agent_target_error_body(err)),
        };

        encode_success(id, ResponseResult::AgentInfo { agent })
    }

    pub(super) fn handle_agent_pin(
        &mut self,
        id: String,
        target: AgentTarget,
        pin: bool,
    ) -> String {
        let agent = match self.pin_agent_target(&target.target, pin) {
            Ok(agent) => agent,
            Err(err) => return encode_error_body(id, self.agent_target_error_body(err)),
        };

        encode_success(id, ResponseResult::AgentInfo { agent })
    }

    pub(super) fn handle_agent_rename(&mut self, id: String, params: AgentRenameParams) -> String {
        let agent = match self.rename_agent_target(&params.target, params.name) {
            Ok(agent) => agent,
            Err(err) => return encode_error_body(id, self.agent_rename_error_body(err)),
        };

        encode_success(id, ResponseResult::AgentInfo { agent })
    }

    pub(super) fn handle_agent_start(&mut self, id: String, params: AgentStartParams) -> String {
        let (agent, argv) = match self.start_agent(params) {
            Ok(started) => started,
            Err(err) => return encode_error_body(id, self.agent_start_error_body(err)),
        };

        encode_success(id, ResponseResult::AgentStarted { agent, argv })
    }

    pub(crate) fn handle_deferred_agent_api_request(
        &mut self,
        request: crate::api::schema::Request,
        respond_to: std::sync::mpsc::Sender<String>,
    ) -> bool {
        let params = match request.method {
            crate::api::schema::Method::AgentPrompt(params) => params,
            crate::api::schema::Method::AgentMessage(params) => {
                self.handle_deferred_agent_send(request.id, params, respond_to);
                return true;
            }
            _ => return false,
        };
        let validated = match self.validate_agent_prompt(request.id, &params) {
            Ok(validated) => validated,
            Err(response) => {
                let _ = respond_to.send(response);
                return true;
            }
        };
        // Start the completion waiter before submitting, so a refused thread
        // fails the request while nothing has been sent yet.
        let (handoff_tx, handoff_rx) = std::sync::mpsc::channel::<(
            String,
            crate::api::schema::AgentInfo,
            std::sync::mpsc::Receiver<std::io::Result<()>>,
        )>();
        let waiter_respond_to = respond_to.clone();
        let spawned = crate::thread_spawn::spawn_named("herdr-agent-prompt", move || {
            let Ok((id, agent, completion)) = handoff_rx.recv() else {
                return;
            };
            let response = match completion.recv() {
                Ok(Ok(())) => encode_success(id, ResponseResult::AgentPrompted { agent }),
                Ok(Err(err)) if err.kind() == std::io::ErrorKind::TimedOut => {
                    encode_error(id, "timeout", err.to_string())
                }
                Ok(Err(err)) => encode_error(id, "agent_prompt_failed", err.to_string()),
                Err(_) => encode_error(id, "agent_prompt_failed", "pty actor closed"),
            };
            let _ = waiter_respond_to.send(response);
        });
        if let Err(err) = spawned {
            tracing::warn!(err = %err, "failed to spawn agent prompt thread");
            let _ = respond_to.send(encode_error(
                validated.id,
                "agent_prompt_failed",
                format!("could not start prompt completion waiter: {err}"),
            ));
            return true;
        }
        match self.submit_agent_prompt(validated, &params) {
            Ok(submission) => {
                let _ = handoff_tx.send(submission);
            }
            Err(response) => {
                let _ = respond_to.send(response);
            }
        }
        true
    }

    /// `agent.message` (fork issue 182): type one line into an agent as the user
    /// would, or hand it to the note command. Answers through `respond_to`
    /// once the text is out, so the app thread never waits on the Enter delay
    /// or on the note command.
    fn handle_deferred_agent_send(
        &mut self,
        id: String,
        params: AgentMessageParams,
        respond_to: std::sync::mpsc::Sender<String>,
    ) {
        let respond = |response: String| {
            let _ = respond_to.send(response);
        };
        let resolved = match self.resolve_agent_target(&params.target) {
            Ok(resolved) => resolved,
            Err(err) => return respond(encode_error_body(id, self.agent_target_error_body(err))),
        };
        let Some(agent) = self.agent_info(resolved.ws_idx, resolved.pane_id) else {
            return respond(agent_not_found(id, &params.target));
        };
        if params.answer.is_none() {
            if params.text.is_empty() {
                return respond(encode_error(
                    id,
                    "empty_agent_send",
                    "agent send must not be empty",
                ));
            }
            if params.text.len() > AGENT_SEND_MAX_BYTES {
                return respond(encode_error(
                    id,
                    "agent_send_too_long",
                    format!("agent send is limited to {AGENT_SEND_MAX_BYTES} bytes"),
                ));
            }
            if params.text.chars().any(char::is_control) {
                return respond(encode_error(
                    id,
                    "multi_line_send",
                    "agent send is one line: remove line breaks and control characters",
                ));
            }
        }
        let blocked = agent.agent_status == AgentStatus::Blocked;
        if blocked && params.answer.is_none() {
            return respond(encode_error(
                id,
                "agent_blocked",
                format!(
                    "agent {} is blocked on a prompt: answer it (answer) or open its pane",
                    params.target
                ),
            ));
        }
        if !blocked && params.answer.is_some() {
            return respond(encode_error(
                id,
                "agent_not_blocked",
                format!("agent {} is not blocked on a prompt", params.target),
            ));
        }
        if params.mode == AgentMessageMode::Note {
            return self.send_agent_note(id, agent, params, respond_to);
        }

        let Some(terminal_id) = self
            .state
            .workspaces
            .get(resolved.ws_idx)
            .and_then(|workspace| workspace.terminal_id(resolved.pane_id))
            .cloned()
        else {
            return respond(agent_not_found(id, &params.target));
        };
        let Some(terminal) = self.state.terminals.get(&terminal_id) else {
            return respond(agent_not_found(id, &params.target));
        };
        let Some(expected_agent) = terminal.effective_known_agent() else {
            return respond(agent_not_ready(id, &params.target));
        };
        if terminal.managed_agent_launch_pending() {
            return respond(agent_not_ready(id, &params.target));
        }
        let Some(runtime) = self.lookup_runtime_sender(resolved.ws_idx, resolved.pane_id) else {
            return respond(agent_not_found(id, &params.target));
        };
        if !super::super::agents::runtime_hosts_agent(runtime, expected_agent) {
            return respond(agent_not_ready(id, &params.target));
        }
        let sent = |pieces, enter_sent, interrupted| ResponseResult::AgentMessaged {
            agent: agent.clone(),
            delivery: AgentMessageMode::Typed,
            pieces,
            enter_sent,
            interrupted,
            note: None,
        };
        let key_bytes = |key: &str| -> Result<Vec<u8>, String> {
            super::super::api_helpers::encode_api_keys(runtime, &[key.to_owned()])
                .map(|encoded| encoded.into_iter().flatten().collect())
        };

        if let Some(answer) = &params.answer {
            let bytes = match key_bytes(answer) {
                Ok(bytes) => bytes,
                Err(key) => {
                    return respond(encode_error(
                        id,
                        "invalid_key",
                        format!("unsupported key {key}"),
                    ))
                }
            };
            if let Err(err) = runtime.try_send_bytes(Bytes::from(bytes)) {
                return respond(encode_error(id, "agent_send_failed", err.to_string()));
            }
            return respond(encode_success(id, sent(0, false, false)));
        }

        let mut interrupted = false;
        if params.busy == AgentMessageBusy::Interrupt && agent.agent_status == AgentStatus::Working
        {
            let bytes = match key_bytes("esc") {
                Ok(bytes) => bytes,
                Err(key) => {
                    return respond(encode_error(
                        id,
                        "invalid_key",
                        format!("unsupported key {key}"),
                    ))
                }
            };
            if let Err(err) = runtime.try_send_bytes(Bytes::from(bytes)) {
                return respond(encode_error(id, "agent_send_failed", err.to_string()));
            }
            interrupted = true;
            std::thread::sleep(AGENT_SEND_PIECE_GAP);
        }

        // Raw writes, not a bracketed paste: the point is that the text
        // arrives as typed. Every piece but the last goes out now; the last
        // goes with Enter, which the pty actor delays after it.
        let pieces =
            super::super::api_helpers::split_utf8_chunks(&params.text, AGENT_SEND_PIECE_BYTES);
        let count = pieces.len();
        let Some((last, head)) = pieces.split_last() else {
            return respond(encode_error(
                id,
                "empty_agent_send",
                "agent send must not be empty",
            ));
        };
        for piece in head {
            if let Err(err) = runtime.try_send_bytes(Bytes::copy_from_slice(piece.as_bytes())) {
                return respond(encode_error(id, "agent_send_failed", err.to_string()));
            }
            std::thread::sleep(AGENT_SEND_PIECE_GAP);
        }
        let (_, enter) = super::super::api_helpers::encode_api_submission_parts(runtime, "");
        let completion = match runtime.queue_user_input_submission(
            Bytes::copy_from_slice(last.as_bytes()),
            Bytes::from(enter),
            AGENT_PROMPT_SUBMIT_DELAY,
            None,
        ) {
            Ok(completion) => completion,
            Err(err) => return respond(encode_error(id, "agent_send_failed", err.to_string())),
        };
        let result = sent(count, true, interrupted);
        let waiter_id = id.clone();
        let waiter_respond_to = respond_to.clone();
        let spawned = crate::thread_spawn::spawn_named("herdr-agent-send", move || {
            let response = match completion.recv() {
                Ok(Ok(())) => encode_success(waiter_id, result),
                Ok(Err(err)) => encode_error(waiter_id, "agent_send_failed", err.to_string()),
                Err(_) => encode_error(waiter_id, "agent_send_failed", "pty actor closed"),
            };
            let _ = waiter_respond_to.send(response);
        });
        if let Err(err) = spawned {
            tracing::warn!(err = %err, "failed to spawn agent send thread");
            respond(encode_error(
                id,
                "agent_send_failed",
                format!("could not start send completion waiter: {err}"),
            ));
        }
    }

    /// A note goes to the agent through the configured note command (by
    /// default `agent-bell`), named by the agent's herdr name.
    fn send_agent_note(
        &self,
        id: String,
        agent: crate::api::schema::AgentInfo,
        params: AgentMessageParams,
        respond_to: std::sync::mpsc::Sender<String>,
    ) {
        let Some(name) = agent.name.clone() else {
            let _ = respond_to.send(encode_error(
                id,
                "agent_unnamed",
                "a note is addressed by name: rename the agent first",
            ));
            return;
        };
        let interrupt = params.busy == AgentMessageBusy::Interrupt;
        let text = params.text;
        let thread_id = id.clone();
        let thread_respond_to = respond_to.clone();
        let spawned = crate::thread_spawn::spawn_named("herdr-agent-note", move || {
            let command = crate::config::load_live_config()
                .map(|loaded| loaded.config.palette.send.note_command)
                .unwrap_or_else(|_| "agent-bell".to_owned());
            let response = match run_note_command(&command, &name, &text, interrupt) {
                Ok(note) => encode_success(
                    thread_id,
                    ResponseResult::AgentMessaged {
                        agent,
                        delivery: AgentMessageMode::Note,
                        pieces: 0,
                        enter_sent: false,
                        interrupted: interrupt,
                        note: Some(note),
                    },
                ),
                Err((code, message)) => encode_error(thread_id, code, message),
            };
            let _ = thread_respond_to.send(response);
        });
        if let Err(err) = spawned {
            tracing::warn!(err = %err, "failed to spawn agent note thread");
            let _ = respond_to.send(encode_error(
                id,
                "agent_send_failed",
                format!("could not start the note command: {err}"),
            ));
        }
    }

    /// Runs every check that can reject a prompt without touching the pane.
    fn validate_agent_prompt(
        &self,
        id: String,
        params: &AgentPromptParams,
    ) -> Result<ValidatedAgentPrompt, String> {
        if params.text.is_empty() {
            return Err(encode_error(
                id,
                "empty_agent_prompt",
                "agent prompt must not be empty",
            ));
        }
        let resolved = match self.resolve_agent_target(&params.target) {
            Ok(resolved) => resolved,
            Err(err) => return Err(encode_error_body(id, self.agent_target_error_body(err))),
        };
        let Some(terminal_id) = self
            .state
            .workspaces
            .get(resolved.ws_idx)
            .and_then(|workspace| workspace.terminal_id(resolved.pane_id))
            .cloned()
        else {
            return Err(agent_not_found(id, &params.target));
        };
        let Some(terminal) = self.state.terminals.get(&terminal_id) else {
            return Err(agent_not_found(id, &params.target));
        };
        if terminal.state == crate::detect::AgentState::Blocked {
            return Err(encode_error(
                id,
                "agent_blocked",
                format!(
                    "agent {} is blocked and requires interactive input",
                    params.target
                ),
            ));
        }
        let Some(expected_agent) = terminal.effective_known_agent() else {
            return Err(agent_not_ready(id, &params.target));
        };
        if terminal.managed_agent_launch_pending() {
            return Err(agent_not_ready(id, &params.target));
        }
        let Some(runtime) = self.lookup_runtime_sender(resolved.ws_idx, resolved.pane_id) else {
            return Err(agent_not_found(id, &params.target));
        };
        if !super::super::agents::runtime_hosts_agent(runtime, expected_agent) {
            return Err(encode_error(
                id,
                "agent_not_ready",
                format!(
                    "agent {} is no longer the pane foreground process",
                    params.target
                ),
            ));
        }
        let Some(agent) = self.agent_info(resolved.ws_idx, resolved.pane_id) else {
            return Err(agent_not_found(id, &params.target));
        };
        Ok(ValidatedAgentPrompt {
            id,
            ws_idx: resolved.ws_idx,
            pane_id: resolved.pane_id,
            expected_agent,
            agent,
        })
    }

    fn submit_agent_prompt(
        &mut self,
        validated: ValidatedAgentPrompt,
        params: &AgentPromptParams,
    ) -> Result<
        (
            String,
            crate::api::schema::AgentInfo,
            std::sync::mpsc::Receiver<std::io::Result<()>>,
        ),
        String,
    > {
        let ValidatedAgentPrompt {
            id,
            ws_idx,
            pane_id,
            expected_agent,
            agent,
        } = validated;
        let Some(runtime) = self.lookup_runtime_sender(ws_idx, pane_id) else {
            return Err(agent_not_found(id, &params.target));
        };
        #[cfg(windows)]
        let submit_deadline = params
            .wait
            .as_ref()
            .and_then(|wait| wait.submission_deadline);
        #[cfg(not(windows))]
        let submit_deadline = None;
        if expected_agent == crate::detect::Agent::GithubCopilot {
            // Copilot ignores synthetic Enter after focus loss until it receives focus gained.
            let focus = match crate::ghostty::encode_focus(crate::ghostty::FocusEvent::Gained) {
                Ok(focus) => focus,
                Err(err) => {
                    return Err(encode_error(id, "agent_prompt_failed", err.to_string()));
                }
            };
            if let Err(err) = runtime.try_send_bytes(Bytes::from(focus)) {
                return Err(encode_error(id, "agent_prompt_failed", err.to_string()));
            }
        }
        let (text, enter) =
            crate::app::api_helpers::encode_api_submission_parts(runtime, &params.text);
        #[cfg(windows)]
        let text = if expected_agent == crate::detect::Agent::Codex {
            let mut text = text;
            append_codex_paste_boundary(runtime, &mut text);
            text
        } else {
            text
        };
        let completion = runtime
            .queue_user_input_submission(
                Bytes::from(text),
                Bytes::from(enter),
                AGENT_PROMPT_SUBMIT_DELAY,
                submit_deadline,
            )
            .map_err(|err| encode_error(id.clone(), "agent_prompt_failed", err.to_string()))?;
        Ok((id, agent, completion))
    }

    pub(super) fn handle_agent_read(
        &mut self,
        id: String,
        params: crate::api::schema::AgentReadParams,
    ) -> String {
        let resolved = match self.resolve_agent_target(&params.target) {
            Ok(resolved) => resolved,
            Err(err) => return encode_error_body(id, self.agent_target_error_body(err)),
        };
        let Some((pane, workspace_id)) = self.lookup_runtime(resolved.ws_idx, resolved.pane_id)
        else {
            return agent_not_found(id, &params.target);
        };
        let snapshot = crate::app::api_helpers::read_terminal_snapshot(
            pane,
            params.source,
            params.format,
            params.lines,
            params.strip_dim,
        );

        encode_success(
            id,
            ResponseResult::PaneRead {
                read: PaneReadResult {
                    pane_id: self
                        .public_pane_id(resolved.ws_idx, resolved.pane_id)
                        .unwrap_or_else(|| params.target.clone()),
                    workspace_id,
                    tab_id: self
                        .public_tab_id(resolved.ws_idx, resolved.tab_idx)
                        .unwrap(),
                    source: params.source,
                    format: params.format,
                    text: snapshot.text,
                    revision: 0,
                    truncated: snapshot.truncated,
                },
            },
        )
    }

    pub(super) fn handle_agent_explain(&mut self, id: String, target: AgentTarget) -> String {
        let resolved = match self.resolve_agent_target(&target.target) {
            Ok(resolved) => resolved,
            Err(err) => return encode_error_body(id, self.agent_target_error_body(err)),
        };
        let Some((pane, _workspace_id)) = self.lookup_runtime(resolved.ws_idx, resolved.pane_id)
        else {
            return agent_not_found(id, &target.target);
        };
        let Some(terminal_id) = self
            .state
            .workspaces
            .get(resolved.ws_idx)
            .and_then(|workspace| workspace.terminal_id(resolved.pane_id))
        else {
            return agent_not_found(id, &target.target);
        };
        let Some(terminal) = self.state.terminals.get(terminal_id) else {
            return agent_not_found(id, &target.target);
        };
        if terminal.full_lifecycle_hook_authority_active() {
            let explain = serde_json::json!({
                "agent": terminal.effective_agent_label().unwrap_or("unknown"),
                "state": crate::detect::manifest::agent_state_label(terminal.state),
                "manifest_source": null,
                "manifest_version": null,
                "cached_remote_version": null,
                "local_override_shadowing_remote": false,
                "remote_update_status": null,
                "remote_update_error": null,
                "matched_rule": null,
                "visible_idle": false,
                "visible_blocker": false,
                "visible_working": false,
                "blocked_reason": terminal.blocked_reason(),
                "screen_detection_skipped": true,
                "screen_detection_skip_reason": "full_lifecycle_hook_authority",
                "skip_state_update": false,
                "skipped_update_reason": null,
                "fallback_reason": null,
                "warning": null,
                "evaluated_rules": [],
            });
            return encode_success(id, ResponseResult::AgentExplain { explain });
        }
        let Some(agent) = terminal.effective_known_agent().or(terminal.detected_agent) else {
            return encode_error(
                id,
                "agent_explain_unavailable",
                format!(
                    "agent target {} does not have a detected agent label",
                    target.target
                ),
            );
        };

        let screen = pane.detection_text();
        let osc_title = pane.agent_osc_title();
        let osc_progress = pane.agent_osc_progress();
        let explain = crate::detect::manifest::explain_with_input(
            agent,
            crate::detect::manifest::DetectionInput {
                screen: &screen,
                osc_title: &osc_title,
                osc_progress: &osc_progress,
            },
        );
        let value = crate::detect::manifest::explain_to_json_value(&explain);

        encode_success(id, ResponseResult::AgentExplain { explain: value })
    }

    pub(super) fn handle_agent_send_keys(
        &mut self,
        id: String,
        params: AgentSendKeysParams,
    ) -> String {
        let resolved = match self.resolve_agent_target(&params.target) {
            Ok(resolved) => resolved,
            Err(err) => return encode_error_body(id, self.agent_target_error_body(err)),
        };
        let Some(terminal_id) = self
            .state
            .workspaces
            .get(resolved.ws_idx)
            .and_then(|workspace| workspace.terminal_id(resolved.pane_id))
        else {
            return agent_not_found(id, &params.target);
        };
        let Some(expected_agent) = self
            .state
            .terminals
            .get(terminal_id)
            .and_then(|terminal| terminal.effective_known_agent())
        else {
            return agent_not_ready(id, &params.target);
        };
        let Some(runtime) = self.lookup_runtime_sender(resolved.ws_idx, resolved.pane_id) else {
            return agent_not_found(id, &params.target);
        };
        if !super::super::agents::runtime_hosts_agent(runtime, expected_agent) {
            return agent_not_ready(id, &params.target);
        }
        let encoded = match super::super::api_helpers::encode_api_keys(runtime, &params.keys) {
            Ok(encoded) => encoded,
            Err(key) => {
                return encode_error(id, "invalid_key", format!("unsupported key {key}"));
            }
        };
        let bytes: Vec<u8> = encoded.into_iter().flatten().collect();
        if let Err(err) = runtime.try_send_bytes(Bytes::from(bytes)) {
            return encode_error(id, "agent_send_keys_failed", err.to_string());
        }

        encode_success(id, ResponseResult::Ok {})
    }
}

struct ValidatedAgentPrompt {
    id: String,
    ws_idx: usize,
    pane_id: crate::layout::PaneId,
    expected_agent: crate::detect::Agent,
    agent: crate::api::schema::AgentInfo,
}

/// Run `<command> send --to NAME --from herdr-palette [--interrupt] TEXT` and
/// read its answer. The command is split on whitespace (a program and its
/// leading arguments); its words become the note's `detail`, and `held` or
/// `injected` in them its `state`, else `queued`.
pub(crate) fn run_note_command(
    command: &str,
    name: &str,
    text: &str,
    interrupt: bool,
) -> Result<AgentMessageNote, (&'static str, String)> {
    use std::process::{Command, Stdio};

    let mut words = command.split_whitespace();
    let Some(program) = words.next() else {
        return Err((
            "note_command_missing",
            "palette.send.note_command is empty".to_owned(),
        ));
    };
    let mut note = Command::new(program);
    note.args(words)
        .args(["send", "--to", name, "--from", AGENT_NOTE_SENDER]);
    if interrupt {
        note.arg("--interrupt");
    }
    note.arg(text)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = note.spawn().map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            (
                "note_command_missing",
                format!("{program} not found: set palette.send.note_command"),
            )
        } else {
            ("note_failed", err.to_string())
        }
    })?;
    let deadline = std::time::Instant::now() + AGENT_NOTE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err((
                    "note_timeout",
                    format!("{program} did not answer in {AGENT_NOTE_TIMEOUT:?}"),
                ));
            }
            Err(err) => return Err(("note_failed", err.to_string())),
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|err| ("note_failed", err.to_string()))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if !output.status.success() {
        let words = if stderr.is_empty() { stdout } else { stderr };
        return Err((
            "note_failed",
            format!("{program} exited with {}: {words}", output.status),
        ));
    }
    let lower = stdout.to_lowercase();
    let state = if lower.contains("held") {
        "held"
    } else if lower.contains("injected") {
        "injected"
    } else {
        "queued"
    };
    Ok(AgentMessageNote {
        state: state.to_owned(),
        detail: stdout.chars().take(400).collect(),
    })
}

fn agent_not_ready(id: String, target: &str) -> String {
    encode_error(
        id,
        "agent_not_ready",
        format!("agent {target} is not an active named agent"),
    )
}

fn agent_not_found(id: String, target: &str) -> String {
    encode_error(
        id,
        "agent_not_found",
        format!("agent target {target} not found"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        api::schema::{AgentStatus, SuccessResponse},
        app::Mode,
        config::Config,
        detect::{Agent, AgentState},
        workspace::Workspace,
    };

    fn app_with_agent() -> App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.workspaces = vec![Workspace::test_new("agent")];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        app
    }

    fn start_deferred_agent_prompt(
        app: &mut App,
        id: &str,
        params: AgentPromptParams,
    ) -> std::sync::mpsc::Receiver<String> {
        let (respond_to, response_rx) = std::sync::mpsc::channel();
        assert!(app.handle_deferred_agent_api_request(
            crate::api::schema::Request {
                id: id.into(),
                method: crate::api::schema::Method::AgentPrompt(params),
            },
            respond_to,
        ));
        response_rx
    }

    fn run_deferred_agent_prompt(app: &mut App, id: &str, params: AgentPromptParams) -> String {
        start_deferred_agent_prompt(app, id, params)
            .recv_timeout(Duration::from_secs(1))
            .expect("agent prompt responds after submission")
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_codex_prompt_flushes_paste_burst_before_enter() {
        let mut app = app_with_agent();
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.state.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let terminal = app.state.terminals.get_mut(&terminal_id).unwrap();
        terminal.set_agent_name("reviewer".into());
        terminal.set_detected_state(Some(Agent::Codex), AgentState::Idle);
        let (runtime, mut rx) = crate::terminal::TerminalRuntime::test_with_channel(80, 24);
        app.state.insert_test_runtime(pane_id, runtime);

        let response = run_deferred_agent_prompt(
            &mut app,
            "req",
            AgentPromptParams {
                target: "reviewer".into(),
                text: "A != B".into(),
                wait: None,
            },
        );
        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        assert!(matches!(
            success.result,
            ResponseResult::AgentPrompted { .. }
        ));
        // The non-character key must precede Enter so Codex commits the paste burst first.
        assert_eq!(rx.try_recv().unwrap(), Bytes::from_static(b"A != B\x1b[C"));
        assert_eq!(rx.try_recv().unwrap(), Bytes::from_static(b"\r"));
    }

    #[tokio::test]
    async fn a_false_process_exit_makes_a_named_live_agent_unreachable_by_name() {
        // Reproduces the registration loss reported on #3225 by rszrszrsz:
        // a live agent pane with an assigned name stops resolving by that name
        // while its process keeps running, and renaming is the only recovery.
        let mut app = app_with_agent();
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.state.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let observed_at = std::time::Instant::now();
        let terminal = app.state.terminals.get_mut(&terminal_id).unwrap();
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Working);
        terminal.set_agent_name("reviewer".into());

        let found = app.handle_agent_get(
            "req:before".into(),
            AgentTarget {
                target: "reviewer".into(),
            },
        );
        assert!(
            serde_json::from_str::<SuccessResponse>(&found).is_ok(),
            "the assigned name must resolve while the agent is running: {found}"
        );

        // One process-exit observation, then the same agent is observed alive
        // again on the next probe - the process never actually went away.
        app.handle_internal_event(crate::events::AppEvent::StateChanged {
            pane_id,
            agent: Some(Agent::Pi),
            state: AgentState::Idle,
            visible_blocker: false,
            visible_working: false,
            background_work: false,
            blocked_reason: None,
            process_exited: true,
            observed_at,
        });
        app.handle_internal_event(crate::events::AppEvent::AgentProcessDetected {
            pane_id,
            agent: Agent::Pi,
            observed_at: observed_at + std::time::Duration::from_secs(1),
            replaced_process: false,
        });

        let terminal = &app.state.terminals[&terminal_id];
        assert_eq!(
            terminal.detected_agent,
            Some(Agent::Pi),
            "the agent process is still there"
        );

        let after = app.handle_agent_get(
            "req:after".into(),
            AgentTarget {
                target: "reviewer".into(),
            },
        );
        assert!(
            serde_json::from_str::<SuccessResponse>(&after).is_ok(),
            "a live agent must stay reachable by its assigned name: {after}"
        );
    }

    #[tokio::test]
    async fn agent_prompt_sends_text_then_delays_enter() {
        let mut app = app_with_agent();
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.state.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let terminal = app.state.terminals.get_mut(&terminal_id).unwrap();
        terminal.set_agent_name("reviewer".into());
        terminal.set_detected_state(Some(Agent::OpenCode), AgentState::Working);
        let (runtime, mut rx) =
            crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
                80, 24, 0, b"", 2,
            );
        runtime.test_process_pty_bytes(b"\x1b[?2004h");
        app.state.insert_test_runtime(pane_id, runtime);

        let public_pane_id = app.public_pane_id(0, pane_id).unwrap();
        let bracketed_started = std::time::Instant::now();
        let response_rx = start_deferred_agent_prompt(
            &mut app,
            "req",
            AgentPromptParams {
                target: public_pane_id,
                text: "A != B".into(),
                wait: None,
            },
        );
        assert!(response_rx.try_recv().is_err());
        let response = response_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("agent prompt responds after submission");
        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::AgentPrompted { agent, .. } = success.result else {
            panic!("expected prompted response");
        };
        assert_eq!(agent.name.as_deref(), Some("reviewer"));
        assert_eq!(
            rx.try_recv().unwrap(),
            Bytes::from_static(b"\x1b[200~A != B\x1b[201~")
        );
        assert_eq!(rx.try_recv().unwrap(), Bytes::from_static(b"\r"));
        assert!(bracketed_started.elapsed() >= AGENT_PROMPT_SUBMIT_DELAY);

        app.lookup_runtime_sender(0, pane_id)
            .unwrap()
            .test_process_pty_bytes(b"\x1b[?2004l");
        let raw_started = std::time::Instant::now();
        let raw = run_deferred_agent_prompt(
            &mut app,
            "req-raw",
            AgentPromptParams {
                target: "reviewer".into(),
                text: "A != B".into(),
                wait: None,
            },
        );
        let raw: SuccessResponse = serde_json::from_str(&raw).unwrap();
        assert!(matches!(raw.result, ResponseResult::AgentPrompted { .. }));
        assert_eq!(rx.try_recv().unwrap(), Bytes::from_static(b"A != B"));
        assert_eq!(rx.try_recv().unwrap(), Bytes::from_static(b"\r"));
        assert!(raw_started.elapsed() >= AGENT_PROMPT_SUBMIT_DELAY);

        let rejected = run_deferred_agent_prompt(
            &mut app,
            "req-label",
            AgentPromptParams {
                target: "opencode".into(),
                text: "wrong target".into(),
                wait: None,
            },
        );
        let error: crate::api::schema::ErrorResponse = serde_json::from_str(&rejected).unwrap();
        assert_eq!(error.error.code, "agent_not_found");
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn agent_prompt_waiter_spawn_failure_fails_before_submitting() {
        let mut app = app_with_agent();
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.state.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let terminal = app.state.terminals.get_mut(&terminal_id).unwrap();
        terminal.set_agent_name("reviewer".into());
        terminal.set_detected_state(Some(Agent::OpenCode), AgentState::Idle);
        let (runtime, mut rx) = crate::terminal::TerminalRuntime::test_with_channel(80, 24);
        app.state.insert_test_runtime(pane_id, runtime);

        crate::thread_spawn::test_hook::fail_next_spawns(1);
        let response = run_deferred_agent_prompt(
            &mut app,
            "req",
            AgentPromptParams {
                target: "reviewer".into(),
                text: "hello".into(),
                wait: None,
            },
        );

        let error: crate::api::schema::ErrorResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(error.error.code, "agent_prompt_failed");
        std::thread::sleep(AGENT_PROMPT_SUBMIT_DELAY * 2);
        assert!(rx.try_recv().is_err(), "a failed prompt must not be sent");
    }

    #[tokio::test]
    async fn agent_prompt_validation_errors_win_over_waiter_spawn_failure() {
        let mut app = app_with_agent();
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.state.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let terminal = app.state.terminals.get_mut(&terminal_id).unwrap();
        terminal.set_agent_name("reviewer".into());
        terminal.set_detected_state(Some(Agent::OpenCode), AgentState::Idle);
        let (runtime, mut rx) = crate::terminal::TerminalRuntime::test_with_channel(80, 24);
        app.state.insert_test_runtime(pane_id, runtime);

        for (target, text, code) in [
            ("reviewer", "", "empty_agent_prompt"),
            ("missing", "hello", "agent_not_found"),
        ] {
            crate::thread_spawn::test_hook::fail_next_spawns(1);
            let response = run_deferred_agent_prompt(
                &mut app,
                "req",
                AgentPromptParams {
                    target: target.into(),
                    text: text.into(),
                    wait: None,
                },
            );

            let error: crate::api::schema::ErrorResponse = serde_json::from_str(&response).unwrap();
            assert_eq!(error.error.code, code);
            assert!(
                crate::thread_spawn::spawn_named("probe", || {}).is_err(),
                "a rejected prompt must not start a waiter thread"
            );
        }
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn agent_prompt_rejects_blocked_agent_without_writing() {
        let mut app = app_with_agent();
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.state.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let terminal = app.state.terminals.get_mut(&terminal_id).unwrap();
        terminal.set_agent_name("reviewer".into());
        terminal.set_detected_state(Some(Agent::GithubCopilot), AgentState::Blocked);
        let (runtime, mut rx) = crate::terminal::TerminalRuntime::test_with_channel(80, 24);
        app.state.insert_test_runtime(pane_id, runtime);

        let response = run_deferred_agent_prompt(
            &mut app,
            "req",
            AgentPromptParams {
                target: "reviewer".into(),
                text: "unrelated prompt".into(),
                wait: None,
            },
        );

        let error: crate::api::schema::ErrorResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(error.error.code, "agent_blocked");
        assert!(
            tokio::time::timeout(
                AGENT_PROMPT_SUBMIT_DELAY + Duration::from_millis(100),
                rx.recv()
            )
            .await
            .is_err(),
            "blocked prompt wrote or scheduled terminal input"
        );
    }

    #[tokio::test]
    async fn agent_prompt_focuses_copilot_before_submitting() {
        let mut app = app_with_agent();
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.state.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let terminal = app.state.terminals.get_mut(&terminal_id).unwrap();
        terminal.set_agent_name("reviewer".into());
        terminal.set_detected_state(Some(Agent::GithubCopilot), AgentState::Idle);
        let (runtime, mut rx) =
            crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
                80, 24, 0, b"", 3,
            );
        runtime.test_process_pty_bytes(b"\x1b[?2004h");
        app.state.insert_test_runtime(pane_id, runtime);

        let response = run_deferred_agent_prompt(
            &mut app,
            "req",
            AgentPromptParams {
                target: "reviewer".into(),
                text: "A != B".into(),
                wait: None,
            },
        );
        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        assert!(matches!(
            success.result,
            ResponseResult::AgentPrompted { .. }
        ));
        assert_eq!(rx.try_recv().unwrap(), Bytes::from_static(b"\x1b[I"));
        assert_eq!(
            rx.try_recv().unwrap(),
            Bytes::from_static(b"\x1b[200~A != B\x1b[201~")
        );
        assert_eq!(rx.try_recv().unwrap(), Bytes::from_static(b"\r"));
    }

    #[tokio::test]
    async fn agent_send_keys_validates_every_key_before_writing() {
        let mut app = app_with_agent();
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.state.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let terminal = app.state.terminals.get_mut(&terminal_id).unwrap();
        terminal.set_agent_name("reviewer".into());
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        let (runtime, mut rx) = crate::terminal::TerminalRuntime::test_with_channel(80, 24);
        app.state.insert_test_runtime(pane_id, runtime);

        let rejected = app.handle_agent_send_keys(
            "req-invalid".into(),
            AgentSendKeysParams {
                target: "reviewer".into(),
                keys: vec!["enter".into(), "not-a-key".into()],
            },
        );
        let error: crate::api::schema::ErrorResponse = serde_json::from_str(&rejected).unwrap();
        assert_eq!(error.error.code, "invalid_key");
        assert!(rx.try_recv().is_err());

        let sent = app.handle_agent_send_keys(
            "req-valid".into(),
            AgentSendKeysParams {
                target: "reviewer".into(),
                keys: vec!["up".into(), "enter".into()],
            },
        );
        let success: SuccessResponse = serde_json::from_str(&sent).unwrap();
        assert!(matches!(success.result, ResponseResult::Ok {}));
        assert_eq!(rx.try_recv().unwrap(), Bytes::from_static(b"\x1b[A\r"));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn agent_prompt_rejects_managed_agent_while_startup_is_pending() {
        let mut app = app_with_agent();
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.state.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let terminal = app.state.terminals.get_mut(&terminal_id).unwrap();
        let now = std::time::Instant::now();
        terminal.begin_managed_agent(
            "reviewer".into(),
            Agent::OpenCode,
            now,
            std::time::Duration::from_secs(3),
            std::time::Duration::from_secs(10),
        );
        terminal.set_detected_state(Some(Agent::OpenCode), AgentState::Idle);
        let (runtime, mut rx) = crate::terminal::TerminalRuntime::test_with_channel(80, 24);
        app.state.insert_test_runtime(pane_id, runtime);

        let response = run_deferred_agent_prompt(
            &mut app,
            "req-pending",
            AgentPromptParams {
                target: "reviewer".into(),
                text: "A != B".into(),
                wait: None,
            },
        );
        let error: crate::api::schema::ErrorResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(error.error.code, "agent_not_ready");
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn agent_focus_marks_already_focused_done_agent_seen() {
        let mut app = app_with_agent();
        app.state.outer_terminal_focus = Some(false);

        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.state.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&terminal_id)
            .unwrap()
            .set_detected_state(Some(Agent::Pi), AgentState::Idle);
        app.state.workspaces[0].tabs[0]
            .panes
            .get_mut(&pane_id)
            .unwrap()
            .seen = false;
        app.state.workspaces[0].tabs[0].layout.focus_pane(pane_id);

        let response = app.handle_agent_focus(
            "req".into(),
            AgentTarget {
                target: app.public_pane_id(0, pane_id).unwrap(),
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::AgentInfo { agent } = success.result else {
            panic!("expected agent info response");
        };
        assert_eq!(agent.agent_status, AgentStatus::Idle);
    }

    // Fork issue 130: an unnamed agent answers to its name-like title.
    #[test]
    fn an_unnamed_agent_is_listed_and_targeted_by_its_title_name() {
        let mut app = app_with_agent();
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.state.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let terminal = app.state.terminals.get_mut(&terminal_id).unwrap();
        terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);
        terminal.set_terminal_title(Some("\u{2733} jev-astra".into()));

        let listed: SuccessResponse =
            serde_json::from_str(&app.handle_agent_list("req".into(), Default::default())).unwrap();
        let ResponseResult::AgentList { agents } = listed.result else {
            panic!("expected agent list");
        };
        assert_eq!(agents[0].name.as_deref(), Some("jev-astra"));
        assert_eq!(
            agents[0].name_source,
            Some(crate::api::schema::AgentNameSource::Title)
        );
        let json = serde_json::to_value(&agents[0]).unwrap();
        assert_eq!(json["name_source"], "title");

        let got: SuccessResponse = serde_json::from_str(&app.handle_agent_get(
            "req".into(),
            AgentTarget {
                target: "jev-astra".into(),
            },
        ))
        .unwrap();
        let ResponseResult::AgentInfo { agent } = got.result else {
            panic!("the title name resolves as a target");
        };
        assert_eq!(agent.terminal_id, terminal_id.to_string());

        // An explicit name takes over and carries no source marker.
        let renamed: SuccessResponse = serde_json::from_str(&app.handle_agent_rename(
            "req".into(),
            AgentRenameParams {
                target: "jev-astra".into(),
                name: Some("reviewer".into()),
            },
        ))
        .unwrap();
        let ResponseResult::AgentInfo { agent } = renamed.result else {
            panic!("expected agent info");
        };
        assert_eq!(agent.name.as_deref(), Some("reviewer"));
        assert_eq!(agent.name_source, None);
    }

    // Fork issue 148: agent.pin / agent.unpin report `pinned` and keep it
    // on the terminal.
    #[test]
    fn agent_pin_and_unpin_set_pinned_on_the_agent_and_its_pane() {
        let mut app = app_with_agent();
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.state.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&terminal_id)
            .unwrap()
            .set_detected_state(Some(Agent::Pi), AgentState::Idle);
        let target = || AgentTarget {
            target: app.public_pane_id(0, pane_id).unwrap(),
        };
        let pin_target = target();
        let unpin_target = target();
        let info = |response: String| {
            let success: SuccessResponse = serde_json::from_str(&response).unwrap();
            let ResponseResult::AgentInfo { agent } = success.result else {
                panic!("expected agent info");
            };
            agent
        };

        let pinned = info(app.handle_agent_pin("req".into(), pin_target, true));
        assert!(pinned.pinned);
        let json = serde_json::to_value(&pinned).unwrap();
        assert_eq!(json["pinned"], true);
        assert!(app.pane_info(0, pane_id).unwrap().pinned);

        let unpinned = info(app.handle_agent_pin("req".into(), unpin_target, false));
        assert!(!unpinned.pinned);
        assert!(serde_json::to_value(&unpinned)
            .unwrap()
            .get("pinned")
            .is_none());
    }

    // Fork issue 137: a watcher reads why and since when an agent is blocked
    // from the API instead of scraping its screen.
    #[test]
    fn agent_list_reports_blocked_reason_and_since_only_while_blocked() {
        let mut app = app_with_agent();
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.state.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let list_json = |app: &mut App| {
            let listed: SuccessResponse =
                serde_json::from_str(&app.handle_agent_list("req".into(), Default::default()))
                    .unwrap();
            let ResponseResult::AgentList { agents } = listed.result else {
                panic!("expected agent list");
            };
            serde_json::to_value(&agents[0]).unwrap()
        };
        let now = std::time::Instant::now();
        app.state
            .terminals
            .get_mut(&terminal_id)
            .unwrap()
            .set_detected_screen_state_at(
                Some(Agent::Claude),
                AgentState::Idle,
                false,
                None,
                false,
                false,
                now,
            );
        let idle = list_json(&mut app);
        assert!(idle.get("blocked_reason").is_none(), "{idle}");
        assert!(idle.get("blocked_since").is_none(), "{idle}");

        app.state
            .terminals
            .get_mut(&terminal_id)
            .unwrap()
            .set_detected_screen_state_at(
                Some(Agent::Claude),
                AgentState::Blocked,
                true,
                Some(crate::detect::BlockedReason::Question),
                false,
                false,
                now,
            );
        let blocked = list_json(&mut app);
        assert_eq!(blocked["agent_status"], "blocked");
        assert_eq!(blocked["blocked_reason"], "question");
        let since = blocked["blocked_since"].as_i64().expect("unix ms");
        assert!(since > 1_700_000_000_000, "{since}");
        let pane = app.pane_info(0, pane_id).expect("pane info");
        assert_eq!(
            pane.blocked_reason,
            Some(crate::detect::BlockedReason::Question)
        );
        assert_eq!(pane.blocked_since, Some(since));

        app.state
            .terminals
            .get_mut(&terminal_id)
            .unwrap()
            .set_detected_screen_state_at(
                Some(Agent::Claude),
                AgentState::Idle,
                false,
                None,
                false,
                false,
                now,
            );
        let done = list_json(&mut app);
        assert!(done.get("blocked_reason").is_none(), "{done}");
        assert!(done.get("blocked_since").is_none(), "{done}");
    }

    #[test]
    fn agent_rename_does_not_replace_the_pane_label() {
        let mut app = app_with_agent();
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.state.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let terminal = app.state.terminals.get_mut(&terminal_id).unwrap();
        terminal.set_manual_label("shell-pane".into());
        terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        let target = app.public_pane_id(0, pane_id).unwrap();

        for name in [Some("reviewer".to_string()), None] {
            let response = app.handle_agent_rename(
                "req".into(),
                AgentRenameParams {
                    target: target.clone(),
                    name,
                },
            );
            let success: SuccessResponse = serde_json::from_str(&response).unwrap();
            assert!(matches!(success.result, ResponseResult::AgentInfo { .. }));
            assert_eq!(
                app.state.terminals[&terminal_id].manual_label.as_deref(),
                Some("shell-pane")
            );
        }
    }
    fn named_agent(app: &mut App, agent: Agent, state: AgentState) -> crate::layout::PaneId {
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.state.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let terminal = app.state.terminals.get_mut(&terminal_id).unwrap();
        terminal.set_agent_name("reviewer".into());
        terminal.set_detected_state(Some(agent), state);
        pane_id
    }

    fn send(app: &mut App, params: AgentMessageParams) -> String {
        let (respond_to, response_rx) = std::sync::mpsc::channel();
        assert!(app.handle_deferred_agent_api_request(
            crate::api::schema::Request {
                id: "send".into(),
                method: crate::api::schema::Method::AgentMessage(params),
            },
            respond_to,
        ));
        response_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("agent send responds")
    }

    fn params(text: &str) -> AgentMessageParams {
        AgentMessageParams {
            target: "reviewer".into(),
            text: text.into(),
            mode: AgentMessageMode::Typed,
            busy: AgentMessageBusy::Queue,
            answer: None,
        }
    }

    fn error_code(response: &str) -> String {
        serde_json::from_str::<serde_json::Value>(response).unwrap()["error"]["code"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    #[test]
    fn split_utf8_chunks_never_cuts_a_character_and_round_trips() {
        let text = "héllo wörld ".repeat(50);
        let pieces = crate::app::api_helpers::split_utf8_chunks(&text, 7);
        assert!(pieces
            .iter()
            .all(|piece| !piece.is_empty() && piece.len() <= 7));
        assert_eq!(pieces.concat(), text);
        assert_eq!(
            crate::app::api_helpers::split_utf8_chunks("日本語", 2),
            ["日", "本", "語"],
            "a character wider than the limit is a piece of its own"
        );
        assert_eq!(crate::app::api_helpers::split_utf8_chunks("", 300), [""]);
    }

    #[tokio::test]
    async fn agent_send_types_the_text_in_pieces_then_presses_enter() {
        let mut app = app_with_agent();
        let pane_id = named_agent(&mut app, Agent::Claude, AgentState::Idle);
        let (runtime, mut rx) =
            crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
                80, 24, 0, b"", 16,
            );
        // Bracketed paste on: a send is still raw typing, never a paste.
        runtime.test_process_pty_bytes(b"\x1b[?2004h");
        app.state.insert_test_runtime(pane_id, runtime);

        let text = "a".repeat(700);
        let started = std::time::Instant::now();
        let response = send(&mut app, params(&text));
        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::AgentMessaged {
            delivery,
            pieces,
            enter_sent,
            interrupted,
            note,
            ..
        } = success.result
        else {
            panic!("expected a sent response: {response}");
        };
        assert_eq!(delivery, AgentMessageMode::Typed);
        assert_eq!((pieces, enter_sent, interrupted), (3, true, false));
        assert!(note.is_none());
        assert_eq!(rx.try_recv().unwrap().len(), 300);
        assert_eq!(rx.try_recv().unwrap().len(), 300);
        assert_eq!(rx.try_recv().unwrap().len(), 100);
        assert_eq!(rx.try_recv().unwrap(), Bytes::from_static(b"\r"));
        assert!(started.elapsed() >= AGENT_PROMPT_SUBMIT_DELAY);
    }

    #[tokio::test]
    async fn agent_send_to_a_working_agent_queues_unless_told_to_interrupt() {
        let mut app = app_with_agent();
        let pane_id = named_agent(&mut app, Agent::Claude, AgentState::Working);
        let (runtime, mut rx) =
            crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
                80, 24, 0, b"", 16,
            );
        app.state.insert_test_runtime(pane_id, runtime);

        let queued = send(&mut app, params("later"));
        let queued: SuccessResponse = serde_json::from_str(&queued).unwrap();
        let ResponseResult::AgentMessaged { interrupted, .. } = queued.result else {
            panic!("expected a sent response");
        };
        assert!(!interrupted);
        assert_eq!(rx.try_recv().unwrap(), Bytes::from_static(b"later"));
        assert_eq!(rx.try_recv().unwrap(), Bytes::from_static(b"\r"));

        let mut now = params("now");
        now.busy = AgentMessageBusy::Interrupt;
        let interrupted = send(&mut app, now);
        let interrupted: SuccessResponse = serde_json::from_str(&interrupted).unwrap();
        let ResponseResult::AgentMessaged { interrupted, .. } = interrupted.result else {
            panic!("expected a sent response");
        };
        assert!(interrupted);
        assert_eq!(rx.try_recv().unwrap(), Bytes::from_static(b"\x1b"));
        assert_eq!(rx.try_recv().unwrap(), Bytes::from_static(b"now"));
        assert_eq!(rx.try_recv().unwrap(), Bytes::from_static(b"\r"));
    }

    #[tokio::test]
    async fn interrupt_does_nothing_extra_to_an_idle_agent() {
        let mut app = app_with_agent();
        let pane_id = named_agent(&mut app, Agent::Claude, AgentState::Idle);
        let (runtime, mut rx) =
            crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
                80, 24, 0, b"", 16,
            );
        app.state.insert_test_runtime(pane_id, runtime);

        let mut idle = params("hello");
        idle.busy = AgentMessageBusy::Interrupt;
        send(&mut app, idle);
        assert_eq!(rx.try_recv().unwrap(), Bytes::from_static(b"hello"));
        assert_eq!(rx.try_recv().unwrap(), Bytes::from_static(b"\r"));
    }

    #[tokio::test]
    async fn agent_send_refuses_free_text_for_a_blocked_agent_and_answers_with_a_key() {
        let mut app = app_with_agent();
        let pane_id = named_agent(&mut app, Agent::Claude, AgentState::Blocked);
        let (runtime, mut rx) =
            crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
                80, 24, 0, b"", 16,
            );
        app.state.insert_test_runtime(pane_id, runtime);

        assert_eq!(
            error_code(&send(&mut app, params("yes please"))),
            "agent_blocked"
        );
        assert!(rx.try_recv().is_err(), "nothing was typed");

        let mut answer = params("");
        answer.answer = Some("1".into());
        let answered: SuccessResponse = serde_json::from_str(&send(&mut app, answer)).unwrap();
        let ResponseResult::AgentMessaged {
            pieces, enter_sent, ..
        } = answered.result
        else {
            panic!("expected a sent response");
        };
        assert_eq!((pieces, enter_sent), (0, false));
        assert_eq!(rx.try_recv().unwrap(), Bytes::from_static(b"1"));
        assert!(rx.try_recv().is_err(), "an answer is a key, not a line");
    }

    #[tokio::test]
    async fn agent_send_answer_needs_a_blocked_agent() {
        let mut app = app_with_agent();
        let pane_id = named_agent(&mut app, Agent::Claude, AgentState::Idle);
        let (runtime, _rx) = crate::terminal::TerminalRuntime::test_with_channel(80, 24);
        app.state.insert_test_runtime(pane_id, runtime);

        let mut answer = params("");
        answer.answer = Some("1".into());
        assert_eq!(error_code(&send(&mut app, answer)), "agent_not_blocked");
    }

    #[tokio::test]
    async fn agent_send_is_one_line_and_bounded() {
        let mut app = app_with_agent();
        let pane_id = named_agent(&mut app, Agent::Claude, AgentState::Idle);
        let (runtime, mut rx) = crate::terminal::TerminalRuntime::test_with_channel(80, 24);
        app.state.insert_test_runtime(pane_id, runtime);

        assert_eq!(error_code(&send(&mut app, params(""))), "empty_agent_send");
        assert_eq!(
            error_code(&send(&mut app, params("one\ntwo"))),
            "multi_line_send"
        );
        assert_eq!(
            error_code(&send(&mut app, params("esc\x1b[2J"))),
            "multi_line_send"
        );
        assert_eq!(
            error_code(&send(&mut app, params(&"a".repeat(4001)))),
            "agent_send_too_long"
        );
        assert!(rx.try_recv().is_err(), "nothing reached the pane");
    }

    #[tokio::test]
    async fn agent_send_to_a_plain_shell_is_not_ready() {
        let mut app = app_with_agent();
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.state.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&terminal_id)
            .unwrap()
            .set_agent_name("reviewer".into());
        let (runtime, mut rx) = crate::terminal::TerminalRuntime::test_with_channel(80, 24);
        app.state.insert_test_runtime(pane_id, runtime);

        assert_eq!(error_code(&send(&mut app, params("hi"))), "agent_not_ready");
        assert!(rx.try_recv().is_err());
    }

    #[cfg(unix)]
    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("herdr-note-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    fn note_script(dir: &std::path::Path, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-note");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.display().to_string()
    }

    #[cfg(unix)]
    #[test]
    fn the_note_command_is_run_with_the_target_sender_and_text() {
        let dir = scratch_dir("args");
        let command = note_script(&dir, "echo \"queued $*\"");
        let note = run_note_command(&command, "reviewer", "look at this", false).unwrap();
        assert_eq!(note.state, "queued");
        assert_eq!(
            note.detail,
            "queued send --to reviewer --from herdr-palette look at this"
        );

        let note = run_note_command(&command, "reviewer", "now", true).unwrap();
        assert!(note.detail.contains("--interrupt now"), "{}", note.detail);
    }

    #[cfg(unix)]
    #[test]
    fn a_held_note_says_held_in_the_commands_own_words() {
        let dir = scratch_dir("held");
        let command = note_script(
            &dir,
            "echo 'held until morning: if urgent, tell your gestore'",
        );
        let note = run_note_command(&command, "reviewer", "hi", false).unwrap();
        assert_eq!(note.state, "held");
        assert_eq!(
            note.detail,
            "held until morning: if urgent, tell your gestore"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_or_missing_note_command_is_an_error_not_a_receipt() {
        let dir = scratch_dir("fail");
        let failing = note_script(&dir, "echo 'no such session' >&2; exit 3");
        let (code, message) = run_note_command(&failing, "ghost", "hi", false).unwrap_err();
        assert_eq!(code, "note_failed");
        assert!(message.contains("no such session"), "{message}");

        let (code, message) =
            run_note_command("/nonexistent/agent-bell", "x", "hi", false).unwrap_err();
        assert_eq!(code, "note_command_missing");
        assert!(message.contains("palette.send.note_command"), "{message}");

        let (code, _) = run_note_command("  ", "x", "hi", false).unwrap_err();
        assert_eq!(code, "note_command_missing");
    }
}
