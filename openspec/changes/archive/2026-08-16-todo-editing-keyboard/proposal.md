# Todo editing: a real text field, keyboard-only link picking, and addressable links

## Why

`pane-todos-ux` shipped the picker and the always-on indicator, and daily use of
it surfaced four gaps that are all about *typing and moving without leaving the
home row*.

**The text field is not a text field.** It is an append-only buffer:
`handle_pane_todo_edit_text_key` only ever does `text.push(c)`, `text.pop()`, or
a whole-word/whole-buffer delete. There is no insertion point, so a typo three
words back means deleting everything after it. Every readline reflex — jump to
the start, jump to the end, delete the character ahead, kill to end of line,
yank it back — is unavailable, not because the keys are taken but because the
buffer has no cursor to move.

**A todo is one line whether or not the thought is.** The store already accepts
newlines (`validate_text` trims the ends and caps length; it does not reject
control characters), but nothing can produce one: `Enter` saves, and paste
filters every control character out. A todo that wants a second line has to
become two todos or one run-on line.

**The picker needs arrow keys once you search.** The link picker opens the
navigator in list mode, where `j`/`k` move. Press `/` to filter — which is the
whole point of a picker over a session with dozens of panes — and the only ways
to move the selection are the arrow keys and `ctrl+n`/`ctrl+p`. The hand leaves
the keyboard's middle for a list that is otherwise entirely keyboard-driven.

**A link says who but not where.** The chip renders the captured label — `zsh`,
`claude`, a manual label — which names the target but does not address it. The
thing that is actually actionable is the pane's public identifier (`w2:pC`):
it is what you type into `herdr pane`, `herdr agent read`, or a sibling agent's
prompt. It is already computed for every live link (`TodoInfo::link_pane_id`)
and simply is not shown.

## What Changes

- **A cursor-bearing text field** backs todo composition, with readline motions
  and kills: `ctrl+a`/`ctrl+e` (line start/end), `ctrl+b`/`ctrl+f` and the arrow
  keys (character), `alt+b`/`alt+f` (word), `ctrl+d` (delete forward),
  `ctrl+k` (kill to end), `ctrl+u` (kill to start), `ctrl+w` (kill word back),
  `ctrl+y` (yank the last kill), and undo. Text is inserted **at the cursor**
  rather than appended.
- **Todo text may hold newlines.** In a multi-line-capable field `Enter` inserts
  a newline and saving moves to an explicit key, so committing a todo is never
  ambiguous with adding a line to it. Paste keeps newlines and still drops other
  control characters. The panel keeps its one-row-per-todo geometry by showing
  the first line with a continuation marker.
- **The link picker moves under `ctrl+j`/`ctrl+k`** in both its list and search
  states, alongside the existing arrows and `ctrl+n`/`ctrl+p`, so a search never
  demotes the keyboard to arrow keys. The same keys work in the navigator's
  ordinary goto mode, because it is the same surface.
- **A live link is addressed, not just named:** the chip and the edit modal's
  link row lead with the target's public pane identifier and follow it with the
  captured label, and the picker's pane rows carry the identifier they would
  stage. A dead link, which has no identifier to show, keeps its label alone.

## Impact

- Affected specs: `pane-todos` — `Cross-pane todo links` and `Pane todo panel
  and editing` modified, `Todo text composition` added.
- Affected code: a new `src/ui/text_field.rs` (cursor, kill ring, undo — pure
  data, no rendering), `src/app/input/modal.rs` (todo edit key handling, the
  navigator's search-state keys), `src/app/state.rs` (`PaneTodoEditState.text`
  becomes the field), `src/app/input/mod.rs` (paste admits newlines),
  `src/ui/todo_panel.rs` (chip composition, first-line rendering),
  `src/ui/navigator.rs` (identifier on pane rows), `src/ui/keybind_help.rs`.
- **No wire changes.** `validate_text` already accepts embedded newlines, and
  `TodoInfo.link_pane_id` already carries the public identifier resolved in the
  target's own workspace. No schema field, protocol version, snapshot shape, or
  CLI flag moves.
- Behaviour changes for existing users, all deliberate:
  - `Enter` no longer saves the edit modal; it inserts a newline. Save is its own
    key. This is the one piece of muscle memory this change breaks.
  - `ctrl+d` in the edit modal stops toggling done and becomes delete-forward;
    the done toggle moves to a key that is not part of the readline set.
  - `ctrl+u` stops clearing the whole buffer and becomes kill-to-start, which
    clears the whole buffer only when the cursor is at the end.
- Depends on `pane-todos-ux` being archived first: both changes modify the same
  two requirements, and this one's deltas are written against that change's text
  (always-on indicator, picker-based linking, launched-command label chain).
- Sequencing with `overlay-ui-kit`: this change introduces the text field for
  todo composition only. `overlay-ui-kit` adopts it for the rename, worktree, and
  search inputs. Deliberately in this order — the user-facing gap ships without
  waiting on a refactor, and the refactor gets a primitive proven in one place
  before it is spread across nine.

## Non-goals

- Rewriting the other text inputs (rename modals, worktree create, the navigator
  and keybind-help search boxes). That is `overlay-ui-kit`.
- A full emacs keymap. Transpose, case-changing, mark-and-region, and a
  multi-entry kill ring are not implemented; the kill ring holds one entry.
- Rendering multi-line todo text anywhere except the edit modal. The panel, the
  CLI, and the API keep showing the stored text as-is.
- Changing which label the *server* captures on a link. Leading with the public
  identifier reduces how much that label has to carry, but the capture chain
  stays as `pane-todos-ux` left it.
