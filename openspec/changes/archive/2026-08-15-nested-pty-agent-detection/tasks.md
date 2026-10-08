Ordered so each group lands green on its own. Group 1 is the platform primitive
with no behaviour change; group 2 makes wrapped panes identify; group 3 corrects
the facts a wrapped pane reports.

There is a live reproduction on this machine — a Claude session behind
`atuin pty-proxy` — so group 5 can verify against a real wrapped pane rather
than only a constructed one.

## 1. Nested job lookup

- [x] 1.1 Add `nested_foreground_job(pid) -> Option<ForegroundJob>` to `src/platform/mod.rs`, documented as "the foreground job of the PTY this process owns", returning `None` when the process owns no PTY
- [x] 1.2 Implement it on macOS by finding children via `proc_bsdinfo`'s `pbi_ppid`, keeping only a child whose controlling terminal differs from the parent's, and delegating to the existing `foreground_job`
- [x] 1.3 Implement it on Linux over `/proc`, with the same different-controlling-terminal condition
- [x] 1.4 Stub it on Windows returning `None`, with a comment recording that ConPTY wrappers do not present this shape
- [x] 1.5 Tests: a child on the same controlling terminal is not followed; a process with no children returns `None`. Keep these to the testable contract in `src/platform/mod.rs` rather than asserting against live process tables

## 2. Identification descends into a recognised wrapper

- [x] 2.1 Add the recognised-wrapper set to `src/detect/mod.rs` as data beside the agent names, seeded with `atuin` and a comment saying why it is there
- [x] 2.2 In `probe_foreground_process_from_jobs` (`src/pane.rs`), when the foreground job yields no agent and its process group leader is a recognised wrapper, identify from `nested_foreground_job` of that leader
- [x] 2.3 Hold the descent to one level and to the process group leader only
- [x] 2.4 Tests on constructed jobs: a wrapper with an agent behind it identifies; an unrecognised wrapper does not descend; a wrapper with nothing behind it reports no agent; an agent in the pane's own job wins over one below it and skips the lookup entirely

## 3. Pane facts follow the identified job

- [x] 3.1 Read the reported process name and working directory from the job the agent was identified in, so a wrapped pane stops reporting the wrapper's cwd
- [x] 3.2 Leave panes with no nested identification reporting their own foreground job, unchanged
- [x] 3.3 Tests: a wrapped pane reports the nested job's cwd and process name; an unwrapped pane's reported facts are byte-identical to today's

## 4. Docs

- [x] 4.1 Troubleshooting entry in `docs/next/website/src/content/docs/troubleshooting.mdx` for an agent pane missing from the sidebar, naming Atuin's PTY proxy and saying which wrappers are recognised
- [x] 4.2 Changelog entry under `docs/next/CHANGELOG.md`

## 5. Verification

- [x] 5.1 `just check` green
- [x] 5.2 Confirm the unwrapped path is unchanged: with a wrapped and an unwrapped pane side by side, the unwrapped pane's probe performs no nested lookup
- [x] 5.3 Dogfood on `-ac-beta` against the live wrapped Claude pane: it appears in the agents sidebar, resolves as an `herdr agent` target, and reports its own working directory rather than the wrapper's
- [x] 5.4 Confirm the diff carries no fork-specific opinion, so it can be lifted upstream as-is

5.1 is green on all five CI jobs of `76c68f23`, including the two known-flaky
ones.

5.2 was confirmed against the live process table rather than only constructed
jobs, with both shapes present at once. On the wrapped pane
(`atuin pty-proxy` at pid 6302), `detect::nested_agent_job` returned
`(7625, Claude, "claude")` — the real Claude one PTY down. On an unwrapped pane
whose leader is Claude itself (pgid 17385), it returned `None` having made
**zero** nested lookups, so the descent costs an unwrapped pane one string
comparison and nothing else. The probe was driven through a throwaway test that
was removed afterwards, per the group 1 note about keeping committed tests to
the pure contract.

5.3 ran on `0.8.0-ac-beta.59-conceicao`, the combined build that also carries
the alt-screen scroll passthrough work, against the same live wrapped pane the
report was filed from — `wG:p1`, an `atuin pty-proxy` on ttys005 with `zsh` and
Claude on ttys006. All three checks pass:

- It appears in `agent list` as `claude`, `agent_status: working`, having been
  absent entirely before.
- It resolves as an `herdr agent` target: `agent get`, `agent read` and
  `agent explain` all answer for it. `agent explain` previously failed with
  `agent_not_found`, so it is a direct before/after.
- `foreground_cwd` is `/Users/ac/_sync/dev/_mcp/protonmail-imap`, the agent's
  own directory, where the wrapper sits in `/Users/ac/_sync/dev/direction`.

One field is knowingly left reading the wrapper: `PaneInfo.cwd`, which is the
pane *shell's* directory rather than a foreground-job fact, and reaches the API
through `PaneRuntime::cwd()` — OSC 7 if the shell reported one, else the pane
child's cwd. Atuin's proxy captures output but does not forward the inner
shell's OSC 7, so the fallback lands on the wrapper. It is out of this change's
scope by the same reasoning as the "Teaching Atuin about Herdr" non-goal, and
the two fields are expected to differ anyway — an unwrapped pane running
pyright shows the same split. Nothing user-facing depends on it here: the
workspace label resolves to `protonmail-mcp`, and new panes inherit through
`follow_cwd`, which does follow the identified job.
