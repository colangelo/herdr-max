# handoff-agent-state-resurface Specification

## Purpose
Define how live handoff (`herdr update --handoff`) brings working agents back
without user interaction: the runtime manifest carries each pane's live agent
detection state, restore seeds imported panes from it instead of hardcoded
Idle, and a post-commit background sweep re-verifies every imported agent pane
— repaint nudge plus a throttle-bypassing detection rescan, staggered per pane,
with no client attached — replacing the one-shot nudge that used to be deferred
to first client attach. Covers the older-manifest fallback and which panes are
still seeded Idle.
## Requirements
### Requirement: Handoff manifest carries live agent state
The handoff runtime manifest SHALL include each pane's live agent detection state (Working, Blocked, or Idle) as an optional per-pane field. When the field is absent (manifest produced by an older server), the receiving server SHALL behave as if the state were Idle.

#### Scenario: State roundtrips through the manifest
- **WHEN** a live handoff is initiated while a pane's agent is detected as Working
- **THEN** the manifest entry for that pane records Working, and the receiving server reads Working back from the manifest

#### Scenario: Old manifest without the field
- **WHEN** the receiving server imports a handoff manifest that has no agent-state field for a pane
- **THEN** the pane is seeded as Idle, matching pre-change behavior

### Requirement: Restore seeds imported panes with pre-restart agent state
For panes imported via live handoff, restore SHALL seed the pane's terminal agent state from the manifest state instead of hardcoded Idle, so the sidebar shows the pre-restart state (including the Working spinner) immediately after restart. Panes restored without an imported runtime (cold restore, relaunched agents) SHALL continue to be seeded Idle.

#### Scenario: Working agent resurfaces immediately
- **WHEN** the server restarts via live handoff while a pane's agent was Working
- **THEN** after restore completes, that pane's agent state is Working and the sidebar shows the working spinner without any user interaction

#### Scenario: Cold restore is not seeded Working
- **WHEN** a pane is restored without an imported handoff runtime and its agent is relaunched via the agent-resume plan
- **THEN** the pane is seeded Idle and its state is subsequently driven by normal detection of the fresh process

#### Scenario: Blank screen does not clobber the seeded state
- **WHEN** a seeded pane's terminal grid is still blank because the re-adopted agent has not repainted yet
- **THEN** screen detection does not downgrade the seeded state; the seed is only verified or corrected once real screen content arrives

### Requirement: Post-handoff background verification sweep
After committing a live handoff and unpausing the imported PTY actors, the server SHALL run a background sweep over all imported agent panes in workspace/tab/pane order, without requiring any client to be attached. For each pane the sweep SHALL trigger a repaint nudge (SIGWINCH shrink/restore) and force a detection rescan that bypasses the idle-scan throttle, and SHALL wait a staggering interval before proceeding to the next pane.

#### Scenario: Finished agent settles back to idle
- **WHEN** a pane was seeded Working from the manifest but its agent actually finished during the restart
- **THEN** the sweep's forced rescan detects the idle screen and the pane settles to Idle without user interaction

#### Scenario: Sweep runs headless
- **WHEN** the handoff commits while no TUI client is attached
- **THEN** the sweep still nudges and rescans every imported agent pane, so a client attaching later sees correct states immediately

#### Scenario: User interaction during the sweep is safe
- **WHEN** the user focuses a pane before the sweep reaches it
- **THEN** normal detection driven by the focus-triggered repaint wins, and the sweep's later nudge and rescan do not change the correct state

### Requirement: Sweep replaces the deferred first-attach nudge
The one-shot repaint nudge deferred to first client attach SHALL be removed in favor of the post-commit sweep, leaving a single nudge path for handoff panes. The first-attach viewport resize for attaching clients SHALL be unaffected.

#### Scenario: No duplicate nudge on first attach
- **WHEN** the first client attaches after the sweep has already nudged the panes
- **THEN** no additional handoff-wide repaint nudge fires, and the client attach only performs its normal viewport resize

