# Design

## Mirror the notification indicator, do not generalize it

The notification indicator already solved every problem this feature has: a
trailing tab-bar slot, a label that grows a count pill, a hit area threaded
through `TabBarView` into `ViewState`, a click that toggles its surface, and a
floating fallback. The todo indicator copies that shape one slot to the left
rather than extracting a shared "indicator framework" — two call sites is not a
pattern yet, and the two differ already (priority coloring, no floating
variant).

## The count is read where the panes already pay for it

The session-wide outstanding count and its highest priority aggregate the same
per-terminal todo data every visible pane border already reads each frame for
its `▾`. One more pass per frame over the terminals map, not per pane, and no
allocation: the fold carries a count and a max-priority. Under the
multiplicative-paths rule this is render-frequency × terminals, the same order
the border indicators already spend.

## Glyphs: the modified-letter language

The sidebar prefix badge set the precedent: a letter with a twist (`Ᵽ`). `τ`
is t for todos, `и` is a mirrored N for notifications. Both are single-width
in monospace fonts. Considered and rejected:

- **`▾` in the tab bar** — one glyph for one concept everywhere was the
  strongest alternative; overruled by the maintainer for the letter language.
  `▾` stays on pane borders, which mark a place rather than an entry point.
- **`ɴ` (small-capital N)** — literal, but it reads as a plain letter at cell
  size; the mirror is what makes `и` a glyph rather than a stray character.

## Click toggles, matching `◆`

Clicking the notification indicator toggles its panel; the todo indicator does
the same for the board rather than being open-only, so the two corner controls
feel like one mechanism. Toggling from the indicator goes through the same
open/close paths as the keybinding, so suspended-surface bookkeeping (the
board remembering to return to a suspended panel) is untouched.
