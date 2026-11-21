# Design

## D1. Where status enters

`App::emit_pane_state_update` (`src/app/api.rs`) is the one place a pane's agent status
change is published (`pane.agent_status_changed`). With the flag on, it also hands the
pane's runtime a `CoreStatusRecord` built from the same `AgentStatus` and agent label.
Runtimes that are not on the core backend ignore it. Turning the flag on or off on reload
runs the same hand-off over every pane (`App::sync_core_status_mirror`).

## D2. The record type

```rust
pub(crate) enum CoreStatusRecord {
    Clear,
    Report { word: CoreStatusWord, app: Option<CoreStatusApp> },
}
pub(crate) enum CoreStatusWord { Idle, Working, Blocked, Done }
pub(crate) struct CoreStatusApp(String); // 1..=64 bytes of [A-Za-z0-9._+-]
```

`CoreStatusApp::parse` refuses any other label (it is omitted, not rewritten), so a
label can never carry `:`, `;`, ESC, BEL or a control byte into the sequence.
`encode()` gives `ESC ]7501;state=<word>[:kind=permission][:app=<app>] ESC \`, always
under 256 bytes. No `id` field: it is the root record.

## D3. Where the write happens

Each core pane already has a parser thread that receives the pane's output stream from
core. It owns a `StatusMirror`:

- `EscapeScanner`, a byte state machine (ground, escape, escape intermediate, CSI,
  string for OSC/DCS/SOS/PM/APC, string-escape, UTF-8 continuation). CAN and SUB cancel
  to ground, as in a VT parser. It runs on every chunk before the chunk is parsed: O(n),
  no allocation; in ground it skips plain ASCII until the next ESC, CAN, SUB or
  non-ASCII byte.
- the time of the last output byte;
- the pending record (latest wins) and the last record written.

The server's thread puts a record in the pane's one-slot mailbox (a mutex over
`Option<CoreStatusRecord>`) and wakes the parser only when the slot was empty, so a
burst of changes leaves one record, never a backlog. While a record is pending the
parser waits for output with a timeout and writes when the gate opens: 100 ms since the
last output byte, and the scanner in ground. A record equal to the last one written is
dropped; so is a `clear` before anything was written (a plain shell pane gets no bytes).

## D4. The write

Platform code (`src/platform/`): the program's controlling terminal from its pid
(`proc_pidinfo(PROC_PIDTBSDINFO)` `e_tdev` and `devname` on macOS; `tty_nr` from
`/proc/<pid>/stat`, matched against `/proc/<pid>/fd/{0,1,2}` on Linux). The device is
opened `O_WRONLY | O_NOCTTY | O_NONBLOCK | O_CLOEXEC`, so herdr never gains it as a
controlling terminal. Before writing, `fstat` must show a character device with the
expected device id, owned by our effective uid, and `isatty` must hold. The record goes
in one `write`. A short write finishes with a bounded retry so a cut record cannot leave
core's parser inside an OSC. `EAGAIN` keeps the record pending and retries; any other
error drops it with a debug log. Other Unix: no device, nothing written. Windows: the
core backend does not exist.

## D5. No feedback into herdr

The record comes back in the pane's own core stream. herdr's terminal (libghostty-vt)
ignores OSC 7501: the screen and detection text do not change. herdr does not subscribe
to core events (M3) and never reads core's `status`. Tests assert both.

## D6. Limits

- The gate narrows but cannot close the race with a program that starts an escape
  sequence after the gate check: core's stream lags the program's writes. 100 ms of
  quiet makes it unlikely; a cut sequence would show as one garbled sequence, as with
  `write(1)`.
- Core itself clears status when the root interactive shell returns to the foreground;
  that is core's rule and matches herdr's release-to-clear.
- A record a previous server wrote stays when the agent ended during a handoff and the
  new server never sees it, until the program exits or a new status arrives.
- herdr turns `done` into `idle` when the pane is viewed, without a state update; the
  mirror keeps `done` until the next status change.
- A program whose output never pauses for 100 ms (a fast animated spinner) delays its
  record until it does. Whether a working agent's screen pauses that long is not
  measured yet; the beta check (verifier 16) shows it.

## Verifier table (required tests)

The answer key for 2, 12 and 13 is herdr-core itself: its OSC 7501 encoder
(`status.rs` `encode`) and its stored `status` as `hc get --json` reports it. Rows 1-11
are unit tests next to the code; 12-15 run with `just core-live-test` against a
throwaway `hc` 0.1.21; 16 is ac's own check on the phone.

| # | Proves | Where | Pass when |
|---|---|---|---|
| 1 | status → record mapping | unit, `src/app/core_status_mirror.rs` | working/blocked/done/idle map to their words; unknown, released and no agent map to `clear` |
| 2 | encoding is core's format | unit, `src/pane/runtime_io/core_status.rs` | bytes equal `ESC ]7501;state=…[:kind=permission][:app=…] ESC \`, no `id`, < 256 bytes |
| 3 | unsafe labels refused | unit, `core_status.rs` | `:`, `;`, ESC, BEL, control, empty and 65-byte labels give no `app` field |
| 4 | scanner ends in ground after whole sequences | unit, `core_backend/status_mirror.rs` | text, CSI, OSC by BEL and by ST, DCS, APC, CAN and SUB all end in ground |
| 5 | scanner state across chunk splits | unit, `status_mirror.rs` | for every split point inside a sequence or a UTF-8 char, the first half is not ground and the whole is |
| 6 | idle gate | unit, `status_mirror.rs` | not due before 100 ms quiet; not due while an OSC is open; due once it ends and 100 ms pass |
| 7 | coalescing | unit, `status_mirror.rs` | three offers while closed write only the last; a repeat of the last written and a first `clear` write nothing |
| 8 | checked write | unit (Unix), `src/platform/unix_common.rs` | a regular file is refused; a fresh pty slave gets exactly the record bytes on its master |
| 9 | config key | unit, `src/config/model.rs` | default false; `core_status_mirror = true` parses true |
| 10 | server hand-off | unit, `src/app/core_status_mirror.rs` | flag on: a state update gives the pane's runtime the mapped record; flag off: none |
| 11 | no feedback into herdr | unit, `src/pane.rs` tests | after the record bytes, visible and detection text are byte-identical and no reply goes to the program |
| 12 | live: core stores the mirror | live, `src/pane/core_backend_live_tests.rs` | `hc get --json` shows `status.state=working`, `app=claude`; after `clear`, no `status` |
| 13 | live: echo is inert | live, same test | no `HookStateReported`/`AgentHintReported`, screen text unchanged |
| 14 | M3 unchanged | live, `core_v8_printed_status_is_not_an_agent_report` | passes unchanged |
| 15 | server path: config, hook report, mirror | live, `scripts/core_live_test.sh` (V11 step) | with `core_status_mirror = true`, a `report-agent --state working --agent pi` in a core pane gives `hc get` status `working pi` |
| 16 | the phone's Inbox | ac, Herdr Mobile on m4m with a beta that has the flag on | a working agent pane appears in the Inbox (not run by the builder) |
