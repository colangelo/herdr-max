## Why

When an agent finishes a turn it often reports what is still open — "tests pass
but the handoff case is flaky", "the deploy still needs rerunning" — and that
information has nowhere to live. It scrolls away in the pane, or it survives
only in the operator's head while they are busy in another pane waiting on
another agent. Coming back to a workspace hours later, there is no per-pane
answer to "what was I supposed to do next here".

Herdr already has the two halves this needs and connects neither: an agent can
address its own pane from inside it (`HERDR_PANE_ID`, `--pane`, `--current`),
and the notification center proved the server-owned-log-plus-dropdown shape.
What is missing is durable, pane-scoped, operator-authored state that an agent
can also write — so a terminating agent can leave its own next steps where they
will be seen.

Notifications are the wrong home for this. They are a transient, server-raised,
newest-first feed with read state; todos are durable, user-authored, ordered by
priority, and outlive any single agent occupancy of the pane.

## What Changes

- **Pane-scoped todo list in server state**: each pane carries an ordered list
  of todos — monotonic per-pane `id`, text, `done` flag, priority
  (high/normal/low), an optional link to another pane, and created/updated
  timestamps. The list lives on `TerminalState` as pure data, testable through
  `AppState::test_new()` without PTYs. Bounded at 50 todos per pane and 500
  characters per todo, enforced server-side with explicit errors rather than
  silent truncation, because agents write these unattended.
- **Persistence with the pane**: todos serialize into `PaneSnapshot` beside
  `label` and `agent_name`, so they survive server restart and
  `herdr update --handoff`. The field is `#[serde(default)]` and skipped when
  empty, so existing session files load unchanged and todo-free panes serialize
  exactly as they do today. Closing a pane with outstanding todos asks for
  confirmation through the existing confirm modal.
- **Cross-pane links**: a todo may point at another pane. The link stores the
  internal `PaneId` plus a label captured at link time; restore remaps the
  `PaneId` through the same `id_map` that fixes layout references. A link whose
  target no longer exists keeps its label, renders dead, and is inert — it never
  silently retargets a different pane.
- **Socket API + events**: `todo.list`, `todo.add`, `todo.update`,
  `todo.remove`, `todo.clear`, and a `todo.changed` subscription event so
  external status bars can consume the same feed. No protocol bump: source is
  already at 19 while the latest released fork tag (`v0.7.4-ac`) shipped 18, so
  these methods ride the existing unreleased bump.
- **CLI**: `herdr todo add|list|done|undone|edit|rm|clear`, resolving the target
  pane exactly as the existing pane verbs do (`--pane <id>`, `--current`,
  `HERDR_PANE_ID` default). A terminating agent needs no arguments:
  `herdr todo add "next: fix the flaky handoff test"`.
- **Pane indicator + dropdown**: a bare `▾N` glyph at the far right of the
  pane's top border, hidden when the pane has no todos, colored by the highest
  outstanding priority. Clicking it — or `open_pane_todos`, default
  `prefix+ctrl+t` alongside `prefix+ctrl+n` — opens a dropdown anchored to the
  pane, in the notification center's existing overlay language.
- **Edit view**: `Enter` on a row opens a modal built from the existing dialog
  structure to edit text, priority, and link.

## Impact

- Affected specs: new `pane-todos` capability.
- Affected code: `src/app/state.rs` (todo store), `src/persist/snapshot.rs` and
  `src/persist/restore.rs` (persistence + link remap), `src/api/schema*`,
  `src/app/api.rs`, `src/api/subscriptions.rs` (API + event), `src/cli/todo.rs`
  and `src/cli/spec.rs` (CLI), `src/ui/panes.rs` (indicator),
  `src/ui/todo_panel.rs` (dropdown, new), `src/ui/dialogs.rs` (edit modal),
  `src/app/input/{mouse,modal,mod}.rs` (input), `src/config/{model,keybinds}.rs`
  (config + bindings), `src/ui/keybind_help.rs` (help entries).
- Release risk: touches persisted state (`PaneSnapshot`), so the store tests
  assert `Workspace::assert_invariants_for_test()` and the persistence tests
  cover the pre-field session file and the unresolvable-link path.
- Deliberately out of scope for v1: the mobile switcher and collapsed-sidebar
  surfaces, and any notification raised by todo changes (adding a todo stays
  quiet; the indicator is the only signal).
