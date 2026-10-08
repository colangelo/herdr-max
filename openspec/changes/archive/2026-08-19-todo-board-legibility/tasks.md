Written before the code. The board's first dogfood produced three complaints at
once — no separation, too narrow, wrongly named — and they are independent
enough that bundling them into one commit would make any one of them hard to
back out.

## 1. The gap item

- [x] 1.1 Add `TodoBoardItem::GroupGap` in `src/app/state.rs`, documenting that it is an item rather than a render offset so one list row answers to one item
- [x] 1.2 Emit it in `todo_board_items()` between groups, never above the first
- [x] 1.3 Step the selection over it in both directions, alongside `PaneHeading`
- [x] 1.4 Make it inert to a click, alongside `PaneHeading`
- [x] 1.5 Measure it as zero-width in the board's content-width measurement, so it cannot widen the box

## 2. Indentation

- [x] 2.1 Draw a todo into a rect indented from the list's left edge, leaving `render_pane_todo_row` and therefore the pane todo panel untouched
- [x] 2.2 Name the indent once so the renderer and the content-width measurement read the same value
- [x] 2.3 Leave headings unindented — they are what the todos are indented from

## 3. Width and title

- [x] 3.1 Raise the width floor from 64 to 80 and the cap from 120 to 140, with content still deciding between them
- [x] 3.2 Title the board `todos/notes`
- [x] 3.3 Title the compose/edit overlay `new todo/note` and `edit todo/note`, both arms together, and check the longer title against the modal's fixed width
- [x] 3.4 Leave the CLI, the config key, the socket API and the protocol named `todo`

## 4. Tests

- [x] 4.1 Projection: a blank row between groups, none above the first, none when a single pane holds every todo
- [x] 4.2 Selection steps over a gap in both directions
- [x] 4.3 Clicking a gap is inert
- [x] 4.4 A gap does not contribute to the measured content width
- [x] 4.5 Re-record the board's rendered-layout snapshots for the title, the gaps and the indent
- [x] 4.6 Geometry: the board is at least the new floor wide, and still clamps at the new cap and at the screen

## 5. Verification

- [x] 5.1 `just check` green
- [x] 5.2 Dogfood on the `-ac-beta` channel against a real multi-pane session
