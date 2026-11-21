//! Panes whose program the session ended, not the program itself: a macOS
//! pane job that was stopped, or a logout (fork issue 288, design D3 of
//! openspec/changes/pane-launchd-jobs), or herdr-core's runtime
//! (openspec/changes/core-phase3, D1). Such a pane keeps its place, its
//! workspace and its resume plan, and starts again in place once launchd
//! accepts pane jobs again and its old program is gone (core-phase3 D2).

use std::time::{Duration, Instant};

use tracing::{info, warn};

use super::api::RespawnTarget;
use super::state::{AppState, ToastKind, ToastNotification};
use super::App;
use crate::layout::PaneId;
use crate::platform::{ProgramIdentity, ProgramLiveness, StartTokenLookup, SuspendCause};
use crate::terminal::state::{ResumeBlock, Suspension};

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

/// When suspended panes start again, the burst count, and the notices a
/// burst raises (core-phase3 D2). Pure: the clock is passed in.
#[derive(Debug, Default)]
pub(crate) struct PaneSuspensions {
    burst: Option<Burst>,
    resume_at: Option<Instant>,
    /// Panes per notice in the burst being counted, posted together once
    /// the burst is over.
    notices: Vec<(CoreNotice, usize)>,
    notice_at: Option<Instant>,
}

#[derive(Debug, Clone, Copy)]
struct Burst {
    last: Instant,
    count: usize,
}

/// What a core loss notice says (core-phase3 D2's table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CoreNotice {
    HolderLost,
    HostShutdown,
    CoreStopped,
    ProgramsLost,
    Unreachable,
}

impl CoreNotice {
    /// The notice a suspension raises; a pane job's end raises none.
    pub(crate) fn for_cause(cause: SuspendCause) -> Option<Self> {
        match cause {
            SuspendCause::HolderLost => Some(Self::HolderLost),
            SuspendCause::HostShutdown => Some(Self::HostShutdown),
            SuspendCause::CoreStopped => Some(Self::CoreStopped),
            SuspendCause::StoppedByRestart | SuspendCause::TerminalGone => Some(Self::ProgramsLost),
            SuspendCause::PaneJobEnded => None,
        }
    }

    pub(crate) fn text(self, count: usize) -> String {
        let one = count == 1;
        match self {
            Self::HolderLost if one => {
                "Herdr Core quit or crashed and ended 1 pane; it resumes in place".into()
            }
            Self::HolderLost => format!(
                "Herdr Core quit or crashed and ended {count} panes; they resume in place"
            ),
            Self::HostShutdown if one => {
                "The login session ended 1 core pane; it resumes in place".into()
            }
            Self::HostShutdown => {
                format!("The login session ended {count} core panes; they resume in place")
            }
            Self::CoreStopped if one => {
                "Herdr Core was stopped; 1 pane resumes when it runs again".into()
            }
            Self::CoreStopped => {
                format!("Herdr Core was stopped; {count} panes resume when it runs again")
            }
            Self::ProgramsLost if one => {
                "Herdr Core lost 1 pane's program; it resumes in place".into()
            }
            Self::ProgramsLost => {
                format!("Herdr Core lost {count} panes' programs; they resume in place")
            }
            Self::Unreachable if one => {
                "Herdr Core is not answering; 1 program keeps running and reattaches when it answers".into()
            }
            Self::Unreachable => format!(
                "Herdr Core is not answering; {count} programs keep running and reattach when it answers"
            ),
        }
    }
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

    /// Counts one pane for `notice`, posted once nothing more is counted
    /// for `BURST_WINDOW`: one notice per burst and cause.
    pub(crate) fn count_notice(&mut self, notice: CoreNotice, now: Instant) {
        match self.notices.iter_mut().find(|(kind, _)| *kind == notice) {
            Some((_, count)) => *count += 1,
            None => self.notices.push((notice, 1)),
        }
        self.notice_at = Some(now + BURST_WINDOW);
    }

    /// The notices whose burst is over.
    pub(crate) fn take_due_notices(&mut self, now: Instant) -> Vec<(CoreNotice, usize)> {
        if self.notice_at.is_none_or(|at| now < at) {
            return Vec::new();
        }
        self.notice_at = None;
        std::mem::take(&mut self.notices)
    }

    /// A client attached: someone is back, so try at once.
    pub(crate) fn request_now(&mut self, now: Instant) {
        self.resume_at = Some(now);
    }

    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.resume_at.into_iter().chain(self.notice_at).min()
    }

    fn notice_deadline(&self) -> Option<Instant> {
        self.notice_at
    }

    fn is_due(&self, now: Instant) -> bool {
        self.resume_at.is_some_and(|at| now >= at)
    }

    /// Look again in `RETRY_EVERY`, or sooner when a newer loss already
    /// asked for an earlier time.
    pub(crate) fn retry_later(&mut self, now: Instant) {
        let retry = now + RETRY_EVERY;
        self.resume_at = Some(match self.resume_at {
            Some(at) if at > now => at.min(retry),
            _ => retry,
        });
    }

    fn done(&mut self) {
        self.resume_at = None;
    }
}

impl AppState {
    /// Keeps `pane_id` in its layout as suspended. False when no pane has it.
    pub(crate) fn suspend_pane(
        &mut self,
        pane_id: PaneId,
        now: Instant,
        suspension: Suspension,
    ) -> bool {
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
        terminal.suspend(now, suspension);
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
                    .is_some_and(|terminal| terminal.is_suspended())
            })
            .collect()
    }
}

impl App {
    /// The program of `pane_id` ended without its own exit. Keeps the pane,
    /// suspended, and returns true; false for a pane that closes as before
    /// (a popup, an overlay, one a worktree removal is stopping).
    pub(super) fn suspend_pane_after_exit(&mut self, pane_id: PaneId, cause: SuspendCause) -> bool {
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
        let mut suspension = Suspension::new(cause);
        if let Some(runtime) = self.terminal_runtimes.remove(&terminal_id) {
            (suspension.program, suspension.agent) = runtime.program_identities();
            suspension.core_link = runtime.core_terminal();
            let cwd = runtime.cwd_for_persistence();
            runtime.shutdown();
            if let (Some(cwd), Some(terminal)) = (cwd, self.state.terminals.get_mut(&terminal_id)) {
                terminal.cwd = cwd;
            }
        }
        let now = Instant::now();
        let on_core = suspension.core_link.is_some();
        self.state.suspend_pane(pane_id, now, suspension);
        info!(
            pane = pane_id.raw(),
            ?cause,
            "pane suspended: its runtime or session ended its program; it starts again in place"
        );
        if let Some(notice) = CoreNotice::for_cause(cause) {
            self.pane_suspensions.count_notice(notice, now);
        }
        // A person's stop is not a loss (D3b); every other core suspension
        // counts toward the loss guard (D3a).
        if on_core && cause != SuspendCause::CoreStopped {
            crate::pane::note_core_loss(now);
        }
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

    /// When the suspension timers next need the loop: none while a resume
    /// round is in flight, whose answers wake it.
    pub(crate) fn suspended_pane_resume_deadline(&self) -> Option<Instant> {
        if self.resume_queue.busy() {
            return self.pane_suspensions.notice_deadline();
        }
        self.pane_suspensions.deadline()
    }

    /// The core loss guard gave up (core-phase3 D3a): one notice; panes
    /// resume on the PTY from now on.
    #[cfg(unix)]
    pub(crate) fn core_gave_up(&mut self) {
        let previous = self.state.toast.clone();
        self.state.post_notification(ToastNotification {
            kind: ToastKind::NeedsAttention,
            title: "Herdr Core".into(),
            context: "Herdr Core keeps ending; panes resume on the PTY. Reload the config to try core again.".into(),
            position: None,
            target: None,
            anchor_pane: None,
        });
        self.sync_toast_deadline(previous);
        self.render_dirty.request_generic();
        self.render_notify.notify_one();
    }

    /// Posts the core loss notices whose burst is over, one per cause.
    fn post_due_core_notices(&mut self, now: Instant) -> bool {
        let due = self.pane_suspensions.take_due_notices(now);
        if due.is_empty() {
            return false;
        }
        for (notice, count) in due {
            let previous = self.state.toast.clone();
            self.state.post_notification(ToastNotification {
                kind: ToastKind::NeedsAttention,
                title: "Herdr Core".into(),
                context: notice.text(count),
                position: None,
                target: None,
                anchor_pane: None,
            });
            self.sync_toast_deadline(previous);
        }
        true
    }

    /// Starts every suspended pane again in place once it is time and its
    /// backend is ready, in batches off the server loop (core-phase3 D2).
    /// Returns whether anything changed.
    pub(crate) fn resume_suspended_panes_if_due(&mut self, now: Instant) -> bool {
        let noticed = self.post_due_core_notices(now);
        if self.resume_queue.busy() || !self.pane_suspensions.is_due(now) {
            return noticed;
        }
        let panes: Vec<PaneId> = self
            .state
            .suspended_panes()
            .into_iter()
            .filter(|pane_id| !self.pane_suspension(*pane_id).is_some_and(|s| s.resuming))
            .collect();
        if panes.is_empty() {
            self.pane_suspensions.done();
            return noticed;
        }
        let on_core = crate::pane::resumes_on_core();
        if !on_core && !(self.pane_jobs_ready_probe)() {
            self.pane_suspensions.retry_later(now);
            return noticed;
        }
        let (mut queued, mut failed, mut still_running, mut unconfirmed) = (0, 0, 0, 0);
        for pane_id in &panes {
            match self.check_old_program_gone(*pane_id) {
                Some(ResumeBlock::StillRuns(_)) => still_running += 1,
                Some(ResumeBlock::CannotConfirm) => unconfirmed += 1,
                None => {
                    // A core a person stopped is waited for in every case (D3b).
                    let needs_core = on_core
                        || self
                            .pane_suspension(*pane_id)
                            .is_some_and(|s| s.cause == SuspendCause::CoreStopped);
                    if self.queue_resume(*pane_id, needs_core) {
                        queued += 1;
                    } else {
                        failed += 1;
                    }
                }
            }
        }
        info!(
            queued,
            failed, still_running, unconfirmed, "suspended panes checked for a resume"
        );
        self.pane_suspensions.done();
        if still_running > 0 {
            self.resume_queue.ask_retry();
        }
        self.dispatch_resume_batch();
        if !self.resume_queue.busy() && self.resume_queue.take_retry() {
            self.pane_suspensions.retry_later(now);
        }
        self.render_dirty.request_generic();
        self.render_notify.notify_one();
        true
    }

    fn pane_suspension(&self, pane_id: PaneId) -> Option<&Suspension> {
        self.state.pane_terminal(pane_id)?.suspension()
    }

    /// D2's dead-program check, recorded on the pane: `None` when it may
    /// start again.
    fn check_old_program_gone(&mut self, pane_id: PaneId) -> Option<ResumeBlock> {
        let lookup = self.program_start_token;
        let terminal = self
            .find_pane(pane_id)
            .map(|(_, pane)| pane.attached_terminal_id.clone())
            .and_then(|terminal_id| self.state.terminals.get_mut(&terminal_id))?;
        let Some(crate::terminal::state::PaneHold::Suspended(suspension)) = terminal.hold.as_mut()
        else {
            return None;
        };
        let block = resume_block(suspension, lookup);
        if suspension.blocked != block {
            if let Some(block) = block {
                warn!(
                    pane = pane_id.raw(),
                    ?block,
                    "suspended pane not resumed: its old program may still run"
                );
            }
            suspension.blocked = block;
            terminal.revision = terminal.revision.saturating_add(1);
        }
        block
    }

    /// Its agent by its resume plan, else its launch command or a shell. A
    /// pane that cannot start says so and stops waiting.
    #[cfg(unix)]
    pub(super) fn resume_suspended_pane(&mut self, pane_id: PaneId) -> bool {
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
            terminal.hold = None;
            terminal.restore_error = Some(
                "This pane could not start again after the session ended it. Run `herdr pane respawn` on it to try again."
                    .into(),
            );
            terminal.revision = terminal.revision.saturating_add(1);
        }
        false
    }
}

/// Whether a suspended pane's old program may still run (core-phase3 D2):
/// the program it last ran, or the agent detection saw in it. An identity
/// never recorded may run.
pub(crate) fn resume_block(
    suspension: &Suspension,
    lookup: StartTokenLookup,
) -> Option<ResumeBlock> {
    let runs = |identity: Option<ProgramIdentity>| {
        identity.filter(|_| ProgramIdentity::liveness(identity, lookup) == ProgramLiveness::Runs)
    };
    if let Some(running) = runs(suspension.program).or(runs(suspension.agent)) {
        return Some(ResumeBlock::StillRuns(running.pid));
    }
    (suspension.program.is_none()).then_some(ResumeBlock::CannotConfirm)
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

    // A2.5 (core-phase3 D2): one notice per burst and cause, with the
    // burst's count, once the burst is over.
    #[test]
    fn a_burst_raises_one_notice_per_cause() {
        let mut suspensions = PaneSuspensions::default();
        let start = Instant::now();
        for step in 0..3 {
            suspensions.count_notice(
                CoreNotice::HolderLost,
                start + Duration::from_millis(500 * step),
            );
        }
        suspensions.count_notice(CoreNotice::Unreachable, start + Duration::from_millis(1200));
        let last = start + Duration::from_millis(1200);
        assert_eq!(suspensions.deadline(), Some(last + BURST_WINDOW));
        assert!(suspensions
            .take_due_notices(last + Duration::from_millis(1999))
            .is_empty());
        let due = suspensions.take_due_notices(last + BURST_WINDOW);
        assert_eq!(
            due,
            [(CoreNotice::HolderLost, 3), (CoreNotice::Unreachable, 1)]
        );
        assert!(suspensions
            .take_due_notices(last + BURST_WINDOW * 2)
            .is_empty());
        assert_eq!(
            CoreNotice::HolderLost.text(3),
            "Herdr Core quit or crashed and ended 3 panes; they resume in place"
        );
        assert_eq!(
            CoreNotice::Unreachable.text(1),
            "Herdr Core is not answering; 1 program keeps running and reattaches when it answers"
        );
        assert_eq!(CoreNotice::for_cause(SuspendCause::PaneJobEnded), None);
        assert_eq!(
            CoreNotice::for_cause(SuspendCause::TerminalGone),
            Some(CoreNotice::ProgramsLost)
        );
    }

    #[test]
    fn a_suspended_pane_keeps_its_place_and_its_workspace() {
        let mut state = AppState::test_with_adversarial_identity_state();
        let ws = &state.workspaces[0];
        let pane_id = ws.tabs[ws.active_tab].root_pane;
        let workspaces = state.workspaces.len();
        let panes = state.workspaces[0].tabs[0].layout.pane_count();

        assert!(state.suspend_pane(
            pane_id,
            Instant::now(),
            Suspension::new(SuspendCause::PaneJobEnded)
        ));

        assert_eq!(state.workspaces.len(), workspaces);
        assert_eq!(state.workspaces[0].tabs[0].layout.pane_count(), panes);
        assert_eq!(state.suspended_panes(), vec![pane_id]);
        state.assert_invariants_for_test();
        assert!(!state.suspend_pane(
            PaneId::from_raw(u32::MAX),
            Instant::now(),
            Suspension::new(SuspendCause::PaneJobEnded)
        ));
    }

    #[cfg(unix)]
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

    #[cfg(unix)]
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

    /// Runs a resume round and installs the resume worker's answers. Other
    /// events are dropped: the exit the old runtime reports when the test's
    /// made-up suspension shuts it down would close the pane.
    #[cfg(unix)]
    async fn resume_and_wait(app: &mut App, now: Instant) -> bool {
        let changed = app.resume_suspended_panes_if_due(now);
        let deadline = Instant::now() + Duration::from_secs(20);
        while app.resume_queue.busy() {
            assert!(Instant::now() < deadline, "the resume never landed");
            while let Ok(ev) = app.event_rx.try_recv() {
                if matches!(ev, AppEvent::PaneResumed { .. }) {
                    app.handle_internal_event(ev);
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        changed
    }

    #[cfg(unix)]
    fn shutdown_runtimes(app: &mut App) {
        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
    }

    /// V6: a keeper gone without an exit keeps the pane, its workspace and
    /// its resume plan, in the session too, and the pane comes back resumed.
    #[cfg(unix)]
    #[tokio::test]
    // The config env lock must cover the whole test, the resume's wait too;
    // nothing else on this current-thread runtime takes it.
    #[allow(clippy::await_holding_lock)]
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
            exit_reason: ChildExitReason::Suspended(SuspendCause::PaneJobEnded),
        });

        assert_eq!(app.state.workspaces.len(), 1, "the workspace stays");
        assert!(app.find_pane(pane_id).is_some(), "the pane stays");
        assert!(app.terminal_runtimes.get(&terminal_id).is_none());
        let terminal = &app.state.terminals[&terminal_id];
        assert!(terminal.is_suspended());
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
        assert!(app.state.terminals[&terminal_id].is_suspended());
        assert_eq!(
            app.suspended_pane_resume_deadline(),
            Some(now + RETRY_EVERY)
        );

        // Back: the next attach starts it in place, resuming its agent.
        app.pane_jobs_ready_probe = || true;
        let now = Instant::now();
        app.request_suspended_pane_resume(now);
        assert!(resume_and_wait(&mut app, now).await);
        assert!(!app.state.terminals[&terminal_id].is_suspended());
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
            exit_reason: ChildExitReason::Suspended(SuspendCause::PaneJobEnded),
        });
        let now = Instant::now() + SETTLE;
        assert!(resume_and_wait(&mut app, now).await);
        assert!(app.terminal_runtimes.get(&terminal_id).is_some());
        assert!(!app.state.terminals[&terminal_id].is_suspended());

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

    const ALL_CAUSES: [SuspendCause; 6] = [
        SuspendCause::HolderLost,
        SuspendCause::HostShutdown,
        SuspendCause::CoreStopped,
        SuspendCause::StoppedByRestart,
        SuspendCause::TerminalGone,
        SuspendCause::PaneJobEnded,
    ];

    const PROGRAM: ProgramIdentity = ProgramIdentity {
        pid: 10,
        started: 1,
    };
    const AGENT: ProgramIdentity = ProgramIdentity {
        pid: 20,
        started: 5,
    };

    // V35 (core-phase3 D2): for every cause, a resume waits for the old
    // program and the detected agent to be gone; an unknown pid may run.
    #[test]
    fn the_dead_program_check_applies_to_every_cause() {
        for cause in ALL_CAUSES {
            let mut suspension = Suspension::new(cause);
            suspension.program = Some(PROGRAM);
            assert_eq!(
                resume_block(&suspension, |pid| (pid == 10).then_some(1)),
                Some(ResumeBlock::StillRuns(10)),
                "{cause:?}: running"
            );
            assert_eq!(resume_block(&suspension, |_| None), None, "{cause:?}: gone");
            assert_eq!(
                resume_block(&suspension, |pid| (pid == 10).then_some(2)),
                None,
                "{cause:?}: pid reused"
            );
            suspension.agent = Some(AGENT);
            assert_eq!(
                resume_block(&suspension, |pid| (pid == 20).then_some(5)),
                Some(ResumeBlock::StillRuns(20)),
                "{cause:?}: the agent runs with the terminal's program gone"
            );
            suspension.program = None;
            assert_eq!(
                resume_block(&suspension, |_| None),
                Some(ResumeBlock::CannotConfirm),
                "{cause:?}: never recorded"
            );
        }
    }

    // V13: identity invariants hold with suspended and waiting core panes,
    // and after they come back.
    #[test]
    fn invariants_hold_with_suspended_and_waiting_core_panes() {
        let mut state = AppState::test_with_adversarial_identity_state();
        let panes: Vec<PaneId> = state.workspaces[0]
            .tabs
            .iter()
            .flat_map(|tab| tab.layout.pane_ids())
            .collect();
        assert!(panes.len() >= 2, "{panes:?}");
        let mut suspension = Suspension::new(SuspendCause::HolderLost);
        suspension.program = Some(PROGRAM);
        assert!(state.suspend_pane(panes[0], Instant::now(), suspension));
        let waiting = crate::terminal::state::CoreWaiting {
            link: crate::pane::CoreTerminalLink {
                session: "herdr".into(),
                address: "t7".into(),
                program: Some(PROGRAM),
            },
            public_pane_id: Some("w1-p2".into()),
        };
        let terminal_id = state.workspaces[0].terminal_id(panes[1]).cloned().unwrap();
        state.terminals.get_mut(&terminal_id).unwrap().hold =
            Some(crate::terminal::state::PaneHold::CoreWaiting(waiting));
        state.assert_invariants_for_test();
        assert_eq!(
            state.suspended_panes(),
            vec![panes[0]],
            "a waiting pane is not suspended"
        );

        for terminal in state.terminals.values_mut() {
            terminal.hold = None;
        }
        state.assert_invariants_for_test();
        assert!(state.suspended_panes().is_empty());
    }

    // V35 through the app: a pane whose old program still runs is not
    // resumed and says so, then resumes once it is gone; one never recorded
    // is not resumed automatically.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_suspended_pane_resumes_only_after_its_old_program_is_gone() {
        let (mut app, pane_id, terminal_id) = app_with_one_pane(false);
        app.handle_internal_event(AppEvent::PaneDied {
            pane_id,
            exit_reason: ChildExitReason::Suspended(SuspendCause::HolderLost),
        });
        let set_program = |app: &mut App, program| {
            let Some(crate::terminal::state::PaneHold::Suspended(suspension)) = app
                .state
                .terminals
                .get_mut(&terminal_id)
                .unwrap()
                .hold
                .as_mut()
            else {
                panic!("suspended");
            };
            suspension.program = program;
        };
        set_program(&mut app, Some(PROGRAM));
        app.program_start_token = |pid| (pid == 10).then_some(1);
        let now = Instant::now() + SETTLE;
        assert!(app.resume_suspended_panes_if_due(now));
        let terminal = &app.state.terminals[&terminal_id];
        assert!(terminal.is_suspended(), "the old program still runs");
        assert!(
            app.terminal_runtimes.get(&terminal_id).is_none(),
            "no second copy"
        );
        assert!(matches!(
            terminal.hold,
            Some(crate::terminal::state::PaneHold::Suspended(Suspension {
                blocked: Some(ResumeBlock::StillRuns(10)),
                ..
            }))
        ));
        assert_eq!(
            app.suspended_pane_resume_deadline(),
            Some(now + RETRY_EVERY)
        );

        set_program(&mut app, None);
        assert!(app.resume_suspended_panes_if_due(now + RETRY_EVERY));
        assert!(app.state.terminals[&terminal_id].is_suspended(), "may run");
        assert_eq!(app.suspended_pane_resume_deadline(), None, "not retried");

        set_program(&mut app, Some(PROGRAM));
        app.program_start_token = |_| None;
        app.request_suspended_pane_resume(now);
        assert!(resume_and_wait(&mut app, now).await);
        assert!(
            !app.state.terminals[&terminal_id].is_suspended(),
            "gone: resumed"
        );
        assert!(app.terminal_runtimes.get(&terminal_id).is_some());
        shutdown_runtimes(&mut app);
    }

    /// A `sleep` in a session of its own, standing in for the programs core
    /// runs, so the close-path sweep reaches it and nothing else.
    #[cfg(unix)]
    fn program_in_its_own_session() -> std::process::Child {
        use std::os::unix::process::CommandExt;
        let mut command = std::process::Command::new("sleep");
        command.arg("60");
        // SAFETY: setsid is async-signal-safe and touches no parent state.
        unsafe {
            command.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        command.spawn().expect("spawn sleep")
    }

    // V2 (core-phase3 D1, D2): a holder loss closes three core panes' streams
    // with `holder_lost`; the panes stay suspended in their tab, and once
    // their program is gone each starts again, the agent by its plan.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_holder_loss_suspends_core_panes_in_place_and_they_resume() {
        let mut program = program_in_its_own_session();
        let core = crate::pane::FakeCore::start_with(|config| config.pid = program.id());
        std::env::set_var("HC_SOCKET", &core.socket);

        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        crate::pane::set_preferred_runtime(true);
        let mut workspace = crate::workspace::Workspace::test_new("holder");
        workspace.test_split(ratatui::layout::Direction::Horizontal);
        workspace.test_split(ratatui::layout::Direction::Vertical);
        let panes = workspace.tabs[0].layout.pane_ids();
        assert_eq!(panes.len(), 3);
        app.state.workspaces = vec![workspace];
        app.state.active = Some(0);
        app.state.ensure_test_terminals();
        app.state.default_shell = "/bin/cat".into();
        for pane in &panes {
            assert!(app.respawn_pane_runtime(*pane, RespawnTarget::Shell));
        }
        let terminal_of =
            |app: &App, pane: PaneId| app.state.workspaces[0].terminal_id(pane).cloned().unwrap();
        for pane in &panes {
            let runtime = app
                .terminal_runtimes
                .get(&terminal_of(&app, *pane))
                .unwrap();
            assert!(runtime.core_terminal().is_some(), "a core pane");
        }
        let agent_terminal = terminal_of(&app, panes[0]);
        app.state
            .terminals
            .get_mut(&agent_terminal)
            .unwrap()
            .restore_reported_resume(crate::agent_resume::ReportedAgentResume {
                source: "herdr-test".into(),
                agent: "claude".into(),
                argv: vec!["herdr-p3-stub".into(), "--resume".into(), "sess-p3".into()],
            });

        for channel in 1..=3 {
            core.close_stream(channel, br#"{"reason":"holder_lost"}"#);
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.state.suspended_panes().len() < 3 {
            assert!(Instant::now() < deadline, "panes not suspended");
            app.drain_internal_events();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(app.state.workspaces.len(), 1, "the workspace stays");
        assert_eq!(app.state.workspaces[0].tabs[0].layout.pane_ids(), panes);
        for pane in &panes {
            let terminal = &app.state.terminals[&terminal_of(&app, *pane)];
            let Some(crate::terminal::state::PaneHold::Suspended(suspension)) = &terminal.hold
            else {
                panic!("suspended");
            };
            assert_eq!(suspension.cause, SuspendCause::HolderLost);
            assert_eq!(suspension.program.map(|p| p.pid), Some(program.id()));
            assert!(suspension.core_link.is_some(), "kept for the next restore");
        }
        app.state.assert_invariants_for_test();
        assert!(app.pane_agent_resume_plan(panes[0]).is_some());

        let _ = program.kill();
        let _ = program.wait();
        let runs_before = core
            .methods()
            .iter()
            .filter(|method| *method == "terminal.run")
            .count();
        app.request_suspended_pane_resume(Instant::now());
        assert!(resume_and_wait(&mut app, Instant::now()).await);
        assert!(app.state.suspended_panes().is_empty());
        let runs: Vec<_> = core
            .requests()
            .into_iter()
            .filter(|request| request["method"] == "terminal.run")
            .collect();
        assert_eq!(runs.len() - runs_before, 3, "each pane started again");
        let sent: String = runs
            .iter()
            .map(|run| run["params"]["argv"].to_string())
            .chain(
                core.received()
                    .iter()
                    .map(|frame| String::from_utf8_lossy(&frame.payload).into_owned()),
            )
            .collect();
        assert!(
            sent.contains("sess-p3"),
            "the agent resumed by its plan: {sent}"
        );
        shutdown_runtimes(&mut app);
    }
}
