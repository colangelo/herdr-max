## 1. Wrapped panes (issue 87)

- [x] 1.1 `available_pane_shell` looks through a recognised wrapper; tests for prompt, command, subshell, no nested PTY, unrecognised program, shared job
- [x] 1.2 Probe reports the wrapped shell; `WrappedShellWatch` forces a probe on its group change; tests
- [x] 1.3 `live_runtime_agent` descends through the wrapper
- [x] 1.4 Live: `agent start --kind codex` in atuin-wrapped panes, three in a row, each detected idle and named

## 2. Hand-launched Codex naming (issue 86)

- [x] 2.1 Platform process start time (macOS, Linux; none elsewhere)
- [x] 2.2 Find the pane's Codex process; thread id from `resume <id>`, else its start time
- [x] 2.3 Rename job falls back to the anchor; first naming (no old name) runs too; tests against the fake daemon
- [x] 2.4 Live: hand-typed launch, `agent rename`, thread named, `agent-bell who` shows it

## 3. Ship

- [x] 3.1 `just check` green
- [x] 3.2 Beta published; live proof on m4m with the beta

Verified 2026-09-27 on m4m with the `0.8.2-ac-beta.99-danilo` release binary in a
throwaway session whose panes all ran `atuin pty-proxy`: `agent start --kind codex`
came up idle and named its thread; a hand-typed Codex was detected and `agent
rename` named its thread; `config check` accepted `[agents]`; after `server
reload-config` a start still used `--remote` and named its thread. `agent-bell who`
listed all three threads as codex under their pane names.
