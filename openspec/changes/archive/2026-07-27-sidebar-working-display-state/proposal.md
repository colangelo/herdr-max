# Show Working Agents In The Sidebar Instead Of Masking Them Behind Done

## Why

A sidebar space row can render `done` (green) while an agent inside that space is actively
working, and only flips to `working` (yellow) when the user switches into the space —
caused by the switch itself, not by anything changing in the working agent.

Root cause: `pane_attention_priority` (`src/workspace/aggregate.rs:83`) ranks
`Idle + unseen` ("done") at 3 and `Working` at 2. `Workspace::aggregate_state` takes
`max_by_key` over that ranking and the sidebar space row renders the winner
(`src/ui/sidebar.rs:949`). So one finished-but-unvisited agent masks an actively working
sibling. Switching into the space marks every pane in the tab `seen`
(`src/workspace.rs:485-493`, `src/app/actions.rs:1467-1473`), dropping the finished pane to
priority 1 so `Working` finally wins.

The defect is reusing an **attention** ranking to choose a **displayed state**. Ranking
"finished, needs you" above "still working" is a defensible attention/notification
ordering; it is wrong for deciding what a row *is*.

Confirmed live against `herdr-beta 0.7.5-ac-beta.50-milik`: space `w5` rendered `done` for
36s while pane `w5:p3` was working, and cleared at the exact instant of a focus switch to a
sibling pane. Tracked in
https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/39.

This is **not** a regression from the v0.7.5 sync — the ranking traces to upstream
`3397e1ce` — and it is **not** the previously-fixed live-handoff resurface bug, whose fix
(`2c7a1792`, `3bf6a09d`) is intact and compiled into every `-ac-beta` build. It is a
distinct, older mechanism that the fork's priority sorting made far more visible.

## What Changes

- Introduce a display-oriented aggregate state for sidebar rows in which `Working`
  outranks `Idle + unseen`, so a space containing a working agent renders as working.
- Keep the existing attention ranking for attention/notification ordering, and make the
  ordering choice explicit rather than incidental: the sidebar's priority sorts state which
  ranking they follow.
- Collapse the three byte-identical copies of the ranking table
  (`src/workspace/aggregate.rs:83`, `src/ui/sidebar.rs:293`, `src/app/api_helpers.rs:1`)
  into one shared definition so the two rankings cannot silently drift apart — along with a
  fourth copy, the navigator's `state_priority` (`src/app/actions.rs`), which already
  implements the display ranking this change generalizes.
- A row's rendered state must not change as a side effect of focusing an unrelated sibling
  pane.
- The two aggregate `agent_status` fields the JSON API exposes
  (`WorkspaceInfo`, `TabInfo`) follow the display ranking too, and say so on the field.

## Capabilities

### New Capabilities

- `sidebar-agent-state-display`: Deriving the state a sidebar row displays from a
  display ranking that is separate from the attention ranking used for sorting and
  notifications.

### Modified Capabilities

<!-- none — no existing capability in openspec/specs/ covers aggregate state display -->

## Impact

- `src/workspace/aggregate.rs`: `pane_attention_priority` split into display vs attention
  ranking; `aggregate_state` switches to the display ranking.
- `src/ui/sidebar.rs`: space row rendering follows the display aggregate;
  `workspace_attention_priority` replaced by the shared definition.
- `src/app/api_helpers.rs`: `tab_attention_priority` replaced by the shared definition.
- `src/app/agent_view.rs`: `apply_agent_view` priority sort — decide and document which
  ranking it follows.
- Sort-motion target keys (`agent_panel_target_keys`, `workspace_unit_target_keys`) must
  use the same ranking as the sort they animate, or the bubble motion will chase a
  different order than the sort produced.
- API surface: per-pane `agent_status` carries no aggregation and is unchanged.
  `WorkspaceInfo.agent_status` (`src/app/creation.rs`) and `TabInfo.agent_status` move from
  the attention ranking to the display ranking and are documented as such — a shared runtime
  fact stated explicitly, not TUI presentation leaking outward.
- No wire-protocol change: `src/protocol/wire.rs::PROTOCOL_VERSION` is untouched.

## Non-Goals

- Changing when a pane becomes `seen`, or the tab-wide `seen` sweep on switch.
- Changing notification/toast ordering or the attention semantics themselves.
- Changing per-pane agent rows, which already render correct per-pane state.
