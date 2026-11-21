//! Suspended panes start again off the server loop (openspec/changes/
//! core-phase3, D2). The loop builds each pane's spawn, which is pure and
//! in memory, and hands batches of at most `RESUME_BATCH` to one resume
//! worker. The worker runs the spawns (on core, a `terminal.run` and an
//! attach, each with a 5 s timeout) and sends each runtime back as
//! `AppEvent::PaneResumed`, which the loop installs like a respawn. The next
//! batch leaves only when the last one has answered.
//!
//! Cardinality: per loss burst times suspended panes, never per render or
//! per byte. The loop's work per pane is one spec build and one install.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::mpsc;
use tracing::{info, warn};

use super::agent_resume::{AgentResumeFinish, AgentResumeLaunch};
use super::api::RespawnTarget;
use super::App;
use crate::events::{AppEvent, ResumeOutcome};
use crate::layout::PaneId;
use crate::terminal::state::PaneHold;
use crate::terminal::TerminalId;

/// Panes started per batch, so a mass resume never puts more than this
/// many `terminal.run` calls on core at once.
pub(super) const RESUME_BATCH: usize = 8;

/// What a pane's runtime runs.
pub(crate) enum SpawnProgram {
    Shell {
        shell: String,
        mode: crate::config::ShellModeConfig,
    },
    Argv(Vec<String>),
}

/// Everything one `TerminalRuntime` spawn needs, owned, so it can run on
/// another thread.
pub(crate) struct RuntimeSpawn {
    pane_id: PaneId,
    rows: u16,
    cols: u16,
    cwd: std::path::PathBuf,
    scrollback_limit_bytes: usize,
    theme: crate::terminal_theme::TerminalTheme,
    appearance: Option<crate::terminal_theme::HostAppearance>,
    launch_env: crate::pane::PaneLaunchEnv,
    program: SpawnProgram,
    runtime: crate::pane::RuntimeChoice,
    events: mpsc::Sender<AppEvent>,
    render_notify: Arc<tokio::sync::Notify>,
    render_dirty: Arc<crate::render_signal::RenderSignal>,
}

impl RuntimeSpawn {
    /// Starts the program. Needs a tokio runtime entered: the pane's tasks
    /// run there.
    pub(crate) fn run(self) -> std::io::Result<crate::terminal::TerminalRuntime> {
        match &self.program {
            SpawnProgram::Shell { shell, mode } => crate::terminal::TerminalRuntime::spawn(
                self.pane_id,
                self.rows,
                self.cols,
                self.cwd,
                self.scrollback_limit_bytes,
                self.theme,
                self.appearance,
                crate::pane::PaneShellConfig::new(shell, *mode),
                &self.launch_env,
                self.runtime,
                self.events,
                self.render_notify,
                self.render_dirty,
            ),
            SpawnProgram::Argv(argv) => crate::terminal::TerminalRuntime::spawn_argv_command(
                self.pane_id,
                self.rows,
                self.cols,
                self.cwd,
                argv,
                &self.launch_env,
                crate::pane::AgentDetection::Enabled,
                self.runtime,
                self.scrollback_limit_bytes,
                self.theme,
                self.appearance,
                self.events,
                self.render_notify,
                self.render_dirty,
            ),
        }
    }
}

/// A spawn and what to record once it runs.
pub(super) struct Prepared<F> {
    pub terminal_id: TerminalId,
    pub spawn: RuntimeSpawn,
    pub finish: F,
}

/// What a respawn records once its runtime runs.
pub(super) struct RespawnFinish {
    pub cwd: std::path::PathBuf,
    pub launch_argv: Option<Vec<String>>,
}

enum ResumeFinish {
    Agent {
        cwd: std::path::PathBuf,
        agent: String,
        finish: AgentResumeFinish,
    },
    Respawn(RespawnFinish),
}

struct ResumeJob {
    pane_id: PaneId,
    terminal_id: TerminalId,
    ready: ResumeReady,
    spawn: RuntimeSpawn,
}

/// What must answer before a pane resumes: the primary core when the pane
/// resumes on core, and the session a person stopped (D3b), which is never
/// started. A stopped secondary holds the pane even with the primary up
/// (security review M7).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ResumeReady {
    pub primary: bool,
    pub stopped: Option<String>,
}

impl ResumeReady {
    pub(super) fn for_pane(
        on_core: bool,
        suspension: Option<&crate::terminal::state::Suspension>,
    ) -> Self {
        let stopped = suspension
            .filter(|suspension| suspension.cause == crate::platform::SuspendCause::CoreStopped)
            .map(|suspension| {
                suspension
                    .core_link
                    .as_ref()
                    .map(|link| link.session.clone())
            });
        match stopped {
            None => Self {
                primary: on_core,
                stopped: None,
            },
            Some(Some(session)) if session != crate::pane::core_session_name() => Self {
                primary: on_core,
                stopped: Some(session),
            },
            Some(_) => Self {
                primary: true,
                stopped: None,
            },
        }
    }
}

/// Suspended panes on their way back.
#[derive(Default)]
pub(crate) struct ResumeQueue {
    queued: VecDeque<ResumeJob>,
    /// Every queued or in-flight pane's finish, by pane.
    finishes: HashMap<PaneId, (TerminalId, ResumeFinish)>,
    /// Panes whose program the worker is starting: each is reserved from
    /// launch to install, so nothing else starts one for it meanwhile
    /// (security review M3, Sol H8).
    in_flight: HashSet<PaneId>,
    /// Some pane of this round must be looked at again later.
    retry: bool,
    /// The most panes one batch sent at once, for tests.
    #[cfg(test)]
    pub(super) widest_batch: usize,
}

impl ResumeQueue {
    pub(crate) fn busy(&self) -> bool {
        !self.in_flight.is_empty() || !self.queued.is_empty()
    }

    /// Whether the worker may be starting `pane_id`'s program now.
    pub(crate) fn launching(&self, pane_id: PaneId) -> bool {
        self.in_flight.contains(&pane_id)
    }

    /// Marks `pane_id` as being started, for the Unix handoff test.
    #[cfg(all(test, unix))]
    pub(crate) fn reserve_for_test(&mut self, pane_id: PaneId) {
        self.in_flight.insert(pane_id);
    }

    /// How many panes the worker may be starting now.
    // Only the Unix live handoff asks.
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(crate) fn launches(&self) -> usize {
        self.in_flight.len()
    }

    pub(super) fn ask_retry(&mut self) {
        self.retry = true;
    }

    pub(super) fn take_retry(&mut self) -> bool {
        std::mem::take(&mut self.retry)
    }
}

enum Prepare {
    Job(Box<ResumeJob>, ResumeFinish),
    /// The pane cannot start and now says why.
    Settled,
    Failed,
}

impl App {
    pub(super) fn shell_program(&self) -> SpawnProgram {
        SpawnProgram::Shell {
            shell: self.state.default_shell.clone(),
            mode: self.state.shell_mode,
        }
    }

    pub(super) fn runtime_spawn(
        &self,
        pane_id: PaneId,
        rows: u16,
        cols: u16,
        cwd: std::path::PathBuf,
        launch_env: crate::pane::PaneLaunchEnv,
        program: SpawnProgram,
    ) -> RuntimeSpawn {
        let argv = match &program {
            SpawnProgram::Argv(argv) => Some(argv.as_slice()),
            SpawnProgram::Shell { .. } => None,
        };
        let runtime = self.pane_runtime_choice(pane_id, argv);
        RuntimeSpawn {
            pane_id,
            rows,
            cols,
            cwd,
            scrollback_limit_bytes: self.state.pane_scrollback_limit_bytes,
            theme: self.state.host_terminal_theme,
            appearance: self.pane_host_terminal_appearance(),
            launch_env,
            program,
            runtime,
            events: self.event_tx.clone(),
            render_notify: self.render_notify.clone(),
            render_dirty: self.render_dirty.clone(),
        }
    }

    /// Which backend a respawn or resume of `pane_id` starts its program
    /// on, `argv` when it runs one: the PTY when the pane's stable name, or
    /// else the name `argv` gives its agent, matches `core_pty_panes`
    /// (core-phase3 D13).
    pub(super) fn pane_runtime_choice(
        &self,
        pane_id: PaneId,
        argv: Option<&[String]>,
    ) -> crate::pane::RuntimeChoice {
        let pane_name = self
            .state
            .pane_terminal(pane_id)
            .and_then(crate::terminal::TerminalState::stable_name);
        match (pane_name, argv) {
            (None, Some(argv)) => crate::pane::RuntimeChoice::for_argv(argv),
            (name, _) => crate::pane::RuntimeChoice::for_pane(name),
        }
    }

    /// Queues `pane_id` to start again: its agent by its resume plan, else
    /// its launch command or a shell. False when it cannot start; the pane
    /// then says so.
    pub(super) fn queue_resume(&mut self, pane_id: PaneId, ready: ResumeReady) -> bool {
        match self.prepare_resume(pane_id, ready) {
            Prepare::Job(job, finish) => {
                let Some(terminal) = self.state.pane_terminal_mut(pane_id) else {
                    return false;
                };
                if let Some(PaneHold::Suspended(suspension)) = terminal.hold.as_mut() {
                    suspension.resuming = true;
                    terminal.revision = terminal.revision.saturating_add(1);
                }
                self.resume_queue
                    .finishes
                    .insert(pane_id, (job.terminal_id.clone(), finish));
                self.resume_queue.queued.push_back(*job);
                true
            }
            Prepare::Settled => {
                if let Some(terminal) = self.state.pane_terminal_mut(pane_id) {
                    terminal.hold = None;
                }
                false
            }
            Prepare::Failed => {
                self.pane_could_not_resume(pane_id);
                false
            }
        }
    }

    fn prepare_resume(&mut self, pane_id: PaneId, ready: ResumeReady) -> Prepare {
        let Some((ws_idx, pane)) = self.find_pane(pane_id) else {
            return Prepare::Failed;
        };
        let terminal_id = pane.attached_terminal_id.clone();
        if let Some(plan) = self.pane_agent_resume_plan(pane_id) {
            let Some(cwd) = self
                .state
                .terminals
                .get(&terminal_id)
                .map(|terminal| terminal.cwd.clone())
            else {
                return Prepare::Failed;
            };
            let (rows, cols) = self.respawn_size(ws_idx, pane_id, &terminal_id);
            match self.agent_resume_launch(
                pane_id,
                &terminal_id,
                cwd.clone(),
                &plan,
                rows,
                cols,
                true,
            ) {
                AgentResumeLaunch::Spawn(spawn, finish) => {
                    let job = ResumeJob {
                        pane_id,
                        terminal_id,
                        ready,
                        spawn: *spawn,
                    };
                    let finish = ResumeFinish::Agent {
                        cwd,
                        agent: plan.agent,
                        finish,
                    };
                    return Prepare::Job(Box::new(job), finish);
                }
                AgentResumeLaunch::Settled => return Prepare::Settled,
                AgentResumeLaunch::NotNow => {}
            }
        }
        let Some((_, prepared)) = self.prepare_respawn(pane_id, RespawnTarget::LaunchArgv) else {
            return Prepare::Failed;
        };
        let job = ResumeJob {
            pane_id,
            terminal_id: prepared.terminal_id,
            ready,
            spawn: prepared.spawn,
        };
        Prepare::Job(Box::new(job), ResumeFinish::Respawn(prepared.finish))
    }

    fn pane_could_not_resume(&mut self, pane_id: PaneId) {
        if let Some(terminal) = self.state.pane_terminal_mut(pane_id) {
            terminal.hold = None;
            terminal.restore_error = Some(
                "This pane could not start again after the session ended it. Run `herdr pane respawn` on it to try again."
                    .into(),
            );
            terminal.revision = terminal.revision.saturating_add(1);
        }
    }

    /// Whether `pane_id` still shows `terminal_id`, suspended and waiting
    /// for its resume, with no runtime: a close, a respawn by a person or a
    /// new suspension in between makes a resume's result stale.
    fn awaits_resume(&self, pane_id: PaneId, terminal_id: &TerminalId) -> bool {
        self.find_pane(pane_id)
            .is_some_and(|(_, pane)| &pane.attached_terminal_id == terminal_id)
            && self.terminal_runtimes.get(terminal_id).is_none()
            && self
                .state
                .terminals
                .get(terminal_id)
                .and_then(|terminal| terminal.suspension())
                .is_some_and(|suspension| suspension.resuming)
    }

    /// Sends the next batch to the resume worker when none is in flight.
    pub(super) fn dispatch_resume_batch(&mut self) {
        if !self.resume_queue.in_flight.is_empty() {
            return;
        }
        let mut batch = Vec::new();
        while batch.len() < RESUME_BATCH {
            let Some(job) = self.resume_queue.queued.pop_front() else {
                break;
            };
            if self.awaits_resume(job.pane_id, &job.terminal_id) {
                batch.push(job);
            } else {
                self.resume_queue.finishes.remove(&job.pane_id);
            }
        }
        if batch.is_empty() {
            return;
        }
        #[cfg(test)]
        {
            self.resume_queue.widest_batch = self.resume_queue.widest_batch.max(batch.len());
        }
        let panes: Vec<(PaneId, TerminalId)> = batch
            .iter()
            .map(|job| (job.pane_id, job.terminal_id.clone()))
            .collect();
        self.resume_queue.in_flight = panes.iter().map(|(pane_id, _)| *pane_id).collect();
        let events = self.event_tx.clone();
        let worker = tokio::runtime::Handle::try_current()
            .map_err(std::io::Error::other)
            .and_then(|handle| {
                std::thread::Builder::new()
                    .name("resume-worker".into())
                    .spawn(move || run_resume_batch(batch, &events, &handle))
            });
        if let Err(err) = worker {
            warn!(err = %err, "could not start the resume worker; suspended panes retry later");
            self.resume_queue.in_flight.clear();
            for (pane_id, terminal_id) in panes {
                self.resume_not_ready(pane_id, &terminal_id);
            }
        }
    }

    fn resume_not_ready(&mut self, pane_id: PaneId, terminal_id: &TerminalId) {
        self.resume_queue.finishes.remove(&pane_id);
        self.resume_queue.retry = true;
        if let Some(terminal) = self.state.terminals.get_mut(terminal_id) {
            if let Some(PaneHold::Suspended(suspension)) = terminal.hold.as_mut() {
                suspension.resuming = false;
                terminal.revision = terminal.revision.saturating_add(1);
            }
        }
    }

    /// The worker's answer for one pane: install its runtime like a
    /// respawn, or leave the pane as the answer says.
    pub(crate) fn install_resumed_pane(
        &mut self,
        pane_id: PaneId,
        terminal_id: TerminalId,
        outcome: ResumeOutcome,
        now: Instant,
    ) {
        self.resume_queue.in_flight.remove(&pane_id);
        let current = self.awaits_resume(pane_id, &terminal_id);
        let finish = self
            .resume_queue
            .finishes
            .remove(&pane_id)
            .filter(|(id, _)| *id == terminal_id)
            .map(|(_, finish)| finish);
        match (outcome, finish) {
            (ResumeOutcome::Started(runtime), Some(finish)) if current => {
                self.install_resumed_runtime(pane_id, terminal_id, finish, *runtime);
            }
            (ResumeOutcome::Started(runtime), _) => {
                info!(
                    pane = pane_id.raw(),
                    "a resumed program arrived for a pane that no longer waits for it; ending it"
                );
                runtime.shutdown();
            }
            (ResumeOutcome::Failed(err), _) if current => {
                warn!(pane = pane_id.raw(), err = %err, "suspended pane could not start again");
                self.pane_could_not_resume(pane_id);
            }
            (ResumeOutcome::CoreNotReady, _) if current => {
                self.resume_not_ready(pane_id, &terminal_id);
            }
            (ResumeOutcome::Failed(_) | ResumeOutcome::CoreNotReady, _) => {}
        }
        self.dispatch_resume_batch();
        if !self.resume_queue.busy() && self.resume_queue.take_retry() {
            self.pane_suspensions.retry_later(now);
        }
        self.render_dirty.request_generic();
        self.render_notify.notify_one();
    }

    fn install_resumed_runtime(
        &mut self,
        pane_id: PaneId,
        terminal_id: TerminalId,
        finish: ResumeFinish,
        runtime: crate::terminal::TerminalRuntime,
    ) {
        match finish {
            ResumeFinish::Respawn(finish) => {
                self.install_respawn(pane_id, terminal_id, finish, runtime);
            }
            ResumeFinish::Agent { cwd, agent, finish } => {
                if self
                    .mark_agent_respawned(&terminal_id, cwd, &agent)
                    .is_none()
                {
                    runtime.shutdown();
                    return;
                }
                if !self.finish_agent_resume(pane_id, terminal_id, finish, runtime) {
                    self.pane_could_not_resume(pane_id);
                }
                self.schedule_session_save();
            }
        }
    }
}

/// The resume worker: one batch, each pane on a thread of its own so the
/// batch takes as long as its slowest pane, and each answer sent as it
/// lands.
fn run_resume_batch(
    batch: Vec<ResumeJob>,
    events: &mpsc::Sender<AppEvent>,
    handle: &tokio::runtime::Handle,
) {
    let primary_answers = batch
        .iter()
        .any(|job| job.ready.primary)
        .then(crate::pane::primary_core_answers);
    let mut stopped_answers: HashMap<String, bool> = HashMap::new();
    for session in batch.iter().filter_map(|job| job.ready.stopped.as_ref()) {
        if !stopped_answers.contains_key(session) {
            let answers = crate::pane::core_session_answers(session);
            stopped_answers.insert(session.clone(), answers);
        }
    }
    let stopped_answers = &stopped_answers;
    std::thread::scope(|scope| {
        for job in batch {
            scope.spawn(move || {
                let ResumeJob {
                    pane_id,
                    terminal_id,
                    ready,
                    spawn,
                } = job;
                let waits_for_primary = ready.primary && primary_answers == Some(false);
                let waits_for_stopped = ready
                    .stopped
                    .as_ref()
                    .is_some_and(|session| stopped_answers.get(session) != Some(&true));
                let outcome = if waits_for_primary || waits_for_stopped {
                    ResumeOutcome::CoreNotReady
                } else {
                    let _runtime = handle.enter();
                    match spawn.run() {
                        Ok(runtime) => ResumeOutcome::Started(Box::new(runtime)),
                        Err(err) => ResumeOutcome::Failed(err.to_string()),
                    }
                };
                let _ = events.blocking_send(AppEvent::PaneResumed {
                    pane_id,
                    terminal_id,
                    outcome,
                });
            });
        }
    });
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::platform::{ProgramIdentity, SuspendCause};
    use crate::terminal::state::Suspension;
    use std::time::Duration;

    const GONE: ProgramIdentity = ProgramIdentity {
        pid: 10,
        started: 1,
    };

    fn app_with_suspended_panes(count: usize, on_core: bool) -> (App, Vec<PaneId>) {
        let (mut app, panes) = app_with_panes(count, on_core);
        let now = Instant::now();
        for pane in &panes {
            let mut suspension = Suspension::new(SuspendCause::HolderLost);
            suspension.program = Some(GONE);
            assert!(app.state.suspend_pane(*pane, now, suspension));
        }
        (app, panes)
    }

    /// One workspace per pane, each pane with a terminal and no program.
    fn app_with_panes(count: usize, on_core: bool) -> (App, Vec<PaneId>) {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.workspaces = (0..count)
            .map(|index| crate::workspace::Workspace::test_new(&format!("resume-{index}")))
            .collect();
        app.state.active = Some(0);
        app.state.ensure_test_terminals();
        app.state.default_shell = "/bin/cat".into();
        app.program_start_token = |_| crate::platform::ProcessStart::table(None);
        // After `App::new`, which applies the config's runtime.
        crate::pane::set_preferred_runtime(on_core);
        let panes: Vec<PaneId> = app
            .state
            .workspaces
            .iter()
            .map(|ws| ws.tabs[0].root_pane)
            .collect();
        (app, panes)
    }

    /// V32's panes: the first named by `herdr agent rename`, the second by
    /// the `--name` its agent was launched with, the third only by a title
    /// and a name the agent reported about itself.
    fn name_the_host_panes(app: &mut App, panes: &[PaneId]) {
        crate::pane::set_core_pty_panes(vec!["*-hlp".into()]);
        let terminal = app.state.pane_terminal_mut(panes[0]).expect("terminal");
        terminal.set_agent_name("manager-lab-hlp".into());
        let terminal = app.state.pane_terminal_mut(panes[1]).expect("terminal");
        terminal.restore_agent_launch(crate::agent_resume::AgentLaunchFlags {
            agent: "claude".into(),
            flags: vec!["--name".into(), "manager-aruba-hlp".into()],
            started_at_ms: None,
            env: Vec::new(),
        });
        let terminal = app.state.pane_terminal_mut(panes[2]).expect("terminal");
        terminal.set_detected_state(
            Some(crate::detect::Agent::Claude),
            crate::detect::AgentState::Idle,
        );
        terminal.set_agent_metadata(crate::terminal::AgentMetadataReport {
            source: "custom:claude".into(),
            agent_label: Some("claude".into()),
            applies_to_source: None,
            title: Some("manager-lab-hlp".into()),
            display_agent: Some("manager-lab-hlp".into()),
            state_labels: HashMap::new(),
            clear_title: false,
            clear_display_agent: false,
            clear_state_labels: false,
            ttl: None,
            seq: None,
        });
    }

    fn on_core(app: &App, pane: PaneId) -> bool {
        let terminal_id = app
            .state
            .workspaces
            .iter()
            .find_map(|ws| ws.terminal_id(pane).cloned())
            .expect("terminal");
        app.terminal_runtimes
            .get(&terminal_id)
            .expect("a runtime")
            .core_terminal()
            .is_some()
    }

    // V32 (core-phase3 D13): with core on, both computer-use host panes
    // resume on the PTY; a pane with the name only as a title resumes on core.
    #[tokio::test(flavor = "multi_thread")]
    async fn excluded_panes_resume_on_the_pty() {
        let core = crate::pane::FakeCore::start();
        std::env::set_var("HC_SOCKET", &core.socket);
        let (mut app, panes) = app_with_suspended_panes(3, true);
        name_the_host_panes(&mut app, &panes);
        assert_eq!(
            app.pane_runtime_choice(panes[0], None),
            crate::pane::RuntimeChoice::Pty(crate::pane::PtyReason::Excluded)
        );
        longest_turn_while_resuming(&mut app).await;
        assert!(app.state.suspended_panes().is_empty(), "all resumed");
        assert!(!on_core(&app, panes[0]), "manager-lab-hlp resumed on core");
        assert!(
            !on_core(&app, panes[1]),
            "manager-aruba-hlp resumed on core"
        );
        assert!(on_core(&app, panes[2]), "a title is not a name");
        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
    }

    // V32: a respawn honours the list, and a reload that drops the pattern
    // changes the answer for the next respawn.
    #[tokio::test(flavor = "multi_thread")]
    async fn excluded_panes_respawn_on_the_pty_until_the_list_changes() {
        let core = crate::pane::FakeCore::start();
        std::env::set_var("HC_SOCKET", &core.socket);
        let (mut app, panes) = app_with_panes(3, true);
        name_the_host_panes(&mut app, &panes);
        for pane in &panes {
            assert!(app.respawn_pane_runtime(*pane, RespawnTarget::Shell));
        }
        assert!(
            !on_core(&app, panes[0]),
            "manager-lab-hlp respawned on core"
        );
        assert!(
            !on_core(&app, panes[1]),
            "manager-aruba-hlp respawned on core"
        );
        assert!(on_core(&app, panes[2]), "a title is not a name");
        crate::pane::set_core_pty_panes(Vec::new());
        assert!(app.respawn_pane_runtime(panes[1], RespawnTarget::Shell));
        assert!(
            on_core(&app, panes[1]),
            "with no pattern the pane goes to core"
        );
        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
    }

    // V32: a new pane whose command names its agent `manager-lab-hlp` starts
    // on the PTY; the same command without the name starts on core.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_new_pane_named_by_its_command_starts_on_the_pty() {
        let core = crate::pane::FakeCore::start();
        std::env::set_var("HC_SOCKET", &core.socket);
        let (mut app, _panes) = app_with_panes(1, true);
        crate::pane::set_core_pty_panes(vec!["*-hlp".into()]);
        let folder = std::env::temp_dir().join(format!("hcs-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("folder");
        let claude = folder.join("claude");
        std::fs::write(&claude, "#!/bin/sh\nexec sleep 30\n").expect("stand-in");
        std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .expect("mode");
        let claude = claude.display().to_string();
        let named = [claude.clone(), "--name".into(), "manager-lab-hlp".into()];
        let (_, pane) = app
            .spawn_overlay_argv_command(&named, None, Vec::new(), Vec::new())
            .expect("named pane");
        assert!(
            pane.runtime.core_terminal().is_none(),
            "the named pane is on core"
        );
        pane.runtime.shutdown();
        let (_, pane) = app
            .spawn_overlay_argv_command(&[claude], None, Vec::new(), Vec::new())
            .expect("unnamed pane");
        assert!(
            pane.runtime.core_terminal().is_some(),
            "the unnamed pane is not on core"
        );
        pane.runtime.shutdown();
        let _ = std::fs::remove_dir_all(folder);
    }

    /// Drives the loop's two resume entry points until the round is over;
    /// returns the longest single call.
    async fn longest_turn_while_resuming(app: &mut App) -> Duration {
        let mut longest = Duration::ZERO;
        let mut timed = |app: &mut App, turn: &dyn Fn(&mut App)| {
            let started = Instant::now();
            turn(app);
            longest = longest.max(started.elapsed());
        };
        app.request_suspended_pane_resume(Instant::now());
        timed(app, &|app| {
            app.resume_suspended_panes_if_due(Instant::now());
        });
        let deadline = Instant::now() + Duration::from_secs(30);
        while app.resume_queue.busy() {
            assert!(Instant::now() < deadline, "the resume never landed");
            timed(app, &|app| {
                app.drain_internal_events();
            });
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        longest
    }

    // V43 (core-phase3 D2): with a core that takes 2 s per `terminal.run`,
    // suspended panes resume in batches of at most 8, one batch in flight,
    // and the loop's longest turn stays under 50 ms for 1 and for 16 panes.
    #[tokio::test(flavor = "multi_thread")]
    async fn suspended_panes_resume_off_the_loop_in_batches_of_eight() {
        let core = crate::pane::FakeCore::start_with(|config| {
            config.run_delay = Duration::from_secs(2);
        });
        std::env::set_var("HC_SOCKET", &core.socket);
        let mut turns = Vec::new();
        for count in [1, 16] {
            let (mut app, panes) = app_with_suspended_panes(count, true);
            let runs_before = core
                .methods()
                .iter()
                .filter(|method| *method == "terminal.run")
                .count();
            let longest = longest_turn_while_resuming(&mut app).await;
            eprintln!(
                "V43: {count} suspended pane(s), the loop's longest turn while they resumed: {longest:?}"
            );
            turns.push((count, longest));
            assert!(
                app.state.suspended_panes().is_empty(),
                "{count}: all resumed"
            );
            for pane in &panes {
                let terminal_id = app.state.workspaces[0..count]
                    .iter()
                    .find_map(|ws| ws.terminal_id(*pane).cloned())
                    .expect("terminal");
                let runtime = app.terminal_runtimes.get(&terminal_id).expect("a runtime");
                assert!(
                    runtime.core_terminal().is_some(),
                    "{count}: resumed on core"
                );
            }
            let runs = core
                .methods()
                .iter()
                .filter(|method| *method == "terminal.run")
                .count();
            assert_eq!(runs - runs_before, count, "one start per pane");
            assert!(app.resume_queue.widest_batch <= RESUME_BATCH);
            for (_, runtime) in app.terminal_runtimes.drain() {
                runtime.shutdown();
            }
        }
        assert!(
            core.most_runs_at_once() <= RESUME_BATCH,
            "one batch in flight: {} runs at once",
            core.most_runs_at_once()
        );
        assert_eq!(
            core.most_runs_at_once(),
            RESUME_BATCH,
            "16 panes fill a batch"
        );
        for (count, longest) in turns {
            assert!(
                longest < Duration::from_millis(50),
                "{count} panes: the loop's longest turn was {longest:?}"
            );
        }
    }

    // D2: a pane closed while its program starts gets no program, and the
    // started one is ended.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pane_closed_during_its_resume_gets_no_program() {
        let core = crate::pane::FakeCore::start_with(|config| {
            config.run_delay = Duration::from_millis(300);
        });
        std::env::set_var("HC_SOCKET", &core.socket);
        let (mut app, _panes) = app_with_suspended_panes(1, true);
        app.request_suspended_pane_resume(Instant::now());
        assert!(app.resume_suspended_panes_if_due(Instant::now()));
        assert!(app.resume_queue.busy());
        app.state.workspaces.clear();
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.resume_queue.busy() {
            assert!(Instant::now() < deadline, "the resume never answered");
            app.drain_internal_events();
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(
            app.terminal_runtimes.values().next().is_none(),
            "no runtime installed"
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !core
            .methods()
            .iter()
            .any(|method| method == "terminal.kill")
        {
            assert!(Instant::now() < deadline, "{:?}", core.methods());
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    // Security review M3 (Sol H8): from the worker's launch to the install,
    // a respawn of the pane starts nothing, so the pane runs one program;
    // once the resume lands, a respawn works again.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_resuming_pane_is_reserved_from_launch_to_install() {
        let core = crate::pane::FakeCore::start_with(|config| {
            config.run_delay = Duration::from_millis(400);
        });
        std::env::set_var("HC_SOCKET", &core.socket);
        let (mut app, panes) = app_with_suspended_panes(1, true);
        app.request_suspended_pane_resume(Instant::now());
        assert!(app.resume_suspended_panes_if_due(Instant::now()));
        assert!(app.resume_queue.launching(panes[0]));
        let runs = |core: &crate::pane::FakeCore| {
            core.methods()
                .iter()
                .filter(|method| *method == "terminal.run")
                .count()
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while runs(&core) == 0 {
            assert!(Instant::now() < deadline, "the worker never launched");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(!app.respawn_pane_runtime(panes[0], RespawnTarget::LaunchArgv));
        assert_eq!(app.respawn_pane_agent(panes[0]), None);
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.resume_queue.busy() {
            assert!(Instant::now() < deadline, "the resume never landed");
            app.drain_internal_events();
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(app.state.suspended_panes().is_empty(), "resumed");
        assert!(
            on_core(&app, panes[0]),
            "the worker's program is the pane's"
        );
        assert_eq!(runs(&core), 1, "one program: {:?}", core.methods());
        assert!(app.respawn_pane_runtime(panes[0], RespawnTarget::LaunchArgv));
        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
    }

    // Security review M7 (Sol H11): a pane whose secondary session a person
    // stopped waits for that session with the primary up, and resumes once
    // it answers.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_stopped_secondary_holds_its_panes_with_the_primary_up() {
        let runtime_dir = std::path::PathBuf::from(format!("/tmp/hcs7-{}", std::process::id()));
        std::env::set_var("XDG_RUNTIME_DIR", &runtime_dir);
        let primary = crate::pane::FakeCore::start();
        std::env::set_var("HC_SOCKET", &primary.socket);
        let (mut app, panes) = app_with_suspended_panes(1, true);
        let terminal = app.state.pane_terminal_mut(panes[0]).expect("terminal");
        let Some(PaneHold::Suspended(suspension)) = terminal.hold.as_mut() else {
            panic!("suspended");
        };
        suspension.cause = SuspendCause::CoreStopped;
        suspension.core_link = Some(crate::pane::CoreTerminalLink {
            session: "herdrdev-m7".into(),
            address: "t7".into(),
            program: Some(GONE),
            agent: None,
        });
        let ask = |app: &mut App| {
            let now = Instant::now();
            app.request_suspended_pane_resume(now);
            assert!(app.resume_suspended_panes_if_due(now));
        };
        ask(&mut app);
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.resume_queue.busy() {
            assert!(Instant::now() < deadline, "no answer");
            app.drain_internal_events();
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(app.state.suspended_panes(), panes, "still suspended");
        assert!(
            !primary
                .methods()
                .iter()
                .any(|method| method == "terminal.run"),
            "nothing started on the primary: {:?}",
            primary.methods()
        );

        let _secondary = crate::pane::FakeCore::start_in(
            runtime_dir.join("herdr-core"),
            "herdrdev-m7.sock",
            |_| {},
        );
        ask(&mut app);
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.resume_queue.busy() {
            assert!(Instant::now() < deadline, "no answer");
            app.drain_internal_events();
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(app.state.suspended_panes().is_empty(), "resumed");
        assert!(on_core(&app, panes[0]), "on the primary");
        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
        let _ = std::fs::remove_dir_all(runtime_dir);
    }

    // D3b: a pane a person's core stop suspended waits for the core even
    // with the PTY runtime, and never resumes while it does not answer.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_core_stopped_pane_waits_for_the_core() {
        let missing = std::env::temp_dir().join(format!("hcm-{}.sock", std::process::id()));
        std::env::set_var("HC_SOCKET", &missing);
        let (mut app, panes) = app_with_suspended_panes(1, false);
        let terminal = app.state.pane_terminal_mut(panes[0]).expect("terminal");
        let Some(PaneHold::Suspended(suspension)) = terminal.hold.as_mut() else {
            panic!("suspended");
        };
        suspension.cause = SuspendCause::CoreStopped;
        let now = Instant::now();
        app.request_suspended_pane_resume(now);
        assert!(app.resume_suspended_panes_if_due(now));
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.resume_queue.busy() {
            assert!(Instant::now() < deadline, "no answer");
            app.drain_internal_events();
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(app.state.suspended_panes(), panes, "still suspended");
        assert!(app.terminal_runtimes.values().next().is_none());
        assert!(
            app.suspended_pane_resume_deadline()
                .is_some_and(|at| at > Instant::now()),
            "looked at again later"
        );
    }
}
