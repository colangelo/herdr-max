## ADDED Requirements

### Requirement: A runtime loss suspends a core pane

When a core pane's terminal finishes with reason `holder_lost`, `host_shutdown`, `server_stop` or
`stopped_by_restart`, the server SHALL keep the pane in its layout as suspended, with its agent's resume plan and
the cause, and SHALL NOT close the pane or its workspace. A terminal that finishes as `exited` with a code, or
`killed` with no signal, SHALL end the pane as it does today.

#### Scenario: The holder is quit
- **WHEN** the core's holder process ends and every terminal it held finishes with `holder_lost`
- **THEN** each of those panes stays in its place as suspended, the workspace stays open, and one notice says Herdr
  Core quit or crashed

#### Scenario: A person closes a terminal from another client
- **WHEN** a core pane's terminal finishes as `killed` with no signal
- **THEN** the pane closes as an exited pane

### Requirement: A lost stream is gone only when the program is gone

After a core pane's stream ends, the server SHALL suspend the pane as gone only when the core answers that the
terminal is not found or finished **and** the program's pid, checked with the start time recorded at attach, no
longer runs.

#### Scenario: Core back, terminal gone
- **WHEN** the core answers `not_found` for the pane's terminal and its pid no longer runs
- **THEN** the pane is suspended and later resumes by its plan

#### Scenario: Core says gone, program still runs
- **WHEN** the core answers `not_found` but the recorded pid with its start time still runs
- **THEN** the pane is not suspended

### Requirement: An unreachable core never suspends a pane

When the core does not answer, for any length of time, or the program may still run, the server SHALL keep the
pane in place showing that Herdr Core is not answering, SHALL keep retrying the attach, and SHALL NOT suspend or
resume the pane. An unknown pid SHALL count as a program that may run.

#### Scenario: Core unreachable, program alive
- **WHEN** the core's socket is gone for two minutes and the pane's program still runs under its holder
- **THEN** the pane is not suspended, it shows that Herdr Core is not answering, and no second copy of its program
  starts

### Requirement: Every resume checks that the old program ended

Before resuming a suspended pane, for every suspend cause, the server SHALL check that the last program, and the
detected agent process where known, no longer runs. A pane whose pid is 0 or was never recorded SHALL NOT be
resumed automatically.

#### Scenario: Old program still runs at resume time
- **WHEN** a suspended pane is due to resume and its recorded program still runs
- **THEN** the pane is not resumed and says the old program still runs

#### Scenario: Unknown pid
- **WHEN** a suspended pane has no recorded pid
- **THEN** it is not resumed automatically and says to run `herdr pane respawn`

### Requirement: The saved link records the program

A saved core link SHALL record the program's pid and start time as optional fields that older builds ignore, and a
link without them SHALL still be read.

#### Scenario: A link from phase 2
- **WHEN** a saved link has no pid and its core does not answer
- **THEN** the pane is restored waiting for Herdr Core; it is not started fresh

### Requirement: Restore never starts a pane fresh while its program may run

On restore and live handoff, the server SHALL start a core pane fresh only when its program is known to be gone.
When the core is unreachable and the program runs or may run, the pane SHALL be restored waiting for Herdr Core,
with no program started, and SHALL attach when the core answers.

#### Scenario: Herdr restart while the core server is down
- **WHEN** the core server was killed, its holder still runs the pane's program, and herdr restarts
- **THEN** no program is started for the pane, and once a core answers the pane attaches to the same pid

#### Scenario: Live handoff while a legacy core is down
- **WHEN** a pane's link names a secondary core session that does not answer and herdr hands off
- **THEN** the new server shows the pane waiting for Herdr Core and starts nothing for it

### Requirement: Restore searches every known session before a fresh start

Before starting a core pane fresh, the server SHALL look for a live terminal labelled for that pane in every core
session it knows and can reach without starting one. One match SHALL be attached; more than one SHALL start
nothing and raise one notice.

#### Scenario: The pane's terminal is in another session
- **WHEN** a pane's link names session `herdr` but a live terminal labelled for that pane is in session `default`
- **THEN** the pane attaches to that terminal and its link is rewritten

### Requirement: Suspended core panes resume on the backend they will use

The server SHALL resume suspended panes after the settle time of pane suspension (5 s, or 30 s after a burst of
three losses within 2 s), or at once on a client attach, once the backend the pane will start on is ready. An agent
pane SHALL resume by its saved plan; any other pane SHALL re-run its launch command or a shell. One notice per
burst SHALL name the cause and how many panes it ended.

#### Scenario: Agent pane after a holder loss
- **WHEN** a suspended Claude pane's core is ready again and its old program is gone
- **THEN** the pane starts in place and runs its resume plan with the same session id

### Requirement: Resume does not block the server loop

The server loop SHALL NOT wait on core while resuming panes. Core calls SHALL run on a worker in batches of at most
8, one batch at a time.

#### Scenario: Many panes resume on a slow core
- **WHEN** sixteen panes resume and each `terminal.run` takes 2 s
- **THEN** the server keeps answering clients, and no more than eight spawns are in flight

### Requirement: The server starts its primary core again after a loss

The server SHALL start its primary core session again, with the clean environment of the first start, when that
session's socket has been missing or refusing connections for at least 3 s without a break. It SHALL NOT start any
other core session, and SHALL NOT run `hc server stop`, `hc server restart` or `hc update`.

#### Scenario: Only the core server died
- **WHEN** the core server process is killed, its socket file stays and refuses, and its holder keeps running
- **THEN** the server starts a core after 3 s, the panes reattach, and every program keeps its pid

#### Scenario: A core restart in progress
- **WHEN** `hc server restart` leaves the socket missing for under 3 s
- **THEN** the server does not start a core and the panes reattach to the restarted one

### Requirement: The loss guard counts loss bursts

The server SHALL count loss bursts: core-pane suspensions and core starts after a loss, with events within 2 s
being one burst. After three bursts within ten minutes it SHALL stop starting the core until a config reload or a
new server, resume suspended panes on the PTY, show one notice, and report the state in `herdr server status`.

#### Scenario: The holder keeps ending
- **WHEN** the holder is lost a third time within ten minutes, with no core start in between
- **THEN** no further core start is attempted, the suspended panes resume on the PTY, one notice says so, and
  `herdr server status` shows that core was given up

### Requirement: A stopped core is not started by herdr

When terminals finish with `server_stop`, the server SHALL suspend their panes and SHALL NOT start that core
session itself. The panes SHALL resume when the core answers again, or when a new herdr server starts.

#### Scenario: Someone runs `hc server stop`
- **WHEN** a person stops the primary core
- **THEN** its panes are suspended with one notice, herdr starts no core, and the panes resume after the person
  starts the core again
