//! The program status reporter (openspec/changes/core-backend, D9, roundtable
//! R2 and R4): a program's own report becomes the hook report and hint a hook
//! would send.

use super::runtime_io::{ProgramState, ProgramStatus};
use super::*;

fn status(app: &str, state: ProgramState, kind: Option<&str>) -> ProgramStatus {
    ProgramStatus {
        app: Some(app.into()),
        state,
        kind: kind.map(str::to_string),
        message: None,
    }
}

/// Runs the reporter the way core does: from a plain thread, not a runtime one.
fn report_all(statuses: Vec<ProgramStatus>) -> Vec<AppEvent> {
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let (events, mut rx) = mpsc::channel(64);
    let reporter = {
        let _guard = runtime.enter();
        program_status_reporter(PaneId::from_raw(7), events)
    };
    std::thread::spawn(move || {
        for status in statuses {
            reporter(status);
        }
    })
    .join()
    .expect("reporter thread");
    let mut seen = Vec::new();
    while let Ok(event) = rx.try_recv() {
        seen.push(event);
    }
    seen
}

fn hook(event: &AppEvent) -> Option<(&str, &str, AgentState, Option<u64>)> {
    match event {
        AppEvent::HookStateReported {
            source,
            agent_label,
            state,
            seq,
            ..
        } => Some((source.as_str(), agent_label.as_str(), *state, *seq)),
        _ => None,
    }
}

#[test]
fn a_report_becomes_a_hook_report_from_herdr_core() {
    let events = report_all(vec![
        status("claude-code", ProgramState::Working, None),
        status("claude-code", ProgramState::Done, None),
    ]);
    let hooks: Vec<_> = events.iter().filter_map(hook).collect();
    assert_eq!(hooks.len(), 2);
    assert_eq!(
        (hooks[0].0, hooks[0].1, hooks[0].2),
        ("herdr:core", "claude", AgentState::Working)
    );
    assert_eq!(
        hooks[1].2,
        AgentState::Idle,
        "done is idle; the unseen mark comes from the transition"
    );
}

#[test]
fn a_blocked_report_carries_its_reason_as_a_hint() {
    let events = report_all(vec![status(
        "codex",
        ProgramState::Blocked,
        Some("permission"),
    )]);
    assert_eq!(hook(&events[0]).map(|h| h.2), Some(AgentState::Blocked));
    match &events[1] {
        AppEvent::AgentHintReported { report, .. } => {
            assert_eq!(report.kind, Some(crate::detect::BlockedReason::Permission));
            assert_eq!(report.source, "herdr:core");
        }
        other => panic!("expected a hint, got {other:?}"),
    }
}

#[test]
fn a_program_herdr_does_not_know_is_left_to_detection() {
    assert!(report_all(vec![status("some-new-agent", ProgramState::Working, None)]).is_empty());
}

#[test]
fn seq_rises_from_a_wall_clock_base() {
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;
    let events = report_all(vec![
        status("pi", ProgramState::Working, None),
        status("pi", ProgramState::Idle, None),
        status("pi", ProgramState::Working, None),
    ]);
    let seqs: Vec<u64> = events.iter().filter_map(hook).filter_map(|h| h.3).collect();
    assert_eq!(seqs.len(), 3);
    assert!(
        seqs[0] >= before,
        "a new server's first seq beats any carried one"
    );
    assert!(seqs.windows(2).all(|pair| pair[1] > pair[0]), "{seqs:?}");
}
