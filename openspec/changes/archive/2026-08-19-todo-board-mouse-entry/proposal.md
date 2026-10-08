# A mouse entry for the todo board

## Why

The session todo board has exactly one way in: a keybinding that ships unbound.
Every sibling surface — the notification center, the pane todo panel — can be
reached with the mouse (the `◆` indicator, the `▾` border indicator); the board,
the surface meant for triage, cannot. Dogfooding also asked for the outstanding
count to be visible without opening anything.

## What Changes

A todo indicator in the tab bar's trailing corner, immediately left of the
notification indicator: glyph plus session-wide outstanding count, colored by
the highest outstanding priority (the border indicator's rule), always visible,
click toggles the board.

The two tab-bar indicators adopt the fork's modified-letter glyph language:
`τ` for todos, and the notification `◆` becomes `и`. The sidebar prefix badge
set the precedent — a letter with a twist (`Ᵽ`); `τ` is a twisted t, `и` a
mirrored N. The per-pane `▾` is deliberately unchanged: it marks a place on a
pane, not an entry in the chrome.

## Impact

- Affected capability: `pane-todos` (one added requirement). The
  notification-center spec names no glyph, so `◆` → `и` is presentation only.
- Affected code: the tab bar's label/width/layout/render and its hit-area
  plumbing through the view state; a session-wide outstanding-count read on
  `AppState`; the mouse layer's click routing; the notification indicator label
  and its tests.
- Tab-bar layout work is per-render: the new count aggregates the same per-pane
  todo data the pane borders already read each frame, once more per frame, not
  per pane.
- No server, API, protocol or config surface: presentation only, client-side
  under the runtime/client guardrail.

## Non-goals

- A floating todo indicator for tab-bar-less layouts (the notification center
  has one at bottom-right). Left out until someone runs that layout and wants
  it; the keybinding still works there.
- The mobile header. Same reasoning.
- A config knob for the glyphs or for hiding the indicator. Not asked for.
