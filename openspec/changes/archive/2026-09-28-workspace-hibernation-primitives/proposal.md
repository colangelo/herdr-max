## Why

context-astra (CONTEXT) is building workspace hibernation: close a workspace idle
for 4+ days after recording what rebuilds it, and restore it later. Three herdr
facts it needs are missing or wrong on m4m:

- `pane process-info` cannot see a job behind `atuin pty-proxy`, so a busy pane
  looks idle (https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/90).
- No pane records when it last got input, so inactivity cannot be measured
  (https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/91).
- A Codex pane on the app-server daemon never records its thread id, so neither
  the tool nor herdr's own restore can resume it
  (https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/92).

## What Changes

- `pane process-info` adds the nested foreground job behind a recognised
  wrapper, and `shell_at_prompt`.
- Panes carry `last_input_at_unix`, stamped by input from users and callers,
  not by automatic writes, shown on pane info and kept in session.json.
- A naming job that resolves a Codex pane's thread records it as the pane's
  agent session (`herdr:codex`, id) when no hook reported one.

## Impact

- `src/app/api/panes.rs`, `src/api/schema/panes.rs`, `src/pane.rs`,
  `src/terminal/runtime.rs`, `src/server/alt_screen_read.rs`,
  `src/persist/{snapshot,restore}.rs`, `src/app/agent_resume.rs`,
  `src/codex_app_server.rs`, `src/events.rs`, `src/app/actions.rs`.
- API: additive fields only; no protocol bump.
