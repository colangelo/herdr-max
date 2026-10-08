# Design: Sidebar Editorial Style

## Context

Mechanics mapped 2026-07-19: jump numbers are hardcoded prefixes injected on row index 1 (`src/ui/sidebar.rs:1483-1493` spaces, `1710-1718` agents), colored by `workspace_number_color`/`agent_number_color` with `p.overlay0` fallback. Token spans flow strictly left-to-right with truncation budgeting (`resolved_token_spans`, `sidebar.rs:1054-1225`); the only right-alignment in the sidebar is separate `Paragraph`s with `Alignment::Right`. State glyph colors are fixed palette slots in `state_dot`/`agent_icon`/`state_label_color` (`src/ui/status.rs:241-284`) with no override surface. Headers are bold lowercase literals (`sidebar.rs:1309`, `1598`). Active-row background is pre-filled into cells for spaces but carried on the row `Paragraph` style for agents — an overlaid number must respect both. Approved mockup: `.superpowers/brainstorm/86999-1784467884/content/sidebar-style-final.html`; Gitea issue #20.

## Goals / Non-Goals

**Goals:**

- Editorial preset: right-aligned muted number on the name row, thin uppercase headers, dimmed inactive meta line — both sidebar sections, one config switch.
- User-tunable state colors with theme fallback, applied consistently to glyphs and state text.
- Defaults unchanged: without opting in, rendering is byte-identical to today.

**Non-Goals:**

- Collapsed-sidebar restyle (keeps its positional integers and current styling).
- A git clean/dirty indicator (the `git_status` token stays ahead/behind only).
- New row tokens or token-system changes; the preset composes existing rendering.
- Mobile switcher styling.

## Decisions

### 1. One preset enum, not per-tweak options

`ui.sidebar_style = "default" | "editorial"` gates number placement, header treatment, and meta-line dimming as one coherent look. Rationale: the pieces are aesthetically interdependent (the right-aligned number only reads well against a dim meta line), and the user's stated preference is customize-once. State colors stay a separate `[ui.state_colors]` table because they are theme-adjacent and useful without the preset.

### 2. Right-aligned number as an overlay paragraph with reserved width

In editorial mode the row-1 prefix injection is skipped. The name row's token budget (`max_width` passed to `resolved_token_spans`) shrinks by `symbol_width + 1` so names truncate before colliding with the number. The number renders as a separate right-aligned `Paragraph` on the name-row rect: transparent background for spaces (the active band is pre-filled into cells), `row_style` background for agents (the band lives on the row paragraph). Colors: existing `workspace_number_color` / `agent_number_color` chain, unchanged fallback `p.overlay0`.

### 3. State colors resolved once, threaded as a struct

`AppState::state_icon_colors()` returns a small struct (working/idle/done/blocked/unknown `Color`s) resolving `[ui.state_colors]` overrides against the palette slots (yellow/green/teal/red/overlay0). `state_dot`, `agent_icon`, and `state_label_color` take that struct instead of `&Palette`, so glyphs and state text always agree. Call sites (expanded + collapsed, spaces + agents) update mechanically.

### 4. Editorial header and meta treatment

Headers render uppercase without bold, `p.overlay0` + DIM (closest terminal analog to the mockup's thin letter-spaced caps; real letter-spacing would spend columns). The agents sort-toggle label keeps its style. Inactive meta line (secondary/branch style when the entry is not active) gains DIM; the active entry keeps the accent branch color.

## Risks / Trade-offs

- [Dim-on-dim themes] overlay0 + DIM may be faint on some themes → editorial is opt-in; state colors and number colors remain overridable.
- [Number overlap on narrow sidebars] mitigated by reserving the symbol width from the name row's truncation budget; at pathological widths the name truncates, the number always fits.
- [Signature churn in status.rs] three functions change parameters → mechanical, all call sites in two files, no behavior change in default mode (struct resolves to the same palette slots).

## Migration Plan

Additive config with defaults preserving current rendering; rollback = revert or unset config. No version bumps.

## Open Questions

None — mechanics fully mapped before implementation.
