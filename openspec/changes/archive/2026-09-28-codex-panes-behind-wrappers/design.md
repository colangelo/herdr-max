## Context

`atuin pty-proxy` is exec'd by the shell rc, so it is the pane's child and its
own process group; the shell it hosts is its child, session leader of a nested
PTY. `detect::nested_agent_job` already looks one PTY down for an agent, but
only from inside the foreground-job probe, and that probe runs on a change of
the pane's foreground group, on content-driven acquisition, or on the
identified-agent recheck.

## Decisions

**Target-shell check.** `available_pane_shell(child_pid, is_wrapper)` accepts
the pane's own shell as before. When the lone foreground process is a
recognised wrapper, it asks for the wrapper's nested PTY *with its owner* and
applies the same lone-leader check to that owner. A command or a subshell in
front of the nested PTY is busy. Platform code takes the wrapper predicate as a
parameter, so the wrapper list stays in `detect`.

**Wrapped-shell watch.** A probe that finds a wrapper reports the hosted shell
and the nested foreground group it saw. Each tick then reads that shell's
foreground group; a change, or the shell vanishing, forces a probe (even when
hook lifecycle authority is active, as an outer group change would) and counts
as a group change for acquisition. The tracked outer group is unchanged, so the
existing guard against re-probing a wrapped pane on every tick still holds.
Cost: one `tcgetpgrp`-equivalent per tick for wrapped panes, none for others.

**Naming a thread herdr did not launch.** The daemon's `thread/read` exposes no
client or pid, so the anchor is the pane's Codex process, found by the same
foreground job (and wrapper descent) detection uses. Its argv gives the id after
`resume`; otherwise its start time bounds discovery to threads created in
`[start - slack, start + 30 s]`, which Codex does at startup. Rename tries, in
order: the reported session id, the thread named with the old agent name, then
this anchor. The upper bound keeps a later hand-launched pane's unnamed thread
from being taken for this one.

## Risks

- Two Codex processes started within the window in one cwd: ambiguous, named
  none, as for `agent start`.
- Process start time needs a platform read (macOS `proc_bsdinfo`, Linux
  `/proc/<pid>/stat` plus boot time); Windows returns none and naming falls back
  to the old behaviour.
