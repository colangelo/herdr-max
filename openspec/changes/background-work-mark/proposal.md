## Why

https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/172. ac, 2026-10-08:
"replace the current with the braille icon, one per bg job or workflow or
subagent ... that alternates, at the current speed, with the small central dot
that we have now. can you make it so we can choose in config which one is
active, so that I test both, and maybe later more?"

Today an agent row that is only waiting on work it launched (a shell, subagents,
a workflow, MCP tasks) shows a two-frame pulse (`[ui.state_symbols]
background` / `background_alt`). The sidebar knows only *that* background work
runs, not *how much*.

Status: **approved by ac, 2026-10-09: option (a), idle plus the braille mark.** Built on branch
`feat/background-work-mark` for the first batch after the v0.9.3 cutover.

## What Changes

- New `[ui] background_mark = "frames" | "braille"` (default `"frames"`, which
  is exactly today's behaviour). Unknown values fall back to `"frames"` with a
  config diagnostic. The enum is open on purpose: more styles are one more
  variant.
- `"braille"`: the row icon alternates, at the existing pulse speed, between the
  small dot (`state_symbols.background`, default `·` in this mode) and one
  braille cell whose dots count the background items (1..8, `⣿` for 8 or more).
- The detector learns a *count*, not only a flag: `AgentDetection` gains
  `background_count`, the server publishes it in the optional JSON
  `resource_facts` next to `background_activity`, and the sidebar reads it. No
  wire-protocol bump: resource facts are optional JSON and never enter a
  published binary codec.
- Status follows upstream (987b070f): a Claude at its prompt reads idle even with shells running;
  the fork's `background_shell_working` rule is removed and the mark carries the information.
- The count comes from the screen first (the two background rules already match
  a line that contains a number). Where the screen gives no number the count is
  1. A later phase can add the Claude `Stop` hook's `background_tasks` snapshot
  as a second, exact source; it is designed for here but not built.

## Impact

- `src/config/model.rs` (new `BackgroundMarkConfig`), `src/main.rs` (the
  commented config template), config reference JSON in `docs/next`.
- `src/detect/manifest.rs`, `src/detect/manifests/claude.toml` (capture groups),
  `src/pane/agent_detection.rs`, `src/terminal/state.rs`.
- `src/server/client_shell.rs`, `src/protocol/resource_facts.rs`.
- `src/client/shell/{state_presentation,agent_sidebar,sidebar}.rs`.
- No new dependency. Render stays pure: the glyph is picked from a `const`
  table, no allocation per row.
