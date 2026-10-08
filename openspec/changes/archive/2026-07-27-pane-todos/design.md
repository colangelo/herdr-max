# Design: pane todos

## Runtime/client boundary

Per the guardrail in `CLAUDE.md`, a pane's todo list is a **shared
runtime/session fact**: it is authored by agents over the CLI, consumed by
external status bars over the event stream, and persisted with the session. It
therefore lives in server state and is exposed through the JSON API.

Only presentation state stays in the TUI client: whether the dropdown is open,
which row is selected, and the edit modal's in-progress buffer. None of that is
persisted or exposed over the API.

API names are neutral (`todo.*`, `pane_id`, `link_pane_id`) — no `dropdown`,
`row`, or `panel` in the wire vocabulary.

## Data model

```rust
pub struct PaneTodo {
    pub id: u64,                 // monotonic per pane, never reused
    pub text: String,
    pub done: bool,
    pub priority: TodoPriority,  // High | Normal | Low
    pub link: Option<TodoLink>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

pub struct TodoLink {
    pub pane: Option<PaneId>,    // None = dead link (target gone)
    pub label: String,           // captured at link time, display only
}
```

Stored in insertion order on **`TerminalState`** (`src/terminal/state.rs`), not
`PaneState`. `PaneState` is viewport-only — its own doc comment says identity,
cwd, labels, and agent metadata live in `TerminalState`, and `PaneSnapshot` is
built entirely from the terminal, so todos placed on `PaneState` would not
persist. Storing them on the terminal also means todos follow the work through
`break_pane` and `move_pane_to_tab`, which preserve the running terminal.

**Sorting is a view concern**: the
display order sinks `done` items to the bottom first, then sorts by priority
descending, then by creation order. `done` is the primary key, so an outstanding
low-priority todo still ranks above a completed high-priority one. Keeping the stored order stable means an edit that changes
priority cannot renumber ids or lose the original sequence, and the API returns
a deterministic list independent of how the TUI chooses to sort it.

Limits: 50 todos per pane, 500 characters per todo. Both are server-enforced
with explicit errors. Agents write these unattended, and without limits a
looping agent grows the session snapshot without bound.

### Why the link stores a `PaneId`, not a public id

`PaneId` is a process-global monotonic counter (`layout.rs`, `NEXT_PANE_ID`) and
is never recycled within a server lifetime. Restore does not reuse saved raw
ids: it allocates fresh ones and builds an `id_map` (old raw → new `PaneId`)
that already fixes layout references. Storing the internal `PaneId` lets link
targets ride that same remap.

Public ids (`w1:p3`) come off a per-tab counter and are what the CLI accepts, so
`--link` takes the public id and resolves it to a `PaneId` at add time. The
captured `label` exists so a dead link can still say what it meant. Resolving a
link by a unique live agent name is deferred (see below).

## Persistence

`PaneSnapshot` gains:

```rust
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub todos: Vec<PaneTodoSnapshot>,
```

`PaneTodoSnapshot` stores the link target as the **old raw u32**, matching how
the rest of the snapshot refers to panes. On restore, each link is remapped
through `id_map`; a target missing from the map becomes `link.pane = None`,
preserving the label as a dead link. The todo itself is never dropped.

Because live handoff serializes through the same `PaneSnapshot` path, todos ride
`herdr update --handoff` with no additional work.

## API

| Method | Params | Result |
|---|---|---|
| `todo.list` | `{ pane_id? }` | `{ todos: [TodoInfo] }`, each carrying its `pane_id`; omitting `pane_id` returns every pane's todos |
| `todo.add` | `{ pane_id, text, priority?, link_pane_id? }` | `{ todo }` |
| `todo.update` | `{ pane_id, id, text?, done?, priority?, link_pane_id?, clear_link? }` | `{ todo }` |
| `todo.remove` | `{ pane_id, id }` | `{}` |
| `todo.clear` | `{ pane_id, done_only? }` | `{ removed: u32 }` |

Event: `todo.changed { pane_id }` on the existing subscription stream, emitted
once per mutating call, mirroring `notification.posted`.

Errors: `pane_not_found`, `todo_not_found`, `todo_text_empty`,
`todo_text_too_long`, `todo_limit_reached`, `todo_link_unresolved` (unknown or
ambiguous link target).

**No protocol bump.** `CLAUDE.md` says to bump `PROTOCOL_VERSION` only when the
current source protocol is not already greater than the latest released one.
Source is at 19 (the unreleased notification-center bump) and the latest
released fork tag `v0.7.4-ac` shipped 18, so 19 already covers these additions —
exactly as `notification.clear` rode the same bump. Existing protocol
expectations in `tests/cli/sessions.rs`, `tests/api_ping.rs`, and
`tests/support/mod.rs` therefore stay at 19 and need no edits.

## CLI

```
herdr todo add <text> [--pane ID|--current] [--priority high|normal|low] [--link <target>]
herdr todo list [--pane ID|--current|--all] [--json]
herdr todo done <id> [--pane ID|--current]
herdr todo undone <id> [--pane ID|--current]
herdr todo edit <id> [--text <s>] [--priority <p>] [--link <target>|--unlink] [--pane ID|--current]
herdr todo rm <id> [--pane ID|--current]
herdr todo clear [--done] [--pane ID|--current]
```

Target resolution reuses the grammar in `src/cli/pane.rs`
(`parse_optional_current_pane_args`): explicit `--pane`, `--current`, or the
`HERDR_PANE_ID` environment default. The default matters most: an agent about to
exit runs `herdr todo add "..."` with no target and it lands on its own pane.

`--link <target>` accepts a public pane id (`w1:p2`). Resolving a link by a
unique live agent name, using the same uniqueness rule agent commands already
enforce, is **deferred out of phase 1**: phase 1 resolves link targets with
`App::parse_pane_id`, so any agent name — unique or ambiguous — fails with
`todo_link_unresolved`. That keeps the "Ambiguous link targets are rejected"
requirement satisfied (an ambiguous name never links), and leaves the unique
case as a strictly additive follow-up rather than a behaviour change. Phase 2
picks it up (tasks 4.4); the CLI docs describe pane-id targets only until then.

## TUI

### Indicator

Drawn in the pane's top border at the far right, next to `render_pane_border_titles`
in `src/ui/panes.rs`. A single helper

```rust
pub(crate) fn pane_todo_indicator_rect(pane_rect: Rect) -> Option<Rect>
```

is used by **both** the renderer and the mouse hit-test in
`src/app/input/mouse.rs`, modeled on `expanded_sidebar_toggle_rect`. Sharing one
function is what keeps the drawn glyph and the click target from drifting; a
test asserts they agree.

- `▾N` where N is the count of **outstanding** (not done) todos.
- All todos done → bare dimmed `▾`. No todos at all → nothing rendered, so quiet
  panes look exactly as they do today.
- Colored by the highest outstanding priority; `ui.pane_todo_color` overrides.
- The indicator reserves its cells before the title is laid out, so a narrow
  pane truncates the title rather than dropping the control. Below the existing
  minimum-width threshold, neither is drawn.

### Dropdown

A new `Mode::PaneTodos` overlay in `src/ui/todo_panel.rs`, anchored under the
indicator and clamped to the frame, reusing the notification center's overlay
and footer-button language.

Rows render `<priority glyph> <text> <→ link chip>`; done rows are dimmed and
struck; dead links are dimmed and inert.

| Input | Action |
|---|---|
| `Up`/`Down`, `j`/`k` | move selection |
| `Enter`, row click | open the edit view |
| `Space` | toggle done |
| `g`, click the `→` chip | jump to the link target via `focus_pane_in_workspace` |
| `d` | remove the selected todo |
| `c` | clear done todos |
| `Esc`, `q` | close |

Two deliberate divergences from the notification center: `Enter` edits rather
than jumps (todos are authored, notifications are not), and jumping is bound to
the link chip instead, so the mouse-first path matches what is visible.

### Edit view

A modal built from the existing dialog structure in `src/ui/dialogs.rs` and
`src/app/input/modal.rs` — text input prefilled, `Tab` cycles priority, link set
or cleared, Save/Cancel in the settings-panel button language. Not a bespoke
screen.

### Config and bindings

- `ui.show_pane_todo_indicator` (default `true`)
- `ui.pane_todo_color` (unset → theme accent)
- `keys.open_pane_todos` — default `prefix+ctrl+t`, mirroring `prefix+ctrl+n`
- `keys.add_pane_todo` — unbound by default; opens the edit modal on a new todo

Both actions get `help_entry` rows in `src/ui/keybind_help.rs`. This is required
even for the unbound one: the panel renders `unset` until the user binds it,
which is how the action is discovered.

## Alternatives considered

**Reuse the notification log for todos.** Rejected: notifications are transient,
server-raised, and ordered by recency with read state; todos are durable,
user-authored, priority-ordered, and outlive the agent. Sharing the store would
force both to the weaker contract.

**Extract a shared dropdown widget from `notification_center.rs` first.**
Deferred. It is the right end state, but it refactors a working feature to serve
an unwritten one, immediately after an upstream sync that rewrote pane mouse
internals. Two similar dropdowns do not yet justify the abstraction; extract
when a third appears or when the duplication actually hurts.

**Scope todos to the agent name instead of the pane.** Rejected: herdr clears
agent names when the occupant exits, which would orphan todos at exactly the
moment they matter most — an agent writing its next steps as it terminates.

**Keep todos in memory only, like the notification log.** Rejected: it would
lose the notes across `herdr update --handoff` and restarts, defeating the
come-back-tomorrow use case that motivates the feature.
