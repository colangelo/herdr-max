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
    /// The directory a replacement would start in no longer exists.
    pub cwd_missing: bool,
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
    refuse(facts.cwd_missing, Refusal::CwdMissing);
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

/// The directory a moved pane's replacement starts in: where the program
/// runs now, else what the terminal recorded.
fn move_cwd(
    runtime: Option<&crate::terminal::TerminalRuntime>,
    terminal: &crate::terminal::state::TerminalState,
) -> std::path::PathBuf {
    runtime
        .and_then(|runtime| runtime.cwd_for_persistence().or_else(|| runtime.cwd()))
        .unwrap_or_else(|| terminal.cwd.clone())
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
            cwd_missing: !move_cwd(runtime, terminal).is_dir(),
            since_input,
            program,
        })
    }

    /// Kills the old program, and only once that is confirmed starts the
    /// replacement on the primary core (D9's move steps). An agent is asked
    /// to stop first (`stop_agent`), so it saves its session.
    ///
    /// A sent signal is a point of no return (fork issue 301): from then on
    /// the pane either runs its replacement or is suspended with its resume
    /// plan and starts again in place once its old program is gone, and the
    /// answer is `Ok`. Only a kill core refused before anything was signalled
    /// answers `Err`, with the pane as it was.
    fn move_pane(&mut self, pane_id: PaneId, agent: bool) -> Result<(), Refusal> {
        let (ws_idx, pane_state) = self.find_pane(pane_id).ok_or(Refusal::NotMovable)?;
        let terminal_id = pane_state.attached_terminal_id.clone();
        let runtime = self
            .terminal_runtimes
            .get(&terminal_id)
            .ok_or(Refusal::NotMovable)?;
        // Fork issue 305: a replacement that cannot start where the program
        // ran leaves a killed agent and a ghost pane, so refuse before any
        // signal.
        if self
            .state
            .terminals
            .get(&terminal_id)
            .is_none_or(|terminal| !move_cwd(Some(runtime), terminal).is_dir())
        {
            return Err(Refusal::CwdMissing);
        }
        let (program, agent_program) = runtime.program_identities();
        let cwd = runtime.cwd_for_persistence().or_else(|| runtime.cwd());
        let history = (!agent).then(|| runtime.snapshot_history()).flatten();
        let lookup = self.program_start_token;
        // The exit the killed program reports belongs to the old runtime.
        let absorbed =
            runtime.child_pid().is_some() && self.respawn_replaced_runtimes.insert(pane_id);
        if agent {
            stop_agent(pane_id, agent_program, lookup);
        }
        let killed = match runtime.end_program_for_move() {
            Ok(()) => true,
            Err(kill) if !agent && !kill.maybe_ended => {
                tracing::warn!(pane = pane_id.raw(), reason = %kill.reason, "pane not moved: herdr-core did not confirm its program ended");
                if absorbed {
                    self.respawn_replaced_runtimes.remove(&pane_id);
                }
                return Err(Refusal::CoreUnreachable);
            }
            Err(kill) => {
                tracing::warn!(pane = pane_id.raw(), reason = %kill.reason, "herdr-core did not confirm the moved pane's program ended; the pane is suspended");
                false
            }
        };
        let gone = killed && program_gone(program, agent_program, lookup);
        if killed && !gone {
            tracing::warn!(
                pane = pane_id.raw(),
                ?program,
                ?agent_program,
                "the moved pane's old program still runs after the kill; the pane is suspended"
            );
        }
        if let Some(runtime) = self.terminal_runtimes.remove(&terminal_id) {
            runtime.shutdown();
        }
        if let (Some(cwd), Some(terminal)) = (cwd, self.state.terminals.get_mut(&terminal_id)) {
            terminal.cwd = cwd;
        }
        let started = gone
            && if agent {
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

    /// A replacement that failed or could not start after the kill leaves
    /// the pane suspended (`TerminalGone`), so the resume path starts it once
    /// the old program is gone (D2).
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

/// How long a stopped agent may take to save its session and exit before
/// it is killed (fork issue 301).
const AGENT_STOP_GRACE: Duration = Duration::from_secs(3);

/// How long what an agent left running below it may take to end on its
/// SIGTERM before it is killed.
const LEFT_BEHIND_GRACE: Duration = Duration::from_millis(500);

/// Stops the pane's agent before its pane is killed (fork issue 301): a
/// SIGTERM, so it saves its session, then a SIGKILL of it and of every
/// process below it if it outlives `AGENT_STOP_GRACE`. An agent behind a
/// wrapper's nested PTY is in a session of its own, which the pane's kill
/// does not reach, and it outlives the hangup it gets. What it leaves
/// running below it (an MCP server it did not end) is ended too: once the
/// agent is gone those are no longer in the terminal's foreground, so the
/// hangup of its PTY would miss them.
fn stop_agent(
    pane_id: PaneId,
    agent: Option<ProgramIdentity>,
    lookup: crate::platform::StartTokenLookup,
) {
    let Some(agent) = agent else {
        return;
    };
    let runs = |identity: ProgramIdentity| {
        ProgramIdentity::liveness(Some(identity), lookup) == ProgramLiveness::Runs
    };
    if !runs(agent) {
        return;
    }
    // Recorded before the agent ends and they are reparented; by identity,
    // so a pid freed and reused since is left alone.
    let tree: Vec<ProgramIdentity> =
        descendants(agent.pid, &crate::platform::session_members(&[agent.pid]))
            .into_iter()
            .filter_map(|pid| ProgramIdentity::of(pid, lookup))
            .collect();
    let left = || -> Vec<u32> {
        tree.iter()
            .filter(|process| runs(**process))
            .map(|process| process.pid)
            .collect()
    };
    crate::platform::signal_processes(&[agent.pid], crate::platform::Signal::Terminate);
    if wait_until(AGENT_STOP_GRACE, || !runs(agent)) {
        let behind = left();
        if !behind.is_empty() {
            crate::platform::signal_processes(&behind, crate::platform::Signal::Terminate);
            if !wait_until(LEFT_BEHIND_GRACE, || left().is_empty()) {
                crate::platform::signal_processes(&left(), crate::platform::Signal::Kill);
            }
        }
    } else {
        let mut targets = left();
        if runs(agent) {
            targets.push(agent.pid);
        }
        tracing::warn!(
            pane = pane_id.raw(),
            pid = agent.pid,
            processes = targets.len(),
            "the moved pane's agent outlived its SIGTERM; killing it"
        );
        crate::platform::signal_processes(&targets, crate::platform::Signal::Kill);
    }
    let _ = wait_until(KILL_SETTLE, || !runs(agent) && left().is_empty());
}

/// Polls `done` until it holds or `limit` passes; whether it held.
fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Every member whose parent line reaches `root`.
fn descendants(root: u32, members: &[SessionMember]) -> Vec<u32> {
    let parents: std::collections::HashMap<u32, u32> = members
        .iter()
        .map(|member| (member.pid, member.parent))
        .collect();
    members
        .iter()
        .map(|member| member.pid)
        .filter(|pid| *pid != root)
        .filter(|pid| {
            let mut seen = std::collections::HashSet::new();
            let mut current = *pid;
            while let Some(parent) = parents.get(&current).copied() {
                if parent == root {
                    return true;
                }
                if parent <= 1 || !seen.insert(parent) {
                    return false;
                }
                current = parent;
            }
            false
        })
        .collect()
}

/// Whether the killed program left the process table within `KILL_SETTLE`:
/// a probe that cannot tell counts as still running.
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
            cwd_missing: false,
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
        let rows: [(Change, Refusal); 12] = [
            (|f| f.on_primary = true, Refusal::AlreadyOnPrimary),
            (|f| f.excluded = true, Refusal::Excluded),
            (|f| f.core_off = true, Refusal::CoreOff),
            (|f| f.not_movable = true, Refusal::NotMovable),
            (|f| f.cwd_missing = true, Refusal::CwdMissing),
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
                // Core ends the attached stream when the kill finishes.
                config.kill_closes_stream = matches!(kill, Kill::Finished).then_some(1);
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
                program: ProgramIdentity::of(program.id(), crate::platform::process_start),
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

    // V39 (core-phase3 D9, roundtable item 8): a kill core answers as not
    // taken (the terminal still live) refuses `core_unreachable` and leaves
    // the pane as it was: attached in its session, its program running, no
    // replacement.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_kill_core_says_did_not_take_starts_no_replacement() {
        let mut setup = legacy_pane(Kill::StaysLive);
        let pane = setup.pane;
        assert_eq!(
            setup.app.move_pane(pane, false),
            Err(Refusal::CoreUnreachable)
        );
        assert_eq!(core_session(&setup.app, pane).as_deref(), Some("legacy"));
        assert!(alive(setup.program.id()), "the old program was not swept");
        assert!(
            !setup.app.respawn_replaced_runtimes.contains(&pane),
            "no exit is expected"
        );
        assert!(
            !started_on(&setup.primary),
            "nothing started on the primary"
        );
        setup.app.state.assert_invariants_for_test();
    }

    #[cfg(unix)]
    fn started_on(core: &crate::pane::FakeCore) -> bool {
        core.methods().iter().any(|method| method == "terminal.run")
    }

    #[cfg(unix)]
    fn suspension_of(app: &App, pane: PaneId) -> Option<crate::terminal::state::Suspension> {
        let terminal_id = &app.find_pane(pane)?.1.attached_terminal_id;
        match &app.state.terminals.get(terminal_id)?.hold {
            Some(crate::terminal::state::PaneHold::Suspended(suspension)) => {
                Some(suspension.clone())
            }
            _ => None,
        }
    }

    // Fork issue 301: a kill core did not answer may have ended the program,
    // so the pane cannot stay as it was; it is suspended, starts nothing yet,
    // and the exit its old stream may still report does not close it.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_kill_core_did_not_answer_suspends_the_pane() {
        let mut setup = legacy_pane(Kill::Fails);
        let pane = setup.pane;
        assert_eq!(setup.app.move_pane(pane, false), Ok(()));
        let suspension = suspension_of(&setup.app, pane).expect("suspended");
        assert_eq!(
            suspension.cause,
            crate::platform::SuspendCause::TerminalGone
        );
        assert!(
            !started_on(&setup.primary),
            "nothing started on the primary"
        );
        handle_events_for(&mut setup.app, Duration::from_millis(300)).await;
        assert!(setup.app.find_pane(pane).is_some(), "the pane stays");
        setup.app.state.assert_invariants_for_test();
    }

    // V39, fork issue 301: core says `finished`, but the old program's pid
    // still runs: no replacement, and the pane is suspended, not left with a
    // dying program whose exit would close it.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_finished_kill_whose_program_still_runs_suspends_the_pane() {
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
        assert_eq!(setup.app.move_pane(pane, false), Ok(()));
        assert!(
            !started_on(&setup.primary),
            "nothing started on the primary"
        );
        let suspension = suspension_of(&setup.app, pane).expect("suspended");
        assert_eq!(
            suspension.program.map(|program| program.pid),
            Some(setup.program.id()),
            "the resume waits for the old program"
        );
        // Core ended the stream with the kill: that exit keeps the pane.
        handle_events_for(&mut setup.app, Duration::from_millis(500)).await;
        assert!(setup.app.find_pane(pane).is_some(), "the pane stays");
        assert!(suspension_of(&setup.app, pane).is_some(), "still suspended");
        setup.app.state.assert_invariants_for_test();
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

    /// A stand-in agent in a session of its own, as an agent behind a
    /// wrapper's nested PTY is: the pane's session sweep never reaches it.
    /// A thread reaps it, so its pid is gone the moment it exits.
    #[cfg(unix)]
    struct StandInAgent {
        identity: ProgramIdentity,
        exited: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    #[cfg(unix)]
    impl StandInAgent {
        fn start(script: &str) -> Self {
            use std::os::unix::process::CommandExt;
            let mut command = std::process::Command::new("/bin/sh");
            command.args(["-c", script]);
            // SAFETY: setsid is async-signal-safe and touches no parent state.
            unsafe {
                command.pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }
            let mut child = command.spawn().expect("spawn the stand-in agent");
            let identity = ProgramIdentity::of(child.id(), crate::platform::process_start)
                .expect("the stand-in's identity");
            let exited = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let reaped = exited.clone();
            std::thread::spawn(move || {
                let _ = child.wait();
                reaped.store(true, std::sync::atomic::Ordering::Release);
            });
            // Let the shell set its traps before anything signals it.
            std::thread::sleep(Duration::from_millis(200));
            Self { identity, exited }
        }

        fn exited(&self) -> bool {
            self.exited.load(std::sync::atomic::Ordering::Acquire)
        }
    }

    #[cfg(unix)]
    impl Drop for StandInAgent {
        fn drop(&mut self) {
            if !self.exited() {
                unsafe { libc::kill(self.identity.pid as libc::pid_t, libc::SIGKILL) };
            }
        }
    }

    /// An agent that outlives the pane's hangup and a SIGTERM by more than
    /// `KILL_SETTLE`, as Claude did in fork issue 301.
    #[cfg(unix)]
    const STUBBORN_AGENT: &str = "trap '' HUP TERM; sleep 4";

    #[cfg(unix)]
    fn give_agent(app: &mut App, pane: PaneId, agent: &StandInAgent) {
        let terminal_id = app.find_pane(pane).unwrap().1.attached_terminal_id.clone();
        let terminal = app.state.terminals.get_mut(&terminal_id).unwrap();
        let _ = terminal.set_detected_state_with_screen_signals_at(
            Some(crate::detect::Agent::Claude),
            crate::detect::AgentState::Idle,
            false,
            false,
            false,
            false,
            Instant::now(),
        );
        terminal.restore_reported_resume(crate::agent_resume::ReportedAgentResume {
            source: "herdr-test".into(),
            agent: "claude".into(),
            argv: vec!["herdr-move-301".into(), "--resume".into(), "sid-301".into()],
        });
        app.terminal_runtimes
            .get(&terminal_id)
            .unwrap()
            .test_set_agent_program(Some(agent.identity));
        assert!(app.pane_agent_resume_plan(pane).is_some(), "a resume plan");
    }

    /// One PTY pane running `/bin/cat`, which a hangup ends.
    #[cfg(unix)]
    fn pty_pane() -> (App, PaneId) {
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
        // `cat` as the shell: a typed resume command is echoed, never run.
        app.state.default_shell = "/bin/cat".into();
        crate::pane::set_preferred_runtime(false);
        let pane = app.state.workspaces[0].tabs[0].root_pane;
        assert!(app.respawn_pane_runtime(pane, super::super::api::RespawnTarget::Shell));
        (app, pane)
    }

    /// Runs the app's own event handling for `wait`: the exits the killed
    /// programs report arrive and are handled as the server loop would.
    #[cfg(unix)]
    async fn handle_events_for(app: &mut App, wait: Duration) {
        let until = Instant::now() + wait;
        while Instant::now() < until {
            app.drain_internal_events();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[cfg(unix)]
    fn screen_of(app: &App, pane: PaneId) -> String {
        let terminal_id = &app
            .find_pane(pane)
            .expect("the pane")
            .1
            .attached_terminal_id;
        app.terminal_runtimes
            .get(terminal_id)
            .map(|runtime| runtime.visible_text())
            .unwrap_or_default()
    }

    // Fork issue 301 (a): a PTY pane whose agent outlives the hangup of its
    // shell keeps the pane: the move stops the agent, and the pane resumes
    // it by its plan.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pty_agent_that_outlives_the_hangup_moves_and_keeps_its_pane() {
        let agent = StandInAgent::start(STUBBORN_AGENT);
        let (mut app, pane) = pty_pane();
        give_agent(&mut app, pane, &agent);

        let answer = app.move_pane(pane, true);
        handle_events_for(&mut app, Duration::from_secs(1)).await;
        assert!(
            app.find_pane(pane).is_some(),
            "the move closed the pane (it answered {answer:?})"
        );
        assert_eq!(answer, Ok(()));
        assert!(agent.exited(), "the move ended the old agent");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !screen_of(&app, pane).contains("sid-301") {
            assert!(Instant::now() < deadline, "the agent never resumed");
            handle_events_for(&mut app, Duration::from_millis(100)).await;
        }
        app.state.assert_invariants_for_test();
        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
    }

    // Fork issue 305: a pane whose directory was deleted (a `wt remove`
    // under a live session) is refused by the check and by the move before
    // any signal: the stand-in agent keeps running, the pane keeps its
    // runtime, and nothing waits for an exit.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pane_whose_directory_is_gone_is_refused_before_any_signal() {
        // Silent, so its `sleep` does not hold the test's output open.
        let agent = StandInAgent::start("exec >/dev/null 2>&1; trap '' HUP TERM; sleep 6");
        let dir = std::path::PathBuf::from(format!("/tmp/hcg-305-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("the pane's directory");
        let (mut app, pane) = {
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
            crate::pane::set_preferred_runtime(false);
            let pane = app.state.workspaces[0].tabs[0].root_pane;
            let terminal_id = app.find_pane(pane).unwrap().1.attached_terminal_id.clone();
            app.state.terminals.get_mut(&terminal_id).unwrap().cwd = dir.clone();
            assert!(app.respawn_pane_runtime(pane, super::super::api::RespawnTarget::Shell));
            (app, pane)
        };
        give_agent(&mut app, pane, &agent);
        let terminal_id = app.find_pane(pane).unwrap().1.attached_terminal_id.clone();
        let facts = app.move_facts(pane, Instant::now()).expect("facts");
        assert!(!facts.cwd_missing, "the directory exists");

        let _ = std::fs::remove_dir_all(&dir);
        let facts = app.move_facts(pane, Instant::now()).expect("facts");
        assert!(facts.cwd_missing);
        assert!(app
            .respawn_pane_if_idle(pane, true)
            .contains(&Refusal::CwdMissing));
        assert_eq!(app.move_pane(pane, true), Err(Refusal::CwdMissing));

        handle_events_for(&mut app, Duration::from_millis(500)).await;
        assert!(!agent.exited(), "no signal reached the agent");
        assert!(
            app.terminal_runtimes.get(&terminal_id).is_some(),
            "the pane keeps its runtime"
        );
        assert!(
            !app.respawn_replaced_runtimes.contains(&pane),
            "no exit is expected"
        );
        assert!(suspension_of(&app, pane).is_none(), "not suspended");
        app.state.assert_invariants_for_test();
        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
    }

    /// The legacy pane's program is swept by the kill but stays a zombie of
    /// the test: it counts as gone.
    #[cfg(unix)]
    static SWEPT_PROGRAM: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    #[cfg(unix)]
    fn swept_program_is_gone(pid: u32) -> crate::platform::ProcessStart {
        if pid == SWEPT_PROGRAM.load(std::sync::atomic::Ordering::Relaxed) {
            return crate::platform::ProcessStart::table(None);
        }
        crate::platform::process_start(pid)
    }

    // Fork issue 301 (a): the same for a pane in a legacy core session, whose
    // stream core ends with the kill: the agent resumes on the primary core.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_legacy_core_agent_that_outlives_the_kill_moves_and_keeps_its_pane() {
        let agent = StandInAgent::start(STUBBORN_AGENT);
        let mut setup = legacy_pane(Kill::Finished);
        let pane = setup.pane;
        SWEPT_PROGRAM.store(setup.program.id(), std::sync::atomic::Ordering::Relaxed);
        setup.app.program_start_token = swept_program_is_gone;
        give_agent(&mut setup.app, pane, &agent);

        let answer = setup.app.move_pane(pane, true);
        handle_events_for(&mut setup.app, Duration::from_secs(1)).await;
        assert!(
            setup.app.find_pane(pane).is_some(),
            "the move closed the pane (it answered {answer:?})"
        );
        assert_eq!(answer, Ok(()));
        assert!(agent.exited(), "the move ended the old agent");
        assert_eq!(
            core_session(&setup.app, pane),
            Some(crate::pane::core_session_name()),
            "the pane runs on the primary"
        );
        let resumed = || {
            setup
                .primary
                .requests()
                .iter()
                .map(|request| request.to_string())
                .chain(
                    setup
                        .primary
                        .received()
                        .iter()
                        .map(|frame| String::from_utf8_lossy(&frame.payload).into_owned()),
                )
                .any(|sent| sent.contains("sid-301"))
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !resumed() {
            assert!(Instant::now() < deadline, "the agent never resumed");
            handle_events_for(&mut setup.app, Duration::from_millis(100)).await;
        }
        setup.app.state.assert_invariants_for_test();
        for (_, runtime) in setup.app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
    }

    /// The stand-in agent the probe reports as still running, whatever it
    /// does, until the test lets it go (0: none).
    #[cfg(unix)]
    static HELD_AGENT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    #[cfg(unix)]
    static HELD_TOKEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    #[cfg(unix)]
    fn held_agent_runs(pid: u32) -> crate::platform::ProcessStart {
        if pid != 0 && pid == HELD_AGENT.load(std::sync::atomic::Ordering::Relaxed) {
            return crate::platform::ProcessStart::Started(
                HELD_TOKEN.load(std::sync::atomic::Ordering::Relaxed),
            );
        }
        swept_program_is_gone(pid)
    }

    #[cfg(unix)]
    fn hold_agent(agent: &StandInAgent) {
        HELD_TOKEN.store(agent.identity.started, std::sync::atomic::Ordering::Relaxed);
        HELD_AGENT.store(agent.identity.pid, std::sync::atomic::Ordering::Relaxed);
    }

    /// Fork issue 301 (b): an agent herdr still sees after the kill. The
    /// move answers `Ok` and the pane stays suspended with its plan through
    /// the exits the killed program reports; once the agent is gone, it
    /// resumes in place (`resumed`).
    #[cfg(unix)]
    async fn suspends_then_resumes_in_place(
        app: &mut App,
        pane: PaneId,
        agent: &StandInAgent,
        resumed: impl Fn(&App) -> bool,
    ) {
        hold_agent(agent);
        app.program_start_token = held_agent_runs;
        app.pane_jobs_ready_probe = || true;
        let terminal_id = app.find_pane(pane).unwrap().1.attached_terminal_id.clone();

        let answer = app.move_pane(pane, true);
        handle_events_for(app, Duration::from_secs(1)).await;
        assert!(
            app.find_pane(pane).is_some(),
            "the move closed the pane (it answered {answer:?})"
        );
        assert_eq!(answer, Ok(()), "a killed pane is never refused");
        let suspension = suspension_of(app, pane).expect("suspended");
        assert_eq!(
            suspension.cause,
            crate::platform::SuspendCause::TerminalGone
        );
        assert_eq!(suspension.agent, Some(agent.identity));
        assert!(app.terminal_runtimes.get(&terminal_id).is_none());
        assert!(app.pane_agent_resume_plan(pane).is_some(), "the plan stays");
        app.state.assert_invariants_for_test();

        // Still running: a resume round waits for it.
        app.request_suspended_pane_resume(Instant::now());
        app.resume_suspended_panes_if_due(Instant::now());
        handle_events_for(app, Duration::from_millis(300)).await;
        assert!(
            suspension_of(app, pane).is_some(),
            "waits for the old agent"
        );

        HELD_AGENT.store(0, std::sync::atomic::Ordering::Relaxed);
        let deadline = Instant::now() + Duration::from_secs(20);
        while !resumed(app) {
            assert!(Instant::now() < deadline, "the pane never resumed");
            assert!(app.find_pane(pane).is_some(), "the pane stays");
            app.request_suspended_pane_resume(Instant::now());
            app.resume_suspended_panes_if_due(Instant::now());
            handle_events_for(app, Duration::from_millis(100)).await;
        }
        assert!(suspension_of(app, pane).is_none(), "no longer suspended");
        assert_eq!(
            app.state.workspaces[0].terminal_id(pane),
            Some(&terminal_id),
            "the same pane, in place"
        );
        app.state.assert_invariants_for_test();
        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pty_agent_still_running_after_the_kill_suspends_and_resumes_in_place() {
        let agent = StandInAgent::start(STUBBORN_AGENT);
        let (mut app, pane) = pty_pane();
        give_agent(&mut app, pane, &agent);
        suspends_then_resumes_in_place(&mut app, pane, &agent, |app| {
            screen_of(app, pane).contains("sid-301")
        })
        .await;
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_legacy_core_agent_still_running_after_the_kill_suspends_and_resumes_in_place() {
        let agent = StandInAgent::start(STUBBORN_AGENT);
        let mut setup = legacy_pane(Kill::Finished);
        let pane = setup.pane;
        SWEPT_PROGRAM.store(setup.program.id(), std::sync::atomic::Ordering::Relaxed);
        give_agent(&mut setup.app, pane, &agent);
        let primary = &setup.primary;
        suspends_then_resumes_in_place(&mut setup.app, pane, &agent, |_| {
            primary
                .requests()
                .iter()
                .map(|request| request.to_string())
                .chain(
                    primary
                        .received()
                        .iter()
                        .map(|frame| String::from_utf8_lossy(&frame.payload).into_owned()),
                )
                .any(|sent| sent.contains("sid-301"))
        })
        .await;
    }

    // Fork issue 301: the agent is asked to stop before its pane is killed,
    // and has saved its last turn before the move answers, so the resume
    // finds it.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_moved_agent_saves_its_last_turn_before_it_resumes() {
        let record = std::env::temp_dir().join(format!(
            "herdr-move-301-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let agent = StandInAgent::start(&format!(
            "trap '' HUP; trap 'echo last-turn > {}; exit 0' TERM; while :; do sleep 0.1; done",
            record.display()
        ));
        let (mut app, pane) = pty_pane();
        give_agent(&mut app, pane, &agent);

        assert_eq!(app.move_pane(pane, true), Ok(()));
        assert_eq!(
            std::fs::read_to_string(&record).ok().as_deref(),
            Some("last-turn\n"),
            "the agent saved its last turn before the move went on"
        );
        assert!(agent.exited(), "it exited on its own, from the SIGTERM");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !screen_of(&app, pane).contains("sid-301") {
            assert!(Instant::now() < deadline, "the agent never resumed");
            handle_events_for(&mut app, Duration::from_millis(100)).await;
        }
        assert!(app.find_pane(pane).is_some());
        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
        let _ = std::fs::remove_file(record);
    }

    // Fork issue 301: what the agent leaves running below it when it ends
    // on its SIGTERM is ended too, as the hangup of its PTY did before.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_moved_agent_leaves_nothing_running_below_it() {
        let record = std::env::temp_dir().join(format!(
            "herdr-move-301-child-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let agent =
            StandInAgent::start(&format!("sleep 30 & echo $! > {}; wait", record.display()));
        let child: u32 = std::fs::read_to_string(&record)
            .expect("the child's pid")
            .trim()
            .parse()
            .expect("a pid");
        assert!(alive(child));
        let (mut app, pane) = pty_pane();
        give_agent(&mut app, pane, &agent);

        assert_eq!(app.move_pane(pane, true), Ok(()));
        assert!(agent.exited(), "the agent ended on its SIGTERM");
        let deadline = Instant::now() + Duration::from_secs(5);
        while alive(child) {
            assert!(
                Instant::now() < deadline,
                "the agent's child {child} still runs"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
        let _ = std::fs::remove_file(record);
    }

    #[test]
    fn the_agents_tree_is_every_process_below_it() {
        // atuin (10) -> zsh (11) -> claude (12) -> mcp (13) -> helper (14)
        let tree = [
            member(10, 1),
            member(11, 10),
            member(12, 11),
            member(13, 12),
            member(14, 13),
            member(20, 1),
        ];
        let mut below = descendants(12, &tree);
        below.sort_unstable();
        assert_eq!(below, [13, 14]);
        assert_eq!(descendants(14, &tree), Vec::<u32>::new());
    }
}
