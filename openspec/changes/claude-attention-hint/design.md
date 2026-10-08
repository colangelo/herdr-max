## Decisions

### Herdr's own hook, not the gestore-lab file

| | herdr's own hook | gestore-lab events.jsonl |
|---|---|---|
| Coverage | every pane with the Claude integration | controlled repos with a live controller |
| Fidelity | `PermissionRequest` = dialog shown | mixes denials with dialogs |
| Subagents | filtered (`agent_id`) | not filtered |
| Clear events | UserPromptSubmit, Stop, SessionStart already installed | Stop only |
| Coupling | existing socket report path | file tail, cross-repo format |
| Upstream-able | yes | no |

### Evidence, not authority

`full_lifecycle_hook_authority` stays all-or-nothing and untouched. The hint is
a separate `TerminalState.attention_hint { reason, reported_at, session_id,
seq }`. The effective state takes it only when the screen's answer is weak:
the OSC-title idle rule (priority 250) or the default idle fallback. That is
exactly the missed-dialog case (upstream #4573, #4668: dialog open, screen
idle via the title). A strong screen rule always wins, so the #408 fight
cannot happen: the hint never overrides a real screen reading.

### Clearing

- Same session's next report (UserPromptSubmit, Stop, SessionStart), or a
  report with a newer seq; an older `attention` report after a clear is
  dropped (ordering).
- Process exit, session change.
- A strong screen rule held ~1 s: body working (not the OSC working title,
  which can be stale under a dialog, #3467), screen blocked, or the idle
  prompt box (what returns after Esc).
- 30-minute cap.

### Known residual cases

- Esc while the prompt box is hidden: the hint lives until the next prompt or
  the cap. Measure in a throwaway session.
- Approve then a long tool run: the body spinner clears it.
