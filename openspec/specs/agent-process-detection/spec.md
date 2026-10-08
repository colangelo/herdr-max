# agent-process-detection Specification

## Purpose
Define how Herdr identifies the coding agent running in a pane when the pane's
own PTY does not carry it: specifically when a recognised PTY wrapper has
replaced the pane's shell and re-run it inside a PTY of its own, hiding the
agent one level below the foreground-job scan. Covers which wrappers are
recognised, how far and from where the descent may proceed, which job the
pane's reported process facts are read from, and the requirement that none of
this adds work to panes that are not wrapped.

## Requirements

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

The lookup SHALL be reached only through the existing foreground-job probe. For a
pane whose last probe found a recognised wrapper, detection SHALL read the
foreground process group of the shell that wrapper hosts once per tick, and a
change of that group, or the shell's disappearance, SHALL trigger a probe and
count as a foreground change.

#### Scenario: No additional sampling is introduced

- **WHEN** the foreground-job probe is skipped for a pane that is not wrapped
- **THEN** no nested lookup is performed for that pane

#### Scenario: A command starting behind a wrapper is probed

- **WHEN** the shell a wrapper hosts starts a command in a pane with no identified agent
- **THEN** the next tick probes the pane without waiting for screen content to change

### Requirement: agent start accepts a shell behind a recognised wrapper

`agent start` SHALL accept a pane whose foreground job is a recognised PTY
wrapper alone in its group when the shell that wrapper hosts is alone at the
front of the nested PTY, and SHALL type the launch for that shell. It SHALL
refuse the pane as busy when anything else leads the nested PTY.

#### Scenario: A wrapped shell at its prompt takes the launch

- **WHEN** the pane runs `atuin pty-proxy` hosting `zsh` at its prompt
- **THEN** `agent start` launches the agent through `zsh`

#### Scenario: A wrapped shell running a command is busy

- **WHEN** the shell behind the wrapper runs `vim`, or a subshell
- **THEN** `agent start` fails with `agent_pane_busy`
