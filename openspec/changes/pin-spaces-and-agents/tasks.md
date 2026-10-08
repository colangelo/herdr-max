https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/148

## 1. Part A: spaces

- [ ] 1.1 tests first: order under `manual` and `priority`, pin order, unpin, group unit, jump numbers, no motion for pinned
- [ ] 1.2 `Workspace.pin_order`, snapshot field with serde default, restore, invariants
- [ ] 1.3 ordering helper in `workspace_sorted_units`, motion over unpinned only
- [ ] 1.4 `workspace.pin|unpin`, `pinned` on workspace info, CLI, schema artifact
- [ ] 1.5 `toggle_pin_workspace` action, unbound, help entry; marker in the row
- [ ] 1.6 docs, `just check`

## 2. Part B: agents

- [ ] 2.1 tests first: top block under `spaces` and `priority`, over an agent view sort, pin order, unpin
- [ ] 2.2 `TerminalState.pin_order`, `PaneSnapshot` field, restore, resumed agent keeps it
- [ ] 2.3 ordering in `apply_agent_view` and `agent_panel_target_keys`, motion over unpinned only
- [ ] 2.4 `agent.pin|unpin`, `pinned` on agent and pane info, CLI, schema artifact
- [ ] 2.5 `toggle_pin_agent` action, unbound, help entry; marker
- [ ] 2.6 docs, `just check`

## 3. Verify

- [ ] 3.1 throwaway server with pins, live handoff keeps them, capture of 2 pinned spaces and 2 pinned agents on the issue
