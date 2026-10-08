# Sidebar Jump-Number Prefix

## Why

The editorial sidebar's right-aligned jump numbers don't hint at how to jump there: workspaces use `prefix + N`, agents use `prefix + <mod> + N`. Prefixing the number with a leader glyph turns it into a self-documenting shortcut.

Design decided in Gitea issue https://gitea.cat-bluegill.ts.net/AC-forks/herdr/issues/21.

## What Changes

- New optional `ui.workspace_number_prefix` and `ui.agent_number_prefix` strings (default `""`), prepended to the right-aligned jump number in `sidebar_style = "editorial"`. e.g. `"₽"` → `₽5`, `"₽⌥"` → `₽⌥2`.
- Free-form strings so users compose the exact glyphs matching their prefix and modifier bindings; rendered in the existing `*_number_color`. Empty = today's bare number.
- Editorial mode only; default layout (single-char lead column) is unaffected.

## Capabilities

### New Capabilities

- `sidebar-number-prefix`: Optional leader-glyph prefix on the editorial jump numbers, per list.

### Modified Capabilities

<!-- none -->

## Impact

- `src/config/model.rs` + `src/app/state.rs` + `src/app/mod.rs` (startup + live reload) + `src/main.rs` + config-reference: the two options.
- `src/ui/sidebar.rs`: `editorial_number_reserve` / `draw_editorial_number` render a prefix+number label with display-width reservation.
- TUI presentation only.
