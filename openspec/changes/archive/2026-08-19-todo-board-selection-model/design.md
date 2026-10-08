# Design

## Two additive helpers, no change to what the kit already does

`ListCursor` couples scroll to selection through `reveal`, which is right for
every overlay that is a menu. The board needs two things it does not have:

`scroll_by(delta, visible, len)` moves the window and leaves `selected` alone,
clamped so it cannot run past either end. Nothing else calls it, so no existing
overlay changes behaviour.

`reveal_scroll_with_context(scroll, context, index, visible, len)` reveals
`index` exactly as `reveal_scroll` does, then pulls `context` into view as well
when the two fit in the window together. The selection always wins: a group
taller than the window shows the selection, not the heading. This is where the
scroll bug is actually fixed — the floor stops being "the selection's index" and
becomes "the top of the selection's group".

The heading lookup stays on the board (`TodoBoardState::group_heading_index`),
because headings are a board concept and the kit should not learn about them.

## Why click-again rather than double-click

herdr has double-click, twice — pane text selection and the sidebar divider —
and both live in `App`, the runtime layer, because they need `Instant::now()`.
`AppState` is pure data with no clock, which is what lets the whole board be
tested without PTYs or async. Putting a timing window into it to save one
gesture would trade the project's clearest architectural line for a convention
users would still have to learn.

Click-again compares the clicked index to the selected index. No clock, no
window to tune, no double-fire on a slow trackpad, and it is testable in the
same breath as every other board test.

## Why the chip still acts on one click

A chip is a small, deliberately-aimed target that names where it goes. Making it
select-then-follow would mean a two-tap for the one control that is already
unambiguous, and the expensive mistake this change prevents — a stray click on a
row while scanning — is not a mistake anyone makes on a chip.
