# Pane Todos — Phase 1 (headless) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give every pane a durable, priority-ordered todo list that agents can write from inside the pane over the CLI, that persists across restart and live handoff, and that external tools can read and follow over the socket API.

**Architecture:** Todos live on `TerminalState` (`src/terminal/state.rs`) — *not* `PaneState`, which is viewport-only and is not what `PaneSnapshot` captures. A new `src/terminal/todo.rs` holds the types and an `impl TerminalState` block, matching how `src/terminal/metadata.rs` is organised. Persistence rides the existing `PaneSnapshot` path; cross-pane links are remapped in a post-pass once restore's `id_map` exists. The API and CLI mirror `notification.*` and `herdr notification`.

**Tech Stack:** Rust, serde, schemars (API JSON schema), `cargo nextest`, `just`.

## Scope

This plan covers **Phase 1 (headless)** only — OpenSpec task groups 1–4:

1. Todo store on `TerminalState`
2. Persistence + link remap
3. Socket API + `todo.changed` event
4. `herdr todo` CLI

At the end of this plan the feature is fully usable without any TUI work: an agent runs `herdr todo add "..."`, the todo survives `herdr update --handoff`, and `herdr todo list --json` / the socket API expose it.

**Not in this plan** (separate plan, written after this lands): the pane border indicator, the dropdown panel, the edit modal, and the keybindings — OpenSpec groups 5, 6, and the UI half of 7.

## Global Constraints

- Source of truth for requirements: `openspec/changes/pane-todos/` (`proposal.md`, `design.md`, `specs/pane-todos/spec.md`, `tasks.md`). Re-read the relevant requirement before each task.
- **No `unwrap()` in production code** (tests are fine). Use `tracing` for logging. `#[allow]` only with a comment explaining why.
- **No protocol bump.** `PROTOCOL_VERSION` stays at **19**. Source 19 already exceeds the 18 released in `v0.7.4-ac`, so per CLAUDE.md it must not be bumped. Do not touch protocol expectations in `tests/cli/sessions.rs`, `tests/api_ping.rs`, `tests/support/mod.rs`.
- Limits: **50 todos per pane**, **500 characters per todo**. Enforced server-side with explicit errors, never silent truncation.
- Error codes exactly: `pane_not_found`, `todo_not_found`, `todo_text_empty`, `todo_text_too_long`, `todo_limit_reached`, `todo_link_unresolved`.
- API vocabulary stays neutral (`todo.*`, `pane_id`, `link_pane_id`) — no `dropdown`, `row`, `panel`, `widget` on the wire.
- Commit style: lowercase conventional commits, no emojis, no AI co-author lines. Add `refs <gitea-issue-url>` only if an issue exists.
- Run `just check` before the final commit of the plan. Individual tasks may use `cargo nextest run --locked <filter>`.
- Known unrelated failure on macOS: `live_handoff_keeps_unmanaged_agent_name_bound_to_saved_session` fails on clean `upstream/master` too. Ignore it; do not "fix" it as part of this work.

---

### Task 1: Todo model and store on `TerminalState`

**Files:**
- Create: `src/terminal/todo.rs`
- Modify: `src/terminal/mod.rs` (add `mod todo;` + re-exports)
- Modify: `src/terminal/state.rs` (two new fields on `TerminalState` + their initialisers in `TerminalState::new`)
- Test: inline `#[cfg(test)] mod tests` at the bottom of `src/terminal/todo.rs`

**Interfaces:**
- Consumes: `crate::layout::PaneId`, `crate::terminal::state::TerminalState`.
- Produces, relied on by Tasks 2–4:
  - `pub struct PaneTodo { pub id: u64, pub text: String, pub done: bool, pub priority: TodoPriority, pub link: Option<TodoLink>, pub created_at_unix: u64, pub updated_at_unix: u64 }`
  - `pub struct TodoLink { pub pane: Option<PaneId>, pub label: String }`
  - `pub enum TodoPriority { Low, Normal, High }` (`Default` = `Normal`, `Ord` ascending so `High` is greatest)
  - `pub struct TodoUpdate { pub text: Option<String>, pub done: Option<bool>, pub priority: Option<TodoPriority>, pub link: Option<Option<TodoLink>> }` (`Default`)
  - `pub enum TodoError { TextEmpty, TextTooLong, LimitReached, NotFound }` with `pub fn code(&self) -> &'static str` and `pub fn message(&self) -> String`
  - `pub const MAX_TODOS_PER_PANE: usize = 50;` `pub const MAX_TODO_TEXT_LEN: usize = 500;`
  - On `TerminalState`: `todos()`, `add_todo()`, `update_todo()`, `remove_todo()`, `clear_todos()`, `todos_in_display_order()`, `outstanding_todo_count()`, `highest_outstanding_todo_priority()`, `restore_todos()` — signatures in Step 3.

- [ ] **Step 1: Write the failing tests**

Create `src/terminal/todo.rs` containing *only* the test module for now, so the file compiles as soon as the impl lands:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::{state::TerminalState, TerminalId};

    fn terminal() -> TerminalState {
        TerminalState::new(TerminalId::alloc(), std::path::PathBuf::from("/tmp"))
    }

    #[test]
    fn add_todo_defaults_to_normal_priority_and_not_done() {
        let mut t = terminal();
        let todo = t.add_todo("write the plan", TodoPriority::Normal, None, 100).unwrap();

        assert_eq!(todo.id, 1);
        assert_eq!(todo.text, "write the plan");
        assert!(!todo.done);
        assert_eq!(todo.priority, TodoPriority::Normal);
        assert_eq!(todo.created_at_unix, 100);
        assert_eq!(todo.updated_at_unix, 100);
        assert_eq!(t.todos().len(), 1);
    }

    #[test]
    fn add_todo_trims_and_rejects_empty_text() {
        let mut t = terminal();

        assert_eq!(t.add_todo("   ", TodoPriority::Normal, None, 100), Err(TodoError::TextEmpty));
        assert_eq!(t.add_todo("", TodoPriority::Normal, None, 100), Err(TodoError::TextEmpty));
        assert!(t.todos().is_empty());

        let todo = t.add_todo("  padded  ", TodoPriority::Normal, None, 100).unwrap();
        assert_eq!(todo.text, "padded");
    }

    #[test]
    fn add_todo_rejects_text_over_the_limit() {
        let mut t = terminal();
        let long = "x".repeat(MAX_TODO_TEXT_LEN + 1);

        assert_eq!(t.add_todo(&long, TodoPriority::Normal, None, 100), Err(TodoError::TextTooLong));
        assert!(t.todos().is_empty());

        let exact = "y".repeat(MAX_TODO_TEXT_LEN);
        assert!(t.add_todo(&exact, TodoPriority::Normal, None, 100).is_ok());
    }

    #[test]
    fn add_todo_enforces_the_per_pane_cap() {
        let mut t = terminal();
        for i in 0..MAX_TODOS_PER_PANE {
            t.add_todo(&format!("todo {i}"), TodoPriority::Normal, None, 100).unwrap();
        }

        assert_eq!(
            t.add_todo("one too many", TodoPriority::Normal, None, 100),
            Err(TodoError::LimitReached)
        );
        assert_eq!(t.todos().len(), MAX_TODOS_PER_PANE);
    }

    #[test]
    fn todo_ids_are_never_reused() {
        let mut t = terminal();
        let first = t.add_todo("first", TodoPriority::Normal, None, 100).unwrap().id;
        t.remove_todo(first).unwrap();
        let second = t.add_todo("second", TodoPriority::Normal, None, 100).unwrap().id;

        assert!(second > first, "expected {second} > {first}");
    }

    #[test]
    fn update_todo_changes_fields_and_bumps_updated_at() {
        let mut t = terminal();
        let id = t.add_todo("draft", TodoPriority::Normal, None, 100).unwrap().id;

        let updated = t
            .update_todo(
                id,
                TodoUpdate {
                    text: Some("final".into()),
                    priority: Some(TodoPriority::High),
                    ..TodoUpdate::default()
                },
                250,
            )
            .unwrap();

        assert_eq!(updated.text, "final");
        assert_eq!(updated.priority, TodoPriority::High);
        assert_eq!(updated.id, id, "id must be preserved");
        assert_eq!(updated.created_at_unix, 100, "created_at must be preserved");
        assert_eq!(updated.updated_at_unix, 250);
        assert!(!updated.done, "done must be untouched when not in the update");
    }

    #[test]
    fn update_todo_can_set_and_clear_the_link() {
        let mut t = terminal();
        let id = t.add_todo("rerun deploy", TodoPriority::Normal, None, 100).unwrap().id;
        let link = TodoLink { pane: Some(crate::layout::PaneId::from_raw(7)), label: "infra".into() };

        let set = t.update_todo(id, TodoUpdate { link: Some(Some(link.clone())), ..Default::default() }, 200).unwrap();
        assert_eq!(set.link, Some(link));

        let cleared = t.update_todo(id, TodoUpdate { link: Some(None), ..Default::default() }, 300).unwrap();
        assert_eq!(cleared.link, None);

        let untouched = t.update_todo(id, TodoUpdate { done: Some(true), ..Default::default() }, 400).unwrap();
        assert_eq!(untouched.link, None, "link: None must leave the link alone");
    }

    #[test]
    fn update_and_remove_report_missing_todos() {
        let mut t = terminal();

        assert_eq!(t.update_todo(42, TodoUpdate::default(), 100), Err(TodoError::NotFound));
        assert_eq!(t.remove_todo(42), Err(TodoError::NotFound));
    }

    #[test]
    fn update_todo_validates_replacement_text() {
        let mut t = terminal();
        let id = t.add_todo("keep", TodoPriority::Normal, None, 100).unwrap().id;

        assert_eq!(
            t.update_todo(id, TodoUpdate { text: Some("  ".into()), ..Default::default() }, 200),
            Err(TodoError::TextEmpty)
        );
        assert_eq!(
            t.update_todo(id, TodoUpdate { text: Some("z".repeat(MAX_TODO_TEXT_LEN + 1)), ..Default::default() }, 200),
            Err(TodoError::TextTooLong)
        );
        assert_eq!(t.todos()[0].text, "keep", "a rejected update must not mutate the todo");
    }

    #[test]
    fn clear_todos_removes_all_or_only_done() {
        let mut t = terminal();
        let a = t.add_todo("a", TodoPriority::Normal, None, 100).unwrap().id;
        t.add_todo("b", TodoPriority::Normal, None, 100).unwrap();
        t.update_todo(a, TodoUpdate { done: Some(true), ..Default::default() }, 200).unwrap();

        assert_eq!(t.clear_todos(true), 1);
        assert_eq!(t.todos().len(), 1);
        assert_eq!(t.todos()[0].text, "b");

        assert_eq!(t.clear_todos(false), 1);
        assert!(t.todos().is_empty());
    }

    #[test]
    fn display_order_is_priority_then_not_done_then_creation() {
        let mut t = terminal();
        let normal = t.add_todo("normal first", TodoPriority::Normal, None, 100).unwrap().id;
        let high = t.add_todo("high second", TodoPriority::High, None, 101).unwrap().id;
        let low = t.add_todo("low third", TodoPriority::Low, None, 102).unwrap().id;

        let order: Vec<u64> = t.todos_in_display_order().iter().map(|todo| todo.id).collect();
        assert_eq!(order, vec![high, normal, low]);

        // a done high-priority todo sinks below a not-done normal one
        t.update_todo(high, TodoUpdate { done: Some(true), ..Default::default() }, 200).unwrap();
        let order: Vec<u64> = t.todos_in_display_order().iter().map(|todo| todo.id).collect();
        assert_eq!(order, vec![normal, low, high]);
    }

    #[test]
    fn display_order_does_not_change_stored_order() {
        let mut t = terminal();
        t.add_todo("first", TodoPriority::Normal, None, 100).unwrap();
        let high = t.add_todo("second", TodoPriority::High, None, 101).unwrap().id;

        let _ = t.todos_in_display_order();

        assert_eq!(t.todos()[0].text, "first", "stored order stays insertion order");
        assert_eq!(t.todos()[1].id, high);
    }

    #[test]
    fn outstanding_count_and_highest_priority_ignore_done_todos() {
        let mut t = terminal();
        let high = t.add_todo("high", TodoPriority::High, None, 100).unwrap().id;
        t.add_todo("normal", TodoPriority::Normal, None, 100).unwrap();

        assert_eq!(t.outstanding_todo_count(), 2);
        assert_eq!(t.highest_outstanding_todo_priority(), Some(TodoPriority::High));

        t.update_todo(high, TodoUpdate { done: Some(true), ..Default::default() }, 200).unwrap();
        assert_eq!(t.outstanding_todo_count(), 1);
        assert_eq!(
            t.highest_outstanding_todo_priority(),
            Some(TodoPriority::Normal),
            "a done high-priority todo must not drive the indicator colour"
        );

        t.clear_todos(false);
        assert_eq!(t.outstanding_todo_count(), 0);
        assert_eq!(t.highest_outstanding_todo_priority(), None);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run --locked terminal::todo`
Expected: compile failure — `PaneTodo`, `TodoPriority`, `add_todo` etc. do not exist yet.

- [ ] **Step 3: Write the implementation**

Prepend to `src/terminal/todo.rs` (above the test module):

```rust
//! Pane-scoped todo lists.
//!
//! Todos live on `TerminalState` rather than `PaneState`: `PaneState` is
//! viewport-only, and `PaneSnapshot` — the thing that persists across restart
//! and live handoff — is built entirely from the terminal. Keeping todos here
//! also means they follow the work through `break_pane` / `move_pane_to_tab`,
//! which preserve the running terminal.

use crate::layout::PaneId;
use crate::terminal::state::TerminalState;

/// Maximum todos retained per pane. Agents write these unattended, so the list
/// is bounded to keep the session snapshot from growing without limit.
pub const MAX_TODOS_PER_PANE: usize = 50;
/// Maximum characters in a single todo, for the same reason.
pub const MAX_TODO_TEXT_LEN: usize = 500;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoPriority {
    Low,
    #[default]
    Normal,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoLink {
    /// `None` once the target pane is gone: a dead link that keeps its label
    /// and never resolves to some other pane.
    pub pane: Option<PaneId>,
    /// Captured when the link was made, so a dead link can still say what it meant.
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneTodo {
    pub id: u64,
    pub text: String,
    pub done: bool,
    pub priority: TodoPriority,
    pub link: Option<TodoLink>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

/// Partial update. `None` leaves a field alone; for `link`, `Some(None)` clears
/// it and `Some(Some(_))` sets it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TodoUpdate {
    pub text: Option<String>,
    pub done: Option<bool>,
    pub priority: Option<TodoPriority>,
    pub link: Option<Option<TodoLink>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoError {
    TextEmpty,
    TextTooLong,
    LimitReached,
    NotFound,
}

impl TodoError {
    pub fn code(self) -> &'static str {
        match self {
            Self::TextEmpty => "todo_text_empty",
            Self::TextTooLong => "todo_text_too_long",
            Self::LimitReached => "todo_limit_reached",
            Self::NotFound => "todo_not_found",
        }
    }

    pub fn message(self) -> String {
        match self {
            Self::TextEmpty => "todo text cannot be empty".to_string(),
            Self::TextTooLong => {
                format!("todo text cannot exceed {MAX_TODO_TEXT_LEN} characters")
            }
            Self::LimitReached => {
                format!("pane already has the maximum of {MAX_TODOS_PER_PANE} todos")
            }
            Self::NotFound => "no todo with that id on this pane".to_string(),
        }
    }
}

fn validate_text(text: &str) -> Result<String, TodoError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(TodoError::TextEmpty);
    }
    if trimmed.chars().count() > MAX_TODO_TEXT_LEN {
        return Err(TodoError::TextTooLong);
    }
    Ok(trimmed.to_string())
}

impl TerminalState {
    pub fn todos(&self) -> &[PaneTodo] {
        &self.todos
    }

    pub fn add_todo(
        &mut self,
        text: &str,
        priority: TodoPriority,
        link: Option<TodoLink>,
        now_unix: u64,
    ) -> Result<PaneTodo, TodoError> {
        let text = validate_text(text)?;
        if self.todos.len() >= MAX_TODOS_PER_PANE {
            return Err(TodoError::LimitReached);
        }
        let todo = PaneTodo {
            id: self.next_todo_id,
            text,
            done: false,
            priority,
            link,
            created_at_unix: now_unix,
            updated_at_unix: now_unix,
        };
        self.next_todo_id += 1;
        self.todos.push(todo.clone());
        Ok(todo)
    }

    pub fn update_todo(
        &mut self,
        id: u64,
        update: TodoUpdate,
        now_unix: u64,
    ) -> Result<PaneTodo, TodoError> {
        // Validate before mutating so a rejected update leaves the todo intact.
        let text = update.text.as_deref().map(validate_text).transpose()?;
        let Some(todo) = self.todos.iter_mut().find(|todo| todo.id == id) else {
            return Err(TodoError::NotFound);
        };
        if let Some(text) = text {
            todo.text = text;
        }
        if let Some(done) = update.done {
            todo.done = done;
        }
        if let Some(priority) = update.priority {
            todo.priority = priority;
        }
        if let Some(link) = update.link {
            todo.link = link;
        }
        todo.updated_at_unix = now_unix;
        Ok(todo.clone())
    }

    pub fn remove_todo(&mut self, id: u64) -> Result<(), TodoError> {
        let before = self.todos.len();
        self.todos.retain(|todo| todo.id != id);
        if self.todos.len() == before {
            return Err(TodoError::NotFound);
        }
        Ok(())
    }

    /// Removes todos and returns how many went. `done_only` keeps outstanding ones.
    pub fn clear_todos(&mut self, done_only: bool) -> usize {
        let before = self.todos.len();
        if done_only {
            self.todos.retain(|todo| !todo.done);
        } else {
            self.todos.clear();
        }
        before - self.todos.len()
    }

    /// Presentation order: not-done before done, then priority descending, then
    /// creation order. Stored order stays insertion order.
    pub fn todos_in_display_order(&self) -> Vec<&PaneTodo> {
        let mut ordered: Vec<&PaneTodo> = self.todos.iter().collect();
        ordered.sort_by(|a, b| {
            a.done
                .cmp(&b.done)
                .then_with(|| b.priority.cmp(&a.priority))
                .then_with(|| a.id.cmp(&b.id))
        });
        ordered
    }

    pub fn outstanding_todo_count(&self) -> usize {
        self.todos.iter().filter(|todo| !todo.done).count()
    }

    pub fn highest_outstanding_todo_priority(&self) -> Option<TodoPriority> {
        self.todos
            .iter()
            .filter(|todo| !todo.done)
            .map(|todo| todo.priority)
            .max()
    }

    /// Restore path only: install saved todos and the saved id counter.
    pub fn restore_todos(&mut self, todos: Vec<PaneTodo>, next_todo_id: u64) {
        let highest = todos.iter().map(|todo| todo.id).max().unwrap_or(0);
        self.next_todo_id = next_todo_id.max(highest + 1);
        self.todos = todos;
    }
}
```

Note the display-order comparator sorts `done` **first** (false < true, so not-done leads), then priority descending, then id. That is what makes a done high-priority todo sink below an outstanding normal one, as the spec requires.

Add the two fields to `TerminalState` in `src/terminal/state.rs` — after `pub launch_argv: Option<Vec<String>>,`:

```rust
    pub(crate) todos: Vec<crate::terminal::todo::PaneTodo>,
    pub(crate) next_todo_id: u64,
```

and in `TerminalState::new`, after `launch_argv: None,`:

```rust
            todos: Vec::new(),
            next_todo_id: 1,
```

Register the module in `src/terminal/mod.rs` alongside the existing `mod metadata;` style declarations:

```rust
pub mod todo;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run --locked terminal::todo`
Expected: PASS, 12 tests.

Then confirm nothing else broke: `cargo nextest run --locked --no-fail-fast`
Expected: only the known macOS `live_handoff_keeps_unmanaged_agent_name_bound_to_saved_session` failure.

- [ ] **Step 5: Commit**

```bash
git add src/terminal/todo.rs src/terminal/mod.rs src/terminal/state.rs
git commit -m "feat: add pane-scoped todo store to terminal state"
```

---

### Task 2: Persist todos in the session snapshot, with link remapping

**Files:**
- Modify: `src/persist/snapshot.rs` (add `PaneTodoSnapshot`, extend `PaneSnapshot`, populate at the capture site around line 340–372)
- Modify: `src/persist/restore.rs` (apply saved todos at both terminal-construction sites near lines 547 and 641; add the link-resolution post-pass)
- Test: inline `#[cfg(test)] mod tests` additions in `src/persist/snapshot.rs` and `src/persist/restore.rs`

**Interfaces:**
- Consumes from Task 1: `PaneTodo`, `TodoLink`, `TodoPriority`, `TerminalState::todos()`, `TerminalState::restore_todos()`, and the `next_todo_id` field.
- Produces, relied on by nothing later in this plan (persistence is terminal):
  - `pub struct PaneTodoSnapshot { pub id: u64, pub text: String, pub done: bool, pub priority: TodoPriority, pub link_pane: Option<u32>, pub link_label: Option<String>, pub created_at_unix: u64, pub updated_at_unix: u64 }`
  - `PaneSnapshot.todos: Vec<PaneTodoSnapshot>` and `PaneSnapshot.next_todo_id: u64`

**Why a post-pass for links:** restore builds its old-raw → new `PaneId` map (`src/persist/restore.rs`, the function returning `HashMap<u32, PaneId>` around line 124) per tab, and terminals are constructed before that map is complete for every tab. So restore records `(TerminalId, todo_id, old_raw_pane)` in a side table while building terminals, then resolves every link in one pass once all tabs are mapped. Links whose old raw id is absent from the map become `TodoLink { pane: None, label }` — a dead link that keeps its label. The todo is never dropped.

- [ ] **Step 1: Write the failing tests**

Add to the test module in `src/persist/snapshot.rs`:

```rust
    #[test]
    fn pane_snapshot_round_trips_todos() {
        let snapshot = PaneSnapshot {
            cwd: std::path::PathBuf::from("/tmp"),
            label: None,
            agent_name: None,
            managed_agent_kind: None,
            agent_session: None,
            launch_argv: None,
            todos: vec![PaneTodoSnapshot {
                id: 3,
                text: "rerun the deploy".into(),
                done: false,
                priority: crate::terminal::todo::TodoPriority::High,
                link_pane: Some(11),
                link_label: Some("infra".into()),
                created_at_unix: 100,
                updated_at_unix: 200,
            }],
            next_todo_id: 4,
        };

        let json = serde_json::to_string(&snapshot).unwrap();
        let back: PaneSnapshot = serde_json::from_str(&json).unwrap();

        assert_eq!(back.todos.len(), 1);
        assert_eq!(back.todos[0].id, 3);
        assert_eq!(back.todos[0].text, "rerun the deploy");
        assert_eq!(back.todos[0].priority, crate::terminal::todo::TodoPriority::High);
        assert_eq!(back.todos[0].link_pane, Some(11));
        assert_eq!(back.todos[0].link_label.as_deref(), Some("infra"));
        assert_eq!(back.next_todo_id, 4);
    }

    #[test]
    fn pane_snapshot_without_todos_omits_the_field() {
        let snapshot = PaneSnapshot {
            cwd: std::path::PathBuf::from("/tmp"),
            label: None,
            agent_name: None,
            managed_agent_kind: None,
            agent_session: None,
            launch_argv: None,
            todos: Vec::new(),
            next_todo_id: 1,
        };

        let json = serde_json::to_string(&snapshot).unwrap();

        assert!(!json.contains("todos"), "todo-free panes must serialize as before: {json}");
    }

    #[test]
    fn pane_snapshot_loads_session_files_written_before_todos_existed() {
        let json = r#"{"cwd":"/tmp"}"#;

        let snapshot: PaneSnapshot = serde_json::from_str(json).unwrap();

        assert!(snapshot.todos.is_empty());
        assert_eq!(snapshot.next_todo_id, 1, "counter must default to 1, not 0");
    }
```

Add to the test module in `src/persist/restore.rs`:

```rust
    #[test]
    fn restore_remaps_a_todo_link_to_the_new_pane_id() {
        // old raw 10 and 11 are two panes in the saved session; 11 is the link target
        let id_map: std::collections::HashMap<u32, PaneId> = std::collections::HashMap::from([
            (10, PaneId::from_raw(101)),
            (11, PaneId::from_raw(102)),
        ]);

        let resolved = resolve_todo_link(&id_map, Some(11), Some("infra".to_string()));

        assert_eq!(
            resolved,
            Some(crate::terminal::todo::TodoLink {
                pane: Some(PaneId::from_raw(102)),
                label: "infra".into(),
            })
        );
    }

    #[test]
    fn restore_turns_an_unmapped_todo_link_into_a_dead_link() {
        let id_map: std::collections::HashMap<u32, PaneId> =
            std::collections::HashMap::from([(10, PaneId::from_raw(101))]);

        let resolved = resolve_todo_link(&id_map, Some(999), Some("gone".to_string()));

        assert_eq!(
            resolved,
            Some(crate::terminal::todo::TodoLink { pane: None, label: "gone".into() }),
            "an unresolvable target must keep its label, not vanish or retarget"
        );
    }

    #[test]
    fn restore_leaves_unlinked_todos_unlinked() {
        let id_map: std::collections::HashMap<u32, PaneId> = std::collections::HashMap::new();

        assert_eq!(resolve_todo_link(&id_map, None, None), None);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run --locked persist::`
Expected: compile failure — `PaneTodoSnapshot`, the `todos`/`next_todo_id` fields, and `resolve_todo_link` do not exist.

- [ ] **Step 3: Write the implementation**

In `src/persist/snapshot.rs`, add the snapshot type next to `PaneAgentSessionSnapshot`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneTodoSnapshot {
    pub id: u64,
    pub text: String,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub priority: crate::terminal::todo::TodoPriority,
    /// Old raw pane id of the link target; remapped on restore.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_pane: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_label: Option<String>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

fn default_next_todo_id() -> u64 {
    1
}
```

Extend `PaneSnapshot` with two fields:

```rust
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub todos: Vec<PaneTodoSnapshot>,
    #[serde(default = "default_next_todo_id", skip_serializing_if = "is_initial_todo_id")]
    pub next_todo_id: u64,
```

and the helper beside it:

```rust
fn is_initial_todo_id(id: &u64) -> bool {
    *id == 1
}
```

At the capture site (the `panes.insert(...)` around line 362), derive the todo fields from the terminal and add them to the struct literal:

```rust
        let todos = terminal
            .map(|terminal| {
                terminal
                    .todos()
                    .iter()
                    .map(|todo| PaneTodoSnapshot {
                        id: todo.id,
                        text: todo.text.clone(),
                        done: todo.done,
                        priority: todo.priority,
                        link_pane: todo.link.as_ref().and_then(|link| link.pane).map(|pane| pane.raw()),
                        link_label: todo.link.as_ref().map(|link| link.label.clone()),
                        created_at_unix: todo.created_at_unix,
                        updated_at_unix: todo.updated_at_unix,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let next_todo_id = terminal.map(|terminal| terminal.next_todo_id).unwrap_or(1);
```

then add `todos,` and `next_todo_id,` to the `PaneSnapshot { ... }` literal.

Every other `PaneSnapshot { .. }` literal in the codebase (the test constructors in `snapshot.rs` and `restore.rs`) needs the two new fields; add `todos: Vec::new(), next_todo_id: 1,` to each. `cargo check` will list them all.

In `src/persist/restore.rs`, add the resolver near the other free functions:

```rust
/// Remap a saved todo link onto the restored pane ids. A target missing from
/// the map becomes a dead link that keeps its label — never a different pane.
fn resolve_todo_link(
    id_map: &std::collections::HashMap<u32, PaneId>,
    link_pane: Option<u32>,
    link_label: Option<String>,
) -> Option<crate::terminal::todo::TodoLink> {
    let label = link_label?;
    Some(crate::terminal::todo::TodoLink {
        pane: link_pane.and_then(|raw| id_map.get(&raw).copied()),
        label,
    })
}
```

At both terminal-construction sites (near lines 547 and 641, where `terminal.set_manual_label(label)` is called), install the saved todos with links left unresolved for now:

```rust
            if !saved_todos.is_empty() || saved_next_todo_id > 1 {
                let todos = saved_todos
                    .iter()
                    .map(|snap| crate::terminal::todo::PaneTodo {
                        id: snap.id,
                        text: snap.text.clone(),
                        done: snap.done,
                        priority: snap.priority,
                        // Links are resolved in the post-pass below, once every
                        // tab's id map exists.
                        link: snap.link_label.clone().map(|label| {
                            crate::terminal::todo::TodoLink { pane: None, label }
                        }),
                        created_at_unix: snap.created_at_unix,
                        updated_at_unix: snap.updated_at_unix,
                    })
                    .collect();
                terminal.restore_todos(todos, saved_next_todo_id);
                for snap in &saved_todos {
                    if let Some(raw) = snap.link_pane {
                        pending_todo_links.push((terminal_id.clone(), snap.id, raw));
                    }
                }
            }
```

where `saved_todos` / `saved_next_todo_id` are read from the `PaneSnapshot` alongside `saved_label`, and `pending_todo_links: Vec<(TerminalId, u64, u32)>` is declared before the pane loop.

After all tabs are restored and the full `id_map` is available, run the post-pass:

```rust
    for (terminal_id, todo_id, old_raw) in pending_todo_links {
        let Some(terminal) = terminals.get_mut(&terminal_id) else {
            continue;
        };
        let Some(todo) = terminal.todos_mut().iter_mut().find(|todo| todo.id == todo_id) else {
            continue;
        };
        if let Some(link) = todo.link.as_mut() {
            link.pane = id_map.get(&old_raw).copied();
        }
    }
```

That needs a mutable accessor on `TerminalState`; add it to `src/terminal/todo.rs`:

```rust
    pub(crate) fn todos_mut(&mut self) -> &mut Vec<PaneTodo> {
        &mut self.todos
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run --locked persist::`
Expected: PASS including the six new tests.

Run: `cargo nextest run --locked --no-fail-fast`
Expected: only the known macOS live-handoff failure.

- [ ] **Step 5: Commit**

```bash
git add src/persist/snapshot.rs src/persist/restore.rs src/terminal/todo.rs
git commit -m "feat: persist pane todos in the session snapshot"
```

---

### Task 3: Socket API and `todo.changed` event

**Files:**
- Create: `src/api/schema/todos.rs`
- Modify: `src/api/schema.rs` (module decl, re-exports, `Method` variants)
- Modify: `src/api/schema/response.rs` (`ResponseResult` variants)
- Modify: `src/api/schema/events.rs` (`Subscription` + `EventKind` + wire name + the all-kinds array)
- Modify: `src/api/subscriptions.rs` (subscription → event kind)
- Modify: `src/api/server.rs` (method → wire-name mapping)
- Create: `src/app/api/todos.rs` (handlers)
- Modify: `src/app/api.rs` (dispatch arms), `src/app/api/mod.rs` or the `mod` list wherever `panes` is declared
- Test: inline tests in `src/app/api/todos.rs`; extend `src/api/schema/tests.rs`

**Interfaces:**
- Consumes from Tasks 1–2: the whole todo store API, plus `TerminalState::todos_in_display_order()`.
- Produces, relied on by Task 4:
  - `Method::TodoList(TodoListParams)`, `Method::TodoAdd(TodoAddParams)`, `Method::TodoUpdate(TodoUpdateParams)`, `Method::TodoRemove(TodoRemoveParams)`, `Method::TodoClear(TodoClearParams)`
  - `pub struct TodoInfo { pub pane_id: String, pub id: u64, pub text: String, pub done: bool, pub priority: TodoPriority, pub link_pane_id: Option<String>, pub link_label: Option<String>, pub link_alive: bool, pub created_at_unix: u64, pub updated_at_unix: u64 }`
  - `pub struct TodoListParams { pub pane_id: Option<String> }`
  - `pub struct TodoAddParams { pub pane_id: String, pub text: String, pub priority: Option<TodoPriority>, pub link_pane_id: Option<String> }`
  - `pub struct TodoUpdateParams { pub pane_id: String, pub id: u64, pub text: Option<String>, pub done: Option<bool>, pub priority: Option<TodoPriority>, pub link_pane_id: Option<String>, pub clear_link: bool }`
  - `pub struct TodoRemoveParams { pub pane_id: String, pub id: u64 }`
  - `pub struct TodoClearParams { pub pane_id: String, pub done_only: bool }`
  - `ResponseResult::TodoList { todos: Vec<TodoInfo> }`, `ResponseResult::Todo { todo: TodoInfo }`, `ResponseResult::TodoCleared { removed: u32 }`
  - Wire names: `todo.list`, `todo.add`, `todo.update`, `todo.remove`, `todo.clear`, event `todo.changed`

**Handler shape** — every pane-targeted handler resolves like the existing ones (copied from `handle_pane_set_label`, `src/app/api/panes.rs:1138`):

```rust
let Some((ws_idx, pane_id)) = self.parse_pane_id(&params.pane_id) else {
    return pane_not_found(id, &params.pane_id);
};
let Some(terminal_id) = self
    .state
    .workspaces
    .get(ws_idx)
    .and_then(|ws| ws.terminal_id(pane_id))
    .cloned()
else {
    return pane_not_found(id, &params.pane_id);
};
let Some(terminal) = self.state.terminals.get_mut(&terminal_id) else {
    return pane_not_found(id, &params.pane_id);
};
```

After any mutation: call `self.state.mark_session_dirty();` (this is what schedules the session save — without it todos will not persist) and emit the event.

- [ ] **Step 1: Write the failing tests**

Add to `src/app/api/todos.rs` (create the file with just this test module first):

```rust
#[cfg(test)]
mod tests {
    use crate::api::schema::{Method, Request, TodoAddParams, TodoClearParams, TodoListParams, TodoRemoveParams, TodoUpdateParams};
    use crate::app::App;
    use crate::config::Config;
    use crate::terminal::todo::TodoPriority;
    use crate::workspace::Workspace;

    /// Same shape as `app_with_test_workspace()` in `src/app/api/panes.rs`
    /// (that one is private to its own test module).
    fn test_app_with_pane() -> (App, String) {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &Config::default(),
            true,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.workspaces = vec![Workspace::test_new("todos")];
        app.state.ensure_test_terminals();
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let public_pane_id = app.public_pane_id(0, pane_id).unwrap();
        (app, public_pane_id)
    }

    /// `handle_api_request` returns the encoded JSON response. Parsing to
    /// `Value` keeps success and error assertions in one shape.
    fn request_json(app: &mut App, method: Method) -> serde_json::Value {
        let raw = app.handle_api_request(Request { id: "test".into(), method });
        serde_json::from_str(&raw).unwrap()
    }

    #[test]
    fn todo_add_creates_a_todo_on_the_named_pane() {
        let (mut app, pane_id) = test_app_with_pane();

        let response = request_json(&mut app, Method::TodoAdd(TodoAddParams {
            pane_id: pane_id.clone(),
            text: "fix the flaky test".into(),
            priority: Some(TodoPriority::High),
            link_pane_id: None,
        }));

        let todo = &response["result"]["todo"];
        assert_eq!(todo["text"], "fix the flaky test");
        assert_eq!(todo["priority"], "high");
        assert_eq!(todo["done"], false);
        assert_eq!(todo["pane_id"], pane_id);
    }

    #[test]
    fn todo_add_rejects_an_unknown_pane() {
        let (mut app, _pane_id) = test_app_with_pane();

        let response = request_json(&mut app, Method::TodoAdd(TodoAddParams {
            pane_id: "w9:p9".into(),
            text: "nope".into(),
            priority: None,
            link_pane_id: None,
        }));

        assert_eq!(response["error"]["code"], "pane_not_found");
    }

    #[test]
    fn todo_add_surfaces_store_errors_as_codes() {
        let (mut app, pane_id) = test_app_with_pane();

        let response = request_json(&mut app, Method::TodoAdd(TodoAddParams {
            pane_id: pane_id.clone(),
            text: "   ".into(),
            priority: None,
            link_pane_id: None,
        }));

        assert_eq!(response["error"]["code"], "todo_text_empty");
    }

    #[test]
    fn todo_add_rejects_an_unresolvable_link() {
        let (mut app, pane_id) = test_app_with_pane();

        let response = request_json(&mut app, Method::TodoAdd(TodoAddParams {
            pane_id: pane_id.clone(),
            text: "rerun deploy".into(),
            priority: None,
            link_pane_id: Some("w9:p9".into()),
        }));

        assert_eq!(response["error"]["code"], "todo_link_unresolved");
    }

    #[test]
    fn todo_list_returns_display_order_for_one_pane() {
        let (mut app, pane_id) = test_app_with_pane();
        for (text, priority) in [("normal", TodoPriority::Normal), ("high", TodoPriority::High)] {
            request_json(&mut app, Method::TodoAdd(TodoAddParams {
                pane_id: pane_id.clone(),
                text: text.into(),
                priority: Some(priority),
                link_pane_id: None,
            }));
        }

        let response = request_json(&mut app, Method::TodoList(TodoListParams {
            pane_id: Some(pane_id.clone()),
        }));

        let todos = response["result"]["todos"].as_array().unwrap();
        assert_eq!(todos.len(), 2);
        assert_eq!(todos[0]["text"], "high", "priority orders the list");
    }

    #[test]
    fn todo_list_without_a_pane_returns_every_pane() {
        let (mut app, pane_id) = test_app_with_pane();
        request_json(&mut app, Method::TodoAdd(TodoAddParams {
            pane_id: pane_id.clone(),
            text: "only one".into(),
            priority: None,
            link_pane_id: None,
        }));

        let response = request_json(&mut app, Method::TodoList(TodoListParams { pane_id: None }));

        let todos = response["result"]["todos"].as_array().unwrap();
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0]["pane_id"], pane_id, "each entry identifies its pane");
    }

    #[test]
    fn todo_update_changes_done_state() {
        let (mut app, pane_id) = test_app_with_pane();
        let created = request_json(&mut app, Method::TodoAdd(TodoAddParams {
            pane_id: pane_id.clone(),
            text: "check me off".into(),
            priority: None,
            link_pane_id: None,
        }));
        let todo_id = created["result"]["todo"]["id"].as_u64().unwrap();

        let response = request_json(&mut app, Method::TodoUpdate(TodoUpdateParams {
            pane_id: pane_id.clone(),
            id: todo_id,
            text: None,
            done: Some(true),
            priority: None,
            link_pane_id: None,
            clear_link: false,
        }));

        assert_eq!(response["result"]["todo"]["done"], true);
    }

    #[test]
    fn todo_update_reports_a_missing_todo() {
        let (mut app, pane_id) = test_app_with_pane();

        let response = request_json(&mut app, Method::TodoUpdate(TodoUpdateParams {
            pane_id: pane_id.clone(),
            id: 999,
            text: None,
            done: Some(true),
            priority: None,
            link_pane_id: None,
            clear_link: false,
        }));

        assert_eq!(response["error"]["code"], "todo_not_found");
    }

    #[test]
    fn todo_remove_and_clear_report_counts() {
        let (mut app, pane_id) = test_app_with_pane();
        let created = request_json(&mut app, Method::TodoAdd(TodoAddParams {
            pane_id: pane_id.clone(),
            text: "remove me".into(),
            priority: None,
            link_pane_id: None,
        }));
        let todo_id = created["result"]["todo"]["id"].as_u64().unwrap();

        let removed = request_json(&mut app, Method::TodoRemove(TodoRemoveParams {
            pane_id: pane_id.clone(),
            id: todo_id,
        }));
        assert!(removed["result"].is_object());

        request_json(&mut app, Method::TodoAdd(TodoAddParams {
            pane_id: pane_id.clone(),
            text: "clear me".into(),
            priority: None,
            link_pane_id: None,
        }));
        let cleared = request_json(&mut app, Method::TodoClear(TodoClearParams {
            pane_id: pane_id.clone(),
            done_only: false,
        }));

        assert_eq!(cleared["result"]["removed"], 1);
    }
}
```

The two helpers are local to this test module because `app_with_test_workspace()` lives inside the private `tests` module of `src/app/api/panes.rs` and is not importable. If you would rather share them, promote that one to `pub(crate)` and import it instead — but do not change its behaviour, since the pane tests depend on it.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run --locked app::api::todos`
Expected: compile failure — the `Todo*` methods and params do not exist.

- [ ] **Step 3: Write the implementation**

Create `src/api/schema/todos.rs` with `TodoInfo` and the five params structs from the Interfaces block, each deriving `Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema`, with `#[serde(default)]` on every optional field and on `clear_link` / `done_only`.

Register in `src/api/schema.rs`: `pub mod todos;`, re-export the types beside the notification ones, and add the five `Method` variants following the existing naming (`TodoList(TodoListParams)` …).

Add the wire names in `src/api/server.rs` next to the notification arms:

```rust
        Method::TodoList(_) => "todo.list",
        Method::TodoAdd(_) => "todo.add",
        Method::TodoUpdate(_) => "todo.update",
        Method::TodoRemove(_) => "todo.remove",
        Method::TodoClear(_) => "todo.clear",
```

Add the response variants in `src/api/schema/response.rs`:

```rust
    TodoList {
        todos: Vec<super::todos::TodoInfo>,
    },
    Todo {
        todo: super::todos::TodoInfo,
    },
    TodoCleared {
        removed: u32,
    },
```

Add the event in `src/api/schema/events.rs`: a `TodoChanged { pane_id: String }` event data variant, an `EventKind::TodoChanged`, the wire name `"todo.changed"`, and an entry in the all-kinds array. Wire the subscription in `src/api/subscriptions.rs` mirroring `Subscription::NotificationPosted`.

Create `src/app/api/todos.rs` handlers above the test module. Each handler: resolve the pane with the block quoted in the Interfaces section, call the store, and on success `self.state.mark_session_dirty()` and emit `EventKind::TodoChanged` with the pane id. Map `TodoError` with `encode_error(id, err.code(), &err.message())`. Link resolution for `link_pane_id`:

```rust
fn resolve_link(&self, raw: &str) -> Option<crate::terminal::todo::TodoLink> {
    let (ws_idx, pane_id) = self.parse_pane_id(raw)?;
    // PaneInfo.label and .agent are both Option<String>; fall back to the
    // caller's own target string so a link always carries a usable label.
    let label = self
        .pane_info(ws_idx, pane_id)
        .and_then(|pane| pane.label.or(pane.agent))
        .unwrap_or_else(|| raw.to_string());
    Some(crate::terminal::todo::TodoLink { pane: Some(pane_id), label })
}
```

returning `todo_link_unresolved` when it yields `None`. Declare `mod todos;` wherever `mod panes;` is declared, and add the five dispatch arms in `src/app/api.rs` beside `Method::PaneClearScrollback`.

`TodoInfo` is built from a `PaneTodo` plus its owning pane's public id; `link_alive` is `link.pane.is_some()`, and `link_pane_id` is the public id of the linked pane when it still resolves.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run --locked app::api::todos`
Expected: PASS, 9 tests.

Regenerate the API schema doc and confirm the schema tests pass:

Run: `cargo nextest run --locked api::schema`
Expected: PASS. If `docs/next/api/herdr-api.schema.json` is checked by a test, regenerate it the way that test's failure message instructs.

Verify the protocol was **not** bumped:

Run: `grep 'PROTOCOL_VERSION: u32' src/protocol/wire.rs`
Expected: `pub const PROTOCOL_VERSION: u32 = 19;`

- [ ] **Step 5: Commit**

```bash
git add src/api src/app/api docs/next/api/herdr-api.schema.json
git commit -m "feat: add todo socket api and todo.changed event"
```

---

### Task 4: `herdr todo` CLI

**Files:**
- Create: `src/cli/todo.rs`
- Modify: `src/cli/mod.rs` (declare `mod todo;`, route the `todo` command)
- Modify: `src/cli/spec.rs` (register the verb + help text)
- Modify: `src/main.rs` (dispatch, mirroring the `notification` command)
- Test: inline `#[cfg(test)] mod tests` in `src/cli/todo.rs`

**Interfaces:**
- Consumes from Task 3: every `Method::Todo*` variant and its params struct.
- Produces: nothing later in this plan depends on it.

**Target resolution** reuses the grammar in `src/cli/pane.rs`: an explicit `--pane <id>`, `--current`, or the `HERDR_PANE_ID` environment variable as the default. The env default is the point of the feature — an exiting agent runs `herdr todo add "..."` with no target.

- [ ] **Step 1: Write the failing tests**

Create `src/cli/todo.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn add_args_default_to_the_calling_pane() {
        let parsed = parse_todo_add_args(&args(&["fix the flaky test"]), Some("w1:p2")).unwrap();

        assert_eq!(parsed.pane_id, "w1:p2");
        assert_eq!(parsed.text, "fix the flaky test");
        assert_eq!(parsed.priority, None);
        assert_eq!(parsed.link_pane_id, None);
    }

    #[test]
    fn add_args_accept_an_explicit_pane_priority_and_link() {
        let parsed = parse_todo_add_args(
            &args(&["rerun deploy", "--pane", "w1:p3", "--priority", "high", "--link", "infra"]),
            Some("w1:p2"),
        )
        .unwrap();

        assert_eq!(parsed.pane_id, "w1:p3", "explicit --pane beats the env default");
        assert_eq!(parsed.priority, Some(crate::terminal::todo::TodoPriority::High));
        assert_eq!(parsed.link_pane_id.as_deref(), Some("infra"));
    }

    #[test]
    fn add_args_require_a_target_when_not_in_a_pane() {
        let error = parse_todo_add_args(&args(&["orphan todo"]), None).unwrap_err();

        assert_eq!(
            error,
            TodoArgError::Message(
                "no pane target: run inside a herdr pane or pass --pane <pane_id>".into()
            )
        );
    }

    #[test]
    fn add_args_reject_an_invalid_priority() {
        let error =
            parse_todo_add_args(&args(&["x", "--priority", "urgent"]), Some("w1:p1")).unwrap_err();

        assert_eq!(
            error,
            TodoArgError::Message("invalid priority: urgent (expected high, normal, or low)".into())
        );
    }

    #[test]
    fn add_args_require_text() {
        assert_eq!(parse_todo_add_args(&args(&[]), Some("w1:p1")).unwrap_err(), TodoArgError::Usage);
        assert_eq!(
            parse_todo_add_args(&args(&["--pane", "w1:p1"]), Some("w1:p1")).unwrap_err(),
            TodoArgError::Usage,
            "a flag must not be swallowed as the todo text"
        );
    }

    #[test]
    fn add_args_reject_a_missing_flag_value() {
        let error = parse_todo_add_args(&args(&["x", "--priority"]), Some("w1:p1")).unwrap_err();

        assert_eq!(error, TodoArgError::Message("missing value for --priority".into()));
    }

    #[test]
    fn list_args_support_all_and_json() {
        let parsed = parse_todo_list_args(&args(&["--all", "--json"]), Some("w1:p1")).unwrap();

        assert_eq!(parsed.pane_id, None, "--all lists every pane");
        assert!(parsed.json);

        let scoped = parse_todo_list_args(&args(&[]), Some("w1:p1")).unwrap();
        assert_eq!(scoped.pane_id.as_deref(), Some("w1:p1"));
        assert!(!scoped.json);
    }

    #[test]
    fn edit_args_parse_text_priority_and_unlink() {
        let parsed = parse_todo_edit_args(
            &args(&["3", "--text", "new text", "--unlink"]),
            Some("w1:p1"),
        )
        .unwrap();

        assert_eq!(parsed.id, 3);
        assert_eq!(parsed.text.as_deref(), Some("new text"));
        assert!(parsed.clear_link);
        assert_eq!(parsed.link_pane_id, None);
    }

    #[test]
    fn edit_args_reject_link_and_unlink_together() {
        let error =
            parse_todo_edit_args(&args(&["3", "--link", "infra", "--unlink"]), Some("w1:p1"))
                .unwrap_err();

        assert_eq!(error, TodoArgError::Message("--link and --unlink are mutually exclusive".into()));
    }

    #[test]
    fn id_args_reject_a_non_numeric_id() {
        let error = parse_todo_id_args(&args(&["abc"]), Some("w1:p1")).unwrap_err();

        assert_eq!(error, TodoArgError::Message("invalid todo id: abc".into()));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run --locked cli::todo`
Expected: compile failure — the parsers do not exist.

- [ ] **Step 3: Write the implementation**

Prepend to `src/cli/todo.rs` a module modelled directly on `src/cli/notification.rs`:

- `pub(super) fn run_todo_command(args: &[String]) -> std::io::Result<i32>` dispatching `add | list | done | undone | edit | rm | clear | help`.
- `#[derive(Debug, Clone, PartialEq, Eq)] enum TodoArgError { Usage, Message(String) }`.
- Parsed-args structs: `TodoAddArgs { pane_id: String, text: String, priority: Option<TodoPriority>, link_pane_id: Option<String>, json: bool }`, `TodoListArgs { pane_id: Option<String>, json: bool }`, `TodoEditArgs { pane_id: String, id: u64, text: Option<String>, priority: Option<TodoPriority>, link_pane_id: Option<String>, clear_link: bool, json: bool }`, `TodoIdArgs { pane_id: String, id: u64, json: bool }`.
- Each parser takes `(args: &[String], env_pane_id: Option<&str>)` so the tests can drive it without touching the environment; the command functions call `std::env::var("HERDR_PANE_ID").ok()` and pass it in, exactly as `pane_current` does.
- `--current` resolves to the env pane id; a missing target yields the `no pane target: …` message asserted in the tests.
- `parse_priority(value) -> Result<TodoPriority, TodoArgError>` accepting `high | normal | low`.
- `done` maps to `TodoUpdate { done: Some(true) }`, `undone` to `Some(false)`.
- Human-readable `list` output mirrors `notification_list`: a `*` marker for outstanding todos, the id right-aligned, a priority column, the text, and ` → label` appended when linked (with a trailing ` (gone)` when `link_alive` is false). `--json` defers to `super::print_response`.

Register the verb: `mod todo;` in `src/cli/mod.rs`, a `"todo" => todo::run_todo_command(&args[1..])` arm wherever `"notification"` is routed, and the help/spec entry in `src/cli/spec.rs` listing:

```
  herdr todo add <text> [--pane ID|--current] [--priority high|normal|low] [--link <target>]
  herdr todo list [--pane ID|--current|--all] [--json]
  herdr todo done <id> [--pane ID|--current]
  herdr todo undone <id> [--pane ID|--current]
  herdr todo edit <id> [--text TEXT] [--priority high|normal|low] [--link <target>|--unlink]
  herdr todo rm <id> [--pane ID|--current]
  herdr todo clear [--done] [--pane ID|--current]
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run --locked cli::todo`
Expected: PASS, 10 tests.

Then exercise it end to end against a debug server, clearing inherited socket overrides as CLAUDE.md requires:

```bash
env -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH cargo run -- todo add "smoke test" --pane <pane_id>
env -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH cargo run -- todo list --all
```

Expected: the todo is created and listed.

- [ ] **Step 5: Commit**

```bash
git add src/cli/todo.rs src/cli/mod.rs src/cli/spec.rs src/main.rs
git commit -m "feat: add herdr todo cli"
```

---

### Task 5: Docs and full validation

**Files:**
- Modify: `docs/next/CHANGELOG.md`
- Modify: the existing CLI and socket-API pages under `docs/next/website/src/content/docs/`
- Modify: `openspec/changes/pane-todos/tasks.md` (tick groups 1–4)

Extend **existing** pages rather than adding a new `.mdx`: `release-docs-check` enforces ja/zh-cn translation parity for `docs/next`, so a new English-only page would block the next release.

- [ ] **Step 1: Add the changelog entry**

Under `## Unreleased` → `### Added` in `docs/next/CHANGELOG.md`:

```markdown
- Added per-pane todos: each pane carries a durable list of next steps that agents can write from inside it with `herdr todo add "..."` (no pane argument needed — it defaults to the calling pane). Todos carry a priority (high/normal/low), a done flag, and an optional link to another pane, are capped at 50 per pane and 500 characters each, and persist across server restarts and `herdr update --handoff`. The `todo.list` / `todo.add` / `todo.update` / `todo.remove` / `todo.clear` socket methods and a `todo.changed` event expose the same state to external status bars, and `herdr todo list [--all] [--json]` reads it from the command line.
```

- [ ] **Step 2: Document the CLI verbs and socket methods**

Add the `herdr todo` verbs to the existing CLI reference page and the `todo.*` methods plus the `todo.changed` event to the existing socket API page, matching how `herdr notification` and `notification.*` are documented there.

- [ ] **Step 3: Tick the OpenSpec tasks**

Mark groups 1–4 complete in `openspec/changes/pane-todos/tasks.md`, leaving the UI groups unticked.

- [ ] **Step 4: Run the full check**

Run: `just check`
Expected: green apart from the known macOS `live_handoff_keeps_unmanaged_agent_name_bound_to_saved_session` failure, which reproduces on clean `upstream/master`.

Run: `openspec validate pane-todos --strict`
Expected: `Change 'pane-todos' is valid`.

- [ ] **Step 5: Commit**

```bash
git add docs/next openspec/changes/pane-todos/tasks.md
git commit -m "docs: document pane todos cli and socket api"
```

---

## Self-Review Notes

**Spec coverage.** Phase 1 requirements from `specs/pane-todos/spec.md` map to tasks as follows: *Pane-scoped todo store* → Task 1; *Todo display ordering* → Task 1; *Todos persist with their pane* → Task 2 (except the close-confirmation scenario, which needs UI and moves to the Phase 2 plan); *Cross-pane todo links* → Tasks 1–3; *Todo socket API and event* → Task 3; *Todo CLI* → Task 4. The *Pane todo indicator* and *Pane todo panel and editing* requirements are deliberately out of this plan.

**Carried into the Phase 2 plan:** the pane-close confirmation for outstanding todos, the border indicator, the dropdown panel, the edit modal, the two keybindings, and their `keybind_help` entries.

**Existing symbols verified against the tree at `30315b8a`** — every one of these is referenced by the plan and confirmed to exist with the receiver shown:

| Symbol | Location |
|---|---|
| `App::parse_pane_id(&self, &str) -> Option<(usize, PaneId)>` | `src/app/ids.rs:106` (`pub(super)`) |
| `App::public_pane_id(..)` | `src/app/ids.rs:27` (`pub(super)`) |
| `App::pane_info(&self, usize, PaneId) -> Option<PaneInfo>` | `src/app/creation.rs:419` |
| `App::handle_api_request(&mut self, Request) -> String` | `src/app/api.rs:985` |
| `Workspace::terminal_id(&self, PaneId) -> Option<&TerminalId>` | `src/workspace.rs:1179` |
| `Workspace::test_new(&str)` | `src/workspace.rs:1246` (`pub(crate)`) |
| `AppState::ensure_test_terminals(&mut self)` | `src/app/state.rs:2366` |
| `AppState::mark_session_dirty(&mut self)` | `src/app/state.rs:1865` |
| `pane_not_found(String, &str) -> String` | `src/app/api/pane_graphics.rs:260` |
| `encode_error(String, &str, impl Into<String>) -> String` | `src/app/api/responses.rs:7` |
| `PaneInfo.label` / `.agent` | both `Option<String>`, `src/api/schema/panes.rs:440,442` |

`pub(super)` items in `src/app/ids.rs` resolve to module `app`, so they are visible from `app::api::todos`.

**Type consistency.** `TodoPriority`, `TodoLink`, `PaneTodo`, `TodoUpdate`, and `TodoError` are defined once in Task 1 and referenced unchanged in Tasks 2–4. The snapshot type `PaneTodoSnapshot` (Task 2) is distinct from the wire type `TodoInfo` (Task 3) on purpose: the snapshot stores raw `u32` pane ids for remapping, the wire type stores public pane id strings.
