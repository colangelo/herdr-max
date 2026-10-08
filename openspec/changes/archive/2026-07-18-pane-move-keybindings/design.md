## Context

Herdr's hierarchy is Workspace → Tab → Pane, with panes arranged in a BSP tree (`TileLayout`) per tab. The runtime already supports relocating a pane across tabs and workspaces through `pane.move` (`src/api/schema.rs` → `handle_pane_move` in `src/app/api/panes.rs:616`), whose destination is a tagged `PaneMoveDestination` enum (`src/api/schema/panes.rs:54`) with `Tab`, `NewTab`, and `NewWorkspace` variants. The CLI (`herdr pane move`) already wraps all three. What is missing is any TUI affordance: `NavigateAction` has no move/break variants and `KeysConfig` has no bindings for them, so an interactive user cannot do what tmux users expect from `break-pane` / `join-pane`.

Relevant existing surfaces this change reuses:
- `NavigateAction` enum and its dispatch — `src/app/input/navigate.rs` (e.g. swap panes at `swap_pane_direction_via_api`, `NavigateAction::SwapPane*`).
- The keymap→action binding table — `src/app/input/navigate.rs:1467+`.
- `KeysConfig` struct and defaults — `src/config/model.rs:429`, defaults at `~1095`.
- Existing modal/picker UX (the workspace picker `prefix+w`) — reused for the move-to-tab picker per the project's "reuse UI patterns" principle.
- `pane.move` client call path already used by the CLI.

Constraint: the runtime/client boundary guardrail. This change must not add shared server state or a new socket message; it is purely client input wired to an existing runtime operation.

## Goals / Non-Goals

**Goals:**
- Give interactive users tmux-equivalent pane relocation: break a pane to a new tab, and move a pane into an existing tab.
- Reuse the existing `pane.move` API path unchanged — no protocol change, no version bump.
- Follow herdr's existing modal/picker UI language for the target-tab selection.
- Make all new actions configurable and rebindable via `KeysConfig`.
- Handle the edge cases the runtime already enforces (zoomed tab rejection, single-pane source, no adjacent/other tab) with non-blocking user feedback.

**Non-Goals:**
- Drag-and-drop of a pane onto the tab bar or into another tab (deferred follow-up; the only pane drag target today is split-resize).
- Moving a pane to a new *workspace* from the TUI (the `NewWorkspace` destination stays CLI/API-only for now).
- Any change to `pane.move` semantics, request/response schema, or the protocol version.
- Reordering tabs or panes (already covered by `tab.move` drag and `pane.swap`).

## Decisions

### Decision 1: Wire to `pane.move`, add no protocol surface

Implement each new action as a `NavigateAction` variant whose handler calls the existing `pane.move` client path (the same request the CLI sends), then relies on the emitted `pane.moved` / layout-updated events to refresh the view. Rationale: `pane.move` is already implemented, tested, and handles cross-tab tree surgery, id reassignment, and rejection reasons. Duplicating any of that in the TUI would violate the boundary guardrail and risk divergence.

Alternative considered: a TUI-local layout mutation that bypasses the API. Rejected — it would fork the move logic, break the server-owned-runtime direction, and desync other clients.

### Decision 2: Three actions, tmux-style defaults

- `BreakPaneToNewTab` → `pane.move` `new_tab` destination (current workspace). Default `prefix+!` (tmux `break-pane`).
- `MovePaneToTab` → opens the tab picker, then `pane.move` `tab` destination. Default `prefix+m`.
- `MovePaneToNextTab` / `MovePaneToPrevTab` → `pane.move` `tab` destination targeting the adjacent tab, no picker. Default bindings proposed `prefix+>` and `prefix+<` (see Open Questions — these are the least-anchored chords).

Rationale: `prefix+!` and `prefix+m` map to tmux muscle memory. The break action needs no target so it is a single stateless keystroke; the general move needs a target so it uses a picker; the adjacent-move actions are the fast path for the common "shove it one tab over" case.

Alternative considered: a single "move" action that always opens a picker. Rejected — the adjacent-tab case is common enough to deserve a no-UI fast path, matching how herdr already offers both `switch_tab` jumps and `next_tab`/`previous_tab`.

### Decision 3: Reuse the existing picker modal for target selection

`MovePaneToTab` opens a modal built on the same infrastructure as the workspace picker, listing the workspace's other tabs by number/label. Selection resolves to a `tab_id` and issues `pane.move`. Rationale: the project principle "UI patterns should be reused"; avoids a one-off screen and keeps close/cancel affordances consistent.

Alternative considered: an inline "type the tab number" prompt. Rejected — inconsistent with herdr's mouse-first modal language and worse for discoverability.

### Decision 4: Default split direction for `tab` destination moves

`pane.move` into an existing tab requires a `split` direction and splits next to the target tab's focused pane. Use a single default direction (`right`) for the picker and adjacent-move actions, matching `split_vertical` as the primary split. Rationale: keeps the actions single-gesture; users can re-split/rebalance afterward. The picker MAY later grow a direction sub-choice, but v1 keeps it one step.

Alternative considered: prompt for direction on every move. Rejected — adds friction to what should be a quick action.

### Decision 5: Edge cases surface as non-blocking toasts, layout untouched

Single-pane source (nothing to break), no other/adjacent tab, and `pane.move` rejections (e.g. `zoomed_tab`) all resolve to a non-blocking indication using the existing toast/notification path, leaving the layout in its prior state. Rationale: matches how the runtime already reports `changed: false` with a reason; the TUI simply reflects it.

## Risks / Trade-offs

- **Chord collisions for the adjacent-move actions** → `prefix+>` / `prefix+<` are not tmux-anchored and could clash with user configs or feel arbitrary. Mitigation: they are configurable `KeysConfig` entries; finalize in Open Questions before defaulting, and consider shipping them bound but easily overridden.
- **Default split direction is opinionated** → `right` may not suit stacked layouts. Mitigation: it is a starting layout only; balance/resize/swap already exist. Revisit a direction sub-choice in a follow-up.
- **Picker adds a step for power users** → mitigated by the adjacent-move fast path and by `prefix+!` needing no picker at all.
- **Feedback parity** → if a `pane.move` rejection is not surfaced, a user could think the keystroke did nothing silently. Mitigation: explicit toast on every no-op/rejection path, covered by the spec scenarios.

## Migration Plan

Additive, client-only, no persisted state or protocol change — no migration or rollback concerns beyond the new config keys. New `KeysConfig` fields default to the chosen bindings; existing user configs without those keys inherit the defaults. Rollback is reverting the client code; no server or on-disk format is touched. Docs land in `docs/next/` (`keyboard.mdx`, `configuration.mdx` `[keys]`) until release.

## Open Questions

- Final default chords for `MovePaneToNextTab` / `MovePaneToPrevTab`. Candidates `prefix+>` / `prefix+<`; alternative is to ship them unbound and let users opt in. (Break `prefix+!` and move-picker `prefix+m` are settled.)
- Whether the move-to-tab picker should offer a split-direction choice in v1 or defer that to a follow-up (current plan: defer, default `right`).
- Whether to also expose these as right-click pane context-menu items in this change or a follow-up (the `ContextMenuKind::Pane` menu already exists and carries `source_pane_id`).
