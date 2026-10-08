## Why

https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/148, asked by ac
2026-10-02 ("a way to pin a space at the top ... in second position", then
"same for agents"). A space's place in the sidebar comes from the sort mode, and
an agent's from the agent panel sort: under `priority` both move as agent
states change, under `manual` a space has to be dragged back by hand. Nothing
keeps one at the top. Upstream has no such feature (checked 2026-10-02).

## What Changes

- **Pin a space or an agent.** Pinned spaces go to the top of the space list,
  pinned agents to the top of the agent panel, each block **in pin order**: the
  first pinned is 1st, the next is 2nd.
- **Under every sort.** Spaces `manual` and `priority`, agent panel `spaces`
  (grouped) and `priority`. The unpinned rest sorts below exactly as now.
  Unpinning puts the row back where the sort says.
- **Shared server state, not TUI state.** The pin is a fact about the session:
  a `pin_order` on the workspace and on the pane's terminal, saved in the
  session snapshot (so it survives a restart and a live handoff, and a resumed
  agent keeps it), shown as `pinned: bool` in the JSON API.
- **API and CLI.** `workspace.pin` / `workspace.unpin` (`herdr workspace
  pin|unpin <id>`) and `agent.pin` / `agent.unpin` (`herdr agent pin|unpin
  <target>`); `pinned` on workspace and agent list/get (and pane list/get).
- **Keys.** Actions `toggle_pin_workspace` and `toggle_pin_agent`, unbound by
  default, with entries in the `prefix+?` help.
- **Marker.** Pinned rows show a narrow single-cell `↑` before the name.
- **Motion.** The priority-sort bubble motion does not animate pinned rows.

## Capabilities

### New Capabilities

- `sidebar-pins`: pinning spaces and agents to the top of the sidebar.

## Impact

`src/workspace.rs`, `src/terminal/state.rs`, `src/persist/{snapshot,restore}.rs`
(optional fields with serde defaults, old files load), `src/ui/sidebar.rs`,
`src/app/agent_view.rs`, `src/api/schema/*` (optional fields and four methods),
`src/cli/*`, `src/config/keybinds.rs`, `src/ui/keybind_help.rs`, docs.
`PROTOCOL_VERSION` is not bumped: only optional response fields and new methods
are added, nothing existing changes shape.

## Open UI points (ac reviews before any roll; each is one constant)

1. The marker glyph (default `↑`, U+2191, one cell; no emoji, which are two
   cells wide and break the column maths).
2. A default key (default: none).
3. A separator line under the pinned block (default: none).
