# Design

## The gap is an item, not an offset

The obvious way to put a blank row between groups is to add one to the row
offset while rendering. It is also the way that breaks three things at once.

The board's list is windowed by `ListCursor::window(rect, items.len())`, its
selection is an index into `items`, and its mouse hit-test converts a clicked
row back into an index. All three rest on one invariant: **list row *n* is item
*n***. A renderer-side offset holds that invariant only in the renderer, so a
click below the first gap lands on the wrong todo and the scroll window
computes a height the list does not have.

So the gap is a `TodoBoardItem::GroupGap` emitted by the projection, between
groups and never before the first. It costs a variant and the projection gets
slightly longer; in exchange nothing downstream has to know it exists. The
board already has a non-selectable item kind — `PaneHeading` — and the
selection stepping, the hit-test and the width measurement each already match
exhaustively over the enum, so each one gets a one-line arm and the compiler
finds every site.

## Indentation narrows the rect, it does not fork the renderer

`render_pane_todo_row` is shared with the pane todo panel, deliberately: a todo
is supposed to look the same in both. Adding an `indent` parameter would put a
board concern into the panel's renderer and give every future caller a decision
to make.

Instead the board passes a rect that is already indented — origin moved right,
width reduced by the same amount — and the renderer draws what it always draws
into the space it is given. The todo's text budget shrinks by the indent, which
is correct: there genuinely is less room.

This is also why the indent is not applied to headings. A heading is the thing
being indented *from*; giving it the same treatment would leave the list looking
exactly as it does now, two columns to the right.

## Why a blank row rather than a rule

A rule (`"─".repeat(width)`, as the navigator and settings already draw one)
costs exactly the same single row as a blank. It was declined on ink, not cost:
eleven rules across a twenty-two-row list is more structure than the content
has, and the indent already tells the eye where a group starts. Blank rows are
also what the overlay kit's header and footer blocks already use to separate
title from content and content from buttons, so the board separating its groups
the same way keeps one idea rather than introducing a second.

If the separation still reads weakly once dogfooded, a rule replaces the blank
in the same row without changing the item model — which is the point of making
the gap an item.

## Why the heading keeps its dim weight

The heading is `overlay0` and bold, the weight the pane-move picker and the
sidebar give their section headings. Brightening it was considered as part of
the separation fix and rejected: hierarchy here is carried by position — the
indent puts todos visibly inside their heading — and dim-heading/bright-content
is the existing house reading of chrome versus content. Changing the colour
would diverge one overlay from the shared language to solve a problem the
indent already solves.

## `todos/notes`, and why nothing else is renamed

The title is the only place the user reads the feature's name while using it.
The CLI, the config key, the socket method and the protocol are addresses:
things typed once into a script or a config file and then depended on. Renaming
the title costs a string; renaming the addresses costs a protocol bump, a
migration for every existing `keys.open_todo_board` binding, and a permanent
divergence from upstream's `todo` naming on every sync.

`todos/notes` rather than `notes` because the store really is both, and the
existing `done` state, the priority glyphs and `clear done` all speak task. A
title that dropped `todos` would be describing half of it.

## Alternatives considered

**Nest panes under one space heading.** `infra` appears twice in the dogfooded
session, `herdr` twice. Grouping space → pane → todo removes the repetition and
adds a level of indentation and a second heading kind. Deferred, not rejected:
the repetition is far less costly than the missing separation, and it is much
easier to judge whether it still grates once the groups are apart.

**Widen by a fraction of the screen instead of a floor.** A board at 70% of a
wide terminal is enormous for a session with three todos. The floor-and-cap
already sizes to content; the complaint was that the floor was too low, so the
floor moved.
