Ordered so each group lands green on its own. Group 1 is the resolution itself,
group 2 publishes it into the runtime, group 3 removes the workaround it
replaces.

## 1. Resolve a directory through one nested PTY

- [x] 1.1 A pure resolver in `src/pane.rs` taking the pane's child pid plus
      injected process lookups, so the ordering rules are testable without a
      real PTY: nested job first, direct child second; within a job the leader
      first, then any member that can answer
- [x] 1.2 Tests: leader wins over an earlier member; falls back to a member;
      falls back to the direct child when there is no nested job; `None` when
      nothing can answer

## 2. Publish it from the detection task

- [x] 2.1 A resolved-cwd cell on `PaneRuntime`, filled by the per-pane detection
      task on its own fixed interval, independent of the agent-probe throttle
- [x] 2.2 `PaneRuntime::cwd` reads reported, then resolved, then the direct
      child; the walk never runs behind `cwd`
- [x] 2.3 Tests: `cwd` prefers a reported directory over a resolved one, and a
      resolved one over the direct child; refresh cadence is not gated on a
      foreground process group change

## 3. Remove the respawn workaround

- [x] 3.1 Delete `PaneRuntime::interactive_cwd` and its `TerminalRuntime`
      wrapper; point `respawn_pane_runtime` at `cwd`
- [x] 3.2 Keep the respawn cwd test passing against `cwd`
- [x] 3.3 `just check` green; dogfood on `-ac-beta`: the `inbox-management`
      space shows its branch, and a split of a wrapped pane starts in the
      shell's directory
