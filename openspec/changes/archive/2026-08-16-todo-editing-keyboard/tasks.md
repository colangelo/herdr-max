Ordered so each group lands green on its own. Group 1 is the primitive
everything else calls; groups 4 and 5 are independent of it and can land first
if the field turns out to need more design time.

Prerequisite: `pane-todos-ux` archived, since groups 2, 3, and 5 modify
requirements that change is still holding open.

## 1. The text field primitive

- [x] 1.1 Add `src/ui/text_field.rs`: `TextField { text: String, cursor: usize, kill: String, undo: Vec<(String, usize)> }`, pure data, no rendering and no keymap. Cursor is a byte offset moved only over `char` boundaries
- [x] 1.2 Motions: `move_char(dir)`, `move_word(dir)`, `move_home()`, `move_end()`. Word class reuses the existing `word_delete_class` from `src/app/input/modal.rs` rather than inventing a second definition — lift it into the field
- [x] 1.3 Edits: `insert_str`, `delete_backward`, `delete_forward`, `kill_to_end`, `kill_to_start`, `kill_word_backward`, `yank`, each pushing the pre-edit `(text, cursor)` onto a bounded undo stack (cap 32, oldest dropped)
- [x] 1.4 `max_chars` guard on the field itself so every insertion path stops at the store's limit
- [x] 1.5 Tests: each motion and each edit against multi-byte text (accents, CJK, emoji); undo after a kill and after a run of insertions; yank at a moved cursor; the limit rejecting an insert that would exceed it while leaving the buffer intact

## 2. Wire the todo edit modal to the field

- [x] 2.1 Change `PaneTodoEditState.text` (`src/app/state.rs`) from `String` to `TextField`, keeping the saved value as its `text`
- [x] 2.2 Rewrite `handle_pane_todo_edit_text_key` (`src/app/input/modal.rs:686`) as a translation layer: `ctrl+a/e/b/f`, `alt+b/f`, arrows, `ctrl+d/k/u/w/y`, `ctrl+_` (accepting `ctrl+-` when reported), Backspace, Delete
- [x] 2.3 Move the commit key off `Enter` to `ctrl+s`, additionally accepting `alt+Enter`; keep `Esc` cancelling
- [x] 2.4 Move the done toggle off `ctrl+d` to `ctrl+t` in `handle_pane_todo_edit_key_via_api` (`modal.rs:1195`), leaving the done row clickable
- [x] 2.5 Render the cursor in the edit modal's text row, including when the text scrolls horizontally
- [x] 2.6 Update `src/ui/keybind_help.rs` for the moved commit and done-toggle keys
- [x] 2.7 Tests: typing mid-buffer inserts rather than appends; `ctrl+d` deletes forward and does not toggle done; `ctrl+s` saves and `Enter` does not; `Esc` still cancels leaving the todo untouched

## 3. Newlines end to end

- [x] 3.1 `Enter` inserts a newline in the field
- [x] 3.2 Stop stripping `\n` in `paste_into_active_text_input` (`src/app/input/mod.rs`) for the todo field; keep dropping every other control character
- [x] 3.3 Render the edit modal's text row as multiple lines, sizing the modal for a bounded number of them and scrolling beyond that
- [x] 3.4 Render a multi-line todo in the panel as its first line plus a continuation marker, leaving `pane_todo_panel_rect`'s one-row-per-todo sizing untouched
- [x] 3.5 Tests: a two-line todo saves, round-trips through snapshot restore, and comes back over `todo.list`; paste keeps `\n` and drops `\x1b`; the panel shows one row for it; the character limit counts newlines

## 4. Keyboard-only picker movement

- [x] 4.1 Add `ctrl+j` / `ctrl+k` to the navigator's search-focused arms (`src/app/input/modal.rs:180-213`), beside the existing `ctrl+n` / `ctrl+p` and arrows
- [x] 4.2 Make the list-state arms (`modal.rs:263-268`) explicit about accepting `ctrl+j` / `ctrl+k`, which they currently match only because they carry no modifier guard
- [x] 4.3 Tests: with search focused, `ctrl+j` moves the selection and leaves the query unchanged; with search not focused, `ctrl+j` and plain `j` both move; the same in the picker's `PaneTodoLink` purpose

## 5. Links that address as well as name

- [x] 5.1 Compose the chip in `pane_todo_link_chip` (`src/ui/todo_panel.rs:122`) as `→ <public id> · <label>` for a live link and `→ <label>` for a dead one, deriving the id from the live target
- [x] 5.2 Same composition for the edit modal's link row (`pane_todo_edit_link_label`, `src/app/state.rs:2200`)
- [x] 5.3 Show the public identifier on the picker's pane rows (`src/ui/navigator.rs`), in the `PaneTodoLink` purpose at minimum
- [x] 5.4 Keep the chip's truncation budget honest: the identifier is short and fixed-ish, the label takes what is left
- [x] 5.5 Tests: a live link renders id-then-label and a dead one renders label alone; a target that moves renders its new id; the rendered chip cells still equal what `pane_todo_link_chip` reports for hit-testing

## 6. Docs

- [x] 6.1 `docs/next/website/src/content/docs/keyboard.mdx`: the editing set, the moved commit and done-toggle keys, `ctrl+j`/`ctrl+k` in the picker
- [x] 6.2 `docs/next/website/src/content/docs/configuration.mdx` if any binding becomes configurable — none did: every chord this change adds belongs to the edit modal's own keymap, not `KeysConfig`, so they are documented in `keyboard.mdx` and the keybind help panel instead
- [x] 6.3 `docs/next/CHANGELOG.md` under Unreleased, calling out the three behaviour changes (`Enter`, `ctrl+d`, `ctrl+u`) explicitly rather than burying them
- [x] 6.4 Translation parity for `ja` and `zh-cn` per `scripts/docs_translation_parity.py` — the check is structural (heading outlines) and passes; no headings were added, and neither translation has ever carried the pane-todos section

## 7. Dogfood

- [x] 7.1 Ship a beta per the `herdr-dogfood` skill and confirm on a live session: mid-buffer editing, a two-line todo, `ctrl+j`/`ctrl+k` while searching the picker, and a chip reading `w2:pC · claude` — shipped as `0.7.5-ac-beta.51-fagioli` and handed off live with panes preserved; confirmed against a seeded todo on `w5:p3` linked to `w2:p1`, whose chip reads `→ w2:p1 · infra-homepage`
- [x] 7.2 Confirm undo arrives on the host terminal in use, and note in the change which chord actually landed — on Ghostty, **both `ctrl+_` and `ctrl+-` land**. That is the enhanced-protocol path, where each arrives as its own `Char(..)+CONTROL`. `ctrl+/` is the same action and was not distinguished separately. The legacy path is covered by the `ctrl+-` arm either way: Herdr's own parser maps the bare `0x1F` byte to `ctrl+-` (`src/input/parse.rs`), which is what makes undo reachable without the enhanced protocol — a detail the design had the wrong way round, assuming `0x1F` would surface as `ctrl+_`
