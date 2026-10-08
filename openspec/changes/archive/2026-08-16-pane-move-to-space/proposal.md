# Move a pane to a different or new space from the keyboard

## Why

`pane-move-keybindings` gave the TUI three ways to move a pane and stopped at the
tab boundary. All three are pinned to the current space:

- `prefix+m` opens a picker built from `workspace.tabs` of the current workspace
  only (`src/app/input/navigate.rs:1736`)
- `prefix+>` / `prefix+<` step to an adjacent tab
- `prefix+!` breaks to a new tab with `workspace_id: Some(current)`
  (`src/app/input/navigate.rs:1333`)

There is no mouse route either: the mouse layer only clicks entries inside that
picker, and a pane cannot be dragged onto a space in the sidebar. So a pane that
belongs in another space has to be closed and restarted there, losing its
process and scrollback — the exact thing `pane.move` exists to avoid.

**The runtime already does all of it.** `PaneMoveDestination`
(`src/api/schema/panes.rs:81`) is upstream's, and carries three variants:
`Tab`, `NewTab { workspace_id, .. }` where the workspace may be *any* space, and
`NewWorkspace { label, tab_label }`. Both cross-space forms are already exposed
on the CLI:

```
herdr pane move <pane_id> --new-tab --workspace <ws_id>
herdr pane move <pane_id> --new-workspace [--label TEXT] [--tab-label TEXT]
```

`NewWorkspace` is constructed **nowhere in the TUI** — not in this fork and not
on `upstream/master`. It is CLI-only surface with no UI anywhere. The server
side, the validation, the events and the restore path are all in place; only the
picker never learned to ask.

## What Changes

The existing `prefix+m` picker widens from "the other tabs in this workspace" to
"anywhere this pane can go", and grows two destinations it could not previously
express:

- tabs in **other spaces**, grouped under their space so the list stays readable
- a **new tab** in any space
- a **new space**, created by the move

The keys, the modal language, the zoomed-tab rejection and the `pane.move`
failure feedback are unchanged. `prefix+>` / `prefix+<` and `prefix+!` keep their
current tab-scoped meaning; this change adds reach, it does not redefine what
exists.

## Impact

- Affected capability: `pane-move-controls` (one requirement modified, one added)
- Affected code: `src/app/input/navigate.rs` (picker construction and dispatch),
  `src/app/state.rs` (`PaneMoveTargetPickerState` entries gain a group and a
  destination kind), `src/ui/dialogs.rs` (picker rendering), `src/ui/keybind_help.rs`
- No server, API, protocol or config surface: the destinations, their validation
  and their events already exist. Client-side only under the runtime/client
  guardrail.
- Upstream: the API is entirely upstream's and they ship no UI for it, so this is
  shaped to be liftable as a UI-only `feat:` PR. Keep it free of fork-specific
  styling so the diff stays portable.

## Non-goals

- Dragging a pane onto a space in the sidebar. Mouse reach is worth having but is
  a separate interaction with its own hit-testing and drop-target design.
- Moving a *tab* between spaces. Same family, different object; upstream's
  `move_tab_previous`/`move_tab_next` only reorder within a workspace.
- Changing what `prefix+!` does. Break-to-new-tab stays same-space; the picker is
  where the new reach lives.
