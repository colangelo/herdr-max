# A todo board you can work in

## Why

The board shipped, was dogfooded against a real session — eleven panes with
work, across ten spaces — and the verdict was "it's not great to work in
there". Three separate causes, none of them the grouping itself:

**Nothing separates one group from the next.** A heading and the todos under it
share a left edge and sit on adjacent rows, so eleven groups read as one
twenty-two-row block. The eye has to re-parse which row is a heading on every
line.

**It is too narrow.** The board sizes to its content between a floor of 64
columns and a cap of 120. A session whose longest row is 66 columns therefore
gets a 66-column box in the middle of a very wide terminal, and every todo is
truncated at a width the screen had no need to impose.

**It is called `todos`.** What a pane actually accumulates is as often a note to
self as a task — "answer question", "check verification done, anything to do?" —
and a title that names only tasks quietly tells the user the rest does not
belong.

## What Changes

A blank row between groups, and the todos of a group indented under its heading.
The blank row is a real item in the list, not an offset the renderer adds, so
one row still answers to exactly one item everywhere the list is scrolled,
selected or clicked.

The width floor rises from 64 to 80 and the cap from 120 to 140; content still
decides between them.

The title becomes `todos/notes`.

Nothing else is renamed. The `herdr todo` CLI, `keys.open_todo_board`, the
socket API and the protocol keep their names — the title is what the user
reads, and renaming the capability would cross the runtime/client boundary,
need a protocol bump, and add conflict surface on every upstream sync for no
gain the user asked for.

## Impact

- Affected capability: `pane-todos` (the board requirement modified for group
  separation, indentation, title and sizing)
- Affected code: `TodoBoardItem` and the board projection; the board's
  selection stepping and mouse hit-test, which gain a second inert item kind;
  the board's renderer, geometry constants and title
- The board grows one row per group beyond the first. On a session with ten
  groups that is ten rows, and a short terminal that previously just fit will
  now scroll. Accepted: the separation is what was asked for, and the screen
  clamp and scrolling already exist.
- No server, API, protocol or config surface: presentation only, client-side
  under the runtime/client guardrail
- **Archive order matters.** This change's `pane-todos` delta is a superset of
  the one in `overlay-header-gap`, because both modify the same requirement and
  archiving replaces its whole text. Archive `overlay-header-gap` first, or its
  heading rules are lost.

## Non-goals

- Renaming the capability. Considered and declined above.
- Rules between groups. A rule costs the same row a blank does and reads
  heavier; the blank plus the indent is what separation needs, and a rule can be
  added later without undoing this.
- Nesting panes under a shared space heading. It would stop the space name
  repeating across the two panes of one space, but costs a nesting level and
  reshapes the projection. Worth revisiting only if the repetition still grates
  once the groups are actually separated.
