# Design

## An allowlist, not a process-tree walk

The obvious general fix is to walk the pane child's whole subtree, ignore PTY
boundaries, and take the best agent match. It is rejected on two grounds.

The first is correctness, and it is the serious one. A process-tree walk cannot
tell "the agent this pane is running" from "an agent process that happens to be
underneath this pane". A pane whose shell has a stray background `claude`, a
pane that ran an agent once in a subshell that has not yet reaped, a nested
Herdr, or `ssh` to a box where the remote side is irrelevant — each yields a
confident, wrong answer. The failure is silent: the sidebar names an agent and
attributes state to it, and nothing about the display suggests the attribution
is guesswork. Under-detection is visible and reportable; misattribution is not.

The second is cost. Enumerating a subtree is unbounded in the number of
processes, on a path that is sampled per pane. The existing scan is bounded by
construction: one `tcgetpgrp`, then the members of exactly one process group.

The allowlist keeps that shape. A wrapper is recognised by process name, the
descent is one job lookup, and the total work on a wrapped pane is two bounded
scans instead of one. On an unwrapped pane the allowlist check is a string
comparison against the leader's name and nothing else changes.

The cost of the allowlist is that a wrapper nobody has named is invisible. That
is the right failure: it is the behaviour we have today, it degrades to
"no agent" rather than "wrong agent", and adding a name is a one-line change
with a test.

## Descend one level, and only from the leader

Depth is one. A wrapper inside a wrapper is not a configuration anyone runs on
purpose, and every level of permitted recursion multiplies both the cost and the
number of ways a wrong process can be reached. If a second level ever turns out
to matter, raising a constant is a smaller change than having guessed wide now.

The descent starts only from the job's **process group leader**. The leader is
the process the pane's PTY handed control to, so it is the only member that can
legitimately own a nested PTY on the pane's behalf. A non-leader member matching
the allowlist is a wrapper that some other process started, and following it
would be the same guesswork the allowlist exists to avoid.

## Only when the pane's own job yields nothing

The nested lookup runs only after identification on the pane's own job has
already failed. A pane whose PTY does carry a recognisable agent is answered
without any extra work, and an agent that is somehow visible at both levels
resolves to the outer one — the process the pane actually launched.

This also keeps the change off the fast path entirely. `should_probe_foreground_job`
already gates probing behind a foreground-process-group change and the
acquisition/recheck intervals, so the descent inherits that throttling: it can
only run on a probe that was going to happen anyway, on a pane that is wrapped,
in the case where the answer would otherwise have been "no agent".

## The identified job is the job everything is read from

`probe_foreground_process_from_jobs` derives more than the agent from a job: the
process name it reports, and the working directory the pane advertises. Today a
wrapped pane reports the wrapper's cwd, which is why the pane in the original
report claimed a directory its session had never been in.

Once the descent identifies an agent in the nested job, that job becomes the one
those facts are read from. Anything else would fix the sidebar while leaving the
pane describing itself wrongly, and the two would disagree with each other.

A pane where the descent finds nothing keeps reporting the outer job, exactly as
now — there is no better answer available and the wrapper's cwd is at least a
real directory the pane is associated with.

## Platform shape

The lookup a platform must provide is "given a wrapper pid, the foreground job
of the PTY it owns". Each platform already has the second half:
`foreground_job(pid)` resolves a pid's controlling terminal and enumerates its
foreground group. So the new primitive only has to find the wrapper's child and
hand it to the existing one, which keeps the platform-specific surface to child
enumeration:

- macOS: children by `proc_bsdinfo`'s `pbi_ppid`, alongside the existing
  `proc_bsdinfo` use.
- Linux: `/proc/<pid>/task/<pid>/children`, or the ppid field of `/proc/*/stat`.
- Windows: returns nothing. ConPTY wrappers do not present this shape and there
  is no evidence of the failure there; a stub keeps the contract total rather
  than pretending to support it.

A child on the *same* controlling terminal as the wrapper is not a nested PTY —
it is an ordinary child — and must not be followed. Requiring a different
controlling terminal is what makes this specifically about nested PTYs rather
than a one-level process-tree walk wearing a disguise.

## Naming the allowlist

`atuin pty-proxy` is the entry the evidence supports, matched on the process
name `atuin`. The list lives beside the agent names in `src/detect/`, as data
with a comment recording why each entry is on it, so adding one is obviously a
data change and not a design decision.

Matching on the bare process name is deliberately loose: it will also match
`atuin` invoked as something other than the proxy. That is acceptable because a
false match costs one extra bounded lookup that finds no nested PTY and returns
nothing — the same answer as not matching at all.

## Alternatives considered

**Trust the reported `agent_session` instead.** A terminal that an integration
reported a session for could be identified from that report when process
detection finds nothing. It is nearly free and needs no platform work. Rejected
as the primary mechanism: it only helps agents whose integration reports, so it
would fix Claude and leave every other agent behind the same wrapper broken, and
it widens who may name an agent — detection authority — to solve what is
actually a process-visibility problem. It remains available as a later,
independent improvement.

**Make Herdr set an environment marker that stops wrappers engaging.** Herdr
could suppress the wrapper rather than see through it. Rejected: the wrapper is
doing something the user asked for, and per-pane output capture inside Herdr is
only possible from inside the pane. Suppressing it would trade a Herdr display
bug for silently breaking a working Atuin feature.
