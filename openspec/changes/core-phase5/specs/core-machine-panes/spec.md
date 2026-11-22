## ADDED Requirements

### Requirement: Local terminal scans see only this Mac's terminals

herdr SHALL parse every terminal address it reads from core into a local or a machine address. The restore label
search, the waiting-pane search, the orphan scan and the launch-label recovery for local panes SHALL keep local
addresses only, and SHALL never use the pid of a machine terminal for a local fact or sweep. This SHALL ship before
any other part of this change.

#### Scenario: The far herdr's pane has our labels
- **WHEN** the primary core lists `ac-mbm5/t1` with `herdr.session=herdr` and `herdr.pane_id=w1:p1`, and a saved
  local pane `w1:p1` is restored
- **THEN** the restore neither adopts, kills nor signals `ac-mbm5/t1` or its pid, and the orphan scan does not list
  it

### Requirement: Panes start on a machine only through an advertised capability

The server SHALL advertise `spawn_on_machine` and accept an optional `on` on `pane.split`, `tab.create`,
`workspace.create` and `agent.start`. The CLI SHALL send `on` (flag `--on <machine>`) only to a server that
advertises the capability, and SHALL refuse otherwise without starting a pane.

#### Scenario: Old server
- **WHEN** `herdr pane split --on ac-mbm5` targets a server without `spawn_on_machine`
- **THEN** the CLI prints that the server cannot place panes on machines, and no pane starts

### Requirement: A pane on a machine is owned by a label the other herdr never matches

A pane started on a machine SHALL carry `herdr.session=<session label>@<primary core id>` and `herdr.pane_id`.
The orphan scan, claims and kills for panes on machines SHALL use that value.

#### Scenario: The far side's own herdr
- **WHEN** ac-mbm5's herdr scans its default core while m4m's pane runs there
- **THEN** it neither claims, lists as orphan, nor kills that terminal

### Requirement: A pane on a machine gets no herdr integration variables

The spawn SHALL NOT pass `HERDR_ENV`, `HERDR_PANE_ID`, `HERDR_TAB_ID`, `HERDR_WORKSPACE_ID`, `HERDR_SOCKET_PATH`
or `HERDR_CLIENT_SOCKET_PATH`, and SHALL wrap the program in `env -u` for each of them, so none reaches the
program even when the far core's own environment has them. Every `machine.exec` argv SHALL carry the same wrapper.
Its status SHALL come from OSC 7501 and screen detection.

#### Scenario: A hook on the far side
- **WHEN** an agent's hook in a pane on ac-mbm5 runs `herdr pane report-agent`
- **THEN** it finds no `HERDR_PANE_ID` and reports to no pane

#### Scenario: A far core started from a herdr pane
- **WHEN** the far core was started with `HERDR_PANE_ID` in its environment and a pane starts on that machine
- **THEN** the pane's program has no `HERDR_*` integration variable

### Requirement: Host-dependent facts follow the pane's host

For a pane on a machine, pid-based process facts SHALL answer none, the close sweep SHALL NOT run, and close SHALL
be `terminal.kill` only. Pane JSON SHALL carry optional `machine` and `core_address` fields; panes on this Mac SHALL
carry no `machine` field.

#### Scenario: Close
- **WHEN** a person closes a pane on ac-mbm5
- **THEN** herdr sends `terminal.kill` for its address, signals no local process, and the far side's core and other
  terminals keep running

#### Scenario: Listing
- **WHEN** `herdr pane list --json` includes a pane on ac-mbm5
- **THEN** that pane has `machine: "ac-mbm5"` and `core_address: "ac-mbm5/t7"`, and every other field keeps its
  meaning

### Requirement: A pane on an unavailable machine waits in place

On restore, live handoff or link loss, a pane whose machine is not connected SHALL wait as `CoreWaiting` with the
cause `machine_dormant` or `machine_unreachable`, SHALL NOT start fresh, and SHALL reattach by its saved session and
address when the machine is connected again. Input to it while it waits SHALL NOT be sent; the first key SHALL post
a wake to the machine's dialer without waiting for it.

#### Scenario: Server restart while dormant
- **WHEN** herdr restarts while ac-mbm5 is dormant
- **THEN** the pane on ac-mbm5 keeps its place as waiting, and after the first key it reattaches to the same
  far-side program
