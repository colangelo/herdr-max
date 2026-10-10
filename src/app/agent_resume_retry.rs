//! A restored agent whose resume command dies at boot (a network that is not
//! up yet, a daemon that is still starting) is left at a bare shell prompt.
//! Retry the typed command a bounded number of times, then say so (fork
//! issue 177).
//!
//! herdr types the resume command into the pane's shell, so it never sees the
//! agent's exit status. It looks instead: a while after the launch, an agent
//! that is running ends the watch, a shell prompt with no agent means the
//! command died. The command is only typed again at a shell prompt, never
//! into an agent that is still starting.

use std::collections::HashMap;
use std::hash::Hash;
use std::time::{Duration, Instant};

use bytes::Bytes;

use super::state::{ToastKind, ToastNotification, ToastTarget};
use super::App;

/// How long to wait before each look: after the launch, after the first retry,
/// after the second retry. The third look is the last: two retries at most.
pub(crate) const RESUME_RETRY_WAITS: [Duration; 3] = [
    Duration::from_secs(30),
    Duration::from_secs(120),
    Duration::from_secs(30),
];
/// While the foreground is neither the shell nor a known agent (an agent that
/// is still starting), look again this often...
const RESUME_RETRY_POLL: Duration = Duration::from_secs(10);
/// ...at most this many times; after that something else owns the pane.
pub(crate) const RESUME_RETRY_MAX_POLLS: u8 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetryDecision {
    /// An agent is up, or the pane is no longer ours to retry.
    Done,
    /// Not at a prompt yet; look again soon.
    Wait,
    /// At a prompt with no agent: type the command again.
    Retry,
    /// At a prompt with no agent and no retries left: say so.
    GiveUp,
}

/// One restored terminal being watched.
#[derive(Debug, Clone)]
pub(crate) struct ResumeRetry {
    pane_id: crate::layout::PaneId,
    command: String,
    agent: String,
    retries: usize,
    polls: u8,
    check_at: Instant,
    /// The saved session and launch flags being resumed, put back when the
    /// command dies (fork issue 201).
    session: Option<crate::agent_resume::PersistedAgentSession>,
    launch: Option<crate::agent_resume::AgentLaunchFlags>,
}

impl ResumeRetry {
    pub(crate) fn new(
        pane_id: crate::layout::PaneId,
        command: String,
        agent: String,
        launched: Instant,
    ) -> Self {
        Self {
            pane_id,
            command,
            agent,
            retries: 0,
            polls: 0,
            check_at: launched + RESUME_RETRY_WAITS[0],
            session: None,
            launch: None,
        }
    }

    /// Remembers what the restore resumes, so a failed command keeps it.
    pub(crate) fn resuming(
        mut self,
        (session, launch): (
            Option<crate::agent_resume::PersistedAgentSession>,
            Option<crate::agent_resume::AgentLaunchFlags>,
        ),
    ) -> Self {
        self.session = session;
        self.launch = launch;
        self
    }

    /// Decide what the look at `now` means and schedule the next one.
    pub(crate) fn advance(
        &mut self,
        now: Instant,
        agent_present: bool,
        at_prompt: bool,
    ) -> RetryDecision {
        if agent_present {
            return RetryDecision::Done;
        }
        if !at_prompt {
            if self.polls >= RESUME_RETRY_MAX_POLLS {
                return RetryDecision::Done;
            }
            self.polls += 1;
            self.check_at = now + RESUME_RETRY_POLL;
            return RetryDecision::Wait;
        }
        if self.retries + 1 >= RESUME_RETRY_WAITS.len() {
            return RetryDecision::GiveUp;
        }
        self.retries += 1;
        self.polls = 0;
        self.check_at = now + RESUME_RETRY_WAITS[self.retries];
        RetryDecision::Retry
    }
}

/// The earliest look, if any terminal is being watched.
pub(crate) fn next_check<K: Eq + Hash>(retries: &HashMap<K, ResumeRetry>) -> Option<Instant> {
    retries.values().map(|retry| retry.check_at).min()
}

impl App {
    /// Start watching a terminal whose resume command was just typed.
    pub(super) fn watch_resume(
        &mut self,
        terminal_id: crate::terminal::TerminalId,
        retry: ResumeRetry,
    ) {
        self.resume_retries.insert(terminal_id, retry);
    }

    pub(super) fn next_resume_retry_at(&self) -> Option<Instant> {
        next_check(&self.resume_retries)
    }

    /// Keep the wake-up deadline no later than the next look.
    pub(super) fn fold_resume_retry_deadline(&mut self) {
        if let Some(next) = self.next_resume_retry_at() {
            self.pending_agent_resume_deadline = Some(
                self.pending_agent_resume_deadline
                    .map_or(next, |deadline| deadline.min(next)),
            );
        }
    }

    /// Look at every watched terminal whose look is due. Returns whether
    /// anything user-visible changed.
    pub(crate) fn run_resume_retries(&mut self, now: Instant) -> bool {
        let due: Vec<crate::terminal::TerminalId> = self
            .resume_retries
            .iter()
            .filter(|(_, retry)| now >= retry.check_at)
            .map(|(id, _)| id.clone())
            .collect();
        let mut changed = false;
        let mut save = false;
        for terminal_id in due {
            let Some(mut retry) = self.resume_retries.remove(&terminal_id) else {
                continue;
            };
            let (Some(terminal), Some(runtime)) = (
                self.state.terminals.get(&terminal_id),
                self.terminal_runtimes.get(&terminal_id),
            ) else {
                continue; // the terminal is gone
            };
            let agent_present = terminal.is_agent_terminal()
                || terminal.managed_agent_kind().is_some()
                || terminal.detected_agent.is_some();
            let at_prompt = super::agents::available_shell_name(runtime).is_some();
            let decision = retry.advance(now, agent_present, at_prompt);
            if matches!(decision, RetryDecision::Retry | RetryDecision::GiveUp) {
                // The command died at the prompt: the pane's saved session is
                // still the one to resume, not lost with the dead attempt.
                if let Some(terminal) = self.state.terminals.get_mut(&terminal_id) {
                    if terminal
                        .reinstate_resume_identity(retry.session.clone(), retry.launch.clone())
                    {
                        changed = true;
                        save = true;
                    }
                }
            }
            match decision {
                RetryDecision::Done => {}
                RetryDecision::Wait => {
                    self.resume_retries.insert(terminal_id, retry);
                }
                RetryDecision::Retry => {
                    let mut input = retry.command.clone();
                    input.push('\r');
                    // Restore relaunches the agent; that is not activity in the pane.
                    if let Err(err) = runtime.try_send_bytes_untracked(Bytes::from(input)) {
                        tracing::warn!(
                            pane = retry.pane_id.raw(),
                            terminal = %terminal_id,
                            agent = %retry.agent,
                            err = %err,
                            "failed to retype the resume command"
                        );
                    } else {
                        tracing::info!(
                            pane = retry.pane_id.raw(),
                            terminal = %terminal_id,
                            agent = %retry.agent,
                            attempt = retry.retries,
                            "retyped the resume command: the agent was not running"
                        );
                    }
                    self.resume_retries.insert(terminal_id, retry);
                }
                RetryDecision::GiveUp => {
                    self.post_resume_failed(&retry);
                    changed = true;
                }
            }
        }
        if save {
            self.schedule_session_save();
        }
        changed
    }

    /// Say that a restored agent did not come back.
    pub(crate) fn post_resume_failed(&mut self, retry: &ResumeRetry) {
        let Some((ws_idx, _)) = self.find_pane(retry.pane_id) else {
            return;
        };
        let workspace_id = self.public_workspace_id(ws_idx);
        let previous = self.state.toast.clone();
        self.state.post_notification(ToastNotification {
            kind: ToastKind::NeedsAttention,
            title: format!("{} did not restart", retry.agent),
            context: format!(
                "Its resume command failed {} times. Run it by hand in this pane.",
                retry.retries + 1
            ),
            position: None,
            target: Some(ToastTarget {
                workspace_id,
                pane_id: retry.pane_id,
            }),
            anchor_pane: Some(retry.pane_id),
        });
        self.sync_toast_deadline(previous);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn retry_at(t0: Instant) -> ResumeRetry {
        ResumeRetry::new(
            crate::layout::PaneId::from_raw(1),
            "codex resume abc".into(),
            "codex".into(),
            t0,
        )
    }

    #[test]
    fn a_running_agent_ends_the_watch() {
        let t0 = Instant::now();
        let mut retry = retry_at(t0);
        assert_eq!(retry.check_at, t0 + Duration::from_secs(30));
        assert_eq!(
            retry.advance(t0 + Duration::from_secs(30), true, false),
            RetryDecision::Done
        );
    }

    #[test]
    fn a_stand_in_that_fails_once_is_retried_once_and_then_left_alone() {
        let t0 = Instant::now();
        let mut retry = retry_at(t0);
        // 30 s after launch the pane is back at its prompt: type the command again.
        let first = t0 + Duration::from_secs(30);
        assert_eq!(retry.advance(first, false, true), RetryDecision::Retry);
        assert_eq!(retry.retries, 1);
        assert_eq!(retry.check_at, first + Duration::from_secs(120));
        // 2 min later the agent is up.
        assert_eq!(
            retry.advance(first + Duration::from_secs(120), true, false),
            RetryDecision::Done
        );
    }

    #[test]
    fn an_agent_that_never_starts_is_retried_twice_then_reported_and_never_a_third_time() {
        let t0 = Instant::now();
        let mut retry = retry_at(t0);
        let mut now = t0;
        let mut decisions = Vec::new();
        for _ in 0..3 {
            now = retry.check_at;
            decisions.push(retry.advance(now, false, true));
        }
        assert_eq!(
            decisions,
            vec![
                RetryDecision::Retry,
                RetryDecision::Retry,
                RetryDecision::GiveUp
            ]
        );
        assert_eq!(retry.retries, 2);
        // The waits are 30 s, then 120 s, then 30 s for the last look.
        assert_eq!(now, t0 + Duration::from_secs(30 + 120 + 30));
    }

    #[test]
    fn an_agent_still_booting_is_polled_a_few_times_and_never_typed_into() {
        let t0 = Instant::now();
        let mut retry = retry_at(t0);
        let mut now = t0 + Duration::from_secs(30);
        for polls in 1..=RESUME_RETRY_MAX_POLLS {
            assert_eq!(retry.advance(now, false, false), RetryDecision::Wait);
            assert_eq!(retry.polls, polls);
            now = retry.check_at;
        }
        // Still not at a prompt after the polls: something else owns the pane.
        assert_eq!(retry.advance(now, false, false), RetryDecision::Done);
        assert_eq!(retry.retries, 0);
    }

    #[test]
    fn the_earliest_check_is_the_next_wakeup() {
        let t0 = Instant::now();
        let mut retries = std::collections::HashMap::new();
        assert_eq!(next_check(&retries), None);
        retries.insert("a".to_owned(), retry_at(t0));
        retries.insert("b".to_owned(), retry_at(t0 + Duration::from_secs(5)));
        assert_eq!(next_check(&retries), Some(t0 + Duration::from_secs(30)));
    }

    #[cfg(unix)]
    fn test_app() -> App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        )
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn typing_a_resume_command_starts_the_watch_and_schedules_a_wakeup() {
        let mut app = test_app();
        let workspace = crate::workspace::Workspace::test_new("restored");
        let pane_id = workspace.tabs[0].root_pane;
        let terminal_id = workspace.terminal_id(pane_id).unwrap().clone();
        app.state.workspaces = vec![workspace];
        app.state.active = Some(0);
        app.state.ensure_test_terminals();
        let terminal = app.state.terminals.get_mut(&terminal_id).unwrap();
        terminal.cwd = std::env::current_dir().unwrap();
        terminal.pending_agent_resume_plan = Some(crate::agent_resume::AgentResumePlan {
            agent: "codex".into(),
            argv: vec!["/bin/sh".into(), "-c".into(), "exit 3".into()],
            dedupe_key: "retry-test".into(),
        });
        assert!(app.resume_retries.is_empty());

        app.start_pending_agent_resume_for_terminal(&terminal_id, 24, 80, true);

        assert!(app.resume_retries.contains_key(&terminal_id));
        let first_look = app.next_resume_retry_at().expect("a look is scheduled");
        app.sync_pending_agent_resume_deadline(Instant::now());
        assert_eq!(app.pending_agent_resume_deadline, Some(first_look));
        // Not due yet: nothing is typed, nothing is posted.
        assert!(!app.run_resume_retries(Instant::now()));
        assert!(app.state.toast.is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn giving_up_posts_a_needs_attention_note_anchored_to_the_pane() {
        let mut app = test_app();
        let workspace = crate::workspace::Workspace::test_new("restored");
        let pane_id = workspace.tabs[0].root_pane;
        app.state.workspaces = vec![workspace];
        app.state.active = Some(0);
        let mut retry = retry_at(Instant::now());
        retry.pane_id = pane_id;
        retry.retries = 2;

        app.post_resume_failed(&retry);

        let toast = app.state.toast.as_ref().expect("a note is posted");
        assert_eq!(toast.kind, ToastKind::NeedsAttention);
        assert_eq!(toast.title, "codex did not restart");
        assert!(toast.context.contains("3 times"), "{}", toast.context);
        assert_eq!(toast.anchor_pane, Some(pane_id));
        assert_eq!(app.state.notification_log.len(), 1);
    }
}
