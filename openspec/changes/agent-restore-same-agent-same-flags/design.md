## Inventory: what each kind restores as

| Pane kind | Session id from | Flags from | Restore command |
|---|---|---|---|
| claude (plain) | Claude hook | hook (mode/model/effort) + launch record | `claude --resume <id> [launch flags] --model … --effort … --permission-mode …` |
| claude via `--settings` (GPT) | Claude hook | same; `--settings` from the launch record | `claude --resume <id> --settings <abs file> --model gpt-6-astra …` |
| codex, hand-started | Codex hook; else `resume <id>` in its argv | launch record | `codex resume <id> -m … -s … -c k=v …` |
| codex on herdr's daemon (#85) | naming thread discovery (unchanged) | launch record | `codex resume <id> [flags] --remote unix://… -C <cwd>` (as today + flags) |
| pane ran claude, then codex | codex (claude's dropped on the agent change) | codex's launch record | codex's command |
| codex run by claude's tools | claude (codex never holds the foreground) | claude's | claude's command |

## Decisions

### Launch flags are read by the server, not the hook

The server has each process's exact argv (`KERN_PROCARGS2` on macOS,
`/proc/<pid>/cmdline` on Linux) and already walks into wrapper PTYs to find
the agent. That keeps quoting, covers codex and claude alike, and needs no
integration change. The Claude hook keeps reporting what changes during a
session (mode, model, effort); the server adds what only the launch knows.

The read happens once per agent change in a pane (the detected agent or its
process id changes), off the render path, reusing the naming code's
`agent_processes_in_job` + `wrapped_shell_job` lookup. Only carry-listed flags
are stored, so prompts and one-shot values never reach `session.json`.

### Composition, not replacement

The reported resume command stays the base (it tracks live mode changes).
Launch flags are appended only when the base does not already carry that flag,
so a hook value (the model the session switched to) wins over the launch one.

### Agent identity guards (three layers)

1. On agent change: drop the saved session and launch record of the old agent
   (the reported resume already was). Detection names the foreground agent,
   so a tool-run codex under claude is not a change; a claude suspended with
   ctrl+z while codex runs in the foreground is, and restores as codex.
2. At snapshot: skip a session whose agent differs from a known detected agent.
3. On report: a SessionStart report for agent B refused because detection
   still says A, or because A's session is on record, is held for 60 s and
   taken when detection sees B. A report not applied is logged at `info`.
4. A Codex launched with `--no-daemon` is restored without `--remote`.

### Dry run as a field, not a command

`restore_argv` on the agent session info in `pane list`/`pane get` costs one
composition per pane per request (not per frame) and uses the same code the
restore uses. An offline `herdr session restore-plan` reading `session.json`
would be 150-250 lines more; not needed while the server is up, and after a
crash `session.json` itself holds the parts.

## Risks

- A wrapper that hides the agent's argv (no `KERN_PROCARGS2` access): no launch
  record, restore behaves as today.
- A carried relative path whose base directory is gone: Claude/Codex report the
  error in the pane, as with any bad flag.
- `validate_resume_argv` rejects apostrophes; a `-c key='v'` value would drop
  the whole command. The composer skips a flag whose value would fail
  validation instead of losing the resume.

## Testing (a unit test per kind, plus the proof)

- Carry tables: claude GPT argv → `--settings` kept, prompts dropped; codex
  hand-started argv → `-m -s -c` kept, `resume <id>` and prompt dropped;
  relative paths absolute; `--flag=value` forms.
- Composer: hook argv + launch flags without duplicates; codex built-in +
  flags; daemon codex keeps `--remote`.
- Guards: claude session then codex detected → codex restores; codex inside a
  live claude → claude kept; snapshot skips a mismatched session; a codex
  SessionStart report replaces a saved claude session before detection.
- Persistence: old `session.json` loads; new field round-trips.
- Proof: throwaway session with one pane of each kind (compiled stand-in
  `claude`/`codex` binaries, hooks fed by wrapper scripts), a real server stop
  and restart, every restored pane's argv read back with `ps`.
