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

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
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

    /// Restore path only: mutable access for the link post-pass, which cannot
    /// resolve cross-pane targets until every restored pane has an id.
    pub(crate) fn todos_mut(&mut self) -> &mut Vec<PaneTodo> {
        &mut self.todos
    }

    /// Restore path only: install saved todos and the saved id counter.
    pub fn restore_todos(&mut self, todos: Vec<PaneTodo>, next_todo_id: u64) {
        let highest = todos.iter().map(|todo| todo.id).max().unwrap_or(0);
        self.next_todo_id = next_todo_id.max(highest + 1);
        self.todos = todos;
    }
}

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
        let todo = t
            .add_todo("write the plan", TodoPriority::Normal, None, 100)
            .unwrap();

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

        assert_eq!(
            t.add_todo("   ", TodoPriority::Normal, None, 100),
            Err(TodoError::TextEmpty)
        );
        assert_eq!(
            t.add_todo("", TodoPriority::Normal, None, 100),
            Err(TodoError::TextEmpty)
        );
        assert!(t.todos().is_empty());

        let todo = t
            .add_todo("  padded  ", TodoPriority::Normal, None, 100)
            .unwrap();
        assert_eq!(todo.text, "padded");
    }

    #[test]
    fn add_todo_rejects_text_over_the_limit() {
        let mut t = terminal();
        let long = "x".repeat(MAX_TODO_TEXT_LEN + 1);

        assert_eq!(
            t.add_todo(&long, TodoPriority::Normal, None, 100),
            Err(TodoError::TextTooLong)
        );
        assert!(t.todos().is_empty());

        let exact = "y".repeat(MAX_TODO_TEXT_LEN);
        assert!(t.add_todo(&exact, TodoPriority::Normal, None, 100).is_ok());
    }

    /// A todo may hold more than one line. The store already allowed it —
    /// `validate_text` trims the ends and counts characters, and never
    /// inspects for control characters — but nothing could produce one until
    /// the edit modal's field grew a newline key, so it was never pinned.
    #[test]
    fn a_todo_keeps_its_embedded_newlines_through_add_update_and_restore() {
        let mut t = terminal();

        let todo = t
            .add_todo("  first\nsecond  ", TodoPriority::Normal, None, 100)
            .unwrap();
        assert_eq!(
            todo.text, "first\nsecond",
            "the ends are trimmed, the newline inside is not"
        );

        t.update_todo(
            todo.id,
            TodoUpdate {
                text: Some("one\ntwo\nthree".into()),
                ..TodoUpdate::default()
            },
            200,
        )
        .unwrap();
        assert_eq!(t.todos()[0].text, "one\ntwo\nthree");

        // The snapshot path a session restore comes back through.
        let saved = t.todos().to_vec();
        let mut restored = terminal();
        restored.restore_todos(saved, 99);
        assert_eq!(restored.todos()[0].text, "one\ntwo\nthree");

        // Newlines count toward the cap like any other character.
        let at_cap = "z\n".repeat(MAX_TODO_TEXT_LEN / 2);
        assert_eq!(at_cap.chars().count(), MAX_TODO_TEXT_LEN);
        assert_eq!(
            restored.add_todo(&format!("{at_cap}z"), TodoPriority::Normal, None, 300),
            Err(TodoError::TextTooLong)
        );
    }

    #[test]
    fn add_todo_enforces_the_per_pane_cap() {
        let mut t = terminal();
        for i in 0..MAX_TODOS_PER_PANE {
            t.add_todo(&format!("todo {i}"), TodoPriority::Normal, None, 100)
                .unwrap();
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
        let first = t
            .add_todo("first", TodoPriority::Normal, None, 100)
            .unwrap()
            .id;
        t.remove_todo(first).unwrap();
        let second = t
            .add_todo("second", TodoPriority::Normal, None, 100)
            .unwrap()
            .id;

        assert!(second > first, "expected {second} > {first}");
    }

    #[test]
    fn update_todo_changes_fields_and_bumps_updated_at() {
        let mut t = terminal();
        let id = t
            .add_todo("draft", TodoPriority::Normal, None, 100)
            .unwrap()
            .id;

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
        assert!(
            !updated.done,
            "done must be untouched when not in the update"
        );
    }

    #[test]
    fn update_todo_can_set_and_clear_the_link() {
        let mut t = terminal();
        let id = t
            .add_todo("rerun deploy", TodoPriority::Normal, None, 100)
            .unwrap()
            .id;
        let link = TodoLink {
            pane: Some(crate::layout::PaneId::from_raw(7)),
            label: "infra".into(),
        };

        let set = t
            .update_todo(
                id,
                TodoUpdate {
                    link: Some(Some(link.clone())),
                    ..Default::default()
                },
                200,
            )
            .unwrap();
        assert_eq!(set.link, Some(link));

        let cleared = t
            .update_todo(
                id,
                TodoUpdate {
                    link: Some(None),
                    ..Default::default()
                },
                300,
            )
            .unwrap();
        assert_eq!(cleared.link, None);

        let untouched = t
            .update_todo(
                id,
                TodoUpdate {
                    done: Some(true),
                    ..Default::default()
                },
                400,
            )
            .unwrap();
        assert_eq!(untouched.link, None, "link: None must leave the link alone");
    }

    #[test]
    fn update_and_remove_report_missing_todos() {
        let mut t = terminal();

        assert_eq!(
            t.update_todo(42, TodoUpdate::default(), 100),
            Err(TodoError::NotFound)
        );
        assert_eq!(t.remove_todo(42), Err(TodoError::NotFound));
    }

    #[test]
    fn update_todo_validates_replacement_text() {
        let mut t = terminal();
        let id = t
            .add_todo("keep", TodoPriority::Normal, None, 100)
            .unwrap()
            .id;

        assert_eq!(
            t.update_todo(
                id,
                TodoUpdate {
                    text: Some("  ".into()),
                    ..Default::default()
                },
                200
            ),
            Err(TodoError::TextEmpty)
        );
        assert_eq!(
            t.update_todo(
                id,
                TodoUpdate {
                    text: Some("z".repeat(MAX_TODO_TEXT_LEN + 1)),
                    ..Default::default()
                },
                200
            ),
            Err(TodoError::TextTooLong)
        );
        assert_eq!(
            t.todos()[0].text,
            "keep",
            "a rejected update must not mutate the todo"
        );
    }

    #[test]
    fn clear_todos_removes_all_or_only_done() {
        let mut t = terminal();
        let a = t.add_todo("a", TodoPriority::Normal, None, 100).unwrap().id;
        t.add_todo("b", TodoPriority::Normal, None, 100).unwrap();
        t.update_todo(
            a,
            TodoUpdate {
                done: Some(true),
                ..Default::default()
            },
            200,
        )
        .unwrap();

        assert_eq!(t.clear_todos(true), 1);
        assert_eq!(t.todos().len(), 1);
        assert_eq!(t.todos()[0].text, "b");

        assert_eq!(t.clear_todos(false), 1);
        assert!(t.todos().is_empty());
    }

    #[test]
    fn display_order_is_priority_then_not_done_then_creation() {
        let mut t = terminal();
        let normal = t
            .add_todo("normal first", TodoPriority::Normal, None, 100)
            .unwrap()
            .id;
        let high = t
            .add_todo("high second", TodoPriority::High, None, 101)
            .unwrap()
            .id;
        let low = t
            .add_todo("low third", TodoPriority::Low, None, 102)
            .unwrap()
            .id;

        let order: Vec<u64> = t
            .todos_in_display_order()
            .iter()
            .map(|todo| todo.id)
            .collect();
        assert_eq!(order, vec![high, normal, low]);

        // a done high-priority todo sinks below a not-done normal one
        t.update_todo(
            high,
            TodoUpdate {
                done: Some(true),
                ..Default::default()
            },
            200,
        )
        .unwrap();
        let order: Vec<u64> = t
            .todos_in_display_order()
            .iter()
            .map(|todo| todo.id)
            .collect();
        assert_eq!(order, vec![normal, low, high]);
    }

    #[test]
    fn display_order_does_not_change_stored_order() {
        let mut t = terminal();
        t.add_todo("first", TodoPriority::Normal, None, 100)
            .unwrap();
        let high = t
            .add_todo("second", TodoPriority::High, None, 101)
            .unwrap()
            .id;

        let _ = t.todos_in_display_order();

        assert_eq!(
            t.todos()[0].text,
            "first",
            "stored order stays insertion order"
        );
        assert_eq!(t.todos()[1].id, high);
    }

    #[test]
    fn outstanding_count_and_highest_priority_ignore_done_todos() {
        let mut t = terminal();
        let high = t
            .add_todo("high", TodoPriority::High, None, 100)
            .unwrap()
            .id;
        t.add_todo("normal", TodoPriority::Normal, None, 100)
            .unwrap();

        assert_eq!(t.outstanding_todo_count(), 2);
        assert_eq!(
            t.highest_outstanding_todo_priority(),
            Some(TodoPriority::High)
        );

        t.update_todo(
            high,
            TodoUpdate {
                done: Some(true),
                ..Default::default()
            },
            200,
        )
        .unwrap();
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
