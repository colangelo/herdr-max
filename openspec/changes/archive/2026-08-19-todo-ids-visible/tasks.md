## 1. Rows

- [x] 1.1 `render_pane_todo_row` draws `#<id>` dim at the row's right edge, with the link chip beside it and the text truncating first
- [x] 1.2 The board's content-width measurement includes the id

## 2. The editor

- [x] 2.1 Title reads `edit todo/note #<id>` when editing; `new todo/note` unchanged when composing

## 3. Tests

- [x] 3.1 Row shows the id; with a chip both survive and the text truncates first
- [x] 3.2 Editor title with and without an id
- [x] 3.3 Re-record board snapshots

## 4. Verification

- [x] 4.1 `just check` green
- [x] 4.2 Dogfood on the `-ac-beta` channel, including referencing a shown id straight into `herdr todo done`
