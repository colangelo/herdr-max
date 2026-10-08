# Design

## One layout, three consumers

Everything hangs off a single pure function: given the text and a width, produce
the list of visual rows, each naming its source line and the byte range of that
line it shows. The renderer draws the rows, the caret math asks "which visual
row and column is the cursor's (line, column) on", and the mouse hit-test asks
the inverse. One layout means the caret can never sit where the renderer did
not put the character, which is the invariant the modal's existing rects
comment already demands ("clicking priority lands on the row that says
priority" — now per character).

The layout lives beside `TextField` rather than in `dialogs.rs`, because it is
a property of editable text, not of this modal; the modal keeps only its
geometry.

## Wrapping rules

Break at the last space that fits; a word wider than the block hard-breaks at
the width. Widths are display columns (`unicode-width`), not bytes or chars, so
wide glyphs wrap where they visually are. The space a line breaks at is
consumed by the break — rendering it at the row edge would show a ragged
gutter; it still exists in the stored text. An explicit newline always ends a
row. The empty text still yields one empty row, so the caret has somewhere to
be.

## Scroll becomes visual-vertical only

`pane_todo_edit_column_scroll` is deleted, not adapted: horizontal scroll
exists only because lines could exceed the width, which wrapping makes
impossible. `pane_todo_edit_line_scroll` keeps its derived shape — the least
scroll keeping the caret visible — but counts wrapped visual rows instead of
logical lines, and stays derived from the caret so the view cannot drift.

## Cost

The layout runs per rendered frame over at most 500 characters (the store's
cap, enforced server-side) at a fixed width: one linear pass, a handful of
small allocations, only while the modal is open. Nothing here is on a
pane-scaled path.

## Sizing

60x14 → 84x20, text block 3 → 8 rows. 84 gives the text ~80 columns; at the
500-character cap a single unbroken paragraph wraps to ~7 rows, so a whole
typical todo is visible at once. Still fixed and centered: the modal's job did
not change, it was only cramped. The shell clamps to the screen as it already
does for every modal.
