## Context

Order of spaces: `AppState.workspaces` is the manual order; `priority` sorts
units in `ui::sidebar::workspace_sorted_units`, and a bubble motion animates
changes. The agent panel is built by `collect_agent_panel_entries_with_runtimes`
in workspace order and sorted by `app::agent_view::apply_agent_view`, with its
own motion. Both feed render, jump numbers and mouse hit-testing, so one
ordering helper per list keeps them equal.

## Decisions

- **State.** `Workspace.pin_order: Option<u64>` and `TerminalState.pin_order:
  Option<u64>`. The order is a number, not a flag plus a list, so pin order is
  just ascending `pin_order`. Pinning takes `max(existing pin_order) + 1` over
  the same kind (workspaces, or terminals), so no counter is stored and a
  restored session continues where it left off. Unpinning clears it; gaps are
  fine.
- **Agent pins live on the terminal**, the object a pane's agent identity and
  restore record already live on, saved in `PaneSnapshot`. A resumed agent is
  the same pane/terminal, so it keeps the pin; a pane whose agent goes away
  keeps the number but is not an agent row, and shows nothing.
- **Persistence.** `WorkspaceSnapshot.pin_order` and `PaneSnapshot.pin_order`,
  `#[serde(default, skip_serializing_if = "Option::is_none")]`: older files load
  unpinned, `SNAPSHOT_VERSION` is unchanged. Live handoff carries the same
  `SessionSnapshot`.
- **Ordering.** After the sort, a stable partition: pinned units first by
  ascending `pin_order`, then the rest in their sorted order. A worktree group
  (parent plus members) is one unit; it counts as pinned when any of its
  workspaces is, ordered by the smallest `pin_order` among them. In the agent
  panel pinned agents form one top block above everything (the grouped mode has
  no group headers to float inside), also over a plugin agent-view sort.
- **Motion.** Motion runs over the unpinned rows only; pinned rows are drawn
  in their fixed order and never animate. A row entering or leaving the pinned
  block just appears or disappears from the animated set.
- **Jump numbers and hit-testing** read the same ordered list as render.
- **Manual move.** Dragging a space changes the manual order as before but not
  the pin order: to reorder pins, unpin and pin again. (The issue sketched
  reordering by dragging inside the block; deferred, noted in the issue.)
- **API.** `workspace.pin|unpin` (target: workspace id), `agent.pin|unpin`
  (target: agent name or pane id, like the other agent calls). Pinning is
  idempotent: pinning a pinned row keeps its place; unpinning an unpinned row
  is a no-op. Results return the updated info. `pinned: bool` is added to
  `WorkspaceInfo`, `AgentInfo` and `PaneInfo` (omitted when false, serde
  default), so `PROTOCOL_VERSION` stays.
- **Keys.** `toggle_pin_workspace` toggles the selected/active space,
  `toggle_pin_agent` the focused pane's agent; both unbound, with
  `help_entry` lines in `src/ui/keybind_help.rs`.
- **Marker.** One constant `PIN_MARKER = "↑"` drawn before the name of pinned
  rows, taking one cell from the label width.

## Risks

- Persisted identity and order: covered by `assert_invariants_for_test` on
  adversarial state and a snapshot round-trip including a file without the
  field.
- Column maths: the marker is one cell wide; a test renders a pinned row at the
  narrowest sidebar width.

## Rollout

No beta from this change until ac answers the three UI points.
