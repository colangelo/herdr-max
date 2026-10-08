Written after the code, not before: this began as two dogfooding observations
on the todo board rather than as planned work, and the spec delta is what makes
the heading-order decision reviewable rather than a silent contradiction of the
requirement `todo-board` archived a day earlier.

## 1. The kit

- [x] 1.1 Add `HEADER_ROWS` to `src/ui/widgets.rs` beside `FOOTER_ROWS`, documenting it as the same rule at the other end
- [x] 1.2 Add `header_split(inner)` for the overlays that place rows by offset from `inner`
- [x] 1.3 Name the pane-move picker's two-line header once (`PANE_MOVE_TARGET_HEADER_ROWS`) so its renderer and its mouse hit-test read the same value

## 2. The overlays that were flush

- [x] 2.1 Todo board: chrome rows become border + header block + footer block, and the list starts below the header block
- [x] 2.2 Pane-move picker: one row taller, rows start below the header block, and the mouse row mapping shifts with them
- [x] 2.3 New worktree: one row taller, a spacer row under the title, and every row below it remapped
- [x] 2.4 Open worktree: one row taller, search / separator / entries all below the header block
- [x] 2.5 Remove worktree: one row taller, a spacer row under the title
- [x] 2.6 Leave the four that already left the gap alone (keybind help, rename, todo edit, release notes / product announcement)

## 3. The board's heading

- [x] 3.1 Carry the owning space's display name on the heading item, resolved without the terminal runtimes so the projection stays a pure `&self` read
- [x] 3.2 Render `space · label`, omitting either part when the pane does not have it rather than showing it empty
- [x] 3.3 Drop the addressable identifier from the heading, leave the todo link chip carrying its own, and record why one shows it and the other does not

## 4. Sizing

- [x] 4.1 Grow the board to its content in both directions, clamped by the screen, instead of a fixed 64x20 box that scrolled as soon as a few panes had work
- [x] 4.2 Measure the content width on `AppState` so the renderer and the mouse hit-test read one answer
- [x] 4.3 Keep a minimum height so an empty board still reads as a panel, and a maximum width so it never spans a very wide terminal

## 5. The board's `clear done` scope

- [x] 5.1 Clear the completed todos of every pane the board is showing, before the selected-todo lookup, rather than the selected todo's pane alone
- [x] 5.2 Record the reversal of `todo-board`'s decision, and the rule it leaves behind

## 6. Tests

- [x] 6.1 Re-record the board's rendered-layout snapshots for the header block and the new heading
- [x] 6.2 Assert the board's blank row between title and first entry, alongside the existing footer-gap assertion
- [x] 6.3 Update the pane-move picker's render and mouse tests to the named header height rather than a literal offset
- [x] 6.4 Update the new-worktree caret test to track the input row
- [x] 6.5 Heading-text unit tests: full form, missing label, missing space — and no identifier in any of them

- [x] 6.6 Sizing tests: grows for a long row and for many items, clamps at its cap and at the screen, and keeps its minimum when empty

- [x] 6.7 Clear-done tests: reaches a pane the selection is not on, and leaves outstanding todos alone

## 7. Verification

- [x] 7.1 `just check` green
- [x] 7.2 Dogfood on the `-ac-beta` channel: the board's title gap and heading, and one worktree dialog
