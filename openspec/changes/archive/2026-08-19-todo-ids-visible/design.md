# Design

## The id goes rightmost, the chip beside it

The row already right-aligns its link chip. The id claims the true right edge
and the chip moves one id-width left: ids then sit in one scannable column
whether or not a row has a chip, which is what "read an id off the board"
needs. Both are addresses, and the todo's own text is what truncates first —
the text identifies, the addresses act.

## `#12`, not `12`

A bare dim number at a right edge reads as a count next to the indicators that
actually are counts (`τ 6`, `▾ 3`). The `#` marks it as an identity. The CLI
accepts the bare number; the `#` is display only.

## Editing gets the id, composing does not

`edit todo/note #12` names the thing being changed — the same id the user may
just have read off the board. A new todo has no id until the store assigns
one, and showing a placeholder would teach that ids are guessable.
