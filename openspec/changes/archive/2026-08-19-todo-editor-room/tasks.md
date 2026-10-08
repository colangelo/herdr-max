## 1. The wrap layout

- [x] 1.1 A pure wrap-layout beside `TextField`: text + width → visual rows, each naming its source line and byte range; word-boundary breaks, hard breaks for over-wide words, newlines always end a row, empty text yields one row
- [x] 1.2 Caret mapping: (line, column) → (visual row, visual column) through the layout
- [x] 1.3 Click mapping: (visual row, visual column) → (line, column), clamping into the nearest real position

## 2. The modal

- [x] 2.1 Grow the modal 60x14 → 84x20 and the input block 3 → 8 rows
- [x] 2.2 Render the input block through the layout; delete the column scroll
- [x] 2.3 `pane_todo_edit_line_scroll` counts wrapped visual rows, still derived from the caret
- [x] 2.4 Route the mouse click through the click mapping

## 3. Tests

- [x] 3.1 Layout unit tests: word break, over-wide word, preserved newlines, consumed break space, wide glyphs, empty text
- [x] 3.2 Caret and click mappings round-trip on wrapped rows
- [x] 3.3 Rendered-layout: long prose shows wrapped, nothing truncated right, caret visible after typing past a wrap
- [x] 3.4 Existing editor tests re-recorded for the new geometry

## 4. Verification

- [x] 4.1 `just check` green
- [x] 4.2 Dogfood on the `-ac-beta` channel with a cap-length multi-point todo
