//! Agent hints (fork issues 137, 157): a source tells herdr that an agent waits
//! on the user (`question` or `permission`) and, later, that it does not. A hint
//! is evidence for the effective state, never a state of its own: it can only
//! raise `Blocked` with a reason. Runtime only: never saved, never carried
//! through a live handoff.

use std::time::{Duration, Instant};

use super::{TerminalState, TerminalStateMutation};
use crate::detect::BlockedReason;

/// How long the screen must read something other than blocked before a
/// `permission` hint is dropped, once the screen has shown the dialog. A mod
/// cannot see a permission prompt being answered (only a deny, an Esc or the
/// tool's end), so the screen clears it.
///
/// Short on purpose (about two detection polls): the first proof run showed the
/// screen alone leaves blocked within about 0.4 s of an allow, and a longer
/// grace made the hint hold the pane blocked 1.4 s after that.
pub(crate) const PERMISSION_HINT_SCREEN_GRACE: Duration = Duration::from_millis(500);

/// The same, while the screen has not yet shown the dialog the hint announced:
/// the mod reports a few milliseconds before the dialog is drawn and detection
/// polls, so a slow scan must not drop a live hint.
pub(crate) const PERMISSION_HINT_UNBACKED_GRACE: Duration = Duration::from_secs(4);

/// A source's current claim that its agent waits on the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AgentHint {
    pub(super) kind: BlockedReason,
    pub(super) id: Option<String>,
    pub(super) source: String,
    pub(super) agent_label: String,
    pub(super) expires_at: Instant,
    /// Since when the screen has read something other than blocked, while the
    /// hint is a `permission` one.
    pub(super) screen_unblocked_since: Option<Instant>,
    /// The screen has read blocked since the hint arrived, so it has shown the
    /// dialog and may be trusted to show its end.
    pub(super) screen_backed: bool,
}

impl AgentHint {
    fn screen_grace(&self) -> Duration {
        if self.screen_backed {
            PERMISSION_HINT_SCREEN_GRACE
        } else {
            PERMISSION_HINT_UNBACKED_GRACE
        }
    }
}

/// A hint report, or with `kind: None` the end of the source's hint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentHintReport {
    pub source: String,
    pub agent_label: String,
    pub kind: Option<BlockedReason>,
    pub id: Option<String>,
    pub ttl: Duration,
    pub seq: Option<u64>,
}

impl TerminalState {
    /// Apply a hint report. `None` when it changed nothing: a stale `seq`, a
    /// second source's report while another's hint is live, a clear with no
    /// hint to clear, or a heartbeat that restates the live hint.
    pub fn set_agent_hint_at(
        &mut self,
        report: AgentHintReport,
        now: Instant,
    ) -> Option<TerminalStateMutation> {
        if let Some(seq) = report.seq {
            let last = self.agent_hint_sequences.get(&report.source).copied();
            if last.is_some_and(|last| seq <= last) {
                return None;
            }
            self.agent_hint_sequences.insert(report.source.clone(), seq);
        }
        // One source at a time: the first live hint owns the terminal until it
        // clears or ages out.
        if let Some(live) = self.agent_hint.as_ref() {
            if live.source != report.source && live.expires_at > now {
                return None;
            }
        }
        let Some(kind) = report.kind else {
            if self
                .agent_hint
                .as_ref()
                .is_none_or(|live| live.source != report.source)
            {
                return None;
            }
            return Some(self.change_agent_hint(None, now));
        };
        let expires_at = now + report.ttl;
        let restates = self.agent_hint.as_ref().is_some_and(|live| {
            live.source == report.source
                && live.kind == kind
                && live.id == report.id
                && live.agent_label == report.agent_label
        });
        if restates {
            if let Some(live) = self.agent_hint.as_mut() {
                live.expires_at = expires_at;
            }
            return None;
        }
        Some(self.change_agent_hint(
            Some(super::hint::AgentHint {
                kind,
                id: report.id,
                source: report.source,
                agent_label: report.agent_label,
                expires_at,
                screen_unblocked_since: None,
                screen_backed: false,
            }),
            now,
        ))
    }

    /// The loop's tick: drop a hint whose time is up, or a `permission` hint
    /// the screen has not backed for the grace. `None` when nothing was due.
    pub fn expire_agent_hint_at(&mut self, now: Instant) -> Option<TerminalStateMutation> {
        let due = self.agent_hint.as_ref().is_some_and(|hint| {
            hint.expires_at <= now
                || hint
                    .screen_unblocked_since
                    .is_some_and(|since| since + hint.screen_grace() <= now)
        });
        due.then(|| self.change_agent_hint(None, now))
    }

    /// When the live hint next needs the loop's attention, for the wake-up
    /// list. `None` without a hint.
    pub fn agent_hint_deadline(&self) -> Option<Instant> {
        let hint = self.agent_hint.as_ref()?;
        let screen = hint
            .screen_unblocked_since
            .map(|since| since + hint.screen_grace());
        Some(screen.map_or(hint.expires_at, |screen| screen.min(hint.expires_at)))
    }

    /// The kind of the live hint that applies now: fresh, and for the agent
    /// that is in this terminal.
    pub(super) fn live_agent_hint_kind(&self, now: Instant) -> Option<BlockedReason> {
        let hint = self.agent_hint.as_ref()?;
        (hint.expires_at > now && self.effective_agent_label() == Some(hint.agent_label.as_str()))
            .then_some(hint.kind)
    }

    /// Note what the screen says, for the permission grace and for dropping a
    /// hint of an agent that is gone. Runs inside the state recompute.
    pub(super) fn note_agent_hint_screen(&mut self, now: Instant) {
        let screen_blocked = self.fallback_state == crate::detect::AgentState::Blocked;
        let other_agent = self.agent_hint.as_ref().is_some_and(|hint| {
            self.effective_agent_label()
                .is_some_and(|label| label != hint.agent_label)
        });
        if other_agent {
            self.agent_hint = None;
            return;
        }
        if let Some(hint) = self.agent_hint.as_mut() {
            if hint.kind == BlockedReason::Permission && !screen_blocked {
                hint.screen_unblocked_since.get_or_insert(now);
            } else {
                hint.screen_unblocked_since = None;
                hint.screen_backed |= screen_blocked;
            }
        }
    }

    fn change_agent_hint(
        &mut self,
        hint: Option<AgentHint>,
        now: Instant,
    ) -> TerminalStateMutation {
        let previous_agent_label = self.effective_agent_label().map(str::to_string);
        let previous_known_agent = self.effective_known_agent();
        let previous_state = self.state;
        let previous_presentation = self.effective_presentation_for_state_at(previous_state, now);
        self.agent_hint = hint;
        TerminalStateMutation {
            effective_state_change: self.recompute_effective_state(
                previous_agent_label,
                previous_known_agent,
                previous_state,
                previous_presentation,
                now,
            ),
            session_ref_changed: false,
            agent_released: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{AgentHintReport, PERMISSION_HINT_SCREEN_GRACE, PERMISSION_HINT_UNBACKED_GRACE};
    use crate::detect::{Agent, AgentState, BlockedReason};
    use crate::terminal::{TerminalId, TerminalState};

    fn claude_terminal(screen: AgentState, now: Instant) -> TerminalState {
        let mut terminal = TerminalState::new(TerminalId::alloc(), "/tmp".into());
        screen_says(&mut terminal, screen, now);
        terminal
    }

    fn screen_says(terminal: &mut TerminalState, state: AgentState, now: Instant) {
        let reason = (state == AgentState::Blocked).then_some(BlockedReason::Other);
        terminal.set_detected_screen_state_at(
            Some(Agent::Claude),
            state,
            state == AgentState::Blocked,
            reason,
            false,
            false,
            now,
        );
    }

    fn hint(kind: Option<BlockedReason>, seq: u64) -> AgentHintReport {
        AgentHintReport {
            source: "herdr:claude-mod".into(),
            agent_label: "claude".into(),
            kind,
            id: Some("toolu_1".into()),
            ttl: Duration::from_secs(15),
            seq: Some(seq),
        }
    }

    #[test]
    fn a_question_hint_blocks_with_its_reason_whatever_the_screen_says_and_clears() {
        let now = Instant::now();
        let mut terminal = claude_terminal(AgentState::Idle, now);
        assert_eq!(terminal.state, AgentState::Idle);

        let mutation = terminal
            .set_agent_hint_at(hint(Some(BlockedReason::Question), 10), now)
            .expect("the hint changes the state");
        assert_eq!(
            mutation.effective_state_change.unwrap().state,
            AgentState::Blocked
        );
        assert_eq!(terminal.blocked_reason(), Some(BlockedReason::Question));

        // The screen reads idle, then working, behind the dialog: still blocked.
        screen_says(
            &mut terminal,
            AgentState::Idle,
            now + Duration::from_secs(5),
        );
        assert_eq!(terminal.blocked_reason(), Some(BlockedReason::Question));
        screen_says(
            &mut terminal,
            AgentState::Working,
            now + Duration::from_secs(9),
        );
        assert_eq!(terminal.blocked_reason(), Some(BlockedReason::Question));
        assert!(terminal
            .expire_agent_hint_at(now + Duration::from_secs(9))
            .is_none());

        let cleared = terminal.set_agent_hint_at(hint(None, 11), now + Duration::from_secs(10));
        assert!(cleared.is_some());
        assert_eq!(
            terminal.state,
            AgentState::Working,
            "the screen is back in charge"
        );
        assert_eq!(terminal.blocked_reason(), None);
    }

    #[test]
    fn a_permission_hint_the_screen_never_showed_waits_longer_before_the_screen_clears_it() {
        let now = Instant::now();
        let mut terminal = claude_terminal(AgentState::Working, now);
        terminal.set_agent_hint_at(hint(Some(BlockedReason::Permission), 10), now);
        assert_eq!(terminal.blocked_reason(), Some(BlockedReason::Permission));

        // The mod reported before the screen drew the dialog: a slow scan.
        screen_says(
            &mut terminal,
            AgentState::Working,
            now + Duration::from_millis(300),
        );
        let due = now + PERMISSION_HINT_UNBACKED_GRACE;
        assert_eq!(terminal.agent_hint_deadline(), Some(due));
        assert!(terminal
            .expire_agent_hint_at(due - Duration::from_millis(1))
            .is_none());
        assert_eq!(terminal.blocked_reason(), Some(BlockedReason::Permission));
        assert!(terminal.expire_agent_hint_at(due).is_some());
        assert_eq!(terminal.state, AgentState::Working);
        assert_eq!(terminal.agent_hint_deadline(), None);
    }

    #[test]
    fn a_permission_hint_the_screen_showed_is_cleared_a_short_grace_after_the_screen_moves_on() {
        let now = Instant::now();
        let mut terminal = claude_terminal(AgentState::Blocked, now);
        terminal.set_agent_hint_at(hint(Some(BlockedReason::Permission), 10), now);
        // The screen shows the dialog, then the user allows: the tool runs.
        screen_says(
            &mut terminal,
            AgentState::Blocked,
            now + Duration::from_secs(2),
        );
        let moved_on = now + Duration::from_secs(3);
        screen_says(&mut terminal, AgentState::Working, moved_on);
        assert_eq!(terminal.blocked_reason(), Some(BlockedReason::Permission));

        let due = moved_on + PERMISSION_HINT_SCREEN_GRACE;
        assert_eq!(terminal.agent_hint_deadline(), Some(due));
        assert!(terminal
            .expire_agent_hint_at(due - Duration::from_millis(1))
            .is_none());
        assert!(terminal.expire_agent_hint_at(due).is_some());
        assert_eq!(terminal.state, AgentState::Working);
    }

    #[test]
    fn a_permission_hint_the_screen_backs_is_not_cleared() {
        let now = Instant::now();
        let mut terminal = claude_terminal(AgentState::Blocked, now);
        terminal.set_agent_hint_at(hint(Some(BlockedReason::Permission), 10), now);
        screen_says(
            &mut terminal,
            AgentState::Blocked,
            now + Duration::from_secs(5),
        );
        assert!(terminal
            .expire_agent_hint_at(now + Duration::from_secs(6))
            .is_none());
        assert_eq!(terminal.blocked_reason(), Some(BlockedReason::Permission));
        // A screen blip of idle then blocked again resets the grace.
        screen_says(
            &mut terminal,
            AgentState::Idle,
            now + Duration::from_secs(7),
        );
        screen_says(
            &mut terminal,
            AgentState::Blocked,
            now + Duration::from_secs(8),
        );
        assert!(terminal
            .expire_agent_hint_at(now + Duration::from_secs(10))
            .is_none());
    }

    #[test]
    fn a_hint_ages_out_at_its_ttl_and_a_heartbeat_extends_it() {
        let now = Instant::now();
        let mut terminal = claude_terminal(AgentState::Idle, now);
        terminal.set_agent_hint_at(hint(Some(BlockedReason::Question), 10), now);
        assert_eq!(
            terminal.agent_hint_deadline(),
            Some(now + Duration::from_secs(15))
        );

        // The heartbeat restates the same hint: no state change, a later deadline.
        let beat = now + Duration::from_secs(5);
        assert!(terminal
            .set_agent_hint_at(hint(Some(BlockedReason::Question), 11), beat)
            .is_none());
        assert_eq!(
            terminal.agent_hint_deadline(),
            Some(beat + Duration::from_secs(15))
        );

        let dead = beat + Duration::from_secs(15);
        assert!(terminal
            .expire_agent_hint_at(dead - Duration::from_millis(1))
            .is_none());
        assert!(terminal.expire_agent_hint_at(dead).is_some());
        assert_eq!(terminal.state, AgentState::Idle, "a dead source ages out");
    }

    #[test]
    fn an_expired_hint_does_not_count_even_before_the_tick_runs() {
        let now = Instant::now();
        let mut terminal = claude_terminal(AgentState::Idle, now);
        terminal.set_agent_hint_at(hint(Some(BlockedReason::Question), 10), now);
        screen_says(
            &mut terminal,
            AgentState::Idle,
            now + Duration::from_secs(16),
        );
        assert_eq!(terminal.state, AgentState::Idle);
    }

    #[test]
    fn a_stale_seq_is_ignored_and_a_reloaded_source_with_a_later_clock_is_accepted() {
        let now = Instant::now();
        let mut terminal = claude_terminal(AgentState::Idle, now);
        // Wall clock in microseconds plus a counter, as the mod builds it.
        let first = 1_790_000_000_000_000_u64 + 7;
        terminal.set_agent_hint_at(hint(Some(BlockedReason::Question), first), now);

        // An older clear (a late arrival) must not end the live hint.
        assert!(terminal
            .set_agent_hint_at(hint(None, first - 5), now)
            .is_none());
        assert_eq!(terminal.blocked_reason(), Some(BlockedReason::Question));
        assert!(
            terminal.set_agent_hint_at(hint(None, first), now).is_none(),
            "an equal seq is stale too"
        );

        // /reload-plugins: the module restarts, its counter is back at 1, but
        // its clock is later, so the new report wins.
        let reloaded = 1_790_000_030_000_000_u64 + 1;
        assert!(terminal
            .set_agent_hint_at(hint(None, reloaded), now)
            .is_some());
        assert_eq!(terminal.state, AgentState::Idle);
    }

    #[test]
    fn a_second_source_is_ignored_while_the_first_hint_is_live() {
        let now = Instant::now();
        let mut terminal = claude_terminal(AgentState::Idle, now);
        terminal.set_agent_hint_at(hint(Some(BlockedReason::Question), 10), now);
        let mut other = hint(Some(BlockedReason::Permission), 1);
        other.source = "someone:else".into();
        assert!(terminal.set_agent_hint_at(other.clone(), now).is_none());
        assert_eq!(terminal.blocked_reason(), Some(BlockedReason::Question));

        terminal.set_agent_hint_at(hint(None, 11), now);
        other.seq = Some(2);
        assert!(terminal.set_agent_hint_at(other, now).is_some());
        assert_eq!(terminal.blocked_reason(), Some(BlockedReason::Permission));
    }

    #[test]
    fn a_hint_for_another_agent_does_not_apply_and_is_dropped() {
        let now = Instant::now();
        let mut terminal = TerminalState::new(TerminalId::alloc(), "/tmp".into());
        terminal.set_detected_screen_state_at(
            Some(Agent::Codex),
            AgentState::Idle,
            false,
            None,
            false,
            false,
            now,
        );
        terminal.set_agent_hint_at(hint(Some(BlockedReason::Question), 10), now);
        assert_eq!(
            terminal.state,
            AgentState::Idle,
            "claude's hint on a codex pane"
        );
        assert!(
            terminal.agent_hint_deadline().is_none(),
            "and it is dropped"
        );
    }

    #[test]
    fn no_hint_means_the_screens_own_state_and_reason() {
        let now = Instant::now();
        let mut terminal = claude_terminal(AgentState::Idle, now);
        screen_says(
            &mut terminal,
            AgentState::Blocked,
            now + Duration::from_secs(1),
        );
        assert_eq!(terminal.state, AgentState::Blocked);
        assert_eq!(terminal.blocked_reason(), Some(BlockedReason::Other));
        assert!(terminal.agent_hint_deadline().is_none());
        assert!(terminal
            .expire_agent_hint_at(now + Duration::from_secs(60))
            .is_none());
    }

    #[test]
    fn a_hint_that_arrives_before_the_agent_is_detected_applies_once_it_is() {
        let now = Instant::now();
        let mut terminal = TerminalState::new(TerminalId::alloc(), "/tmp".into());
        terminal.set_agent_hint_at(hint(Some(BlockedReason::Question), 10), now);
        assert_ne!(terminal.state, AgentState::Blocked);
        screen_says(
            &mut terminal,
            AgentState::Idle,
            now + Duration::from_secs(1),
        );
        assert_eq!(terminal.blocked_reason(), Some(BlockedReason::Question));
    }
}
