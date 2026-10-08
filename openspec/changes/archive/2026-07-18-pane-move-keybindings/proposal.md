## Why

Herdr can already move a pane into another tab, break it out to its own tab, or promote it to a new workspace — but only through the `pane.move` socket method and `herdr pane move` CLI. The TUI exposes none of this: there is no keybinding and no mouse gesture, so an interactive user cannot reorganize panes across tabs the way tmux users expect (`break-pane`, `join-pane`). This is a pure ergonomics gap over an already-implemented, already-tested runtime primitive.

## What Changes

- Add a **break-pane-to-new-tab** action that promotes the focused pane to its own new tab in the current workspace, backed by `pane.move` with a `new_tab` destination. Default binding `prefix+!` (matches tmux `break-pane`).
- Add a **move-pane-to-tab picker** action that opens a modal listing the workspace's other tabs; selecting one moves the focused pane into that tab via `pane.move` with a `tab` destination and a default split. The picker reuses the existing modal/picker UI language (as used by the workspace picker), per the "reuse UI patterns" project principle. Default binding `prefix+m`.
- Add **quick move-to-adjacent-tab** actions that move the focused pane into the next / previous tab with a default split, no intermediate UI. Default bindings to be finalized in design (candidate: `prefix+>` / `prefix+<`).
- Register the new actions in `KeysConfig` so users can rebind or unbind them, and document them in the keyboard reference.
- Non-goals for this change: drag-and-drop of a pane onto the tab bar or into another tab (deferred to a follow-up), and any change to the `pane.move` server API or protocol.

## Capabilities

### New Capabilities

- `pane-move-controls`: Interactive (keyboard/TUI) affordances for relocating the focused pane to another tab or to a new tab, delegating to the existing `pane.move` runtime operation. Covers break-to-new-tab, move-to-tab via picker, and quick move to the adjacent tab, plus their default keybindings, config keys, and edge-case behavior (single-pane tab, zoomed tab, no adjacent tab).

### Modified Capabilities

<!-- None. No existing OpenSpec specs; the pane.move server API and protocol are unchanged. -->

## Impact

- **TUI input layer** (client-only): new `NavigateAction` variants and their dispatch in `src/app/input/navigate.rs`; new `KeysConfig` fields and defaults in `src/config/model.rs`; the keymap binding table.
- **Picker/modal UI**: a tab-target picker modal reusing existing modal infrastructure.
- **Runtime/protocol**: none. All new actions call the existing `pane.move` API path (`handle_pane_move`); no new socket message, no protocol version bump, no server state.
- **Docs**: `keyboard.mdx` and the `[keys]` block in `configuration.mdx` (staged under `docs/next/` until release).
- **Boundary guardrail**: this is TUI presentation/input state wired to an existing shared-runtime operation, so it does not deepen server/TUI coupling.
