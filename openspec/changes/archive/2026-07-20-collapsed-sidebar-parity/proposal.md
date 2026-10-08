# Collapsed Sidebar Parity

## Why

The collapsed sidebar drifted behind the expanded sidebar on four counts, found while running 16 agents (Gitea issue https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/24):

1. The active **agent** row has no highlight at all — its style is one flat value with no active branch, so there is no way to see which agent is focused.
2. Rows past the 9th are labelled with plain integers (`10`, `11`, …) while the real jump chords use `jump_symbol` (1-9 then a-z), so the labels name chords that don't exist. This latently affects the spaces list too, and two-digit labels collide with the state icon.
3. Collapsed rows ignore `ui.workspace_number_color` / `ui.agent_number_color`.
4. Even where the active highlight works (spaces), it is only a background band — no border bar, no bold — and with a dark custom `sidebar_active_bg` it is nearly invisible. Expanded gives three reinforcing cues (band + bold + `ui.sidebar_active_border` accent bar).

## What Changes

- Collapsed agent rows gain the same `is_active` treatment collapsed space rows have (band background), plus active-cue upgrades below.
- Both collapsed sections label rows with `crate::config::jump_symbol(...)` (1-char: 1-9 then a-z; blank beyond z), matching the expanded sidebar and the actual `switch_workspace` / `focus_agent` bindings. Icon column position stays fixed.
- Collapsed rows honour `ui.workspace_number_color` / `ui.agent_number_color` for non-active rows, like expanded.
- Collapsed mode honours `ui.sidebar_active_border = "left" | "right"`: the collapsed sidebar reserves one extra edge column for the accent bar (drawn via the same helper/config as expanded: `pane_border_active_color`, `pane_border_active_style`) on the active space row and active agent row. `above`/`below`/`both` stay no-ops in collapsed (no gap rows), matching expanded semantics. The active row's jump symbol renders bold in `text` color.
- No new config options.

## Capabilities

### New Capabilities

- `collapsed-sidebar-parity`: collapsed sidebar active-row cues, jump-symbol labels, and number-color/active-border config parity with the expanded sidebar.

### Modified Capabilities

<!-- none -->

## Impact

- `src/ui/sidebar.rs`: `render_sidebar_collapsed` (labels, styles, active branches, bar), collapsed buffer tests.
- `src/ui.rs`: collapsed sidebar width accounts for the reserved bar column when `sidebar_active_border` is `left`/`right`.
- TUI presentation only; no state, protocol, or config schema changes.
