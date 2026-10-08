# Design — pane cwd through a wrapper

## Decision 1: publish a resolved cwd, do not make `cwd` walk

`PaneRuntime::cwd` is called per pane per render — the sidebar derives workspace
display names from it — so the multiplicative-performance rule forbids putting a
process-tree walk behind it. The respawn path worked around that with a separate
keypress-time `interactive_cwd`, which fixed one caller and left the rest wrong.

Instead the runtime gains a resolved-cwd cell that the per-pane detection task
fills in, and `cwd` reads it. `cwd` stays a lock-and-clone, callers stay
unchanged, and every consumer — sidebar branch, workspace name, split
inheritance, `pane.get` — becomes correct at once.

Resolution order in `cwd`, most authoritative first:

1. **The shell's own OSC 7 report.** The shell claiming its directory beats
   anything inferred from the process table, and it is the only source that
   survives a shell running somewhere Herdr cannot inspect.
2. **The resolved cell.** What the detection task last saw.
3. **The direct child's cwd.** The current behaviour, and the answer before the
   task's first refresh lands.

## Decision 2: the detection task owns the refresh

The task already exists per pane, already runs off the render thread, and
already calls `platform::nested_foreground_job` for nested-PTY agent detection.
It is the natural owner, and adding a second timer would duplicate a loop that
is already ticking.

The refresh is deliberately **not** folded into the existing agent probe, whose
throttle is driven by foreground-process-group changes. `cd` is a shell builtin:
it changes no process group, so it triggers no agent probe. A pane that is
`cd`-ed and then left alone must still report its new directory, so the cwd
refresh runs on its own fixed interval.

One second. The pane's directory is used for display and for seeding new panes,
neither of which needs sub-second freshness, and a slower interval would be
visible as a stale branch right after `cd`.

## Decision 3: descend exactly one nested PTY

`platform::nested_foreground_job` already encodes the rule: follow only a child
holding a controlling terminal *different from* the parent's. A child sharing the
pane's terminal is an ordinary child, and following it would turn this into a
process-tree walk. One level covers a wrapper that re-runs the shell, which is
the observed case.

Within that nested job, the leader's directory wins, falling back to any member
that can answer. The leader is the process the user is sitting in front of; a
member that merely inherited a different directory must not outrank it.

## Decision 4: cost, and the optimization deliberately not taken

Per pane, once per second, off the render thread: one child enumeration, one
controlling-terminal read per child, and a foreground-job lookup plus a cwd read
for the nested job. Bounded by the pane's own children, not by the process table.

The obvious optimization is to cache the nested child's pid so the steady state
is a single `process_cwd` syscall. It is not taken here because it adds a
liveness question (the cached pid dies, or the pane gains a wrapper mid-session
via `ssh` or `docker exec`) for a cost that has not been measured to matter.
Recorded so a later profile has somewhere to start rather than rediscovering it.

## Decision 5: `interactive_cwd` goes away

It exists only because `cwd` was wrong and the render path could not afford the
fix. With the fix published through the cell, keeping a second cwd accessor
would leave two answers to one question and invite them to drift. The respawn
path moves back to `cwd`.

## Alternatives considered

- **Fix only the git-identity lookup.** It runs on its own timer, off the render
  thread, so it could walk the tree directly. Rejected: it fixes the sidebar
  branch and leaves the workspace name, split inheritance, and `pane.get`
  reporting the launch directory — three surfaces disagreeing about one pane.
- **Make the workspace resolve identity from `foreground_cwd`.** That answers a
  different question — where the *running command* is — so a pane's identity
  would move when a command `cd`s and move back when it exits.
- **Have the shell integration report cwd.** Herdr's integration can emit OSC 7,
  but a wrapper setup is exactly where the integration may not be installed in
  the inner shell, and Herdr cannot require it.
