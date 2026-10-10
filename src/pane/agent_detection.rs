use std::sync::atomic::{AtomicU64, Ordering};

use crate::detect::{Agent, AgentDetection, AgentState, BlockedReason};

pub(super) const AGENT_PENDING_IDLE_RECHECK: std::time::Duration =
    std::time::Duration::from_millis(100);
const AGENT_PENDING_IDLE_CONFIRMATIONS: u8 = 3;
pub(super) const AGENT_PENDING_IDLE_CAP: std::time::Duration =
    std::time::Duration::from_millis(700);
pub(super) const STABLE_VISIBLE_SIGNAL_REFRESH: std::time::Duration =
    std::time::Duration::from_millis(800);
pub(super) const AGENT_STARTUP_GRACE_WINDOW: std::time::Duration =
    std::time::Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DetectionPublishState {
    pub(super) state: AgentState,
    pub(super) visible_idle: bool,
    pub(super) visible_blocker: bool,
    pub(super) visible_working: bool,
    pub(super) background_work: bool,
    pub(super) blocked_reason: Option<BlockedReason>,
}

/// Initial detection-task variables derived from a handoff agent seed, so the
/// task resumes as if it had already published the pre-handoff state instead of
/// re-identifying the surviving agent (which would publish Idle over it).
#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BasicDetectionSeedInit {
    pub(super) agent: Option<Agent>,
    pub(super) state: AgentState,
    pub(super) last_visible_idle: bool,
    pub(super) last_visible_blocker: bool,
    pub(super) last_visible_working: bool,
    pub(super) last_background_work: bool,
}

#[cfg(unix)]
impl BasicDetectionSeedInit {
    pub(super) fn from_agent_seed(seed: Option<(Agent, AgentState)>) -> Self {
        let Some((agent, state)) = seed else {
            return Self {
                agent: None,
                state: AgentState::Unknown,
                last_visible_idle: false,
                last_visible_blocker: false,
                last_visible_working: false,
                last_background_work: false,
            };
        };
        Self {
            agent: Some(agent),
            state,
            last_visible_idle: state == AgentState::Idle,
            last_visible_blocker: state == AgentState::Blocked,
            last_visible_working: state == AgentState::Working,
            // The handoff seed carries only (agent, state), not the source
            // detection's background_work flag, so a background-working
            // process resumes needing one real detection pass to republish.
            last_background_work: false,
        }
    }
}

/// While a handoff-seeded pane's screen is still blank (the re-adopted agent
/// has not repainted into the new server's grid), screen detection has no
/// evidence: the known-agent fallback would read blankness as plain Idle and
/// overwrite the seeded state. Hold until real content arrives.
#[cfg(unix)]
pub(super) fn should_hold_seeded_detection(seeded_hold_active: bool, content: &str) -> bool {
    seeded_hold_active && content.trim().is_empty()
}

#[derive(Debug, Default)]
pub(super) struct PendingIdleConfirmation {
    started_at: Option<std::time::Instant>,
    confirmations: u8,
}

impl PendingIdleConfirmation {
    pub(super) fn active(&self) -> bool {
        self.started_at.is_some()
    }

    pub(super) fn clear(&mut self) {
        self.started_at = None;
        self.confirmations = 0;
    }

    pub(super) fn should_hold_working_to_idle(
        &mut self,
        previous: DetectionPublishState,
        next: DetectionPublishState,
        agent_changed: bool,
        process_exited: bool,
        now: std::time::Instant,
    ) -> bool {
        let is_working_to_plain_idle = previous.state == AgentState::Working
            && next.state == AgentState::Idle
            && !next.visible_idle
            && !next.visible_blocker
            && !agent_changed
            && !process_exited;

        if !is_working_to_plain_idle {
            self.clear();
            return false;
        }

        let Some(started_at) = self.started_at else {
            self.started_at = Some(now);
            self.confirmations = 0;
            return true;
        };

        if now.duration_since(started_at) >= AGENT_PENDING_IDLE_CAP {
            self.clear();
            return false;
        }

        self.confirmations = self.confirmations.saturating_add(1);
        if self.confirmations >= AGENT_PENDING_IDLE_CONFIRMATIONS {
            self.clear();
            return false;
        }

        true
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct IdleScreenScanSkipInput {
    pub(super) state: AgentState,
    pub(super) agent: Option<Agent>,
    pub(super) pending_idle_active: bool,
    pub(super) agent_changed: bool,
    pub(super) process_exited: bool,
    pub(super) current_detection_content_seq: Option<u64>,
    pub(super) last_screen_scan_detection_content_seq: Option<u64>,
}

pub(super) fn should_skip_idle_screen_scan(input: IdleScreenScanSkipInput) -> bool {
    let stable_state = input.state == AgentState::Idle
        || (input.state == AgentState::Unknown && input.agent == Some(Agent::Codex));
    if !stable_state
        || input.agent.is_none()
        || input.pending_idle_active
        || input.agent_changed
        || input.process_exited
    {
        return false;
    }

    input.current_detection_content_seq.is_some()
        && input.last_screen_scan_detection_content_seq == input.current_detection_content_seq
}

/// Retains text, not a classification: callers still evaluate discovery and
/// lifecycle bookkeeping on every scheduled observation.
#[derive(Default)]
pub(super) struct DetectionTextCache {
    pub(super) text: String,
    revision: Option<u64>,
}

impl DetectionTextCache {
    pub(super) fn clear(&mut self) {
        self.text.clear();
        self.revision = None;
    }

    /// Returns whether the extracted text changed, not whether PTY bytes arrived.
    pub(super) fn refresh(
        &mut self,
        agent: Option<Agent>,
        content_seq: &AtomicU64,
        read: impl FnOnce() -> (String, bool),
    ) -> bool {
        let before = content_seq.load(Ordering::Acquire);
        // Identified agents retain their existing screen-read behavior everywhere.
        if agent.is_none()
            // The terminal accessor also returns empty on failure; keep retrying it.
            && !self.text.is_empty()
            && before.is_multiple_of(2)
            && self.revision == Some(before)
        {
            return false;
        }
        let (text, reusable) = read();
        let after = content_seq.load(Ordering::Acquire);
        // Writers announce themselves before touching the terminal. A read that
        // overlaps a writer must not bless either revision for future reuse.
        self.revision = (reusable && before == after && before.is_multiple_of(2)).then_some(before);
        let changed = text != self.text;
        self.text = text;
        changed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DetectionScreenReadDecision {
    Read,
    Skip,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct DetectionScreenReadInput {
    pub(super) state: AgentState,
    pub(super) agent: Option<Agent>,
    pub(super) pending_idle_active: bool,
    pub(super) agent_changed: bool,
    pub(super) process_exited: bool,
    pub(super) current_detection_content_seq: Option<u64>,
    pub(super) last_screen_scan_detection_content_seq: Option<u64>,
}

pub(super) fn decide_detection_screen_read(
    input: DetectionScreenReadInput,
) -> DetectionScreenReadDecision {
    if should_skip_idle_screen_scan(IdleScreenScanSkipInput {
        state: input.state,
        agent: input.agent,
        pending_idle_active: input.pending_idle_active,
        agent_changed: input.agent_changed,
        process_exited: input.process_exited,
        current_detection_content_seq: input.current_detection_content_seq,
        last_screen_scan_detection_content_seq: input.last_screen_scan_detection_content_seq,
    }) {
        DetectionScreenReadDecision::Skip
    } else {
        DetectionScreenReadDecision::Read
    }
}

pub(super) fn should_publish_detection_update(
    previous: DetectionPublishState,
    next: DetectionPublishState,
    agent_changed: bool,
    process_exited: bool,
    stable_visible_signal_refresh_due: bool,
) -> bool {
    next.state != previous.state
        || next.visible_idle != previous.visible_idle
        || next.visible_blocker != previous.visible_blocker
        || next.visible_working != previous.visible_working
        || next.background_work != previous.background_work
        || next.blocked_reason != previous.blocked_reason
        || agent_changed
        || process_exited
        || (stable_visible_signal_refresh_due && next.visible_blocker && previous.visible_blocker)
}

pub(super) fn stable_visible_signal_refresh_due(
    previous: DetectionPublishState,
    next: DetectionPublishState,
    last_refresh: Option<std::time::Instant>,
    now: std::time::Instant,
) -> bool {
    let stable_visible_signal = next.visible_blocker && previous.visible_blocker;

    stable_visible_signal
        && last_refresh.is_none_or(|last_refresh| {
            now.duration_since(last_refresh) >= STABLE_VISIBLE_SIGNAL_REFRESH
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DetectionTransitionDecision {
    NoPublish,
    PublishNext,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct DetectionTransitionInput {
    pub(super) previous_publish: DetectionPublishState,
    pub(super) next_publish: DetectionPublishState,
    pub(super) agent_changed: bool,
    pub(super) process_exited: bool,
    pub(super) stable_refresh_due: bool,
    pub(super) now: std::time::Instant,
}

pub(super) fn decide_detection_transition(
    input: DetectionTransitionInput,
    pending_idle: &mut PendingIdleConfirmation,
) -> DetectionTransitionDecision {
    if pending_idle.should_hold_working_to_idle(
        input.previous_publish,
        input.next_publish,
        input.agent_changed,
        input.process_exited,
        input.now,
    ) {
        return DetectionTransitionDecision::NoPublish;
    }

    if should_publish_detection_update(
        input.previous_publish,
        input.next_publish,
        input.agent_changed,
        input.process_exited,
        input.stable_refresh_due,
    ) {
        return DetectionTransitionDecision::PublishNext;
    }

    DetectionTransitionDecision::NoPublish
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DetectionPublishDecision {
    NoPublish,
    Publish {
        state: AgentState,
        visible_idle: bool,
        visible_blocker: bool,
        visible_working: bool,
        background_work: bool,
        blocked_reason: Option<BlockedReason>,
        process_exited: bool,
    },
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ScreenDetectionPublishInput {
    pub(super) current_state: AgentState,
    pub(super) last_visible_idle: bool,
    pub(super) last_visible_blocker: bool,
    pub(super) last_visible_working: bool,
    pub(super) last_background_work: bool,
    pub(super) last_blocked_reason: Option<BlockedReason>,
    pub(super) last_visible_signal_refresh: Option<std::time::Instant>,
    pub(super) screen_detection: AgentDetection,
    pub(super) process_exited: bool,
    pub(super) agent_changed: bool,
    pub(super) now: std::time::Instant,
}

pub(super) fn decide_screen_detection_publish(
    input: ScreenDetectionPublishInput,
    pending_idle: &mut PendingIdleConfirmation,
) -> DetectionPublishDecision {
    let detection = input.screen_detection;
    let new_state = crate::terminal::state::stabilize_agent_detection(detection);
    let visible_idle = detection.visible_idle && new_state == AgentState::Idle;
    let visible_blocker = detection.visible_blocker && new_state == AgentState::Blocked;
    let visible_working = detection.visible_working && new_state == AgentState::Working;
    let background_work = detection.background_work && new_state == AgentState::Working;
    let blocked_reason = detection
        .blocked_reason
        .filter(|_| new_state == AgentState::Blocked);

    let previous_publish = DetectionPublishState {
        state: input.current_state,
        visible_idle: input.last_visible_idle,
        visible_blocker: input.last_visible_blocker,
        visible_working: input.last_visible_working,
        background_work: input.last_background_work,
        blocked_reason: input.last_blocked_reason,
    };
    let next_publish = DetectionPublishState {
        state: new_state,
        visible_idle,
        visible_blocker,
        visible_working,
        background_work,
        blocked_reason,
    };
    let stable_refresh_due = stable_visible_signal_refresh_due(
        previous_publish,
        next_publish,
        input.last_visible_signal_refresh,
        input.now,
    );

    match decide_detection_transition(
        DetectionTransitionInput {
            previous_publish,
            next_publish,
            agent_changed: input.agent_changed,
            process_exited: input.process_exited,
            stable_refresh_due,
            now: input.now,
        },
        pending_idle,
    ) {
        DetectionTransitionDecision::NoPublish => DetectionPublishDecision::NoPublish,
        DetectionTransitionDecision::PublishNext => DetectionPublishDecision::Publish {
            state: new_state,
            visible_idle,
            visible_blocker,
            visible_working,
            background_work,
            blocked_reason,
            process_exited: input.process_exited,
        },
    }
}

#[allow(dead_code)] // shim for tests; detection_update_for_publish_with_osc is the real path
pub(super) fn detection_update_for_publish(
    agent: Option<Agent>,
    content: &str,
    process_exited: bool,
) -> Option<crate::detect::AgentDetection> {
    detection_update_for_publish_with_osc(agent, content, "", "", process_exited)
}

pub(super) fn detection_update_for_publish_with_osc(
    agent: Option<Agent>,
    content: &str,
    osc_title: &str,
    osc_progress: &str,
    process_exited: bool,
) -> Option<crate::detect::AgentDetection> {
    if process_exited {
        return Some(crate::detect::AgentDetection {
            state: AgentState::Idle,
            skip_state_update: false,
            visible_idle: true,
            visible_blocker: false,
            visible_working: false,
            background_work: false,
            blocked_reason: None,
        });
    }

    let detection = crate::detect::detect_agent_with_osc(agent, content, osc_title, osc_progress);
    (!detection.skip_state_update).then_some(detection)
}

pub(super) fn codex_prompt_ready(content: &str) -> bool {
    // The composer can remain visible during a turn; this is startup evidence only.
    let recent: String = content
        .lines()
        .rev()
        .take(12)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .flat_map(str::chars)
        .filter(|ch| !ch.is_whitespace())
        .collect();
    recent.contains("›AskCodextodoanything")
        && !recent.contains("model:loading")
        && !recent.contains("Resumingsession")
}

pub(super) fn observe_detection_content_change(bytes: &[u8], detection_content_seq: &AtomicU64) {
    if !bytes.is_empty() {
        detection_content_seq.fetch_add(1, Ordering::Relaxed);
    }
}

pub(super) fn mark_detection_content_changed(detection_content_seq: &AtomicU64) {
    detection_content_seq.fetch_add(1, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unidentified_text_cache_preserves_text_changes_and_forced_reads() {
        let sequence = AtomicU64::new(0);
        let mut cache = DetectionTextCache::default();
        assert!(cache.refresh(None, &sequence, || ("one".into(), true)));
        for _ in 0..20 {
            assert!(!cache.refresh(None, &sequence, || panic!("unchanged text was extracted")));
        }
        sequence.store(2, Ordering::Release);
        assert!(
            !cache.refresh(None, &sequence, || ("one".into(), true)),
            "bytes are not text changes"
        );
        sequence.store(4, Ordering::Release);
        assert!(cache.refresh(None, &sequence, || ("two".into(), true)));
        for _ in 0..2 {
            let mut read = false;
            assert!(!cache.refresh(Some(Agent::Codex), &sequence, || {
                read = true;
                ("two".into(), true)
            }));
            assert!(read, "identified agents must keep their existing reads");
        }
        cache.clear();
        assert!(cache.refresh(None, &sequence, || ("two".into(), true)));
        sequence.store(6, Ordering::Release);
        assert!(cache.refresh(None, &sequence, || (String::new(), false)));
        assert!(cache.refresh(None, &sequence, || ("retry after empty read".into(), true)));
    }

    #[test]
    fn unidentified_text_cache_does_not_reuse_overlapping_writes() {
        let sequence = AtomicU64::new(0);
        let mut cache = DetectionTextCache::default();
        assert!(cache.refresh(None, &sequence, || {
            sequence.store(2, Ordering::Release);
            ("old".into(), true)
        }));
        assert_eq!(
            cache.revision, None,
            "old text must not acquire the new revision"
        );
        assert!(cache.refresh(None, &sequence, || ("new".into(), true)));

        sequence.store(3, Ordering::Release);
        assert!(cache.refresh(None, &sequence, || ("during write".into(), true)));
        assert_eq!(
            cache.revision, None,
            "an in-progress write cannot be reused"
        );
        sequence.store(4, Ordering::Release);
        assert!(cache.refresh(None, &sequence, || ("completed write".into(), true)));
        assert!(!cache.refresh(None, &sequence, || panic!(
            "completed revision should be reusable"
        )));
    }

    fn publish_state(state: AgentState) -> DetectionPublishState {
        DetectionPublishState {
            state,
            visible_idle: false,
            visible_blocker: false,
            visible_working: false,
            background_work: false,
            blocked_reason: None,
        }
    }

    fn screen_detection(state: AgentState) -> AgentDetection {
        AgentDetection {
            state,
            skip_state_update: false,
            visible_idle: state == AgentState::Idle,
            visible_blocker: false,
            visible_working: state == AgentState::Working,
            background_work: false,
            blocked_reason: (state == AgentState::Blocked).then_some(BlockedReason::Other),
        }
    }

    fn screen_publish_input(
        current_state: AgentState,
        screen_detection: AgentDetection,
        now: std::time::Instant,
    ) -> ScreenDetectionPublishInput {
        ScreenDetectionPublishInput {
            current_state,
            last_visible_idle: false,
            last_visible_blocker: false,
            last_visible_working: false,
            last_background_work: false,
            last_blocked_reason: None,
            last_visible_signal_refresh: None,
            screen_detection,
            process_exited: false,
            agent_changed: false,
            now,
        }
    }

    fn screen_read_input(state: AgentState, current_seq: u64) -> DetectionScreenReadInput {
        DetectionScreenReadInput {
            state,
            agent: Some(Agent::Codex),
            pending_idle_active: false,
            agent_changed: false,
            process_exited: false,
            current_detection_content_seq: Some(current_seq),
            last_screen_scan_detection_content_seq: Some(10),
        }
    }

    #[test]
    fn codex_startup_prompt_survives_terminal_wraps() {
        let wrapped = "header\n› Ask Codex to do\nanything\nfooter";
        assert!(codex_prompt_ready(wrapped));
        assert!(!codex_prompt_ready(&format!("model: load\ning\n{wrapped}")));
    }

    #[test]
    fn screen_read_skips_unchanged_idle_bottom_buffer() {
        assert_eq!(
            decide_detection_screen_read(screen_read_input(AgentState::Idle, 10)),
            DetectionScreenReadDecision::Skip
        );
    }

    #[test]
    fn screen_read_skips_unchanged_ambiguous_codex_but_not_new_content_or_replacement() {
        let mut input = screen_read_input(AgentState::Unknown, 10);
        assert_eq!(
            decide_detection_screen_read(input),
            DetectionScreenReadDecision::Skip
        );
        input.current_detection_content_seq = Some(11);
        assert_eq!(
            decide_detection_screen_read(input),
            DetectionScreenReadDecision::Read
        );
        input.current_detection_content_seq = Some(10);
        input.agent_changed = true;
        assert_eq!(
            decide_detection_screen_read(input),
            DetectionScreenReadDecision::Read
        );
        input.agent_changed = false;
        input.agent = Some(Agent::Pi);
        assert_eq!(
            decide_detection_screen_read(input),
            DetectionScreenReadDecision::Read
        );
    }

    #[test]
    fn screen_read_reads_when_idle_bottom_buffer_changes() {
        assert_eq!(
            decide_detection_screen_read(screen_read_input(AgentState::Idle, 11)),
            DetectionScreenReadDecision::Read
        );
    }

    #[test]
    fn screen_read_reads_for_transitions_and_missing_agent() {
        let mut input = screen_read_input(AgentState::Idle, 10);
        input.pending_idle_active = true;
        assert_eq!(
            decide_detection_screen_read(input),
            DetectionScreenReadDecision::Read
        );

        let mut input = screen_read_input(AgentState::Idle, 10);
        input.agent_changed = true;
        assert_eq!(
            decide_detection_screen_read(input),
            DetectionScreenReadDecision::Read
        );

        let mut input = screen_read_input(AgentState::Idle, 10);
        input.process_exited = true;
        assert_eq!(
            decide_detection_screen_read(input),
            DetectionScreenReadDecision::Read
        );

        let mut input = screen_read_input(AgentState::Idle, 10);
        input.agent = None;
        assert_eq!(
            decide_detection_screen_read(input),
            DetectionScreenReadDecision::Read
        );
    }

    #[test]
    fn pending_idle_holds_working_to_plain_idle_until_confirmed() {
        let now = std::time::Instant::now();
        let previous = publish_state(AgentState::Working);
        let next = publish_state(AgentState::Idle);
        let mut pending = PendingIdleConfirmation::default();

        assert!(pending.should_hold_working_to_idle(previous, next, false, false, now));
        assert!(pending.should_hold_working_to_idle(
            previous,
            next,
            false,
            false,
            now + AGENT_PENDING_IDLE_RECHECK
        ));
        assert!(pending.should_hold_working_to_idle(
            previous,
            next,
            false,
            false,
            now + AGENT_PENDING_IDLE_RECHECK * 2
        ));
        assert!(!pending.should_hold_working_to_idle(
            previous,
            next,
            false,
            false,
            now + AGENT_PENDING_IDLE_RECHECK * 3
        ));
    }

    #[test]
    fn visible_idle_bypasses_plain_idle_hold() {
        let now = std::time::Instant::now();
        let previous = publish_state(AgentState::Working);
        let mut next = publish_state(AgentState::Idle);
        next.visible_idle = true;
        let mut pending = PendingIdleConfirmation::default();

        assert!(!pending.should_hold_working_to_idle(previous, next, false, false, now));
    }

    #[test]
    fn background_work_starting_while_working_publishes() {
        let previous = publish_state(AgentState::Working);
        let mut next = publish_state(AgentState::Working);
        next.background_work = true;

        assert!(should_publish_detection_update(
            previous, next, false, false, false
        ));
    }

    #[test]
    fn background_work_stopping_while_working_publishes() {
        let mut previous = publish_state(AgentState::Working);
        previous.background_work = true;
        let next = publish_state(AgentState::Working);

        assert!(should_publish_detection_update(
            previous, next, false, false, false
        ));
    }

    #[test]
    fn transition_decision_publishes_next_for_visible_blocker() {
        let now = std::time::Instant::now();
        let mut pending_idle = PendingIdleConfirmation::default();
        let mut blocked = publish_state(AgentState::Blocked);
        blocked.visible_blocker = true;

        assert_eq!(
            decide_detection_transition(
                DetectionTransitionInput {
                    previous_publish: publish_state(AgentState::Idle),
                    next_publish: blocked,
                    agent_changed: false,
                    process_exited: false,
                    stable_refresh_due: false,
                    now,
                },
                &mut pending_idle,
            ),
            DetectionTransitionDecision::PublishNext
        );
    }

    #[test]
    fn screen_publish_keeps_visible_working_without_pty_activity() {
        let now = std::time::Instant::now();
        let mut pending_idle = PendingIdleConfirmation::default();

        assert_eq!(
            decide_screen_detection_publish(
                screen_publish_input(AgentState::Idle, screen_detection(AgentState::Working), now,),
                &mut pending_idle,
            ),
            DetectionPublishDecision::Publish {
                state: AgentState::Working,
                visible_idle: false,
                visible_blocker: false,
                visible_working: true,
                background_work: false,
                blocked_reason: None,
                process_exited: false,
            }
        );
    }

    #[test]
    fn screen_publish_can_publish_idle_without_input_taint_delay() {
        let now = std::time::Instant::now();
        let mut pending_idle = PendingIdleConfirmation::default();

        assert_eq!(
            decide_screen_detection_publish(
                screen_publish_input(AgentState::Blocked, screen_detection(AgentState::Idle), now,),
                &mut pending_idle,
            ),
            DetectionPublishDecision::Publish {
                state: AgentState::Idle,
                visible_idle: true,
                visible_blocker: false,
                visible_working: false,
                background_work: false,
                blocked_reason: None,
                process_exited: false,
            }
        );
    }

    // Fork issue 137: a blocked pane whose reason changes (a permission prompt
    // replaced by a question dialog) republishes so the reason stays current.
    #[test]
    fn screen_publish_republishes_when_only_the_blocked_reason_changes() {
        let now = std::time::Instant::now();
        let mut pending_idle = PendingIdleConfirmation::default();
        let mut detection = screen_detection(AgentState::Blocked);
        detection.blocked_reason = Some(BlockedReason::Question);
        let mut input = screen_publish_input(AgentState::Blocked, detection, now);
        input.last_blocked_reason = Some(BlockedReason::Permission);

        assert_eq!(
            decide_screen_detection_publish(input, &mut pending_idle),
            DetectionPublishDecision::Publish {
                state: AgentState::Blocked,
                visible_idle: false,
                visible_blocker: false,
                visible_working: false,
                background_work: false,
                blocked_reason: Some(BlockedReason::Question),
                process_exited: false,
            }
        );

        input.last_blocked_reason = Some(BlockedReason::Question);
        assert_eq!(
            decide_screen_detection_publish(input, &mut pending_idle),
            DetectionPublishDecision::NoPublish
        );
    }

    #[test]
    fn detection_content_change_tracks_raw_nonempty_reads_for_scan_scheduling() {
        let seq = AtomicU64::new(0);

        observe_detection_content_change(b"", &seq);
        assert_eq!(seq.load(Ordering::Relaxed), 0);

        observe_detection_content_change(b"\x1b[?2026h", &seq);
        assert_eq!(seq.load(Ordering::Relaxed), 1);

        observe_detection_content_change(b"body bytes", &seq);
        assert_eq!(seq.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn local_terminal_mutations_can_invalidate_idle_scan_skip() {
        let seq = AtomicU64::new(0);

        mark_detection_content_changed(&seq);

        assert_eq!(seq.load(Ordering::Relaxed), 1);
    }

    #[cfg(unix)]
    #[test]
    fn seed_init_without_seed_matches_cold_start() {
        assert_eq!(
            BasicDetectionSeedInit::from_agent_seed(None),
            BasicDetectionSeedInit {
                agent: None,
                state: AgentState::Unknown,
                last_visible_idle: false,
                last_visible_blocker: false,
                last_visible_working: false,
                last_background_work: false,
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn seed_init_mirrors_seeded_state_as_last_published() {
        assert_eq!(
            BasicDetectionSeedInit::from_agent_seed(Some((Agent::Claude, AgentState::Working))),
            BasicDetectionSeedInit {
                agent: Some(Agent::Claude),
                state: AgentState::Working,
                last_visible_idle: false,
                last_visible_blocker: false,
                last_visible_working: true,
                last_background_work: false,
            }
        );
        assert_eq!(
            BasicDetectionSeedInit::from_agent_seed(Some((Agent::Codex, AgentState::Blocked))),
            BasicDetectionSeedInit {
                agent: Some(Agent::Codex),
                state: AgentState::Blocked,
                last_visible_idle: false,
                last_visible_blocker: true,
                last_visible_working: false,
                last_background_work: false,
            }
        );
        assert_eq!(
            BasicDetectionSeedInit::from_agent_seed(Some((Agent::Pi, AgentState::Idle))),
            BasicDetectionSeedInit {
                agent: Some(Agent::Pi),
                state: AgentState::Idle,
                last_visible_idle: true,
                last_visible_blocker: false,
                last_visible_working: false,
                last_background_work: false,
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn seeded_hold_blocks_only_blank_screens() {
        // A blank grid means the re-adopted agent has not repainted yet; the
        // seeded state must survive until real content arrives.
        assert!(should_hold_seeded_detection(true, ""));
        assert!(should_hold_seeded_detection(true, " \n\t \n"));
        assert!(!should_hold_seeded_detection(true, "esc to interrupt"));
        assert!(!should_hold_seeded_detection(false, ""));
    }
}
