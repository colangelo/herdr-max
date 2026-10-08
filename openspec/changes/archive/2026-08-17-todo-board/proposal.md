# A todo board: every pane's todos in one place

## Why

Pane todos are only ever viewed one pane at a time. The panel is anchored to a
pane and renders `app.pane_todos_in_display_order(panel.pane_id)`
(`src/ui/todo_panel.rs:225`); `open_pane_todos` and `add_pane_todo` are the only
todo bindings and both are pane-scoped. To answer "what is outstanding in this
session" you visit panes one by one and remember what you saw.

That is the wrong shape for what todos are used for. A todo is written *in* a
pane — "rerun the deploy", "check the 403" — because that is where the work is,
but it is read *across* panes, when deciding what to pick up next. The feature
currently supports the writing half and not the reading half.

**The data layer already aggregates.** `todo.list` with `pane_id` omitted returns
every pane's todos — `TodoListParams` says so in its doc comment
(`src/api/schema/todos.rs:31`) — and the CLI already exposes it as
`herdr todo list --all`. So the session-wide view exists everywhere except where
it is most useful. This change is a missing view, not new runtime work.

The board's advantage over `herdr todo list --all` is that it is *actionable*:
from a row you can jump to the pane that owns the todo, which is the step the CLI
output cannot take for you.

## What Changes

A session-wide todo board, opened by a bindable action, listing every pane's
todos grouped by where they live, in the existing overlay language.

The board **complements** the per-pane panel rather than replacing it. The panel
stays the fast path for the pane in front of you; the board is for triage across
the session. Both read the same store and the same presentation order, so a todo
looks and sorts the same in either.

Actions on the board are the panel's, where they still make sense at session
scope: move the selection, toggle done, open for editing, follow a link, remove,
clear done, close. Activating a row focuses the pane that owns the todo — the
board's reason to exist.

## Impact

- Affected capability: `pane-todos` (one requirement added, one modified for
  ordering across panes)
- Affected code: a new `src/ui/todo_board.rs`, its geometry and hit-testing
  alongside the existing panel's in `src/app/input/mouse.rs`, board state in
  `src/app/state.rs`, a `KeysConfig` action and its `help_entry`
- No server, API, protocol or config-schema surface: `todo.list` already returns
  the aggregate, and the todo events already exist. Client-side only under the
  runtime/client guardrail.
- Depends on the panel footer convention (`FOOTER_ROWS` / `footer_split`) landing
  first, since the board is a new panel with a footer button row.

## Non-goals

- Editing a todo's owning pane — moving a todo between panes is a store
  operation that does not exist and is not needed to read across panes.
- Filtering or search. Worth having if the board outgrows a screen; the row
  shape here does not preclude it, and the navigator is the precedent when it
  does.
- Replacing the per-pane panel, or changing any of its keys.
- Persisting board state (selection, scroll) across sessions.
