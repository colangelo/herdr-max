# Room to read in the todo editor

## Why

The compose/edit overlay is a 60-column modal whose text block is 3 rows and
scrolls horizontally: a line longer than ~58 columns slides out of view, and a
multi-line todo shows three lines of itself. Dogfooding multi-point todos (the
kind agents now leave via `herdr todo add`) made both limits bite at once — the
text is unreadable exactly when there is most of it, and reviewing it means
arrow-key travel through text that cannot be seen (raised alongside #68).

## What Changes

The modal grows (60x14 → 84x20, clamped by the screen) and its text block
grows from 3 rows to 8. The block soft-wraps at word boundaries instead of
scrolling horizontally; explicit newlines remain hard breaks; the caret stays
visible by vertical scroll in wrapped visual rows; clicks resolve through the
same wrap layout the renderer used. The stored text is untouched — wrapping is
presentation only.

## Impact

- Affected capability: `pane-todos` (one added requirement on the editor)
- Affected code: a wrap layout shared by render, caret math and the mouse
  hit-test; the editor's geometry constants and rects; the input block
  renderer; the removal of the horizontal column scroll
- The 500-character cap bounds the layout: at 82 text columns even a
  cap-length single paragraph wraps to about 7 rows, so the grown block shows
  a whole typical todo
- No server, API, protocol or config surface: presentation only

## Non-goals

- Up/Down moving by wrapped visual row. They keep moving by logical line —
  with hard newlines marking a todo's points, that is motion between points,
  which is the motion that matters. Revisit only if it grates in practice.
- Wrapping the board's or panel's one-line rows (that discussion is #67).
- A growable or resizable modal.
