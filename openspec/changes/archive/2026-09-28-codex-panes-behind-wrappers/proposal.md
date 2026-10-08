## Why

On m4m every shell runs behind `atuin pty-proxy`, which re-runs the shell in a
PTY of its own. Two things break for Codex panes there
(https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/87):
`agent start` refuses the pane as busy, because its foreground process is the
wrapper, and an agent launched behind the wrapper is found only if a
content-driven acquisition window happens to cover its start, because the
pane's own foreground group never changes.

A Codex pane launched by hand also never gets its daemon thread named: rename
looks the thread up by the name herdr gave it, and herdr gave it none
(https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/86).

## What Changes

- `agent start` looks through a recognised PTY wrapper to the shell it hosts,
  and accepts the pane only when that shell is alone at the front of the
  nested PTY.
- Detection watches the foreground group of the shell a wrapper hosts, for
  wrapped panes only, so a command starting behind the wrapper triggers a probe
  the way an unwrapped pane's group change does.
- `agent send`/`keys` find an agent behind the wrapper.
- Renaming a Codex pane whose thread herdr does not know finds it from the
  pane's Codex process: the thread id after `resume`, else the single unnamed
  top-level thread in the pane's cwd created within a short window after that
  process started. Ambiguity names none.

## Impact

- `src/platform/{mod,macos,linux,windows,fallback}.rs`, `src/detect/mod.rs`,
  `src/pane.rs`, `src/app/agents.rs`, `src/codex_app_server.rs`.
- Wrapped panes gain one foreground-group lookup per detection tick. Unwrapped
  panes are unchanged.
