# agent-hints Specification

## Purpose
Let a source that knows an agent is waiting on the user, such as the Claude Code
mod, tell herdr so, with a `pane.report_hint` call carrying a kind, a source and
a deadline. A hint holds the blocked state against what the screen shows (a
question) or raises it until the screen agrees (a permission prompt), ages out
on its own, and never changes a pane that has no hint. One install command puts
the mod in place.

## Requirements

### Requirement: A source can report that an agent waits on the user

herdr SHALL accept a hint for a pane through `pane.report_hint` (and
`herdr pane report-hint`): a `kind` of `question` or `permission`, the source,
the agent, an optional dialog `id`, an optional `ttl_ms` (default 15000, at most
60000) and a `seq`. A hint with a `seq` lower than the live one from the same
source SHALL be ignored. A source SHALL derive `seq` from its wall clock so a
restart of the source (a mod reload) does not make its reports stale. A hint with `clear` SHALL end the source's live hint.
Hints SHALL be runtime state: not saved in the session snapshot and not carried
through a live handoff. The method SHALL NOT change `PROTOCOL_VERSION`.

#### Scenario: Open and clear

- **WHEN** a source reports `question` for a pane and then reports `clear`
- **THEN** the pane reads `blocked` with reason `question` between the two and
  detection is unchanged after

#### Scenario: A stale report

- **WHEN** a report with `seq` 5 arrives after one with `seq` 9 from the same
  source
- **THEN** it is ignored

#### Scenario: Restart

- **WHEN** the server restarts or hands off with a hint live
- **THEN** no pane has a hint afterwards

#### Scenario: The source reloads

- **WHEN** a source's module reloads and its next report has a lower counter but
  a later clock than the live hint's `seq`
- **THEN** herdr accepts it

### Requirement: A question hint holds the blocked state against the screen

While a live `question` hint exists for a pane whose agent is the hint's agent,
the pane SHALL be `blocked` with reason `question` whatever the screen shows,
until the hint is cleared, expires, or the agent leaves.

#### Scenario: Screen reads idle behind the dialog

- **WHEN** a `question` hint is live and the screen detection reads `idle`
- **THEN** the pane stays `blocked` with reason `question`

#### Scenario: Esc

- **WHEN** the dialog ends by Esc and the source clears the hint
- **THEN** the pane leaves `blocked` within one detection pass of the clear

### Requirement: A permission hint raises the state and the screen may clear it

While a live `permission` hint exists, the pane SHALL be `blocked` with reason
`permission`. The hint SHALL be dropped when screen detection has read
something other than `blocked` continuously for 0.5 seconds after the screen
has shown a blocked prompt, or for 4 seconds if it never has.

#### Scenario: The user answered

- **WHEN** a `permission` hint is live, the screen showed the dialog, and has
  shown no blocked prompt for 0.5 s
- **THEN** the hint is dropped and the pane shows what the screen shows

#### Scenario: A slow scan

- **WHEN** a `permission` hint arrives and the screen has not drawn the dialog yet
- **THEN** the hint stays for 4 s before the screen may clear it

### Requirement: Hints age out

A hint SHALL expire `ttl_ms` after its last report. Expiry SHALL be checked by
the server loop on a deadline, not during rendering. A source that has an
open dialog SHALL re-report it every 5 seconds so a long dialog stays blocked
and a dead source ages out.

#### Scenario: The source died

- **WHEN** a `question` hint is live and no report arrives for 15 s
- **THEN** the hint is gone and the pane shows what the screen shows

### Requirement: No hint, no change

Without a live hint detection SHALL behave as it did before hints existed, and
no hint code SHALL run per rendered frame.

#### Scenario: A pane without the mod

- **WHEN** a Claude pane has never reported a hint
- **THEN** its blocked state and reason are exactly the screen's

### Requirement: A held hint has a ceiling

The mod SHALL stop reporting a dialog that has been open for 30 minutes, clear
it, and log one failure line, so a missed close cannot hold a pane blocked for
as long as Claude runs.

#### Scenario: A close that never came

- **WHEN** a dialog has been open 30 minutes in the mod's map
- **THEN** the mod clears the hint, drops the entry, and logs once

### Requirement: The mod reports only what is open

The Claude Code mod SHALL report a `question` when `AskUserQuestion` opens for
the main agent. It SHALL clear when the call settles or aborts, on
`turn.complete`, and on `session.end`, and SHALL spawn no process for a clear
when nothing is open. It SHALL log only a failed report. It SHALL NOT report
permission prompts (the herdr API still accepts them).

#### Scenario: A turn ends with nothing open

- **WHEN** a turn completes and no dialog is open
- **THEN** the mod starts no process

#### Scenario: A subagent

- **WHEN** a subagent calls `AskUserQuestion`
- **THEN** the mod reports nothing

### Requirement: One install path, gated on the Claude Code version

`herdr integration install claude` SHALL install the mod when the installed
Claude Code is version 2.1.287 or later, and SHALL otherwise install the
settings hooks only and say why, without failing. `uninstall` SHALL remove the
mod. Running Claude sessions SHALL keep working without the mod until reloaded.

#### Scenario: Old Claude

- **WHEN** Claude Code is older than 2.1.287
- **THEN** install skips the mod with a one-line reason and exits successfully

#### Scenario: A running session

- **WHEN** the mod is installed while a Claude session runs
- **THEN** that session keeps screen detection until `/reload-plugins` or a
  restart, and nothing fails
