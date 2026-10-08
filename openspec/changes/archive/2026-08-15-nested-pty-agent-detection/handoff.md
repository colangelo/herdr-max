# Implementing nested-pty-agent-detection

`proposal.md`, `design.md` and `specs/agent-process-detection/spec.md` carry the
what and the why, and the scope decision is settled — allowlist, one level,
leader only, only after the pane's own job comes up empty. Implement it, don't
redesign it. This file is only the operational things that are not in those.

## Leads that save a research pass

**macOS child enumeration is a near-copy of what is already there.**
`process_group_pids` (`src/platform/macos.rs:325`) calls
`libc::proc_listpids(PROC_PGRP_ONLY, ...)` with a doubling-buffer retry, and
`PROC_PGRP_ONLY` is a hand-defined `const u32 = 2` at the top of the file.
Children are the same call with `PROC_PPID_ONLY` — believed to be `6` in
`sys/proc_info.h`, so verify against the SDK header rather than trusting this
line. Task 1.2 is then a parallel function, not a new technique.

**The controlling-terminal comparison already has a reader.** `process_bsdinfo`
(`macos.rs:757`) returns `libc::proc_bsdinfo`, whose `e_tdev` is the controlling
terminal. "Child on a different controlling terminal" is a comparison of that
field between wrapper and child — no new syscall.

**The integration point is `probe_foreground_process_from_jobs`**
(`src/pane.rs:601`). It already tries, in order: the leader job, then the
foreground job, then several hint paths. The descent belongs after the final
`identify_agent_in_job(job)` on the foreground job comes back `None`. Note the
function returns a `ProcessProbeResult` carrying `process_group_id`,
`foreground_is_pane_shell`, `agent` and `process_name` — group 3 is about which
job those last two are read from.

**Probing is already throttled.** `should_probe_foreground_job` (`src/pane.rs:456`)
gates on foreground-pgid change plus acquisition and recheck intervals. The
descent rides that gate; spec requirement three is about not adding sampling
beside it. Nothing new needs a timer.

## The live reproduction

There is a real wrapped Claude pane on this machine, which is what task 5.3
verifies against. Pids change, so find it by shape rather than by number:

```bash
ps -eo pid,ppid,pgid,tty,stat,command | grep -E "atuin pty-proxy" | grep -v grep
```

Each hit is a pane's direct child on the pane's own PTY. Follow it down:
`pgrep -P <atuin-pid>` gives the shell on a *different* tty, and that shell's
child is the agent. The chain is
`herdr server -> atuin pty-proxy (ttyA) -> zsh (ttyB) -> claude (ttyB)`.

The symptom in the API: `herdr pane list` shows the pane with `"agent": null`
and `"agent_status": "unknown"` while `agent_session` is populated — herdr holds
a Claude session id for a pane it does not think is running Claude. `herdr agent
explain <pane>` fails with `agent_not_found` for the same reason, so it is not
available as a debugging tool here until the fix lands.

Panes wrapped/unwrapped split cleanly by age: `~/.zshrc` gained the
`atuin pty-proxy init` block on 2026-08-14, so older panes are unwrapped and
newer ones are wrapped. Both shapes are present, which is what task 5.2 wants.

## Known-red CI that is not yours

Two intermittent master failures are tracked and were investigated already.
**Do not chase either blind** — check whether the failure is one of these first.

- `app::tab_bar_status::tests::reload_aborts_an_in_flight_command_task_and_its_descendants`
  on macOS. Margin was widened to ~2.4s in `bb0ca1c5`; if it recurs at that
  margin the margin theory is wrong and the next move is written in
  <https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/57>.
- The Windows ConPTY smoke step, intermittently. Its cleanup no longer masks the
  real error (`afd78a87`), so a recurrence should now say what actually failed:
  <https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/56>.

## Sequencing already applied

The environment half of the same issue shipped in `3e3e1ac3`: panes no longer
inherit `ATUIN_PTY_PROXY_ACTIVE` / `ATUIN_PTY_PROXY_TMUX`. That made wrapping
deterministic, so every newly created agent pane is now wrapped and therefore
undetectable until this change lands. Do not "fix" that by reverting it.

## Out of scope, on purpose

Teaching Atuin's init snippet about a Herdr pane identity, the way it keys on
`$TMUX`, is an upstream Atuin change with its own ticket. Herdr already exports
`HERDR_PANE_ID` per pane (`src/pane.rs:159`), which is what such a patch would
key on, but nothing in this change depends on it.
