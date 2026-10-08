Planned 2026-09-29 (JOB 13 from herdr-helper; ac said "build").
Issues: https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/123, /120,
/121; base picks in /124.

## 1. Tests first (red)

- [x] 1.1 #121: manifest tests for `· 2 shells` at end of line and `· 1 shell`, plus the existing `· 2 shells · ← 1 agent` (`src/detect/manifest/tests.rs`)
- [x] 1.2 #123 hook: a test harness in `src/integration/claude_hook_tests.rs` (`tests/cli` is off on macOS) that runs the real `herdr-agent-state.sh` against a fake socket with a fixture transcript and hook input, and checks the reported `resume_argv` for: auto + launched-with-bypass, bypass, plan, nothing known, `<synthetic>` model skipped, input mode beats transcript mode
- [x] 1.3 #123 installer: install writes SessionStart, UserPromptSubmit and Stop entries; install on a canonical file is a byte-exact no-op; install migrates a v8 file (SessionStart only); uninstall removes all three and keeps user hooks
- [x] 1.4 #123 restore: a Claude pane whose reported resume carries the flags restores with exactly that command (`src/persist/restore.rs`)
- [x] 1.5 #120: an API `pane.close` with todos leaves client mode and tokens alone and lists the todos; `force` closes and returns `closed_todos`; `tab.close` and `workspace.close` refuse with todos and accept `force`; the TUI modal flow is unchanged; a ConfirmClose whose pane was closed leaves the mode (`src/app/api/panes.rs`, `tabs.rs`, `workspaces.rs`, `src/app/input/modal.rs`)
- [x] 1.6 #120 CLI: `--force` parses for pane, tab and workspace close (`src/cli/`)

## 2. Build (green)

- [x] 2.1 #121 manifest regex, version bump
- [x] 2.2 #123 hook script (.sh), then the .ps1 counterpart without the process walk
- [x] 2.3 #123 installer: list of canonical entries, removal table
- [x] 2.4 #120 schema params, origin marking in `dispatch_runtime_mutation`, pure gates, forced result, orphan cleanup
- [x] 2.5 #120 CLI flags and help text

## 3. Docs

- [x] 3.1 integrations.mdx: what the Claude hook saves and its limits
- [x] 3.2 cli-reference.mdx and socket-api.mdx: `--force` / `force`, `closed_todos`
- [x] 3.3 regenerate `docs/next/api/herdr-api.schema.json`

## 4. Ship

- [x] 4.1 `just check`, push origin and internal
- [ ] 4.2 one beta; throwaway proof on m4m (stand-in claude, no paid tokens): restore types the reported command; API close with todos shows no modal on an attached client
- [ ] 4.3 comments on #120, #121, #123, #124; report to herdr-helper

Decided while building (recorded so review can check them):
- The hook tests live in `src/integration/claude_hook_tests.rs`, not
  `tests/cli/hooks.rs`: `tests/cli` does not build on macOS, so they would
  never run under `just check` here. They run the real script under a parent
  process named `claude` (`exec -a claude`) to test the launch-flag walk.
- #123's restore and typing tests (1.4) passed on first run: they pin the
  ported 2552d101 path with a Claude-shaped command. The red tests for #123
  were the hook's.
- The TUI's tab and workspace closes skip the todo check by origin and send
  no `force`, so their answers are unchanged.
- Measured: the hook takes about 65 ms on a 32 MB transcript and about 57 ms
  with none (10 runs each, m4m), so the transcript read costs about 8 ms.
- The PowerShell hook could not be run here (no pwsh on m4m).
- Integration version stays 9: one bump for the batch (with the a5e1c941
  port), relative to the fork's v0.7.4-ac release (v8).
