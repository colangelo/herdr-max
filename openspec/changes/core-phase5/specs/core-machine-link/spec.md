## ADDED Requirements

### Requirement: Core machines are added by a person with herdr's link command

`herdr machine add --core <target>` SHALL run `hc machine add` against the herdr session's primary core with
`--via '<installed herdr path> core-link <name>'`, in the invoking terminal. It SHALL record the ssh target and
options in herdr's state directory. It SHALL NOT pass `--install`. Without a terminal it SHALL refuse and print the
command to run.

#### Scenario: Add from a terminal
- **WHEN** a person runs `herdr machine add --core ac-mbm5` in a terminal
- **THEN** the primary core has a machine `ac-mbm5` whose target is herdr's link command, and hc's own install
  question, if any, is asked in that terminal

#### Scenario: Add from an unattended run
- **WHEN** `herdr machine add --core ac-mbm5` runs without a terminal
- **THEN** it adds nothing and prints the command a person must run

### Requirement: The link dials only while a lease is in force

`core-link` SHALL read the machine's leases before it starts ssh. With no lease in force it SHALL exit at once
with status 75, start no process and open no socket. While connected it SHALL re-read the leases every 5 s and end
ssh when none is in force.

#### Scenario: Dormant machine at a core start
- **WHEN** the local core starts and dials a machine with no lease in force
- **THEN** `core-link` exits 75 without running ssh, and the machine stays unreachable

#### Scenario: The last lease ends
- **WHEN** the only lease of a connected machine expires
- **THEN** within 5 s ssh ends, the link carries no further bytes, and core's retries are refused locally

### Requirement: Leases come from human input and explicit calls only

A TUI client SHALL write its own lease, expiring 10 min after its last human input, at most once a minute. A fleet
CLI call through a machine SHALL hold its own lease for its duration. `herdr machine wake <name> [--for D]` SHALL
write a manual lease of 30 min by default and at most 12 h. Each holder SHALL write only its own lease file. A
herdr server, a restore, a live handoff and a core start SHALL NOT write a lease.

#### Scenario: Overnight beta install
- **WHEN** a live handoff runs at 03:00 with no client input since 22:00
- **THEN** no lease is written and the machine is not dialled

#### Scenario: First key after dormancy
- **WHEN** a person presses a key in a client whose machines are dormant
- **THEN** the client writes its lease, `machine.connect` is called for the machine the key concerns, and the link
  comes up without waiting for core's next retry

### Requirement: Prompting failures close the gate until a person logs in

`core-link` SHALL record `needs_check` when ssh prints a Tailscale SSH check banner, `auth_stalled` when no byte
arrives from the far side within 20 s, and `auth_failed` on an authentication or host-key failure. After any of
them it SHALL refuse every dial until `herdr machine login <name>`, which runs ssh in the person's terminal, clears
the state and calls `machine.connect`.

#### Scenario: A Tailscale check in the background
- **WHEN** a background dial meets a Tailscale SSH check
- **THEN** ssh is ended, the gate reads `needs_check`, and no further dial starts until a person runs
  `herdr machine login`

### Requirement: Machine status is readable by agents

`herdr machine status [--json]` SHALL report for each core machine its core state, its gate (`open`, `dormant`,
`needs_check`, `auth_stalled`, `auth_failed`), its lease holders with expiry, and its last dial time.

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
