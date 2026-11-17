//! Panes whose program the session ended, not the program itself: a macOS
//! pane job that was stopped, or a logout (fork issue 288, design D3 of
//! openspec/changes/pane-launchd-jobs). Such a pane keeps its place, its
//! workspace and its resume plan, and starts again in place once launchd
//! accepts pane jobs again.

use std::time::{Duration, Instant};

use tracing::{info, warn};

use super::api::RespawnTarget;
use super::state::AppState;
use super::App;
use crate::layout::PaneId;

/// Keepers ending this close together read as one burst (a logout looks like
/// one), reported once.
const BURST_WINDOW: Duration = Duration::from_secs(2);
const BURST_SIZE: usize = 3;
/// Quiet time after the last keeper ended before its panes start again, so a
/// pane does not restart into a session that is still ending. Longer after a
/// burst: a logout takes a while.
const SETTLE: Duration = Duration::from_secs(5);
const SETTLE_AFTER_BURST: Duration = Duration::from_secs(30);
/// How often to look again while launchd accepts no pane jobs (logged out).
const RETRY_EVERY: Duration = Duration::from_secs(10);

/// When suspended panes start again, and the burst count. Pure: the clock is
/// passed in.
#[derive(Debug, Default)]
pub(crate) struct PaneSuspensions {
    burst: Option<Burst>,
    resume_at: Option<Instant>,
}

#[derive(Debug, Clone, Copy)]
struct Burst {
    last: Instant,
    count: usize,
}

impl PaneSuspensions {
    /// A keeper ended. Returns the burst size when this one makes it a burst.
    pub(crate) fn record(&mut self, now: Instant) -> Option<usize> {
        let count = match self.burst {
            Some(burst) if now.saturating_duration_since(burst.last) <= BURST_WINDOW => {
                burst.count + 1
            }
            _ => 1,
        };
        self.burst = Some(Burst { last: now, count });
        let settle = if count >= BURST_SIZE {
            SETTLE_AFTER_BURST
        } else {
            SETTLE
        };
        self.resume_at = Some(now + settle);
        (count == BURST_SIZE).then_some(count)
    }

    /// A client attached: someone is back, so try at once.
    pub(crate) fn request_now(&mut self, now: Instant) {
        self.resume_at = Some(now);
    }

    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.resume_at
    }

    fn is_due(&self, now: Instant) -> bool {
        self.resume_at.is_some_and(|at| now >= at)
    }

    fn retry_later(&mut self, now: Instant) {
        self.resume_at = Some(now + RETRY_EVERY);
    }

    fn done(&mut self) {
        self.resume_at = None;
    }
}

impl AppState {
    /// Keeps `pane_id` in its layout as suspended. False when no pane has it.
    pub(crate) fn suspend_pane(&mut self, pane_id: PaneId, now: Instant) -> bool {
        let Some(pane) = self
            .workspaces
            .iter_mut()
            .find_map(|ws| ws.pane_state_mut(pane_id))
        else {
            return false;
        };
        let Some(terminal) = self.terminals.get_mut(&pane.attached_terminal_id) else {
            return false;
        };
        // Work the session cut off is not work done: no "done" mark for it.
        if terminal.state != crate::detect::AgentState::Idle {
            pane.seen = true;
        }
        terminal.suspend(now);
        true
    }

    /// Every suspended pane, in workspace, tab and layout order.
    pub(crate) fn suspended_panes(&self) -> Vec<PaneId> {
        self.workspaces
            .iter()
            .flat_map(|ws| ws.tabs.iter())
            .flat_map(|tab| tab.layout.pane_ids())
            .filter(|pane_id| {
                self.pane_terminal(*pane_id)
                    .is_some_and(|terminal| terminal.suspended)
            })
            .collect()
    }
}

impl App {
    /// The program of `pane_id` ended without its own exit. Keeps the pane,
    /// suspended, and returns true; false for a pane that closes as before
    /// (a popup, an overlay, one a worktree removal is stopping).
    pub(super) fn suspend_pane_after_exit(&mut self, pane_id: PaneId) -> bool {
        if self.overlay_panes.contains_key(&pane_id)
            || self
                .pending_worktree_remove_runtime_exits
                .contains_key(&pane_id)
        {
            return false;
        }
        let Some(terminal_id) = self
            .find_pane(pane_id)
            .map(|(_, pane)| pane.attached_terminal_id.clone())
        else {
            return false;
        };
        // A logout may take the server next: save the session as it stands,
        // history included, before the runtime goes.
        self.checkpoint_session_before_pane_exit();
        if let Some(runtime) = self.terminal_runtimes.remove(&terminal_id) {
            let cwd = runtime.cwd_for_persistence();
            runtime.shutdown();
            if let (Some(cwd), Some(terminal)) = (cwd, self.state.terminals.get_mut(&terminal_id)) {
                terminal.cwd = cwd;
            }
        }
        let now = Instant::now();
        self.state.suspend_pane(pane_id, now);
        info!(
            pane = pane_id.raw(),
            "pane suspended: the session ended its program; it starts again in place"
        );
        if let Some(count) = self.pane_suspensions.record(now) {
            warn!(
                count,
                "pane keepers ended together (a logout looks like this); their panes are suspended"
            );
        }
        self.render_dirty.request_generic();
        self.render_notify.notify_one();
        true
    }

    /// A client attached: start suspended panes now if launchd accepts jobs.
    pub(crate) fn request_suspended_pane_resume(&mut self, now: Instant) {
        if !self.state.suspended_panes().is_empty() {
            self.pane_suspensions.request_now(now);
        }
    }

    pub(crate) fn suspended_pane_resume_deadline(&self) -> Option<Instant> {
        self.pane_suspensions.deadline()
    }

    /// Starts every suspended pane again in place once it is time and
    /// launchd accepts pane jobs. Returns whether anything changed.
    pub(crate) fn resume_suspended_panes_if_due(&mut self, now: Instant) -> bool {
        if !self.pane_suspensions.is_due(now) {
            return false;
        }
        let panes = self.state.suspended_panes();
        if panes.is_empty() {
            self.pane_suspensions.done();
            return false;
        }
        if !(self.pane_jobs_ready_probe)() {
            self.pane_suspensions.retry_later(now);
            return false;
        }
        let mut failed = 0;
        for pane_id in &panes {
            if !self.resume_suspended_pane(*pane_id) {
                failed += 1;
            }
        }
        info!(
            resumed = panes.len() - failed,
            failed, "suspended panes started again"
        );
        self.pane_suspensions.done();
        self.render_dirty.request_generic();
        self.render_notify.notify_one();
        true
    }

    /// Its agent by its resume plan, else its launch command or a shell. A
    /// pane that cannot start says so and stops waiting.
    fn resume_suspended_pane(&mut self, pane_id: PaneId) -> bool {
        if self.respawn_pane_agent(pane_id).is_some()
            || self.respawn_pane_runtime(pane_id, RespawnTarget::LaunchArgv)
        {
            return true;
        }
        if let Some(terminal) = self
            .find_pane(pane_id)
            .map(|(_, pane)| pane.attached_terminal_id.clone())
            .and_then(|terminal_id| self.state.terminals.get_mut(&terminal_id))
        {
            terminal.suspended = false;
            terminal.restore_error = Some(
                "This pane could not start again after the session ended it. Run `herdr pane respawn` on it to try again."
                    .into(),
            );
            terminal.revision = terminal.revision.saturating_add(1);
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::AppEvent;
    use crate::platform::ChildExitReason;

    #[test]
    fn a_lone_keeper_end_resumes_after_the_settle_time() {
        let mut suspensions = PaneSuspensions::default();
        let now = Instant::now();
        assert_eq!(suspensions.record(now), None);
        assert!(!suspensions.is_due(now + SETTLE - Duration::from_millis(1)));
        assert!(suspensions.is_due(now + SETTLE));
    }

    #[test]
    fn a_burst_is_reported_once_and_waits_longer() {
        let mut suspensions = PaneSuspensions::default();
        let start = Instant::now();
        let reports: Vec<_> = (0..5)
            .map(|step| suspensions.record(start + Duration::from_millis(400 * step)))
            .collect();
        assert_eq!(reports, [None, None, Some(BURST_SIZE), None, None]);
        let last = start + Duration::from_millis(1600);
        assert!(!suspensions.is_due(last + SETTLE));
        assert!(suspensions.is_due(last + SETTLE_AFTER_BURST));

        // Spread out, the ends are no burst.
        let mut spread = PaneSuspensions::default();
        for step in 0..4 {
            assert_eq!(spread.record(start + Duration::from_secs(3 * step)), None);
        }
    }

    #[test]
    fn an_attach_asks_for_a_resume_now() {
        let mut suspensions = PaneSuspensions::default();
        let now = Instant::now();
        suspensions.record(now);
        suspensions.request_now(now);
        assert!(suspensions.is_due(now));
        suspensions.retry_later(now);
        assert_eq!(suspensions.deadline(), Some(now + RETRY_EVERY));
    }

    #[test]
    fn a_suspended_pane_keeps_its_place_and_its_workspace() {
        let mut state = AppState::test_with_adversarial_identity_state();
        let ws = &state.workspaces[0];
        let pane_id = ws.tabs[ws.active_tab].root_pane;
        let workspaces = state.workspaces.len();
        let panes = state.workspaces[0].tabs[0].layout.pane_count();

        assert!(state.suspend_pane(pane_id, Instant::now()));

        assert_eq!(state.workspaces.len(), workspaces);
        assert_eq!(state.workspaces[0].tabs[0].layout.pane_count(), panes);
        assert_eq!(state.suspended_panes(), vec![pane_id]);
        state.assert_invariants_for_test();
        assert!(!state.suspend_pane(PaneId::from_raw(u32::MAX), Instant::now()));
    }

    fn app_with_one_pane(agent: bool) -> (App, PaneId, crate::terminal::TerminalId) {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        let workspace = crate::workspace::Workspace::test_new("suspend");
        let pane_id = workspace.tabs[0].root_pane;
        let terminal_id = workspace.terminal_id(pane_id).cloned().unwrap();
        app.state.workspaces = vec![workspace];
        app.state.active = Some(0);
        app.state.ensure_test_terminals();
        // `cat` as the shell: a typed resume command is echoed, never run.
        app.state.default_shell = "/bin/cat".into();
        assert!(app.respawn_pane_runtime(pane_id, RespawnTarget::Shell));
        if !agent {
            return (app, pane_id, terminal_id);
        }
        let terminal = app.state.terminals.get_mut(&terminal_id).unwrap();
        let _ = terminal.set_detected_state_with_screen_signals_at(
            Some(crate::detect::Agent::Claude),
            crate::detect::AgentState::Working,
            false,
            false,
            false,
            false,
            Instant::now(),
        );
        terminal.restore_reported_resume(crate::agent_resume::ReportedAgentResume {
            source: "herdr-test".into(),
            agent: "claude".into(),
            argv: vec![
                "herdr-suspend-288".into(),
                "--resume".into(),
                "sid-288".into(),
            ],
        });
        (app, pane_id, terminal_id)
    }

    fn wait_for_screen(app: &App, terminal_id: &crate::terminal::TerminalId, needle: &str) {
        let runtime = app.terminal_runtimes.get(terminal_id).expect("a runtime");
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut screen = String::new();
        while Instant::now() < deadline {
            screen = runtime.visible_text();
            if screen.contains(needle) {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("the pane never showed {needle:?}: {screen}");
    }

    fn shutdown_runtimes(app: &mut App) {
        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
    }

    /// V6: a keeper gone without an exit keeps the pane, its workspace and
    /// its resume plan, in the session too, and the pane comes back resumed.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_pane_whose_keeper_ended_is_kept_saved_and_resumed_in_place() {
        let _guard = crate::config::test_config_env_lock().lock().unwrap();
        let config_home = std::env::temp_dir().join(format!(
            "herdr-suspended-pane-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::env::set_var("XDG_CONFIG_HOME", &config_home);
        std::env::remove_var(crate::session::SESSION_ENV_VAR);

        let (mut app, pane_id, terminal_id) = app_with_one_pane(true);
        app.policy.persist_session = true;
        app.pane_jobs_ready_probe = || false;

        app.handle_internal_event(AppEvent::PaneDied {
            pane_id,
            exit_reason: ChildExitReason::Suspended,
        });

        assert_eq!(app.state.workspaces.len(), 1, "the workspace stays");
        assert!(app.find_pane(pane_id).is_some(), "the pane stays");
        assert!(app.terminal_runtimes.get(&terminal_id).is_none());
        let terminal = &app.state.terminals[&terminal_id];
        assert!(terminal.suspended);
        assert_eq!(terminal.state, crate::detect::AgentState::Idle);
        assert_eq!(
            app.pane_metadata(0, pane_id).unwrap().agent_status,
            crate::api::schema::AgentStatus::Idle,
            "cut-off work is not shown as done"
        );
        assert!(app.pane_agent_resume_plan(pane_id).is_some());
        assert!(app.pane_metadata(0, pane_id).unwrap().suspended);
        app.state.assert_invariants_for_test();

        app.save_session_on_shutdown();
        let saved =
            serde_json::to_string(&crate::persist::load().expect("a saved session")).unwrap();
        assert!(
            saved.contains("sid-288"),
            "the resume plan is saved: {saved}"
        );

        // Logged out: launchd takes no jobs, so the pane waits.
        let now = Instant::now();
        app.request_suspended_pane_resume(now);
        assert!(!app.resume_suspended_panes_if_due(now));
        assert!(app.state.terminals[&terminal_id].suspended);
        assert_eq!(
            app.suspended_pane_resume_deadline(),
            Some(now + RETRY_EVERY)
        );

        // Back: the next attach starts it in place, resuming its agent.
        app.pane_jobs_ready_probe = || true;
        let now = Instant::now();
        app.request_suspended_pane_resume(now);
        assert!(app.resume_suspended_panes_if_due(now));
        assert!(!app.state.terminals[&terminal_id].suspended);
        assert_eq!(app.suspended_pane_resume_deadline(), None);
        assert_eq!(
            app.state.workspaces[0].terminal_id(pane_id),
            Some(&terminal_id),
            "the same pane, in place"
        );
        wait_for_screen(&app, &terminal_id, "herdr-suspend-288 --resume sid-288");

        shutdown_runtimes(&mut app);
        std::env::remove_var("XDG_CONFIG_HOME");
        let _ = std::fs::remove_dir_all(config_home);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_shell_pane_comes_back_as_a_shell_and_other_exits_still_close() {
        let (mut app, pane_id, terminal_id) = app_with_one_pane(false);

        app.handle_internal_event(AppEvent::PaneDied {
            pane_id,
            exit_reason: ChildExitReason::Suspended,
        });
        let now = Instant::now() + SETTLE;
        assert!(app.resume_suspended_panes_if_due(now));
        assert!(app.terminal_runtimes.get(&terminal_id).is_some());
        assert!(!app.state.terminals[&terminal_id].suspended);

        app.handle_internal_event(AppEvent::PaneDied {
            pane_id,
            exit_reason: ChildExitReason::Interrupted,
        });
        assert!(
            app.state.workspaces.is_empty(),
            "an exit the program made itself closes the pane as before"
        );
        shutdown_runtimes(&mut app);
    }

    #[test]
    fn an_interrupted_exit_still_closes_the_pane() {
        let mut state = AppState::test_new();
        state.workspaces = vec![crate::workspace::Workspace::test_new("closes")];
        state.active = Some(0);
        state.ensure_test_terminals();
        let pane_id = state.workspaces[0].tabs[0].root_pane;

        state.handle_app_event(AppEvent::PaneDied {
            pane_id,
            exit_reason: ChildExitReason::Interrupted,
        });

        assert!(state.workspaces.is_empty());
        state.assert_invariants_for_test();
    }
}
