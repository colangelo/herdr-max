# The board's selection, pointer and viewport are three things

## Why

Dogfooding the board at real size — 37 todos across eight panes, once agents
started leaving notes — broke two things that a short list had hidden.

**The first group's heading can never be scrolled back to.** Scrolling is driven
entirely by "keep the selection visible with minimum movement", and the
selection can only land on a todo, never on a heading or a gap. Item 0 is always
a heading, so the scroll offset can never reach 0. Scroll to the bottom and back
and the first heading is gone for good — on the keyboard too, not just the
wheel.

**A click teleports.** Pointer motion drags the selection across every row it
crosses, and a click activates whatever is under it: the board closes and throws
you into another space. Reported as "clicking the pane title highlights its first
todo" — the heading click is inert, what moved the selection was the pointer
crossing that row on the way. Scanning a 37-row list with a pointer that selects
and a click that travels means one stray click loses your place entirely.

Both come from one decision: the board treats the selection, the pointer and the
visible window as the same thing. That is fine for a short menu. The board
stopped being one.

## What Changes

Pointer motion no longer moves the selection. The wheel moves the window, not
the selection. A click on an unselected row selects it and stops; a click on the
already-selected row travels to its pane, so travel stays a mouse-only two-tap
in one place. A link chip still follows its link on the first click — it is an
explicit target, not a row.

Selection-driven scrolling keeps the selected todo's group heading visible when
both fit.

## Impact

- Affected capability: `pane-todos` (the board requirement modified)
- Affected code: the board's mouse handling; a viewport-scroll and a
  reveal-with-context helper in the overlay kit's `ListCursor`; the board's
  group-heading lookup
- The kit gains two additive helpers; no existing overlay changes behaviour
- No server, API, protocol or config surface: presentation and input only
- **Archive order matters.** This change's `pane-todos` delta is a superset of
  `todo-board-legibility`'s, which is itself a superset of
  `overlay-header-gap`'s. Archive in that order: `overlay-header-gap`, then
  `todo-board-legibility`, then this one.

## Non-goals

- Changing the pane todo panel or the notification centre, which share the
  hover-selects/click-activates model. They act *in place*; only the board's
  activation closes the surface and moves you elsewhere, which is what makes a
  stray click expensive there and cheap in them. Revisit if either grows into a
  long scannable list too.
- Double-click. It would need a clock in `AppState`, which is pure data by
  design — herdr's existing double-click handling lives in the runtime layer for
  that reason. Click-again-on-the-selected-row needs no clock and no timing
  window to guess wrong.
- Showing a todo's full text on selection. That is the open discussion in
  AC-forks/herdr#67 and wants its own answer.
