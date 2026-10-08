# Sidebar Editorial Style

## Why

The sidebar's current composition fights itself: jump numbers open the second row in saturated red, branch rows repeat `main` at near-name brightness, and state dots are pastel enough to miss. A visual design pass (interactive mockups, 2026-07-19) settled on an "editorial" restyle: right-aligned muted numbers on the name row, one dim meta line per entry, thin uppercase headers, and stronger user-tunable state-dot colors.

Design decided in Gitea issue https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/20.

## What Changes

- New `ui.sidebar_style = "default" | "editorial"` preset (default `default`, no visual change without opt-in). `editorial` moves jump numbers to the right edge of the name row (spaces and agents panels), renders section headers as thin uppercase (`SPACES`, `AGENTS`, no bold, dimmed), and dims the inactive meta line (branch/agent details).
- New `[ui.state_colors]` table: `working`, `idle`, `done`, `blocked`, `unknown` color overrides for the state glyphs (`●`/spinner, `○`, `✓`/`●` unseen, `◉`, `·`) and matching state text, falling back to the theme palette slots (yellow/green/teal/red/overlay0). Independent of `sidebar_style`.
- Number colors continue to use `ui.workspace_number_color` / `ui.agent_number_color`; entry spacing continues to use the existing `row_gap` keys; the agents meta line stays token-configured. The mockup's "clean" git text is out of scope (the `git_status` token renders ahead/behind only).

## Capabilities

### New Capabilities

- `sidebar-editorial-style`: The editorial sidebar preset (number placement, header treatment, meta-line dimming) and the state-color override table.

### Modified Capabilities

<!-- none — no existing specs cover sidebar presentation -->

## Impact

- `src/ui/sidebar.rs`: number placement in `render_workspace_list` / `render_agent_detail` (row-1 prefix vs right-aligned name-row overlay, with token width budgeting), header rendering, secondary-style dimming.
- `src/ui/status.rs`: `state_dot` / `agent_icon` / `state_label_color` take resolved state colors instead of raw palette slots.
- `src/config/model.rs` + `src/config.rs` + `src/app/state.rs` + `src/app/mod.rs` (startup + live reload) + `src/main.rs` template + config-reference data: the new options.
- TUI presentation only; no protocol/API/persistence changes. Collapsed sidebar keeps its current compact numbering.
