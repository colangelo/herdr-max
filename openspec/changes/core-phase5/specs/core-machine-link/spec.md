## ADDED Requirements

### Requirement: Core machines are added by a person with herdr's link command

`herdr machine add --core <target>` SHALL run `hc machine add` against the herdr session's primary core with
`--via '<installed herdr path> core-link <name>'`, in the invoking terminal. It SHALL record the ssh target,
options and the invoking terminal's `SSH_AUTH_SOCK` in the machine's state directory, and SHALL write a `manual`
lease of 30 min before it calls hc. It SHALL NOT pass `--install`. Without a terminal it SHALL refuse and print
the command to run.

#### Scenario: Add from a terminal
- **WHEN** a person runs `herdr machine add --core ac-mbm5` in a terminal
- **THEN** the primary core has a machine `ac-mbm5` whose target is herdr's link command, and hc's own install
  question, if any, is asked in that terminal

#### Scenario: Add from an unattended run
- **WHEN** `herdr machine add --core ac-mbm5` runs without a terminal
- **THEN** it adds nothing and prints the command a person must run

### Requirement: The link dials only while a lease is in force

`core-link` SHALL read the machine's gate and leases before it starts ssh. With a closed gate or no lease in
force it SHALL exit at once with status 75, start no process and open no socket. It SHALL dial only core's POSIX
line and exit 75 for any other line. While connected it SHALL re-read the leases every 5 s and end ssh when none is
in force.

#### Scenario: Dormant machine at a core start
- **WHEN** the local core starts and dials a machine with no lease in force
- **THEN** `core-link` exits 75 without running ssh, and the machine stays unreachable

#### Scenario: Core's Windows fallback line
- **WHEN** core runs `core-link` with its `cmd /d /c ...` line after a failed POSIX dial
- **THEN** `core-link` exits 75 without running ssh

#### Scenario: The last lease ends
- **WHEN** the only lease of a connected machine expires
- **THEN** within 5 s ssh ends, the link carries no further bytes, and core's retries are refused locally

### Requirement: The link is one ssh process at a time

`core-link` SHALL take the machine's dial lock, waiting at most 2 s, and exit 75 without dialling when it cannot. It
SHALL then exec ssh, so that the process core runs and may kill is ssh itself, with a watcher process beside it that
holds none of core's standard streams and exits when ssh does.

#### Scenario: Core kills the via
- **WHEN** core SIGKILLs the process it started for a dial
- **THEN** no ssh, watcher or `core-link` process of that dial is left

#### Scenario: Two dials at once
- **WHEN** a second `core-link` starts while the first holds the dial lock for more than 2 s
- **THEN** the second exits 75 and runs no ssh

### Requirement: No ssh session outlives the link

ssh SHALL run with `ControlMaster=no`, `ControlPath=none`, `ServerAliveInterval=5` and `ServerAliveCountMax=2`.

#### Scenario: The link ends
- **WHEN** the last lease ends and ssh exits
- **THEN** no ssh control master or socket remains, and the next dial authenticates again

#### Scenario: A frozen link
- **WHEN** the far side stops answering while the link is connected
- **THEN** ssh ends within about 15 s and the machine is unreachable within 30 s

### Requirement: The link does not depend on who started the core

`core-link` SHALL find the machine's state directory from the account's passwd home, never from the environment
core passes, and SHALL run ssh with an allowlisted environment plus the `SSH_AUTH_SOCK` recorded by `herdr machine
add --core`.

#### Scenario: A core started from another environment
- **WHEN** the primary core was started with a different `HOME`, `XDG_STATE_HOME` and `SSH_AUTH_SOCK`
- **THEN** `core-link` reads the same leases and gate, and ssh gets the recorded `SSH_AUTH_SOCK`

### Requirement: A dial never starts the far core

The far command SHALL run a guard before core's line that tests the far session's socket without starting a core
and, when no core answers, exits 69 with `herdr-link: herdr-core is not running there`. `core-link` SHALL then
record `far_core_down`, refuse dials for 5 min, and reopen on `herdr machine wake`.

#### Scenario: Far core stopped
- **WHEN** a dial reaches a machine whose core is stopped or left a stale socket
- **THEN** no core starts there, the gate reads `far_core_down`, and no dial runs for 5 min unless a person runs
  `herdr machine wake`

### Requirement: Leases come from human input and explicit calls only

A TUI client SHALL write its own lease, expiring 10 min after its last human input, at most once a minute, and at
its start when a person's command started it. A fleet CLI call through a machine SHALL hold its own lease for its
duration. `herdr machine wake <name> [--for D]` SHALL write a manual lease of 30 min by default and at most 12 h.
Each holder SHALL write only its own lease file. A herdr server, a restore, a live handoff, a client reconnect and
a core start SHALL NOT write a lease.

#### Scenario: Overnight beta install
- **WHEN** a live handoff runs at 03:00 with no client input since 22:00
- **THEN** no lease is written and the machine is not dialled

#### Scenario: First key after dormancy
- **WHEN** a person presses a key in a client whose machines are dormant
- **THEN** the client writes its lease, `machine.connect` is called for the machine the key concerns, and the link
  comes up without waiting for core's next retry

### Requirement: A lease comes before every connect

No herdr code SHALL call `machine.connect`, `hc machine add` or `hc machine login` without a lease in force for
that machine. `herdr machine add --core` and `herdr machine login` SHALL write a 30 min manual lease before they
call hc.

#### Scenario: Login from dormant
- **WHEN** a person runs `herdr machine login ac-mbm5` while no lease is in force
- **THEN** the manual lease exists before hc's first dial, and that dial is not refused

#### Scenario: A view opened while dormant, before any key
- **WHEN** a view server gets a client that wrote no lease
- **THEN** it calls no `machine.connect` and shows that the machine is dormant until a key

### Requirement: `machine.connect` runs off every input path, once per wake

Each herdr process SHALL make `machine.connect` calls only from one dialer per machine, which input, render and API
handlers post to without waiting. Across herdr processes the dialer SHALL hold the machine's connect lock around
the call; a dialer that cannot take it SHALL wait on `machine.list` instead of calling.

#### Scenario: Many keys during a slow dial
- **WHEN** a person types into two panes on a dormant machine while its dial takes 20 s
- **THEN** every key is handled at once, and exactly one `machine.connect` is sent

### Requirement: Prompting failures close the gate until a person logs in

`core-link` SHALL record `needs_check` when ssh prints a Tailscale SSH check banner, `auth_stalled` when the far
guard has not answered within 15 s of the start (before core's own kill), and `auth_failed` on an authentication
or host-key failure. After any of
them it SHALL refuse every dial until `herdr machine login <name>`, which runs ssh in the person's terminal, clears
the state and calls `machine.connect`.

#### Scenario: A Tailscale check in the background
- **WHEN** a background dial meets a Tailscale SSH check
- **THEN** ssh is ended, the gate reads `needs_check`, and no further dial starts until a person runs
  `herdr machine login`

#### Scenario: An unanswered Touch ID
- **WHEN** a background dial's ssh writes nothing for 15 s
- **THEN** `core-link` records `auth_stalled` and ends ssh before core kills it, and no further dial starts until
  a person runs `herdr machine login`

### Requirement: Machine status is readable by agents

`herdr machine status [--json]` SHALL report for each core machine its core state, its gate (`open`, `dormant`,
`needs_check`, `auth_stalled`, `auth_failed`, `far_core_down`), its lease holders with expiry, and its last dial
time.

#### Scenario: An agent checks before acting
- **WHEN** an agent runs `herdr machine status --json` for a machine whose gate is `needs_check`
- **THEN** the output names the machine and the gate, so the agent can announce the SSH check to ac before anyone
  runs `herdr machine login`

### Requirement: herdr never installs or updates hc

herdr SHALL NOT call `machine.update`, pass `--install`, or run `hc update`. A machine whose hc is older than the
supported minimum SHALL raise one notice naming `hc machine update <name>` and its effect.

#### Scenario: Old hc on the far side
- **WHEN** a machine reports hc 0.1.20
- **THEN** herdr shows one notice with `hc machine update ac-mbm5` and does not run it
