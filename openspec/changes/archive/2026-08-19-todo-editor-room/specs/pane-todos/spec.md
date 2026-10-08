# pane-todos

## ADDED Requirements

### Requirement: Todo editor readability

The todo compose/edit overlay SHALL be sized for reading and writing a full
todo: wide enough that prose does not immediately leave the visible area, and
with a text block tall enough that a todo near the length cap is mostly visible
at once, clamped by the screen.

The text block SHALL soft-wrap its content at word boundaries to the block's
width instead of scrolling horizontally. Explicit newlines SHALL be preserved
as hard breaks. A word longer than the block's width SHALL break mid-word
rather than disappear off the edge. The caret SHALL remain visible through
wrapping: the block scrolls vertically, in wrapped visual rows, by the least
amount that keeps the caret's row on screen.

A mouse click on the text SHALL place the caret at the clicked character,
resolved through the same wrap layout the renderer used, so the caret lands
where the pointer is.

Wrapping SHALL be presentation only: the stored todo text is unchanged, and no
soft break introduces a character into it.

#### Scenario: Long prose wraps instead of escaping sideways

- **WHEN** the todo's text is wider than the text block
- **THEN** it wraps at word boundaries onto following rows
- **AND** no horizontal scrolling occurs and no text is cut off at the right edge

#### Scenario: Hard newlines survive wrapping

- **WHEN** the todo's text contains explicit newlines
- **THEN** each newline starts a new row exactly as typed
- **AND** saving returns the text with only the author's own newlines in it

#### Scenario: The caret stays visible while typing past a wrap

- **WHEN** typing carries the caret past the block's width or below its last row
- **THEN** the caret continues on the next wrapped row, scrolling the block vertically when needed

#### Scenario: Clicking wrapped text places the caret at the clicked character

- **WHEN** the user clicks a character on any wrapped row
- **THEN** the caret moves to that character in the underlying text
