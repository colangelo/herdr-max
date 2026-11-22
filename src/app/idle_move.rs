//! The idle move (openspec/changes/core-phase3 D9): a pane whose program runs
//! off the primary herdr-core session starts again there, only while nothing
//! is happening in it. The server decides in one loop turn from facts it
//! already holds (`MoveFacts`, read by `move_refusals`), then kills the old
//! program and starts the replacement only once the kill is confirmed.

use std::time::{Duration, Instant};

use super::App;
use crate::api::schema::PaneMoveRefusal as Refusal;
use crate::layout::PaneId;
use crate::platform::{ProgramIdentity, ProgramLiveness, SessionMember};

/// How long a pane must have been left alone (D9's table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MoveWindows {
    /// No input on any path for this long.
    pub recent_input: Duration,
    /// An agent idle for this long.
    pub agent_idle: Duration,
    /// A shell pane with no input for this long.
    pub shell_quiet: Duration,
}

impl Default for MoveWindows {
    fn default() -> Self {
        Self {
            recent_input: Duration::from_secs(60),
            agent_idle: Duration::from_secs(60),
            shell_quiet: Duration::from_secs(600),
        }
    }
}

impl MoveWindows {
    /// The windows in force. A debug build reads
    /// `HERDR_TEST_MOVE_WINDOWS_MS=<input>,<agent>,<shell>`, so the live test
    /// need not leave a shell alone for ten minutes; a release build never.
    fn current() -> Self {
        #[cfg(debug_assertions)]
        if let Some(windows) = std::env::var("HERDR_TEST_MOVE_WINDOWS_MS")
            .ok()
            .and_then(|value| Self::parse_ms(&value))
        {
            return windows;
        }
        Self::default()
    }

    #[cfg(debug_assertions)]
    fn parse_ms(value: &str) -> Option<Self> {
        let mut parts = value
            .split(',')
            .map(|part| part.trim().parse().ok().map(Duration::from_millis));
        let windows = Self {
            recent_input: parts.next()??,
            agent_idle: parts.next()??,
            shell_quiet: parts.next()??,
        };
        parts.next().is_none().then_some(windows)
    }
}

/// What the server knows about one pane, gathered in one loop turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MoveFacts {
    pub on_primary: bool,
    pub excluded: bool,
    /// A replacement would not start on core (`runtime` is not `core`, or
    /// the loss guard gave up).
    pub core_off: bool,
    /// Suspended, waiting for core, an overlay, starting again, without a
    /// runtime, or being stopped by a worktree removal.
    pub not_movable: bool,
    /// Since the last input on any path; zero while a Submit is queued;
    /// `None` when the pane never had input.
    pub since_input: Option<Duration>,
    pub program: ProgramFacts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProgramFacts {
    Agent(AgentFacts),
    Shell {
        /// Not at its prompt, a foreground job, or more than the shell in
        /// its session.
        busy: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentFacts {
    /// The agent's manifest has the move facts (`has_move_facts`).
    pub supported: bool,
    /// How long the agent has been idle; `None` while it is not.
    pub idle_for: Option<Duration>,
    /// `background_activity`, or `background_count > 0`.
    pub background: bool,
    /// The input box holds text, or herdr cannot see an empty one.
    pub draft: bool,
    pub resume_plan: bool,
    pub foreign_process: bool,
}

/// Every reason the pane may not move, in D9's table order; empty means it
/// may. A pane already on the primary core gives only that.
pub(crate) fn move_refusals(facts: &MoveFacts, windows: MoveWindows) -> Vec<Refusal> {
    if facts.on_primary {
        return vec![Refusal::AlreadyOnPrimary];
    }
    let mut refusals = Vec::new();
    let mut refuse = |refused: bool, refusal: Refusal| {
        if refused {
            refusals.push(refusal);
        }
    };
    refuse(facts.excluded, Refusal::Excluded);
    refuse(!facts.excluded && facts.core_off, Refusal::CoreOff);
    refuse(facts.not_movable, Refusal::NotMovable);
    let input_within = |window: Duration| facts.since_input.is_some_and(|since| since < window);
    refuse(input_within(windows.recent_input), Refusal::RecentInput);
    match &facts.program {
        ProgramFacts::Agent(agent) if !agent.supported => {
            refuse(true, Refusal::UnsupportedAgent);
        }
        ProgramFacts::Agent(agent) => {
            let idle = agent
                .idle_for
                .is_some_and(|idle| idle >= windows.agent_idle);
            refuse(!idle, Refusal::AgentBusy);
            refuse(agent.background, Refusal::BackgroundWork);
            refuse(agent.draft, Refusal::DraftInBox);
            refuse(!agent.resume_plan, Refusal::NoResumePlan);
            refuse(agent.foreign_process, Refusal::ForeignProcess);
        }
        ProgramFacts::Shell { busy } => {
            refuse(*busy, Refusal::ShellBusy);
            refuse(input_within(windows.shell_quiet), Refusal::ShellRecent);
        }
    }
    refusals
}

/// D9's `foreign_process`: true unless the foreground job holds the agent
/// and every process in the pane's sessions is the agent, one of its
/// ancestors or one of its descendants. A job a shell escape left running
/// (`sleep &`, reparented once its shell exits) is in the session but in
/// neither line.
pub(crate) fn has_foreign_process(
    agent_pid: u32,
    foreground_has_agent: bool,
    members: &[SessionMember],
) -> bool {
    if !foreground_has_agent {
        return true;
    }
    let parents: std::collections::HashMap<u32, u32> = members
        .iter()
        .map(|member| (member.pid, member.parent))
        .collect();
    let mut ancestors = std::collections::HashSet::new();
    let mut next = parents.get(&agent_pid).copied();
    while let Some(pid) = next.filter(|pid| *pid > 1 && ancestors.insert(*pid)) {
        next = parents.get(&pid).copied();
    }
    let descends_from_agent = |pid: u32| {
        let mut seen = std::collections::HashSet::new();
        let mut current = pid;
        while current > 1 && seen.insert(current) {
            if current == agent_pid {
                return true;
            }
            match parents.get(&current) {
                Some(parent) => current = *parent,
                None => return false,
            }
        }
        false
    };
    members
        .iter()
        .any(|member| !ancestors.contains(&member.pid) && !descends_from_agent(member.pid))
}

/// D9's shell check past the prompt: the shell, and a wrapper's nested shell,
/// are all its sessions hold.
pub(crate) fn shell_runs_more(shells: &[u32], members: &[SessionMember]) -> bool {
    members.iter().any(|member| !shells.contains(&member.pid))
}

/// How long a killed program may take to leave the process table.
const KILL_SETTLE: Duration = Duration::from_secs(2);

impl App {
    /// `pane.respawn_if_idle`: the refusals, else the move. `check_only`
    /// never changes anything.
    pub(crate) fn respawn_pane_if_idle(
        &mut self,
        pane_id: PaneId,
        check_only: bool,
    ) -> Vec<Refusal> {
        let Some(facts) = self.move_facts(pane_id, Instant::now()) else {
            return vec![Refusal::NotMovable];
        };
        let refusals = move_refusals(&facts, MoveWindows::current());
        if !refusals.is_empty() || check_only {
            return refusals;
        }
        let agent = matches!(facts.program, ProgramFacts::Agent(_));
        match self.move_pane(pane_id, agent) {
            Ok(()) => Vec::new(),
            Err(refusal) => vec![refusal],
        }
    }

    fn move_facts(&self, pane_id: PaneId, now: Instant) -> Option<MoveFacts> {
        let (ws_idx, pane_state) = self.find_pane(pane_id)?;
        let terminal = self.state.terminals.get(&pane_state.attached_terminal_id)?;
        let runtime = self.lookup_runtime_sender(ws_idx, pane_id);
        let on_primary = runtime
            .and_then(crate::terminal::TerminalRuntime::core_terminal)
            .is_some_and(|link| link.session == crate::pane::core_session_name());
        let choice = self.pane_runtime_choice(pane_id, None);
        let excluded = choice == crate::pane::RuntimeChoice::Pty(crate::pane::PtyReason::Excluded);
        let not_movable = runtime.is_none()
            || terminal.hold.is_some()
            || terminal.pending_agent_resume_plan.is_some()
            || self.overlay_panes.contains_key(&pane_id)
            || self
                .pending_worktree_remove_runtime_exits
                .contains_key(&pane_id)
            || self.resume_queue.launching(pane_id);
        let since_input = runtime.and_then(|runtime| {
            if runtime.input_activity().submit_pending() {
                return Some(Duration::ZERO);
            }
            let stamp = crate::terminal::pane_last_input_at_ms(terminal, Some(runtime))?;
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_millis());
            let age = now_ms.saturating_sub(u128::try_from(stamp.max(0)).unwrap_or(0));
            Some(Duration::from_millis(
                u64::try_from(age).unwrap_or(u64::MAX),
            ))
        });
        let child_pid = runtime.and_then(crate::terminal::TerminalRuntime::child_pid);
        let program = if terminal.effective_agent_label().is_some() {
            let agent = terminal.effective_known_agent();
            ProgramFacts::Agent(AgentFacts {
                supported: agent.is_some_and(crate::detect::manifest::has_move_facts),
                idle_for: (terminal.state == crate::detect::AgentState::Idle)
                    .then(|| now.saturating_duration_since(terminal.state_since)),
                background: terminal.background_work() || terminal.background_count() > 0,
                draft: agent != Some(crate::detect::Agent::Claude)
                    || runtime
                        .and_then(|runtime| {
                            crate::detect::manifest::claude_input_box_text(
                                &runtime.detection_ansi(),
                            )
                        })
                        .is_none_or(|text| !text.is_empty()),
                resume_plan: self.pane_agent_resume_plan(pane_id).is_some(),
                foreign_process: match (
                    child_pid,
                    runtime.and_then(|runtime| runtime.program_identities().1),
                ) {
                    (Some(child), Some(agent)) => agent_pane_has_foreign_process(child, agent.pid),
                    _ => true,
                },
            })
        } else {
            ProgramFacts::Shell {
                busy: child_pid.is_none_or(shell_pane_busy),
            }
        };
        Some(MoveFacts {
            on_primary,
            excluded,
            core_off: !crate::pane::starts_on_core(choice),
            not_movable,
            since_input,
            program,
        })
    }

    /// Kills the old program, and only once that is confirmed starts the
    /// replacement on the primary core (D9's move steps). On `Err` the pane
    /// is as it was.
    fn move_pane(&mut self, pane_id: PaneId, agent: bool) -> Result<(), Refusal> {
        let (ws_idx, pane_state) = self.find_pane(pane_id).ok_or(Refusal::NotMovable)?;
        let terminal_id = pane_state.attached_terminal_id.clone();
        let runtime = self
            .terminal_runtimes
            .get(&terminal_id)
            .ok_or(Refusal::NotMovable)?;
        let (program, agent_program) = runtime.program_identities();
        let cwd = runtime.cwd_for_persistence().or_else(|| runtime.cwd());
        let history = (!agent).then(|| runtime.snapshot_history()).flatten();
        // The exit the killed program reports belongs to the old runtime.
        let absorbed =
            runtime.child_pid().is_some() && self.respawn_replaced_runtimes.insert(pane_id);
        if let Err(reason) = runtime.end_program_for_move() {
            tracing::warn!(pane = pane_id.raw(), %reason, "pane not moved: herdr-core did not confirm its program ended");
            if absorbed {
                self.respawn_replaced_runtimes.remove(&pane_id);
            }
            return Err(Refusal::CoreUnreachable);
        }
        if !program_gone(program, agent_program, self.program_start_token) {
            if absorbed {
                self.respawn_replaced_runtimes.remove(&pane_id);
            }
            tracing::warn!(
                pane = pane_id.raw(),
                ?program,
                ?agent_program,
                "pane not moved: its old program still runs after the kill"
            );
            return Err(Refusal::CoreUnreachable);
        }
        if let Some(runtime) = self.terminal_runtimes.remove(&terminal_id) {
            runtime.shutdown();
        }
        if let (Some(cwd), Some(terminal)) = (cwd, self.state.terminals.get_mut(&terminal_id)) {
            terminal.cwd = cwd;
        }
        let started = if agent {
            self.respawn_pane_agent(pane_id).is_some()
                && (self.terminal_runtimes.get(&terminal_id).is_some()
                    || self
                        .state
                        .terminals
                        .get(&terminal_id)
                        .is_some_and(|terminal| terminal.pending_agent_resume_plan.is_some()))
        } else {
            self.start_moved_shell(pane_id, history)
        };
        if !started {
            self.suspend_after_failed_move(pane_id, &terminal_id, program, agent_program);
        }
        tracing::info!(
            pane = pane_id.raw(),
            agent,
            started,
            "pane moved onto the primary herdr-core session"
        );
        self.emit_pane_updated(ws_idx, pane_id);
        Ok(())
    }

    /// A shell in the pane's directory, seeded with herdr's scrollback (D8).
    fn start_moved_shell(&mut self, pane_id: PaneId, history: Option<String>) -> bool {
        let Some((_, mut prepared)) =
            self.prepare_respawn(pane_id, super::api::RespawnTarget::Shell)
        else {
            return false;
        };
        prepared.spawn.seed_history(history);
        match prepared.spawn.run() {
            Ok(runtime) => {
                self.install_respawn(pane_id, prepared.terminal_id, prepared.finish, runtime);
                true
            }
            Err(err) => {
                tracing::warn!(pane = pane_id.raw(), err = %err, "moved shell did not start");
                false
            }
        }
    }

    /// A replacement that failed after a confirmed kill leaves the pane
    /// suspended (`TerminalGone`), so the resume path starts it (D2).
    fn suspend_after_failed_move(
        &mut self,
        pane_id: PaneId,
        terminal_id: &crate::terminal::TerminalId,
        program: Option<ProgramIdentity>,
        agent: Option<ProgramIdentity>,
    ) {
        if self.terminal_runtimes.get(terminal_id).is_some() {
            return;
        }
        let now = Instant::now();
        let mut suspension =
            crate::terminal::state::Suspension::new(crate::platform::SuspendCause::TerminalGone);
        suspension.program = program;
        suspension.agent = agent;
        if self.state.suspend_pane(pane_id, now, suspension) {
            self.pane_suspensions.record(now);
            tracing::warn!(
                pane = pane_id.raw(),
                "moved pane did not start; it is suspended and resumes in place"
            );
        }
    }
}

/// Whether the killed program left the process table within `KILL_SETTLE`:
/// a probe that cannot tell keeps the pane as it was.
fn program_gone(
    program: Option<ProgramIdentity>,
    agent: Option<ProgramIdentity>,
    lookup: crate::platform::StartTokenLookup,
) -> bool {
    let deadline = Instant::now() + KILL_SETTLE;
    loop {
        match ProgramLiveness::of_pane(program, agent, lookup) {
            ProgramLiveness::Gone => return true,
            ProgramLiveness::Unknown => return false,
            ProgramLiveness::Runs if Instant::now() >= deadline => return false,
            ProgramLiveness::Runs => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

fn agent_pane_has_foreign_process(child_pid: u32, agent_pid: u32) -> bool {
    let foreground_has_agent = crate::detect::foreground_job(child_pid).is_some_and(|job| {
        let job = crate::detect::wrapped_shell_job(
            &job,
            crate::platform::nested_foreground_job_with_owner,
        )
        .map_or(job, |(_, nested)| nested);
        job.processes.iter().any(|process| process.pid == agent_pid)
    });
    has_foreign_process(
        agent_pid,
        foreground_has_agent,
        &crate::platform::session_members(&[child_pid, agent_pid]),
    )
}

fn shell_pane_busy(child_pid: u32) -> bool {
    if crate::platform::available_pane_shell(child_pid, crate::detect::is_nested_pty_wrapper)
        .is_none()
    {
        return true;
    }
    let nested =
        crate::platform::nested_foreground_job_with_owner(child_pid).map(|(shell, _)| shell);
    let shells: Vec<u32> = std::iter::once(child_pid).chain(nested).collect();
    shell_runs_more(&shells, &crate::platform::session_members(&shells))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(pid: u32, parent: u32) -> SessionMember {
        SessionMember { pid, parent }
    }

    fn idle_claude() -> MoveFacts {
        MoveFacts {
            on_primary: false,
            excluded: false,
            core_off: false,
            not_movable: false,
            since_input: Some(Duration::from_secs(3600)),
            program: ProgramFacts::Agent(AgentFacts {
                supported: true,
                idle_for: Some(Duration::from_secs(61)),
                background: false,
                draft: false,
                resume_plan: true,
                foreign_process: false,
            }),
        }
    }

    fn agent(facts: &mut MoveFacts) -> &mut AgentFacts {
        match &mut facts.program {
            ProgramFacts::Agent(agent) => agent,
            ProgramFacts::Shell { .. } => unreachable!("an agent pane"),
        }
    }

    fn refusals(facts: &MoveFacts) -> Vec<Refusal> {
        move_refusals(facts, MoveWindows::default())
    }

    // V8 (core-phase3 D9): each fact of the table gives its refusal, and an
    // idle Claude pane with an empty box, no background work, no foreign
    // process and a plan moves.
    #[test]
    fn each_fact_gives_its_refusal_and_an_idle_agent_moves() {
        assert_eq!(refusals(&idle_claude()), []);
        type Change = fn(&mut MoveFacts);
        let rows: [(Change, Refusal); 11] = [
            (|f| f.on_primary = true, Refusal::AlreadyOnPrimary),
            (|f| f.excluded = true, Refusal::Excluded),
            (|f| f.core_off = true, Refusal::CoreOff),
            (|f| f.not_movable = true, Refusal::NotMovable),
            (
                |f| f.since_input = Some(Duration::from_secs(59)),
                Refusal::RecentInput,
            ),
            (|f| agent(f).supported = false, Refusal::UnsupportedAgent),
            (
                |f| agent(f).idle_for = Some(Duration::from_secs(59)),
                Refusal::AgentBusy,
            ),
            (|f| agent(f).background = true, Refusal::BackgroundWork),
            (|f| agent(f).draft = true, Refusal::DraftInBox),
            (|f| agent(f).resume_plan = false, Refusal::NoResumePlan),
            (|f| agent(f).foreign_process = true, Refusal::ForeignProcess),
        ];
        for (change, refusal) in rows {
            let mut facts = idle_claude();
            change(&mut facts);
            assert_eq!(refusals(&facts), [refusal], "{refusal:?}");
        }
        let mut working = idle_claude();
        agent(&mut working).idle_for = None;
        assert_eq!(refusals(&working), [Refusal::AgentBusy], "not idle at all");
        let mut excluded = idle_claude();
        excluded.excluded = true;
        excluded.core_off = true;
        assert_eq!(
            refusals(&excluded),
            [Refusal::Excluded],
            "an excluded pane is never on core"
        );
    }

    #[test]
    fn a_shell_moves_only_at_its_prompt_after_ten_quiet_minutes() {
        let shell = |busy: bool, since: Option<u64>| MoveFacts {
            program: ProgramFacts::Shell { busy },
            since_input: since.map(Duration::from_secs),
            ..idle_claude()
        };
        assert_eq!(refusals(&shell(false, None)), [], "never typed into");
        assert_eq!(refusals(&shell(false, Some(600))), []);
        assert_eq!(refusals(&shell(true, Some(600))), [Refusal::ShellBusy]);
        assert_eq!(refusals(&shell(false, Some(599))), [Refusal::ShellRecent]);
        assert_eq!(
            refusals(&shell(false, Some(10))),
            [Refusal::RecentInput, Refusal::ShellRecent]
        );
    }

    // V40 (core-phase3 D9): a pane whose agent lacks the move facts is only
    // `unsupported_agent`; its other agent facts mean nothing.
    #[test]
    fn an_unsupported_agent_is_refused_whatever_else_it_shows() {
        let mut facts = idle_claude();
        let codex = agent(&mut facts);
        codex.supported = false;
        codex.idle_for = None;
        codex.draft = true;
        assert_eq!(refusals(&facts), [Refusal::UnsupportedAgent]);
    }

    #[cfg(debug_assertions)]
    #[test]
    fn the_windows_parse_only_from_three_millisecond_values() {
        assert_eq!(
            MoveWindows::parse_ms("100, 200,300"),
            Some(MoveWindows {
                recent_input: Duration::from_millis(100),
                agent_idle: Duration::from_millis(200),
                shell_quiet: Duration::from_millis(300),
            })
        );
        for bad in ["", "1,2", "1,2,3,4", "1,x,3"] {
            assert_eq!(MoveWindows::parse_ms(bad), None, "{bad}");
        }
    }

    // V41 (core-phase3 D9): a `sleep &` a shell escape left behind is in
    // the pane's session but outside the agent's tree; the agent's own MCP
    // servers are not foreign, nor the shell and wrapper above it.
    #[test]
    fn a_job_outside_the_agents_tree_is_foreign_and_its_children_are_not() {
        // atuin (10, the pane's child) -> zsh (11) -> claude (12) -> mcp (13, 14)
        let tree = [
            member(10, 1),
            member(11, 10),
            member(12, 11),
            member(13, 12),
            member(14, 13),
        ];
        assert!(!has_foreign_process(12, true, &tree));
        let mut escaped = tree.to_vec();
        escaped.push(member(20, 1));
        assert!(
            has_foreign_process(12, true, &escaped),
            "a reparented sleep"
        );
        let mut beside = tree.to_vec();
        beside.push(member(21, 11));
        assert!(
            has_foreign_process(12, true, &beside),
            "a job of the shell beside claude"
        );
        assert!(
            has_foreign_process(12, false, &tree),
            "the foreground is not the agent"
        );
    }

    #[test]
    fn a_shell_alone_in_its_sessions_is_not_busy() {
        assert!(!shell_runs_more(
            &[10, 11],
            &[member(10, 1), member(11, 10)]
        ));
        assert!(shell_runs_more(
            &[10, 11],
            &[member(10, 1), member(11, 10), member(30, 11)]
        ));
    }

    /// One pane, its program a `sleep` in a session of its own that a fake
    /// legacy core (`legacy`) holds; the primary is another fake core.
    #[cfg(unix)]
    struct LegacyPane {
        app: App,
        pane: PaneId,
        program: std::process::Child,
        primary: crate::pane::FakeCore,
        _legacy: crate::pane::FakeCore,
        dir: std::path::PathBuf,
    }

    #[cfg(unix)]
    impl Drop for LegacyPane {
        fn drop(&mut self) {
            let _ = self.program.kill();
            let _ = self.program.wait();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// What the fake legacy core answers to `terminal.kill`.
    #[cfg(unix)]
    #[derive(Clone, Copy)]
    enum Kill {
        Finished,
        StaysLive,
        Fails,
    }

    #[cfg(unix)]
    fn legacy_pane(kill: Kill) -> LegacyPane {
        let program = program_in_its_own_session();
        let dir = std::path::PathBuf::from(format!("/tmp/hcg-{}", std::process::id()));
        std::env::set_var("XDG_RUNTIME_DIR", &dir);
        let primary = crate::pane::FakeCore::start();
        std::env::set_var("HC_SOCKET", &primary.socket);
        let legacy =
            crate::pane::FakeCore::start_in(dir.join("herdr-core"), "legacy.sock", |config| {
                config.kill_leaves_live = matches!(kill, Kill::StaysLive);
                if matches!(kill, Kill::Fails) {
                    config.failing.push("terminal.kill");
                }
            });
        legacy.add_terminal(serde_json::json!({"address": "t7", "state": "live",
            "pid": program.id(),
            "labels": {"herdr.session": crate::pane::herdr_session_label(), "herdr.pane_id": "w1-p1"}}));
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.workspaces = vec![crate::workspace::Workspace::test_new("move")];
        app.state.active = Some(0);
        app.state.ensure_test_terminals();
        app.state.default_shell = "/bin/cat".into();
        crate::pane::set_preferred_runtime(true);
        let pane = app.state.workspaces[0].tabs[0].root_pane;
        let runtime = crate::terminal::TerminalRuntime::reattach_core(
            pane,
            24,
            80,
            crate::pane::CoreTerminalLink {
                agent: None,
                session: "legacy".into(),
                address: "t7".into(),
                program: None,
            },
            Some("w1-p1".into()),
            0,
            crate::terminal_theme::TerminalTheme::default(),
            None,
            app.event_tx.clone(),
            std::sync::Arc::new(tokio::sync::Notify::new()),
            std::sync::Arc::new(crate::render_signal::RenderSignal::new()),
        )
        .expect("attached in the legacy session");
        let terminal_id = app.state.workspaces[0].tabs[0].panes[&pane]
            .attached_terminal_id
            .clone();
        app.terminal_runtimes.insert(terminal_id, runtime);
        LegacyPane {
            app,
            pane,
            program,
            primary,
            _legacy: legacy,
            dir,
        }
    }

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

    #[cfg(unix)]
    fn core_session(app: &App, pane: PaneId) -> Option<String> {
        let terminal_id = &app.find_pane(pane)?.1.attached_terminal_id;
        Some(
            app.terminal_runtimes
                .get(terminal_id)?
                .core_terminal()?
                .session,
        )
    }

    #[cfg(unix)]
    fn alive(pid: u32) -> bool {
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }

    // V39 (core-phase3 D9, roundtable item 8): a kill core does not confirm
    // (an error, or the terminal still live) refuses `core_unreachable` and
    // leaves the pane as it was: attached in its session, its program
    // running, no replacement.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn an_unconfirmed_kill_starts_no_replacement() {
        for kill in [Kill::StaysLive, Kill::Fails] {
            let mut setup = legacy_pane(kill);
            let pane = setup.pane;
            assert_eq!(
                setup.app.move_pane(pane, false),
                Err(Refusal::CoreUnreachable)
            );
            assert_eq!(core_session(&setup.app, pane).as_deref(), Some("legacy"));
            assert!(alive(setup.program.id()), "the old program was not swept");
            assert!(
                !setup
                    .primary
                    .methods()
                    .iter()
                    .any(|method| method == "terminal.run"),
                "nothing started on the primary"
            );
            setup.app.state.assert_invariants_for_test();
        }
    }

    // V39: core says `finished`, but the old program's pid still runs: no
    // replacement either.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_finished_kill_whose_program_still_runs_is_refused() {
        static TOKEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let mut setup = legacy_pane(Kill::Finished);
        let pane = setup.pane;
        let recorded = crate::platform::process_start(setup.program.id())
            .token()
            .expect("the program's token");
        TOKEN.store(recorded, std::sync::atomic::Ordering::Relaxed);
        setup.app.program_start_token = |_| {
            crate::platform::ProcessStart::Started(TOKEN.load(std::sync::atomic::Ordering::Relaxed))
        };
        assert_eq!(
            setup.app.move_pane(pane, false),
            Err(Refusal::CoreUnreachable)
        );
        assert!(
            !setup
                .primary
                .methods()
                .iter()
                .any(|method| method == "terminal.run"),
            "nothing started on the primary"
        );
    }

    // V39: `finished` and the pid gone: the pane runs a shell on the primary
    // core, in place.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_confirmed_kill_moves_the_pane_onto_the_primary() {
        let mut setup = legacy_pane(Kill::Finished);
        let pane = setup.pane;
        setup.app.program_start_token = |_| crate::platform::ProcessStart::table(None);
        assert_eq!(setup.app.move_pane(pane, false), Ok(()));
        assert_eq!(
            core_session(&setup.app, pane),
            Some(crate::pane::core_session_name()),
            "the pane runs on the primary"
        );
        assert!(setup
            .primary
            .methods()
            .iter()
            .any(|method| method == "terminal.run"));
        setup.app.state.assert_invariants_for_test();
        let facts = setup.app.move_facts(pane, Instant::now()).expect("facts");
        assert_eq!(
            move_refusals(&facts, MoveWindows::default()),
            [Refusal::AlreadyOnPrimary]
        );
    }
}
