# Report the pane's real working directory through a wrapper process

## Why

Herdr answers "what directory is this pane in?" from the pane's direct child
process. That is correct only when the pane's child is the interactive shell.

When a wrapper re-runs the real shell inside a PTY of its own — `atuin` on this
machine, but equally a container shim, an agent runner, or `script` — the direct
child never leaves the directory it was launched in. Herdr then reports the
launch directory forever, no matter where the user has actually `cd`-ed.

Measured 2026-08-16 on `0.8.0-ac-beta.62`:

```
pane wH:p1, shell prompt in /Users/ac/_sync/dev/inbox-management
  child      = atuin, cwd /Users/ac/_sync/dev/_mcp/protonmail-imap
  grandchild = zsh,   cwd /Users/ac/_sync/dev/inbox-management
herdr reports /Users/ac/_sync/dev/_mcp/protonmail-imap
```

Everything derived from a pane's directory is wrong in that setup:

- The workspace shows no branch, because its git identity is resolved from the
  wrapper's launch directory, which is not a repository. Re-discovery on the
  five-minute timer cannot help: the path it re-discovers from is the wrong one.
- The workspace's automatic name is taken from the same wrong directory.
- `pane.split` and new tabs inherit the launch directory instead of the one the
  user is standing in.
- `pane.get` / `pane.list` report a stale `cwd`.

Herdr already knows how to cross this boundary: `platform::nested_foreground_job`
exists for nested-PTY agent detection, and the per-pane detection task already
calls it every probe. What is missing is that nobody publishes the *directory*
it finds.

## What Changes

- **The pane runtime keeps a resolved working directory**, refreshed by the
  per-pane detection task that already walks this process tree, and consulted by
  `PaneRuntime::cwd` after the shell's own OSC 7 report and before the direct
  child's cwd. No caller changes, no new work on any render path.
- **The resolution descends exactly one nested PTY**, reusing
  `platform::nested_foreground_job`: a child holding a controlling terminal of
  its own. An ordinary child sharing the pane's terminal is not followed, so this
  never becomes a process-tree walk.
- **The refresh has its own cadence**, independent of the agent-probe throttle,
  so a pane that merely `cd`s — a shell builtin, which changes no process group
  and therefore triggers no agent probe — still reports its new directory.
- **`PaneRuntime::interactive_cwd` is removed.** It was added by `respawn-pane`
  as a keypress-time workaround for exactly this gap; with `cwd` correct, the
  respawn path uses `cwd` like every other caller.

## Impact

- Affected specs: `pane-working-directory` added.
- Affected code: `src/pane.rs` (the resolved-cwd cell, its refresh in the
  detection task, `cwd`, removal of `interactive_cwd`),
  `src/terminal/runtime.rs` (wrapper removal), `src/app/api.rs` (respawn uses
  `cwd`).
- No API, protocol, config, or keybinding surface changes. Existing fields
  (`cwd`, workspace branch, split inheritance) start reporting the right
  directory in wrapper setups and are unchanged everywhere else.
- Fork tracking: https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/63.
  Builds on `respawn-pane`; designed to be upstream-PR-able.

## Non-goals

- Following more than one nested PTY. One level covers a wrapper that re-runs
  the shell; a tower of them is not a case Herdr has seen.
- Reporting the foreground *command*'s directory. `foreground_cwd` already
  answers that question separately, and conflating the two would make a pane's
  directory jump around while a command runs.
- Changing how a workspace picks which pane supplies its identity.
- Making `terminal.cwd` (the persisted launch directory) track the shell. It is
  updated by OSC 7 reports today and that stays as it is.
