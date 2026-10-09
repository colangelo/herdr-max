Proposed 2026-10-08, approved by ac 2026-10-09 (option a, idle plus the braille mark). Issue: https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/172
Built on `feat/background-work-mark` (internal).

## 0. Measure first

- [x] 0.1 live capture (`herdr-beta agent read <pane> --source detection --format text`) of a Claude pane with: one background shell, two subagents, a workflow, an MCP task. Record the exact footer text for each; add the missing shell rule if the screen has one
- [ ] 0.2 confirm `background_tasks` is present on `Stop` hook input of the installed Claude version (decides whether phase 2 is possible)

## 1. Tests first

- [ ] 1.1 config: `background_mark` parses, defaults to `frames`, bad value -> diagnostic + `frames`
- [ ] 1.2 glyph table: eight entries, each one cell wide (`unicode_width`), count 0/9/255 clamp
- [ ] 1.3 detector: capture gives the number; no number -> 1; `claude.toml` rule ids unchanged
- [ ] 1.4 sidebar: `agent_icon` for frames/braille, both phases, spinner off, with and without a count fact
- [ ] 1.5 resource facts round-trip with and without `background_count`

## 2. Build

- [ ] 2.1 `BackgroundMarkConfig` in `src/config/model.rs`, template lines in `src/main.rs`, config reference JSON in `docs/next`
- [ ] 2.2 `AgentDetection.background_count`, `TerminalState::background_count()`
- [ ] 2.3 `claude.toml` captures; bump manifest version
- [ ] 2.4 `resource_facts.background_count`, set in `src/server/client_shell.rs`
- [ ] 2.5 `AgentRow.background_count` and `StatePresentation::agent_icon`

## 3. Verify

- [ ] 3.1 `just check`, rendered-layout snapshot for 1 and 15 rows
- [ ] 3.2 `herdr-dogfood` a beta; ac flips `background_mark` both ways with live background work
- [ ] 3.3 close the issue with how it was checked

## 4. Phase 2 (only with ac's OK)

- [ ] 4.1 feed `background_tasks.len()` from the `Stop` hook as an exact count, evidence-not-authority, per `claude-attention-hint` rules
