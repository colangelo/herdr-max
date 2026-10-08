//! A pane restores as the agent that ran in it at shutdown, never another
//! (fork issue 127).

use std::time::Instant;

use super::TerminalState;
use crate::detect::Agent;
use crate::terminal::TerminalId;

fn pane() -> TerminalState {
    TerminalState::new(TerminalId::alloc(), "/tmp".into())
}

fn report(terminal: &mut TerminalState, agent: &str, session: &str, seq: u64) -> bool {
    terminal
        .set_agent_session_ref_for_session_start(
            format!("herdr:{agent}"),
            agent.into(),
            crate::agent_resume::AgentSessionRef::id(session),
            Some(seq),
            Some("startup".into()),
        )
        .is_some()
}

/// What the snapshot saves for this pane: (agent, session id).
fn saved(terminal: &TerminalState) -> Option<(String, String)> {
    terminal
        .session_for_snapshot()
        .map(|(_, agent, _, value)| (agent, value))
}

fn claude_pane() -> TerminalState {
    let mut terminal = pane();
    terminal.set_detected_agent_process_at(Agent::Claude, Instant::now());
    report(&mut terminal, "claude", "claude-1", 10);
    assert_eq!(saved(&terminal), Some(("claude".into(), "claude-1".into())));
    terminal
}

#[test]
fn codex_after_claude_in_the_same_pane_is_saved_as_codex() {
    let mut terminal = claude_pane();

    terminal.set_detected_agent_process_at(Agent::Codex, Instant::now());
    assert!(report(&mut terminal, "codex", "codex-1", 1));

    assert_eq!(saved(&terminal), Some(("codex".into(), "codex-1".into())));
}

#[test]
fn a_codex_report_that_beats_detection_is_kept_for_codex() {
    let mut terminal = claude_pane();

    // The hook usually reports before the process probe sees codex.
    report(&mut terminal, "codex", "codex-1", 1);
    terminal.set_detected_agent_process_at(Agent::Codex, Instant::now());

    assert_eq!(saved(&terminal), Some(("codex".into(), "codex-1".into())));
}

#[test]
fn codex_without_a_report_never_saves_the_claude_session() {
    let mut terminal = claude_pane();

    terminal.set_detected_agent_process_at(Agent::Codex, Instant::now());

    assert_eq!(
        saved(&terminal),
        None,
        "a codex pane must not restore as claude"
    );
}

#[test]
fn a_claude_hook_authority_does_not_outlive_claude_either() {
    let mut terminal = pane();
    terminal.set_detected_agent_process_at(Agent::Claude, Instant::now());
    terminal.set_hook_authority_with_session_ref(
        "herdr:claude".into(),
        "claude".into(),
        crate::detect::AgentState::Working,
        None,
        crate::agent_resume::AgentSessionRef::id("claude-1"),
        Some(5),
    );
    assert_eq!(saved(&terminal), Some(("claude".into(), "claude-1".into())));

    terminal.set_detected_agent_process_at(Agent::Codex, Instant::now());

    assert_ne!(
        saved(&terminal).map(|(agent, _)| agent).as_deref(),
        Some("claude")
    );
}

#[test]
fn a_pane_back_at_its_shell_keeps_the_last_agent_session() {
    // Unchanged behaviour: no agent running now is not a different agent.
    let mut terminal = claude_pane();
    terminal.set_detected_state(None, crate::detect::AgentState::Unknown);

    assert_eq!(saved(&terminal), Some(("claude".into(), "claude-1".into())));
}

fn launch(argv: &[&str]) -> crate::agent_resume::AgentLaunchArgv {
    crate::agent_resume::AgentLaunchArgv {
        argv: argv.iter().map(|word| word.to_string()).collect(),
        cwd: None,
        started_at_ms: None,
    }
}

const THREAD: &str = "01a0e4f8-16fa-78f1-b2a4-fe126b57b4b2";

#[test]
fn a_codex_resumed_by_hand_is_saved_from_its_command_line() {
    let mut terminal = claude_pane();

    terminal.set_detected_agent_process_at(Agent::Codex, Instant::now());
    terminal.record_agent_launch(
        Agent::Codex,
        Some(&launch(&["codex", "resume", THREAD, "-m", "gpt-6-astra"])),
    );

    assert_eq!(saved(&terminal), Some(("codex".into(), THREAD.into())));
    assert_eq!(
        terminal
            .agent_launch_for_snapshot()
            .map(|record| record.flags.clone()),
        Some(vec!["-m".to_string(), "gpt-6-astra".to_string()])
    );
}

#[test]
fn the_launch_flags_of_the_agent_that_left_are_dropped() {
    let mut terminal = pane();
    terminal.set_detected_agent_process_at(Agent::Claude, Instant::now());
    terminal.record_agent_launch(
        Agent::Claude,
        Some(&launch(&["claude", "--settings", "/u/gpt.json"])),
    );

    terminal.set_detected_agent_process_at(Agent::Codex, Instant::now());

    assert_eq!(terminal.agent_launch_for_snapshot(), None);
}

#[test]
fn a_codex_report_wins_over_the_id_in_its_command_line() {
    let mut terminal = pane();
    terminal.set_detected_agent_process_at(Agent::Codex, Instant::now());
    assert!(report(&mut terminal, "codex", "codex-live", 1));

    terminal.record_agent_launch(Agent::Codex, Some(&launch(&["codex", "resume", THREAD])));

    assert_eq!(
        saved(&terminal),
        Some(("codex".into(), "codex-live".into()))
    );
}

fn preview(terminal: &TerminalState) -> Option<Vec<String>> {
    terminal.restore_plan_preview().map(|plan| plan.argv)
}

#[test]
fn the_preview_of_a_gpt_claude_pane_carries_its_settings_file() {
    let settings = crate::agent_resume::test_settings_file("preview-gpt.json");
    let mut terminal = claude_pane();
    terminal.record_agent_launch(
        Agent::Claude,
        Some(&launch(&[
            "claude",
            "--settings",
            &settings,
            "--model",
            "gpt-6-astra",
        ])),
    );
    terminal.restore_reported_resume(crate::agent_resume::ReportedAgentResume {
        source: "herdr:claude".into(),
        agent: "claude".into(),
        argv: ["claude", "--resume", "claude-1", "--model", "gpt-6-astra"]
            .map(String::from)
            .to_vec(),
    });

    assert_eq!(
        preview(&terminal).unwrap(),
        [
            "claude",
            "--resume",
            "claude-1",
            "--model",
            "gpt-6-astra",
            "--settings",
            &settings
        ]
    );
}

#[test]
fn the_preview_of_a_codex_pane_that_never_reported_is_empty_not_claude() {
    let mut terminal = claude_pane();
    terminal.set_detected_agent_process_at(Agent::Codex, Instant::now());

    assert_eq!(preview(&terminal), None);
}

#[test]
fn the_preview_of_a_hand_started_codex_is_its_resume_with_its_flags() {
    let mut terminal = pane();
    terminal.set_detected_agent_process_at(Agent::Codex, Instant::now());
    terminal.record_agent_launch(
        Agent::Codex,
        Some(&launch(&["codex", "-m", "gpt-6-astra", "-s", "read-only"])),
    );
    assert!(report(&mut terminal, "codex", "codex-1", 1));

    assert_eq!(
        preview(&terminal).unwrap(),
        [
            "codex",
            "resume",
            "codex-1",
            "-m",
            "gpt-6-astra",
            "-s",
            "read-only"
        ]
    );
}

#[test]
fn a_codex_report_after_claude_left_unseen_is_kept_for_codex() {
    // Claude was killed; detection saw it go (no agent now) but recorded no
    // exit, so its session is still on record when codex's hook reports.
    let mut terminal = claude_pane();
    terminal.set_detected_state(None, crate::detect::AgentState::Unknown);

    report(&mut terminal, "codex", "codex-1", 1);
    terminal.set_detected_agent_process_at(Agent::Codex, Instant::now());

    assert_eq!(saved(&terminal), Some(("codex".into(), "codex-1".into())));
}
