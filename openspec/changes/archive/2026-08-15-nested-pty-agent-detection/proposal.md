# Detect an agent running behind a nested PTY wrapper

## Why

Agent identification reads the foreground job of the pane's own PTY.
`platform::foreground_job` resolves `tcgetpgrp` on the pane child's controlling
terminal and enumerates that process group, and
`detect::identify_agent_in_job` looks for a known agent name among its members.

A shell wrapper that allocates its own PTY defeats that entirely. Atuin's
`pty-proxy` — shipped in v18.13, on by default for anyone who runs its init
snippet — execs the shell inside a nested PTY so it can read OSC 133 marks and
capture each command's output. The pane's PTY then carries only
`atuin pty-proxy`, and the agent lives one PTY down where the scan cannot reach:

```
herdr server
  └─ atuin pty-proxy --shell /bin/zsh   (ttys005)  <- the pane's PTY
       └─ /bin/zsh                      (ttys006)  <- the wrapper's nested PTY
            └─ claude                   (ttys006)
```

The pane is then not an agent as far as Herdr is concerned: absent from the
agents sidebar and from `agent.list`, unresolvable by `agent.get`, `agent.read`,
`agent.send`, `agent.wait` and `agent.explain`, and contributing no state to its
workspace. Nothing reports the gap — the pane simply looks like a plain shell
that happens to be printing an agent's UI.

Every fact derived from the same job is wrong in the same way. A wrapped pane
reports the wrapper's `cwd` as its own, so a session working in one repository
is reported as working wherever the wrapper was started.

This is not a rare configuration. Atuin is widely used, the wrapper is its
recommended setup for output capture, and it wraps every interactive shell it
initialises. Once a user enables it, *every* agent pane they open from then on
is undetectable, while panes opened earlier keep working — so the failure
arrives as a slow, silent erosion rather than an obvious break.

## What Changes

When the foreground job of a pane's PTY yields no agent and its process group
leader is a recognised PTY wrapper, identification descends one level into the
wrapper's nested PTY and identifies there. Everything the probe derives from a
job — the agent, its process name, and the reported working directory — comes
from the job that was actually identified.

Recognition is an explicit allowlist of wrapper process names, not a general
walk of the process tree. A wrapper the list does not name behaves exactly as
today.

## Impact

- Affected capability: `agent-process-detection` (new)
- Affected code: `src/detect/mod.rs` (identification entry point), `src/pane.rs`
  (`probe_foreground_process_from_jobs`), `src/platform/mod.rs` and the three
  platform implementations (a nested-job lookup)
- No server, API, protocol or config surface: the detection result is reported
  through the fields that already carry it. Panes that are not wrapped take an
  identical path to today.
- Upstream: nested-PTY wrappers are not fork-specific and Herdr has no opinion
  of its own here, so this is shaped to be liftable as-is.

## Non-goals

- Following the process tree past a wrapper the allowlist does not name. That is
  the general-walk design this change deliberately rejects; see `design.md`.
- Detecting an agent running on another host through `ssh`. It is out of reach
  of any local process scan, wrapper or not.
- Teaching Atuin about Herdr. Giving its init snippet a Herdr pane identity to
  key on, the way it already keys on `$TMUX`, is an upstream Atuin change and is
  tracked separately.
- OSC 133 handling in Herdr's own terminal. Worth having, unrelated to this.
