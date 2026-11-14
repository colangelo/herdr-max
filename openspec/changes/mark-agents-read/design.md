# Design: pane.mark_seen

## Server

`Method::PaneMarkSeen(PaneMarkSeenParams)` → `"pane.mark_seen"`, flagged as a client
shell method (so the TUI may call it) and as UI-changing.

```rust
pub struct PaneMarkSeenParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>, // absent: every workspace
}
// ResponseResult::PaneMarkSeen { changed: u32 }
```

Handler (in `src/app/api/panes.rs`): resolve `workspace_id` (unknown id →
`workspace_not_found`, nothing changed); for each pane in scope with `seen == false`,
set `seen = true`, count it. Reuse the state helper style of `mark_active_tab_seen`
(add `AppState::mark_panes_seen(scope) -> usize` in `src/app/actions.rs`, pure and
unit-testable with `AppState::test_new()`). When `changed > 0`, mark the session dirty
and go through the same revision/publish path a focus-driven seen sweep uses, so every
attached client and the JSON event stream see the new `agent_status` (`done` → `idle`).
Idempotent: a second call returns `changed: 0` and publishes nothing.

No new socket message: the TUI calls it through the existing endpoint method path
(`push_endpoint_method`), like `agent.pin`.

## Client

`ClientContextMenuAction::MarkSpaceRead` and `::MarkAllRead`.

- Space menu (`ClientContextMenuTarget::Workspace`): add fields `space_unread: bool`,
  `any_unread: bool` filled from the snapshot when the menu opens. Items appended at the
  end: "Mark space read" when `space_unread`, "Mark all read" when `any_unread`.
- Agent menu (`ClientContextMenuTarget::Agent`): add `any_unread: bool`; append
  "Mark all read" when set.
- Both items are omitted when `supports_method("pane.mark_seen")` is false for the
  menu's endpoint.
- Unread is read from the snapshot the client already holds (the same fact the sidebar
  uses to show `done`), never from a new server query.

## Tests

Server: state helper scope + idempotence; API handler result and error; publish
happens only when something changed. Client: rendered context-menu tests under
`src/client/shell/tests/` for presence/absence and the method sent. Schema/method-name
tests updated for the new method (new method only; no frozen generation-1 fixture is
edited).

## Docs

`docs/next/website/src/content/docs/` CLI and socket API pages gain `pane.mark_seen`.
No CHANGELOG edit.

## Verifier table (required tests)

| # | Requirement / scenario | Test (kind, location) |
|---|---|---|
| 1 | session-wide clear, `changed` count | unit, `AppState::mark_panes_seen` in `src/app/actions.rs` |
| 2 | one-space clear | unit, `src/app/actions.rs` |
| 3 | idempotent, `changed: 0` | unit, `src/app/actions.rs` |
| 4 | unknown workspace → `workspace_not_found`, no change | API handler test, `src/app/api/panes.rs` |
| 5 | `agent_status` `done` → `idle` after the call | API test via `agent.list`/`pane.list`, `src/app/api/panes.rs` |
| 6 | no publish when nothing changed | API/headless test |
| 7 | method in schema and client-shell method list | `src/api/schema` tests |
| 8 | CLI `pane mark-seen [--workspace]` builds the request | `src/cli/pane.rs` test |
| 9 | space menu shows both items when unread | rendered-layout test, `src/client/shell/tests/` |
| 10 | agent menu shows "Mark all read" only when unread | rendered-layout test |
| 11 | items hidden when server lacks the method | client test |
| 12 | choosing an item sends `pane.mark_seen` with the right scope | client test |
| 13 | live: right-click clear works in a running beta on m4m | `herdr-dogfood` check, recorded on issue 276 |
