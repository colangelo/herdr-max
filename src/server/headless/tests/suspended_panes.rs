use super::*;

/// Fork issue 288, V6: the server's own exit handling must not release the
/// agent of a pane whose keeper ended, or the resume plan is gone before the
/// app keeps the pane.
#[cfg(unix)]
#[tokio::test]
async fn a_suspended_agent_pane_keeps_its_resume_plan_through_the_server() {
    let mut server = test_headless_server();
    let workspace = crate::workspace::Workspace::test_new("agent");
    let pane_id = workspace.tabs[0].root_pane;
    let terminal_id = workspace.terminal_id(pane_id).cloned().unwrap();
    server.app.state.workspaces = vec![workspace];
    server.app.state.ensure_test_terminals();
    server.app.state.active = Some(0);
    let terminal = server.app.state.terminals.get_mut(&terminal_id).unwrap();
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
        argv: vec!["herdr-suspend-288".into(), "--resume".into(), "sid".into()],
    });

    server.handle_internal_event_with_forwarding(AppEvent::PaneDied {
        pane_id,
        exit_reason: crate::platform::ChildExitReason::Suspended(
            crate::platform::SuspendCause::PaneJobEnded,
        ),
    });

    assert!(server.app.state.terminals[&terminal_id].is_suspended());
    assert!(
        server.app.pane_agent_resume_plan(pane_id).is_some(),
        "the suspended agent keeps the plan it resumes by"
    );
}
