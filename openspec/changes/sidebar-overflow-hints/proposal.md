## Why

https://gitea.cat-bluegill.ts.net/AC-forks/herdr-max/issues/159, asked by ac
2026-10-04: "we need a way to show that the agents (and also the spaces) leftbar
has stuff below, or that we're not showing the top (that usually include some
pinned session)". When the spaces list or the agent panel overflows, only the
one-column scrollbar says so. It does not say that the rows above are pins, or
that a hidden agent below is blocked.

## What Changes

- **Edge rows.** When rows are hidden above a list, its first row is a dim
  summary: `↑↑ 2 pinned · 3 more`, each `↑` in the colour of one hidden pin. When
  rows are hidden below, its last row is `↓ 5 more · ● 1 blocked`, the dot in the
  colour of the most urgent hidden state. A click on an edge row scrolls a page
  toward it. A side with nothing hidden has no row and loses no space.
- **Fog.** The two visible entries next to an edge with hidden rows get a
  progressively lighter background (about 9% and 4% toward the text colour,
  nearest first). The text colour is unchanged: contrast drops by lifting the
  background, never by dimming the text.
- **Two touches.** The fog takes a faint tint of the most urgent hidden state, so
  it glows toward what is waiting; the focused or selected entry never fogs.
- **One setting.** `[ui] sidebar_overflow = "both" | "rows" | "fog" | "off"`,
  default `both`, read at startup and on reload-config, shared by both panels.

## Capabilities

### New Capabilities

- `sidebar-overflow`: showing what a scrolled sidebar list hides.

## Impact

`src/ui/sidebar_overflow.rs` (new: the window, the summary, the fog),
`src/ui/sidebar.rs` (the two lists lay their entries out inside the window and
draw the plan), `src/app/input/{sidebar,mouse}.rs` (the click),
`src/config/model.rs`, `src/app/{mod,state}.rs` (the setting), `src/main.rs`
(the `--default-config` entry), `docs/next` and the README.
