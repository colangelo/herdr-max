# agent-process-detection

## ADDED Requirements

### Requirement: Identify an agent behind a recognised PTY wrapper

Herdr SHALL identify the agent running in a pane whose shell has been replaced
by a process that allocates its own PTY, when that process is a recognised PTY
wrapper.

Identification SHALL first read the foreground job of the pane's own PTY, as it
does for an unwrapped pane. Only when that job yields no agent, and only when
that job's process group leader is a recognised wrapper, SHALL identification
descend into the PTY the wrapper owns and identify the agent from that job.

The descent SHALL follow only a child of the wrapper whose controlling terminal
differs from the wrapper's own, SHALL proceed at most one level, and SHALL start
only from the process group leader.

Recognised wrappers SHALL be an explicit set of process names. A wrapper that is
not in that set SHALL be treated exactly as it is today.

#### Scenario: An agent behind a PTY wrapper is identified

- **WHEN** a pane's PTY carries a recognised wrapper as its foreground process group leader, and the agent runs in the PTY that wrapper owns
- **THEN** the pane is identified as running that agent
- **AND** the pane appears in the agents sidebar and in the agent list, and resolves as a target for the agent operations

#### Scenario: An unwrapped pane is unaffected

- **WHEN** a pane's own PTY carries a recognisable agent
- **THEN** that agent is identified from the pane's own foreground job
- **AND** no nested lookup is performed

#### Scenario: An unrecognised wrapper yields no agent

- **WHEN** a pane's foreground process group leader is a process that owns a nested PTY but is not a recognised wrapper
- **THEN** the pane is reported as running no agent
- **AND** no process outside the pane's own foreground job is inspected

#### Scenario: A wrapper with no agent behind it yields no agent

- **WHEN** a pane's foreground process group leader is a recognised wrapper and the PTY it owns carries no recognisable agent
- **THEN** the pane is reported as running no agent

#### Scenario: An ordinary child of a wrapper is not followed

- **WHEN** a recognised wrapper has a child process that shares the wrapper's controlling terminal
- **THEN** that child is not treated as a nested PTY
- **AND** it is not searched for an agent

#### Scenario: The outer agent wins when both levels carry one

- **WHEN** a pane's own foreground job carries a recognisable agent and a recognised wrapper below it also has one
- **THEN** the agent identified from the pane's own job is the one reported

### Requirement: Pane facts follow the identified job

The process facts a pane reports SHALL be read from the job the agent was
identified in, so that a pane identified through a nested PTY reports the
process name and working directory of that PTY rather than the wrapper's.

A pane where no nested identification occurred SHALL report the facts of its own
foreground job, unchanged.

#### Scenario: A wrapped pane reports the agent's working directory

- **WHEN** a pane is identified through a recognised wrapper, and the wrapper and the agent have different working directories
- **THEN** the pane reports the agent's working directory

#### Scenario: A pane with no nested identification is unchanged

- **WHEN** a pane is identified from its own foreground job, or no agent is identified at all
- **THEN** the pane reports the process facts of its own foreground job

### Requirement: Nested lookup stays off the unwrapped path

The nested lookup SHALL NOT add work to panes that are not wrapped, beyond
comparing the foreground process group leader's name against the recognised set.

The lookup SHALL be reached only through the existing foreground-job probe, so
that it inherits that probe's gating rather than introducing sampling of its
own.

#### Scenario: No additional sampling is introduced

- **WHEN** the foreground-job probe is skipped for a pane by the existing gating
- **THEN** no nested lookup is performed for that pane
