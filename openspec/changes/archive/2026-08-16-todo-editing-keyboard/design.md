# Design

## What is actually reachable from a terminal

The readline set was checked against how Herdr receives keys, not assumed. Herdr
requests the Kitty keyboard protocol with `DISAMBIGUATE_ESCAPE_CODES |
REPORT_EVENT_TYPES | REPORT_ALTERNATE_KEYS` (`src/input/model.rs:64-68`), and
its own raw parser maps a bare `LF` to `ctrl+j` (`src/raw_input.rs:1533`,
`parses_raw_lf_as_ctrl_j`). That gives three tiers:

| Key | Reachable? | Notes |
| --- | --- | --- |
| `ctrl+a` `ctrl+e` `ctrl+b` `ctrl+f` `ctrl+d` `ctrl+k` `ctrl+u` `ctrl+w` `ctrl+y` | Everywhere | Plain C0 control bytes; no protocol needed. |
| `ctrl+j` `ctrl+k` (list motion) | Everywhere | `ctrl+j` is `LF` (`0x0A`), `Enter` is `CR` (`0x0D`) — distinct bytes, and Herdr already parses them apart. Config already accepts `ctrl+j` bindings (`navigate_pane_down = "ctrl+j"` appears in existing tests). |
| `alt+b` `alt+f` `alt+d` | Everywhere in practice | Sent as `ESC`-prefixed; already how Herdr receives `alt`. On macOS these are subject to the same Option dead-key caveat the sidebar jump ranges hit — `alt+f` is fine, `alt+e`/`alt+i`/`alt+n`/`alt+u` are not, and none of those are in this set. |
| `alt+Enter` (alternative save) | Only with the Kitty protocol | Ghostty, kitty, foot, WezTerm report it; Terminal.app does not. Accepted when it arrives, never the only way to save. |
| `ctrl+-` (undo) | Everywhere | See the correction below: this is the arm the legacy `0x1F` byte lands on. |
| `ctrl+/` (undo) | Sometimes | Many terminals send `0x1F` for it, i.e. it arrives on the same arm. Free to accept, not something to promise. |

**Corrected during implementation.** This table originally had the undo rows the
wrong way round, assuming a legacy `0x1F` would surface as `ctrl+_`. Herdr does
not use crossterm's key parser; its own (`src/input/parse.rs:124`) maps the byte
`31` to **`Char('-') + CONTROL`**. So on a terminal without the enhanced
protocol, `ctrl+_` arrives *as `ctrl+-`*, and it is the `ctrl+-` arm — not the
`ctrl+_` one — that satisfies "undo SHALL be bound to a chord a terminal without
the enhanced keyboard protocol can still deliver".

Rather than pick, the field accepts **`ctrl+_`, `ctrl+-`, and `ctrl+/` all
three**. They are the same action, the legacy and enhanced encodings both land,
and no user has to know which tier their terminal is in. Dogfooding on Ghostty
confirmed `ctrl+_` and `ctrl+-` both firing.

## The real work is a cursor, not keybindings

`handle_pane_todo_edit_text_key` (`src/app/input/modal.rs:686`) is append-only:
`text.push(c)`, `text.pop()`, `delete_last_word` popping from the end, `ctrl+u`
clearing the buffer. There is no insertion point anywhere in the modal's state —
`PaneTodoEditState.text` is a bare `String`.

Every motion in this change therefore needs the same missing thing, so the
change introduces one: `src/ui/text_field.rs`, a pure-data `TextField` holding
the buffer, a byte-offset cursor, a one-entry kill ring, and a bounded undo
stack. It renders nothing and owns no keymap; the modal keeps translating keys
into calls on it. That keeps the project's state/render split intact and lets
`overlay-ui-kit` adopt it without dragging modal-specific policy along.

Cursor is a byte offset into a `String`, moved only over `char` boundaries
(`unicode-width` is already a dependency for display, and the field's column
maths reuses it). Grapheme clusters are not handled: a combining sequence moves
one `char` at a time, which matches how the rest of Herdr's text handling
behaves today and avoids a new dependency.

## Key conflicts, and how each is resolved

Three of the requested keys are already taken inside the edit modal
(`handle_pane_todo_edit_key_via_api`, `modal.rs:1195`), and the conflicts are
real rather than cosmetic:

| Key | Today | After | Why |
| --- | --- | --- | --- |
| `Enter` | Save | Insert newline | A multi-line field cannot have `Enter` mean "commit". Save moves to `ctrl+s`, with `alt+Enter` accepted where reported. |
| `ctrl+d` | Toggle done | Delete forward | `ctrl+d` is the single most reflexive readline key; done-toggle is the newer, rarer action, so it yields. It moves to `ctrl+t` — outside the readline set, and the done row stays clickable. |
| `ctrl+u` | Clear whole buffer | Kill to line start | Readline semantics. Clearing everything is still one keystroke whenever the cursor is at the end, which is where it sits right after typing. |
| `ctrl+l` | Open link picker | Unchanged | Emacs' `ctrl+l` is recenter, which a three-line field does not need. |
| `ctrl+k` | (free in the modal) | Kill to line end | It is bound in copy mode, a different mode; no collision. |
| `ctrl+y` | (free) | Yank last kill | Herdr's own kill ring, **not** the system clipboard. System paste stays on bracketed paste. |

`ctrl+s` deserves one note: on a terminal without `IXON` disabled it would be
swallowed as flow control, but Herdr runs in raw mode, so it arrives as a key.

## Newlines end-to-end

The store needs no change. `validate_text` (`src/terminal/todo.rs:102-109`)
trims the ends and enforces `MAX_TODO_TEXT_LEN`; it never inspects for control
characters, so `"a\nb"` already round-trips through `todo.add`, the snapshot, and
restore today. Two places do need to stop stripping or start coping:

- `paste_into_active_text_input` (`src/app/input/mod.rs`) filters
  `!ch.is_control()` for the todo field. Newline must survive; everything else
  keeps being dropped, so a pasted `ESC` still cannot compose an escape sequence
  into a todo.
- The panel lists one row per todo and sizes itself from `todos.len()`
  (`pane_todo_panel_rect`, `mouse.rs:1589`). Multi-line todos render as their
  first line plus a marker; the geometry is untouched. This is deliberate — a
  panel whose row heights vary would have to re-derive hit-testing per row, and
  that is exactly the duplication `overlay-ui-kit` is trying to remove.

The 500-character cap counts characters including newlines, unchanged.

## `ctrl+j` / `ctrl+k` in the picker

The picker opens the navigator with `search_focused = false`
(`open_navigator_from`, `actions.rs:354`), i.e. list mode, where the match arms
for `Char('j')` / `Char('k')` (`modal.rs:263-268`) carry **no modifier guard** —
so `ctrl+j`/`ctrl+k` already move the selection there, by accident rather than
by design. Search mode (`modal.rs:180-213`) handles only `Up`/`Down` and
`ctrl+n`/`ctrl+p`.

The fix is small and mostly about intent: add `ctrl+j`/`ctrl+k` explicitly to
the search-state arms, and make the list-state arms explicit about accepting
them, so the behaviour is specified rather than emergent. Plain `j`/`k` stay
list-only, since in search mode they are text.

## Leading with the identifier

`TodoInfo.link_pane_id` (`src/app/api/todos.rs:76-93`) already resolves the
target to its public identifier *in the target's own workspace*, and is `None`
exactly when the link is dead. So the chip needs no new data:

- live link → `→ w2:pC · claude`, identifier first because it is the part you can
  act on, label second because it is the part you recognise;
- dead link → `→ claude`, dimmed and inert as today, because there is no
  identifier to lead with.

The identifier is derived at render time, never stored. Public pane identifiers
are workspace-scoped and positional, so storing one would let it go stale;
deriving it means it is either correct or absent. The captured label stays in
`TodoLink` as the dead-link fallback, unchanged.

This also lowers the stakes of the open question about whether the *capture*
chain should consult the live terminal title: a link that leads with `w2:pC` is
addressable even when its label is the raw identifier. That decision stays
where it is and is not folded into this change.

## Alternatives considered

**Adopt `tui-textarea` instead of writing `TextField`.** It is the mature
option and would give grapheme handling and multi-line editing for free. Rejected
for now: it owns its own key handling and widget rendering, which collides with
Herdr's "render is pure, keymaps live in `src/app/input`" split, and it would put
a dependency between the modal and a crate whose keymap we would then have to
override key by key. The field this change needs is ~200 lines of pure data.
Worth revisiting if grapheme-correct editing or soft-wrapped multi-line rendering
becomes a requirement.

**Keep `Enter` as save and put newline on `alt+Enter`.** Preserves muscle
memory, but `alt+Enter` is Kitty-protocol-only, so on Terminal.app multi-line
would silently not exist. Rejected: a feature that vanishes based on the host
terminal is worse than a rebinding.

**Make multi-line opt-in per config.** Two code paths through the same field,
two sets of key semantics, and the save key would have to differ between them.
Rejected as more surface than the feature is worth.
