# Pane Todos — Phase 2 (TUI) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the pane todos Phase 1 already stores visible and editable from the TUI — a count on the pane's top border, a panel hanging off it, an edit modal, two bindable actions, and a confirmation before a pane with unfinished work is destroyed.

**Architecture:** The todo *data* is server-owned and already exists (`src/terminal/todo.rs`, `src/app/api/todos.rs`). Phase 2 adds **only client state**: whether the panel is open, which row is selected, which footer button is hovered, and the edit modal's in-progress buffer. Every mutation the TUI makes goes back through the existing `todo.*` API via new `runtime_todo_*` wrappers, so the panel, the CLI, and external subscribers all move the same state and `todo.changed` fires for free. The indicator follows the `expanded_sidebar_toggle_rect` idiom: **one** function computes the cells, and both the renderer and the mouse hit-test call it. The panel follows the notification center (`src/ui/notification_center.rs`, `Mode::NotificationCenter`) one-for-one: geometry helpers on `AppState` in `src/app/input/mouse.rs`, a pure renderer in a new `src/ui/todo_panel.rs`, settings-language footer buttons.

**Tech Stack:** Rust, ratatui (`TestBackend` for render assertions), `cargo nextest`, `just`.

## Scope

This plan covers **Phase 2 (TUI)** only — OpenSpec task groups 5, 6, the deferred 2.3, and the UI half of 7:

1. Pane todo indicator on the top border (+ `ui.show_pane_todo_indicator`, `ui.pane_todo_color`)
2. Todo panel: `Mode::PaneTodos`, TUI-only state, geometry, rendering
3. Panel input: keys, mouse, `keys.open_pane_todos`, follow-link
4. Edit modal: `Mode::PaneTodoEdit`, `keys.add_pane_todo`
5. Pane-close confirmation for outstanding todos, docs, validation

**Not in this plan:** OpenSpec task 4.4 (resolving `--link` by unique live agent name). It is a CLI/server concern, was deferred out of Phase 1 on its own merits, and is not in the Phase 2 requirement set. Leave it unticked.

## Global Constraints

- Source of truth for requirements: `openspec/changes/pane-todos/` (`proposal.md`, `design.md`, `specs/pane-todos/spec.md`, `tasks.md`). Re-read the relevant requirement before each task. Phase 2 implements exactly: **Pane todo indicator**, **Pane todo panel and editing**, and the *"Closing a pane with outstanding todos is confirmed"* scenario of **Todos persist with their pane**.
- **No `unwrap()` in production code** (tests are fine). Use `tracing` for logging. `#[allow]` only with a comment explaining why.
- **No protocol bump. `PROTOCOL_VERSION` stays at 19.** Phase 2 adds no wire types and no methods: `src/api/schema/` is not touched at all, and `docs/next/api/herdr-api.schema.json` does not change. If a task makes you want to edit `src/api/schema/`, stop — the feature belongs on the client side.
- **Runtime/client boundary.** Panel open/closed, `selected`, `hovered_button`, and the edit buffer are TUI-only and live on `AppState`. They never appear in `src/api/schema/`, in a snapshot, or in an event. Mutations go out through `todo.add` / `todo.update` / `todo.remove` / `todo.clear`.
- **Render is pure.** `render_*` takes `&AppState` and only draws. Geometry that both the renderer and the mouse layer need is computed by a shared function (a free `fn` over `Rect`s, or an `&self` method on `AppState`), never cached during render.
- **Every new keybinding gets a `help_entry`** in `src/ui/keybind_help.rs`, including `add_pane_todo`, which ships unbound — the panel renders `unset`, and that is how users discover it. A shortcut absent from the help panel is incomplete.
- Reuse the existing UI language. The notification center is the reference for the panel; the rename modal (`render_rename_overlay`, `rename_button_rects`) is the reference for the edit modal; `expanded_sidebar_toggle_rect` is the reference for the indicator.
- Commit style: lowercase conventional commits, no emojis, no AI co-author or "Generated with" lines.
- Test filter: `cargo nextest run --locked --no-fail-fast <filter>`. Run `just check` before the final commit — and note that `just check` and `just test` are wider than cargo: their `python3 -m unittest` line runs the maintenance scripts, including `scripts/test_config_reference_check.py`. A commit that adds a config key without its `docs/next/.../config-reference.json` row leaves the tree red, so every task commits green on its own, not just the last one.
- Known unrelated failure on macOS: `live_handoff_keeps_unmanaged_agent_name_bound_to_saved_session` fails identically on clean `upstream/master`. Ignore it, never "fix" it, never count it as a regression (fork issue #33).
- `tests/cli` is `#![cfg(not(target_os = "macos"))]`, so it reports "0 tests run" locally (issue #30). That is expected, not a problem.

## Design decisions already settled — do not re-litigate

- The indicator is a bare glyph at the **far right** of the pane's top border: `" ▾ N "` with the **outstanding** count, a bare dimmed `" ▾ "` once every todo is done, and **nothing at all** when the pane has no todos.
- **One** shared function returns the indicator's cells; the renderer draws into exactly those cells and the mouse hit-test reads exactly those cells. A test asserts they agree.
- In the panel, `Enter` opens the **edit** view (todos are authored, unlike notifications). Following a link is bound to the link chip and `g`.
- Dead links (`link.pane == None`, or a target pane that no longer exists) render dimmed and are inert.
- A pane with **no top border** (single-pane tab, or `ui.pane_borders = false`) shows no indicator: nothing in herdr draws chrome on a borderless pane, and inventing a floating badge would be a new UI language. The keybinding still opens the panel there — that is the discoverable path for borderless layouts.
- Docs extend **existing** pages under `docs/next/`. A new `.mdx` would need `ja` and `zh-cn` translations or `just release-docs-check` fails.

---

### Task 1: Pane todo indicator on the top border

**Files:**
- Modify: `src/ui/panes.rs` (indicator type + shared rect fn + rework of `render_pane_border_titles`; tests at the bottom of the file)
- Modify: `src/ui.rs` (re-export `pane_todo_indicator` / `PaneTodoIndicator` for the mouse layer in Task 3)
- Modify: `src/app/state.rs` (`pane_terminal`, `pane_todo_indicator_color`, two config-backed fields, `test_new` initialisers)
- Modify: `src/config/model.rs` (`ui.show_pane_todo_indicator`, `ui.pane_todo_color` + defaults + test)
- Modify: `src/app/mod.rs` (`App::new` mapping **and** `apply_live_config`)
- Modify: `src/main.rs` (`DEFAULT_CONFIG` `[ui]` sample lines)
- Modify: `src/terminal/todo.rs` (drop the two `#[allow(dead_code)]`)
- Modify: `docs/next/website/src/data/config-reference.json` (the two `ui.*` rows — `scripts/config_reference_check.py` runs inside `just check`, so the rows ship with the keys, and OpenSpec task 5.2 pairs them anyway)

**Interfaces:**
- Consumes: `TerminalState::todos()`, `TerminalState::outstanding_todo_count()`, `TerminalState::highest_outstanding_todo_priority()` (all `src/terminal/todo.rs`); `crate::layout::PaneInfo` (`src/layout.rs:34`); `Workspace::pane_state(PaneId) -> Option<&PaneState>` (`src/workspace.rs:1175`); `AppState::pane_title_color(bool) -> Color` (`src/app/state.rs:1959`); `super::text::truncate_end` / `display_width_u16` (`src/ui/text.rs:11`/`:7`).
- Produces, relied on by Tasks 2–5:
  - `pub(crate) struct PaneTodoIndicator { pub rect: Rect, pub label: String, pub priority: Option<TodoPriority> }` in `src/ui/panes.rs` — the label already carries the outstanding count, so no caller needs it as a separate field
  - `pub(crate) fn pane_todo_indicator(app: &AppState, info: &PaneInfo) -> Option<PaneTodoIndicator>` — the single shared definition
  - `impl AppState { pub(crate) fn pane_terminal(&self, pane_id: PaneId) -> Option<&crate::terminal::TerminalState> }`
  - `impl AppState { pub fn pane_todo_indicator_color(&self, priority: Option<TodoPriority>) -> Color }`
  - config: `ui.show_pane_todo_indicator: bool` (default `true`), `ui.pane_todo_color: Option<String>` (unset → priority colours)

- [ ] **Step 1: Write the failing tests**

Append to the existing `#[cfg(test)] mod tests` in `src/ui/panes.rs` (it already has `use super::*;`, `use crate::layout::PaneId;`, `use crate::terminal::TerminalState;`, `use crate::workspace::Workspace;` and the `render_view_pane_borders` helper):

```rust
    use crate::terminal::todo::{TodoPriority, TodoUpdate};

    /// One 30x4 pane with `Borders::ALL`, whose terminal carries `todos` given
    /// as (done, priority) pairs. The workspace lives in `app.workspaces` and
    /// `app.active` points at it, because the indicator resolves a pane's
    /// terminal the same way `render_panes` does.
    fn app_with_pane_todos(todos: &[(bool, TodoPriority)]) -> AppState {
        let mut app = AppState::test_new();
        app.mode = Mode::Terminal;
        app.workspaces = vec![Workspace::test_new("todos")];
        app.active = Some(0);
        app.ensure_test_terminals();

        let pane_id = app.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let terminal = app
            .terminals
            .get_mut(&terminal_id)
            .expect("test terminal should exist");
        for (index, (done, priority)) in todos.iter().enumerate() {
            let todo = terminal
                .add_todo(&format!("todo {index}"), *priority, None, 100)
                .expect("todo should be added");
            if *done {
                terminal
                    .update_todo(
                        todo.id,
                        TodoUpdate {
                            done: Some(true),
                            ..TodoUpdate::default()
                        },
                        200,
                    )
                    .expect("todo should be updated");
            }
        }

        app.view.terminal_area = Rect::new(0, 0, 30, 4);
        app.view.pane_infos = vec![PaneInfo {
            id: pane_id,
            rect: Rect::new(0, 0, 30, 4),
            inner_rect: Rect::new(1, 1, 28, 2),
            scrollbar_rect: None,
            borders: Borders::ALL,
            is_focused: false,
        }];
        app
    }

    fn draw_pane_borders(app: &AppState) -> ratatui::buffer::Buffer {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(30, 4)).unwrap();
        terminal
            .draw(|frame| render_view_pane_borders(app, &app.workspaces[0], frame))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn row_text(buffer: &ratatui::buffer::Buffer, row: u16, width: u16) -> String {
        (0..width).map(|x| buffer[(x, row)].symbol()).collect()
    }

    #[test]
    fn pane_todo_indicator_counts_only_outstanding_todos() {
        let app = app_with_pane_todos(&[
            (false, TodoPriority::High),
            (false, TodoPriority::Normal),
            (false, TodoPriority::Low),
            (true, TodoPriority::High),
        ]);

        let indicator =
            pane_todo_indicator(&app, &app.view.pane_infos[0]).expect("indicator should exist");

        assert_eq!(indicator.label, " ▾ 3 ");
        assert_eq!(indicator.priority, Some(TodoPriority::High));
    }

    /// Spec: "the cells that respond to a click are exactly the cells drawn".
    #[test]
    fn pane_todo_indicator_draws_exactly_the_cells_it_claims() {
        let app = app_with_pane_todos(&[(false, TodoPriority::High), (false, TodoPriority::Low)]);
        let indicator =
            pane_todo_indicator(&app, &app.view.pane_infos[0]).expect("indicator should exist");
        let buffer = draw_pane_borders(&app);

        let drawn: String = (indicator.rect.x..indicator.rect.x + indicator.rect.width)
            .map(|x| buffer[(x, indicator.rect.y)].symbol())
            .collect();
        assert_eq!(drawn, indicator.label, "claimed cells must hold the label");
        assert_eq!(
            buffer[(indicator.rect.x - 1, indicator.rect.y)].symbol(),
            "─",
            "the cell before the indicator is still border"
        );
        assert_eq!(
            buffer[(indicator.rect.x + indicator.rect.width, indicator.rect.y)].symbol(),
            "┐",
            "the corner glyph is never overwritten"
        );
        assert_eq!(
            indicator.rect.x + indicator.rect.width,
            app.view.pane_infos[0].rect.x + app.view.pane_infos[0].rect.width - 1,
            "the indicator hugs the far right of the top border"
        );
    }

    #[test]
    fn a_pane_with_no_todos_renders_the_border_it_has_today() {
        let app = app_with_pane_todos(&[]);

        assert!(pane_todo_indicator(&app, &app.view.pane_infos[0]).is_none());
        let buffer = draw_pane_borders(&app);
        assert_eq!(row_text(&buffer, 0, 30), format!("┌{}┐", "─".repeat(28)));
    }

    #[test]
    fn an_all_done_pane_shows_a_bare_dimmed_glyph() {
        let app = app_with_pane_todos(&[(true, TodoPriority::High), (true, TodoPriority::Normal)]);
        let indicator =
            pane_todo_indicator(&app, &app.view.pane_infos[0]).expect("indicator should exist");

        assert_eq!(indicator.label, " ▾ ", "no count once everything is done");
        assert_eq!(indicator.priority, None);

        let buffer = draw_pane_borders(&app);
        assert_eq!(
            buffer[(indicator.rect.x + 1, indicator.rect.y)].style().fg,
            Some(app.palette.overlay0),
            "a finished pane's indicator is muted"
        );
    }

    #[test]
    fn indicator_color_follows_the_highest_outstanding_priority() {
        let high = app_with_pane_todos(&[(false, TodoPriority::High), (false, TodoPriority::Low)]);
        let normal = app_with_pane_todos(&[(false, TodoPriority::Normal)]);

        assert_eq!(
            high.pane_todo_indicator_color(Some(TodoPriority::High)),
            high.palette.red
        );
        assert_eq!(
            normal.pane_todo_indicator_color(Some(TodoPriority::Normal)),
            normal.palette.yellow
        );

        let mut pinned = app_with_pane_todos(&[(false, TodoPriority::High)]);
        pinned.pane_todo_color = Some(ratatui::style::Color::Magenta);
        assert_eq!(
            pinned.pane_todo_indicator_color(Some(TodoPriority::High)),
            ratatui::style::Color::Magenta,
            "ui.pane_todo_color pins the outstanding colour"
        );
        assert_eq!(
            pinned.pane_todo_indicator_color(None),
            pinned.palette.overlay0,
            "an all-done indicator stays muted even when pinned"
        );
    }

    /// Spec: "the indicator SHALL be laid out before the pane title so the
    /// title truncates instead of the control disappearing".
    #[test]
    fn the_indicator_reserves_its_cells_before_the_title() {
        let mut app = app_with_pane_todos(&[(false, TodoPriority::High)]);
        let pane_id = app.view.pane_infos[0].id;
        let terminal_id = app.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        app.terminals
            .get_mut(&terminal_id)
            .expect("test terminal should exist")
            .set_manual_label("a very long pane label indeed".into());

        let indicator =
            pane_todo_indicator(&app, &app.view.pane_infos[0]).expect("indicator should exist");
        let buffer = draw_pane_borders(&app);

        let drawn: String = (indicator.rect.x..indicator.rect.x + indicator.rect.width)
            .map(|x| buffer[(x, indicator.rect.y)].symbol())
            .collect();
        assert_eq!(drawn, indicator.label, "the control survives intact");
        assert!(
            row_text(&buffer, 0, 30).contains('…'),
            "the title truncates instead"
        );
    }

    #[test]
    fn the_indicator_is_hidden_by_config_and_on_borderless_panes() {
        let mut off = app_with_pane_todos(&[(false, TodoPriority::High)]);
        off.show_pane_todo_indicator = false;
        assert!(pane_todo_indicator(&off, &off.view.pane_infos[0]).is_none());

        let mut borderless = app_with_pane_todos(&[(false, TodoPriority::High)]);
        borderless.view.pane_infos[0].borders = Borders::NONE;
        assert!(pane_todo_indicator(&borderless, &borderless.view.pane_infos[0]).is_none());

        let mut narrow = app_with_pane_todos(&[(false, TodoPriority::High)]);
        narrow.view.pane_infos[0].rect = Rect::new(0, 0, 6, 4);
        assert!(
            pane_todo_indicator(&narrow, &narrow.view.pane_infos[0]).is_none(),
            "below the minimum width neither the title nor the control is drawn"
        );
    }
```

Add to `src/config/model.rs`'s `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn pane_todo_indicator_config_parses_and_defaults() {
        let defaults = Config::default();
        assert!(defaults.ui.show_pane_todo_indicator);
        assert_eq!(defaults.ui.pane_todo_color, None);

        let toml = r##"
[ui]
show_pane_todo_indicator = false
pane_todo_color = "#f38ba8"
"##;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(!config.ui.show_pane_todo_indicator);
        assert_eq!(config.ui.pane_todo_color.as_deref(), Some("#f38ba8"));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run --locked --no-fail-fast ui::panes`
Expected: compile failure — `pane_todo_indicator`, `AppState::pane_todo_color`, `AppState::pane_todo_indicator_color` do not exist.

Run: `cargo nextest run --locked --no-fail-fast config::model::tests::pane_todo_indicator_config`
Expected: compile failure — `UiConfig::show_pane_todo_indicator` does not exist.

- [ ] **Step 3: Write the implementation**

**3a. `src/config/model.rs`** — in `pub struct UiConfig` next to `show_agent_labels_on_pane_borders` (`:1053`):

```rust
    /// Show a todo indicator at the far right of a split pane's top border,
    /// carrying the pane's outstanding todo count. Default: true.
    pub show_pane_todo_indicator: bool,
```

and next to the other colour overrides (after `pane_title_inactive_color`, `:1139`):

```rust
    /// Override colour for the pane todo indicator while todos are outstanding.
    /// Same syntax as `accent`. Unset colours it by the highest outstanding
    /// priority (high red, normal yellow, low blue); an all-done indicator is
    /// always muted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_todo_color: Option<String>,
```

In `impl Default for UiConfig` (`:1342`), matching field order: `show_pane_todo_indicator: true,` and `pane_todo_color: None,`.

**3b. `src/app/state.rs`** — fields next to `show_agent_labels_on_pane_borders` (`:1743`) and the pane colours (`:1783`):

```rust
    pub show_pane_todo_indicator: bool,
    pub pane_todo_color: Option<Color>,
```

`test_new` initialisers alongside `pane_borders: true,` (`:2282`) and `pane_title_inactive_color: None,`:

```rust
            show_pane_todo_indicator: true,
            pane_todo_color: None,
```

Methods, next to `pane_title_color` (`:1959`):

```rust
    /// The terminal backing a pane, wherever that pane lives. Todos are stored
    /// on `TerminalState`, so every todo surface resolves through here.
    pub(crate) fn pane_terminal(
        &self,
        pane_id: PaneId,
    ) -> Option<&crate::terminal::TerminalState> {
        let pane = self
            .workspaces
            .iter()
            .find_map(|workspace| workspace.pane_state(pane_id))?;
        self.terminals.get(&pane.attached_terminal_id)
    }

    /// Colour of a pane's todo indicator. The highest outstanding priority
    /// drives it unless `ui.pane_todo_color` pins it; `None` means every todo
    /// is done, which always reads muted.
    pub fn pane_todo_indicator_color(
        &self,
        priority: Option<crate::terminal::todo::TodoPriority>,
    ) -> Color {
        let Some(priority) = priority else {
            return self.palette.overlay0;
        };
        if let Some(color) = self.pane_todo_color {
            return color;
        }
        match priority {
            crate::terminal::todo::TodoPriority::High => self.palette.red,
            crate::terminal::todo::TodoPriority::Normal => self.palette.yellow,
            crate::terminal::todo::TodoPriority::Low => self.palette.blue,
        }
    }
```

**3c. `src/app/mod.rs`** — in the `App::new` state literal next to `show_agent_labels_on_pane_borders` (`:712`) and the colour block (`:764`):

```rust
            show_pane_todo_indicator: config.ui.show_pane_todo_indicator,
            pane_todo_color: config
                .ui
                .pane_todo_color
                .as_deref()
                .map(crate::config::parse_color),
```

and in `apply_live_config`'s `if !invalid_section("ui")` block (`:1576` / `:1642`) — **omitting this is the classic miss: the key would silently ignore `herdr` config reload**:

```rust
                self.state.show_pane_todo_indicator = config.ui.show_pane_todo_indicator;
                self.state.pane_todo_color = config
                    .ui
                    .pane_todo_color
                    .as_deref()
                    .map(crate::config::parse_color);
```

**3d. `src/main.rs`** — in `DEFAULT_CONFIG`'s `[ui]` block, after the `pane_title_*` lines (`:442`):

```text
# Show a todo indicator (▾ N outstanding) at the far right of a split pane's
# top border. Panes with no todos are unaffected.
# show_pane_todo_indicator = true

# Colour for the pane todo indicator while todos are outstanding (same syntax
# as accent). Unset colours it by the highest outstanding priority.
# pane_todo_color = "#f38ba8"
```

**3e. `src/ui/panes.rs`** — extend the text import (line 12) and add the indicator above `render_pane_border_titles`:

```rust
use super::text::{display_width_u16, truncate_end};
```

```rust
/// Where a pane's todo indicator lives and what it says. The renderer and the
/// mouse hit-test both read this one value, which is what keeps the drawn
/// glyph and the click target from drifting.
pub(crate) struct PaneTodoIndicator {
    /// Exactly the cells the label is drawn into.
    pub rect: Rect,
    pub label: String,
    /// Highest outstanding priority; `None` once every todo is done.
    pub priority: Option<crate::terminal::todo::TodoPriority>,
}

/// `▾ N` for N outstanding todos, a bare `▾` once they are all done, and
/// nothing at all for a pane with no todos — a quiet pane keeps exactly the
/// border it has today. Same spacing grammar as the notification `◆`. The
/// count is at most two digits because `add_todo` caps a pane at
/// `MAX_TODOS_PER_PANE` (50).
fn pane_todo_indicator_label(total: usize, outstanding: usize) -> Option<String> {
    if total == 0 {
        return None;
    }
    if outstanding == 0 {
        return Some(" ▾ ".to_string());
    }
    Some(format!(" ▾ {outstanding} "))
}

/// The entry point for a caller holding only a pane id — the mouse hit-test,
/// which must read the same cells the renderer drew. It lands in phase-2 task
/// 3; until then the bin target reaches this only from tests, so it carries
/// the same scoped allow as its `src/ui.rs` re-export and both go away
/// together.
#[allow(dead_code)]
pub(crate) fn pane_todo_indicator(app: &AppState, info: &PaneInfo) -> Option<PaneTodoIndicator> {
    pane_todo_indicator_for(app, info, app.pane_terminal(info.id)?)
}

/// [`pane_todo_indicator`] for a caller that already holds the pane's
/// terminal. The render path resolves it from the workspace it was handed, so
/// it must not pay `AppState::pane_terminal`'s scan of every workspace once
/// per pane per frame.
fn pane_todo_indicator_for(
    app: &AppState,
    info: &PaneInfo,
    terminal: &crate::terminal::TerminalState,
) -> Option<PaneTodoIndicator> {
    // No top border means no place to put it: a single-pane tab or
    // `ui.pane_borders = false` draws no chrome at all, and the keybinding is
    // the discoverable path there.
    if !app.show_pane_todo_indicator || !info.borders.contains(Borders::TOP) {
        return None;
    }
    let label =
        pane_todo_indicator_label(terminal.todos().len(), terminal.outstanding_todo_count())?;
    let width = display_width_u16(&label);
    // The label plus both corner glyphs is the whole requirement: the title is
    // laid out in what is left over and drops itself when that is too narrow,
    // so the control is what survives a squeeze.
    if width == 0 || info.rect.width < width.saturating_add(2) {
        return None;
    }
    let x = info
        .rect
        .x
        .saturating_add(info.rect.width)
        .saturating_sub(1)
        .saturating_sub(width);
    Some(PaneTodoIndicator {
        rect: Rect::new(x, info.rect.y, width, 1),
        label,
        priority: terminal.highest_outstanding_todo_priority(),
    })
}
```

Rewrite the body of `render_pane_border_titles` (`:630-669`). Note the shape change: the old code `continue`d when a pane had no border label, which would also skip the indicator — a pane with todos and no label must still show one.

```rust
    let buf = frame.buffer_mut();
    let area = buf.area;
    for info in pane_infos {
        if !info.borders.contains(Borders::TOP) || info.rect.width <= 4 {
            continue;
        }
        let y = info.rect.y;
        if y < area.y || y >= area.y.saturating_add(area.height) {
            continue;
        }

        // One lookup feeds both the indicator and the title.
        let terminal = ws
            .pane_state(info.id)
            .and_then(|pane| app.terminals.get(&pane.attached_terminal_id));

        // The indicator claims the far right of the border before the title is
        // laid out, so a narrow pane truncates the title instead of dropping
        // the control.
        let indicator = terminal.and_then(|terminal| pane_todo_indicator_for(app, info, terminal));
        let reserved = indicator
            .as_ref()
            .map(|indicator| indicator.rect.width)
            .unwrap_or(0);

        if let Some(title) = terminal
            .and_then(|terminal| terminal.border_label(app.show_agent_labels_on_pane_borders))
            .and_then(|label| {
                pane_border_title(
                    &label,
                    info.rect.width.saturating_sub(reserved),
                    info.is_focused,
                )
            })
        {
            let start_x = info.rect.x.saturating_add(1);
            let end_x = info
                .rect
                .x
                .saturating_add(info.rect.width)
                .saturating_sub(1)
                .saturating_sub(reserved)
                .min(area.x.saturating_add(area.width));
            if start_x < end_x {
                let mut style = Style::default().fg(app.pane_title_color(info.is_focused));
                if info.is_focused {
                    style = style.add_modifier(Modifier::BOLD);
                }
                buf.set_stringn(
                    start_x,
                    y,
                    title,
                    end_x.saturating_sub(start_x) as usize,
                    style,
                );
            }
        }

        if let Some(indicator) = indicator {
            buf.set_stringn(
                indicator.rect.x,
                indicator.rect.y,
                &indicator.label,
                indicator.rect.width as usize,
                Style::default().fg(app.pane_todo_indicator_color(indicator.priority)),
            );
        }
    }
```

**3f. `src/ui.rs`** — extend the `panes` re-export, which is the `panes::{apply_pane_chrome, pane_inner_rect, pane_is_scrolled_back},` line at `:107` (inside the second `pub(crate) use self::{...}` group, `:101-111`):

```rust
    panes::{
        apply_pane_chrome, pane_inner_rect, pane_is_scrolled_back, pane_todo_indicator,
        PaneTodoIndicator,
    },
```

The re-export and `pane_todo_indicator` itself both carry a scoped, commented allow until Task 3's hit-test calls them: the renderer takes the `pane_todo_indicator_for` path, so in the bin target the pane-id entry point has no production caller yet. **Task 3 removes both allows** — `#[allow(unused_imports)]` in `src/ui.rs` and `#[allow(dead_code)]` on `pane_todo_indicator` in `src/ui/panes.rs` — as part of landing the hit-test.

**3g. `src/terminal/todo.rs`** — delete the `#[allow(dead_code)]` on `outstanding_todo_count` (`:205`) and `highest_outstanding_todo_priority` (`:210`) together with the three-line comment above them (`:202-204`). Both now have real readers.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run --locked --no-fail-fast ui::panes`
Run: `cargo nextest run --locked --no-fail-fast config::model`
Run: `cargo clippy --all-targets -- -D warnings`
Run: `python3 -m unittest scripts.test_config_reference_check`
Expected: green. Clippy matters here because removing `#[allow(dead_code)]` is only safe once the readers exist, and the unittest wraps `scripts/config_reference_check.py`, which fails the moment a `ui.*` key exists without its reference row.

- [ ] **Step 5: Commit**

```bash
git add src/ui/panes.rs src/ui.rs src/app/state.rs src/app/mod.rs src/config/model.rs src/main.rs src/terminal/todo.rs docs/next/website/src/data/config-reference.json
git commit -m "feat: show a pane todo indicator on the top border"
```

---

### Task 2: Todo panel — mode, TUI-only state, geometry, rendering

**Files:**
- Modify: `src/app/state.rs` (`Mode::PaneTodos`, `PaneTodoPanelButton`, `PaneTodoPanelState`, the `pane_todos` field, lifecycle helpers, `wants_ascii_input`, invariants, mode test list)
- Create: `src/ui/todo_panel.rs` (button rects, link chip, `render_pane_todo_panel`, render tests)
- Modify: `src/ui.rs` (`mod todo_panel;`, re-exports, render match arm)
- Modify: `src/app/input/mouse.rs` (panel geometry on `AppState`)
- Modify: `src/app/input/mod.rs` and `src/app/mod.rs` (inert `Mode::PaneTodos` key arms so the exhaustive matches compile; Task 3 fills them in)

**Interfaces:**
- Consumes: `AppState::pane_terminal` (Task 1), `TerminalState::todos_in_display_order() -> Vec<&PaneTodo>`, `AppState::screen_rect()` (`src/app/input/mouse.rs:1340`), `AppState::pane_info_by_id(PaneId) -> Option<&PaneInfo>` (`src/app/input/mouse.rs:1693`), `render_panel_shell` / `action_button_row_rects` / `action_button_width` / `render_action_button` / `panel_contrast_fg` (`src/ui/widgets.rs`).
- Produces, relied on by Tasks 3–5:
  - `pub enum PaneTodoPanelButton { Toggle, ClearDone, Close }`
  - `pub struct PaneTodoPanelState { pub pane_id: PaneId, pub selected: usize, pub hovered_button: Option<PaneTodoPanelButton> }`
  - `AppState::pane_todos: Option<PaneTodoPanelState>` — the `Option` *is* the open/closed flag
  - `AppState::open_pane_todos(&mut self, pane_id: PaneId)`, `close_pane_todos(&mut self)`, `pane_todos_move_selection(&mut self, delta: isize)`
  - `AppState::pane_todos_in_display_order(&self, pane_id: PaneId) -> Vec<&PaneTodo>`
  - `AppState::selected_pane_todo(&self) -> Option<PaneTodo>`
  - `AppState::pane_todo_link_target(&self, todo: &PaneTodo) -> Option<(usize, PaneId)>` — `None` means the link is dead
  - `AppState::pane_todo_panel_rect / _buttons / _list_window` in `src/app/input/mouse.rs`
  - `pub(crate) struct PaneTodoPanelButtonRects { pub toggle: Option<Rect>, pub clear_done: Rect, pub close: Rect }` with `hit(col, row) -> Option<PaneTodoPanelButton>` and `row_y() -> u16`
  - `pub(crate) fn pane_todo_panel_button_rects(inner: Rect) -> Option<PaneTodoPanelButtonRects>`
  - `pub(crate) fn pane_todo_link_chip(row: Rect, label: &str) -> Option<(Rect, String)>`

- [ ] **Step 1: Write the failing tests**

Create `src/ui/todo_panel.rs` containing *only* the test module for now, so the file compiles as soon as the implementation lands — **and declare it in the same step**, next to the other `mod` lines in `src/ui.rs`:

```rust
mod todo_panel;
```

Without that `mod` line the new file is not part of the crate at all, so Step 2 would report "0 tests run" instead of failing: the same silent-zero-tests trap `tests/cli` sets on macOS. Step 3d only adds the re-exports and the render arm.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, layout::Rect, Terminal};

    use crate::app::state::AppState;
    use crate::terminal::todo::{TodoLink, TodoPriority, TodoUpdate};
    use crate::workspace::Workspace;

    /// A workspace with one pane, the panel open on it, and the frame geometry
    /// the notification center tests use.
    fn app_with_open_panel(todos: &[(&str, bool, TodoPriority)]) -> AppState {
        let mut app = AppState::test_new();
        app.workspaces = vec![Workspace::test_new("todos")];
        app.active = Some(0);
        app.ensure_test_terminals();
        app.view.tab_bar_rect = Rect::new(0, 0, 80, 1);
        app.view.terminal_area = Rect::new(0, 1, 80, 24);

        let pane_id = app.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let terminal = app
            .terminals
            .get_mut(&terminal_id)
            .expect("test terminal should exist");
        for (text, done, priority) in todos {
            let todo = terminal
                .add_todo(text, *priority, None, 100)
                .expect("todo should be added");
            if *done {
                terminal
                    .update_todo(
                        todo.id,
                        TodoUpdate {
                            done: Some(true),
                            ..TodoUpdate::default()
                        },
                        200,
                    )
                    .expect("todo should be updated");
            }
        }

        app.open_pane_todos(pane_id);
        app
    }

    fn draw(app: &AppState) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(80, 25)).unwrap();
        terminal
            .draw(|frame| render_pane_todo_panel(app, frame))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn row_text(buffer: &ratatui::buffer::Buffer, rect: Rect) -> String {
        (rect.x..rect.x + rect.width)
            .map(|x| buffer[(x, rect.y)].symbol())
            .collect()
    }

    #[test]
    fn rows_render_in_presentation_order() {
        let app = app_with_open_panel(&[
            ("normal one", false, TodoPriority::Normal),
            ("high one", false, TodoPriority::High),
            ("finished", true, TodoPriority::High),
        ]);
        let (list, _) = app
            .pane_todo_panel_list_window()
            .expect("panel list window should exist");
        let buffer = draw(&app);

        assert!(row_text(&buffer, Rect::new(list.x, list.y, list.width, 1)).contains("high one"));
        assert!(
            row_text(&buffer, Rect::new(list.x, list.y + 1, list.width, 1)).contains("normal one")
        );
        assert!(
            row_text(&buffer, Rect::new(list.x, list.y + 2, list.width, 1)).contains("finished"),
            "done todos sink to the bottom"
        );
    }

    #[test]
    fn done_rows_are_dimmed_and_struck() {
        // Two todos on purpose: `open_pane_todos` starts with `selected: 0`,
        // and a selected row is painted by the selection branch (accent band),
        // never the done branch. Done todos sink, so row 1 is the done one and
        // row 0 keeps the cursor.
        let app = app_with_open_panel(&[
            ("still open", false, TodoPriority::Normal),
            ("finished", true, TodoPriority::Normal),
        ]);
        let (list, _) = app
            .pane_todo_panel_list_window()
            .expect("panel list window should exist");
        let buffer = draw(&app);

        // Column 3 is the first text cell after the three-cell state block.
        let cell = &buffer[(list.x + 3, list.y + 1)];
        assert_eq!(cell.style().fg, Some(app.palette.overlay0));
        assert!(cell
            .style()
            .add_modifier
            .contains(ratatui::style::Modifier::CROSSED_OUT));
    }

    #[test]
    fn a_dead_link_chip_renders_dimmed_and_a_live_one_does_not() {
        // The live/dead distinction only exists on an unselected row: a
        // selected row's chip takes the accent band like the rest of the row.
        // The decoy is High priority so it sorts to row 0 and holds the
        // starting selection, while `todos()` keeps insertion order, so the
        // linked todo is still `todos()[0]` and lands on row 1.
        let mut app = app_with_open_panel(&[
            ("go look", false, TodoPriority::Normal),
            ("decoy", false, TodoPriority::High),
        ]);
        let pane_id = app.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let todo_id = app.terminals[&terminal_id].todos()[0].id;

        // A live link points at a pane that still exists.
        app.terminals
            .get_mut(&terminal_id)
            .expect("test terminal should exist")
            .update_todo(
                todo_id,
                TodoUpdate {
                    link: Some(Some(TodoLink {
                        pane: Some(pane_id),
                        label: "infra".into(),
                    })),
                    ..TodoUpdate::default()
                },
                300,
            )
            .expect("todo should be updated");
        let (list, _) = app
            .pane_todo_panel_list_window()
            .expect("panel list window should exist");
        // Row 1: the linked todo, which is not the selected row.
        let row = Rect::new(list.x, list.y + 1, list.width, 1);
        let (chip, _) = pane_todo_link_chip(row, "infra").expect("chip should fit");
        let buffer = draw(&app);
        assert!(row_text(&buffer, chip).contains('→'));
        assert_eq!(buffer[(chip.x + 1, chip.y)].style().fg, Some(app.palette.blue));

        // A dead link keeps its label and reads as inert.
        app.terminals
            .get_mut(&terminal_id)
            .expect("test terminal should exist")
            .update_todo(
                todo_id,
                TodoUpdate {
                    link: Some(Some(TodoLink {
                        pane: None,
                        label: "infra".into(),
                    })),
                    ..TodoUpdate::default()
                },
                400,
            )
            .expect("todo should be updated");
        let buffer = draw(&app);
        assert!(row_text(&buffer, chip).contains("infra"));
        assert_eq!(
            buffer[(chip.x + 1, chip.y)].style().fg,
            Some(app.palette.overlay0),
            "a dead link is dimmed"
        );
    }

    #[test]
    fn an_empty_pane_shows_the_empty_state_and_no_footer() {
        let app = app_with_open_panel(&[]);
        let buffer = draw(&app);
        let rect = app.pane_todo_panel_rect().expect("panel rect should exist");

        assert!(row_text(
            &buffer,
            Rect::new(rect.x + 1, rect.y + 1, rect.width - 2, 1)
        )
        .contains("no todos"));
        assert!(app.pane_todo_panel_buttons().is_none());
    }

    #[test]
    fn the_footer_sits_below_the_list_in_the_settings_button_language() {
        let app = app_with_open_panel(&[("only one", false, TodoPriority::Normal)]);
        let (list, _) = app
            .pane_todo_panel_list_window()
            .expect("panel list window should exist");
        let buttons = app
            .pane_todo_panel_buttons()
            .expect("footer buttons should exist");
        let rect = app.pane_todo_panel_rect().expect("panel rect should exist");

        assert_eq!(list.y + list.height, buttons.row_y());
        assert_eq!(buttons.row_y(), rect.y + rect.height - 2);

        // A short todo pins the panel to its 30-cell minimum, and 28 inner
        // cells cannot hold all three boxes (12 + 14 + 11 plus two 2-cell gaps
        // = 41), so `toggle` drops first — the same degradation the
        // notification center applies to its `mark read` box at the same
        // `clamp(30, 60)` minimum.
        assert!(buttons.toggle.is_none());

        let buffer = draw(&app);
        let footer = row_text(&buffer, Rect::new(rect.x, buttons.row_y(), rect.width, 1));
        assert!(footer.contains("c clear done"));
        assert!(footer.contains("esc close"));
    }

    #[test]
    fn a_wide_panel_shows_all_three_footer_buttons() {
        // 40 cells of text push the panel to 46 (40 + borders + glyph block +
        // trailing space), whose 44 inner cells clear the 41 all three boxes
        // need.
        let wide = "x".repeat(40);
        let app = app_with_open_panel(&[(wide.as_str(), false, TodoPriority::Normal)]);
        let rect = app.pane_todo_panel_rect().expect("panel rect should exist");
        let buttons = app
            .pane_todo_panel_buttons()
            .expect("footer buttons should exist");

        assert!(buttons.toggle.is_some());

        let buffer = draw(&app);
        let footer = row_text(&buffer, Rect::new(rect.x, buttons.row_y(), rect.width, 1));
        assert!(footer.contains("spc toggle"));
        assert!(footer.contains("c clear done"));
        assert!(footer.contains("esc close"));
    }

    #[test]
    fn the_panel_hangs_from_the_pane_it_belongs_to() {
        let mut app = app_with_open_panel(&[("only one", false, TodoPriority::Normal)]);
        let pane_id = app.workspaces[0].tabs[0].root_pane;
        app.view.pane_infos = vec![crate::layout::PaneInfo {
            id: pane_id,
            rect: Rect::new(20, 4, 40, 10),
            inner_rect: Rect::new(21, 5, 38, 8),
            scrollbar_rect: None,
            borders: ratatui::widgets::Borders::ALL,
            is_focused: true,
        }];

        let rect = app.pane_todo_panel_rect().expect("panel rect should exist");
        assert_eq!(rect.x + rect.width, 60, "right-aligned with the pane");
        assert_eq!(rect.y, 5, "hangs off the pane's top border");
    }

    #[test]
    fn selection_clamps_to_the_list_and_survives_an_empty_pane() {
        let mut app = app_with_open_panel(&[
            ("first", false, TodoPriority::Normal),
            ("second", false, TodoPriority::Normal),
        ]);

        app.pane_todos_move_selection(5);
        assert_eq!(
            app.pane_todos.as_ref().expect("panel state").selected,
            1,
            "selection stops at the last row"
        );
        app.pane_todos_move_selection(-9);
        assert_eq!(app.pane_todos.as_ref().expect("panel state").selected, 0);

        let empty = app_with_open_panel(&[]);
        assert!(empty.selected_pane_todo().is_none());
    }
}
```

Two existing mode tests enumerate every variant by hand and will **not** compile-error when a variant is added — they just silently stop covering it. Extend both as part of Step 3, not Step 1:
- `honors_key_repeat_allowlists_terminal_and_copy` (`src/app/state.rs:2718`): add `Mode::PaneTodos,` to the array.
- `mode_wants_ascii_input_classification` (`src/app/mod.rs:2191`): add `Mode::PaneTodos,` to the *first* (wants-ASCII) array.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run --locked --no-fail-fast ui::todo_panel`
Expected: compile failure — `render_pane_todo_panel`, `AppState::open_pane_todos`, `AppState::pane_todo_panel_rect` do not exist.

- [ ] **Step 3: Write the implementation**

**3a. `src/app/state.rs`** — add the variant to `pub enum Mode` (`:815`) after `NotificationCenter`:

```rust
    PaneTodos,
```

Add it to the `wants_ascii_input` allowlist (`:851`) — the panel is a command/navigation surface, so it belongs there next to `Mode::NotificationCenter`:

```rust
                | Mode::NotificationCenter
                | Mode::PaneTodos
```

Add `Mode::PaneTodos,` to the array in `honors_key_repeat_allowlists_terminal_and_copy` (`:2718`), and `Mode::PaneTodos,` to the first array of `mode_wants_ascii_input_classification` in `src/app/mod.rs` (`:2191`).

State types, next to `NotificationCenterState` (`:1515`):

```rust
/// Footer buttons of the pane todo panel, in render order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneTodoPanelButton {
    Toggle,
    ClearDone,
    Close,
}

/// TUI-only state for the pane todo panel. The todos themselves are
/// server-owned on `TerminalState`; only the cursor and the hover live here,
/// and neither is persisted or exposed over the API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneTodoPanelState {
    /// Pane whose todos the panel is showing.
    pub pane_id: PaneId,
    /// Index into the pane's presentation-order list.
    pub selected: usize,
    /// Which footer button the pointer is over, if any.
    pub hovered_button: Option<PaneTodoPanelButton>,
}
```

Field next to `notification_center` (`:1683`), and `pane_todos: None,` in `test_new` (`:2239` area) and in `App::new` (`src/app/mod.rs:667` area):

```rust
    /// TUI-only panel state; `Some` while `Mode::PaneTodos`.
    pub pane_todos: Option<PaneTodoPanelState>,
```

Lifecycle helpers, next to the notification center's (`:1894`):

```rust
    /// Open the todo panel for a pane. The selection starts at the top of the
    /// presentation order, which is the most urgent outstanding todo.
    pub(crate) fn open_pane_todos(&mut self, pane_id: PaneId) {
        self.pane_todos = Some(PaneTodoPanelState {
            pane_id,
            selected: 0,
            hovered_button: None,
        });
        self.mode = Mode::PaneTodos;
    }

    /// Closes the panel only. Every caller pairs this with `leave_modal` or an
    /// explicit mode, exactly like `close_notification_center`.
    pub(crate) fn close_pane_todos(&mut self) {
        self.pane_todos = None;
    }

    /// A pane's todos in presentation order. Empty when the pane or its
    /// terminal is gone, which is what lets the panel self-heal instead of
    /// holding a stale target.
    pub(crate) fn pane_todos_in_display_order(
        &self,
        pane_id: PaneId,
    ) -> Vec<&crate::terminal::todo::PaneTodo> {
        self.pane_terminal(pane_id)
            .map(|terminal| terminal.todos_in_display_order())
            .unwrap_or_default()
    }

    /// Move (or, with `0`, re-clamp) the panel selection.
    pub(crate) fn pane_todos_move_selection(&mut self, delta: isize) {
        let Some(pane_id) = self.pane_todos.as_ref().map(|panel| panel.pane_id) else {
            return;
        };
        let len = self.pane_todos_in_display_order(pane_id).len();
        let Some(panel) = self.pane_todos.as_mut() else {
            return;
        };
        if len == 0 {
            panel.selected = 0;
            return;
        }
        panel.selected = panel.selected.saturating_add_signed(delta).min(len - 1);
    }

    /// The selected todo, cloned so callers can mutate through the API without
    /// holding a borrow of the store.
    pub(crate) fn selected_pane_todo(&self) -> Option<crate::terminal::todo::PaneTodo> {
        let panel = self.pane_todos.as_ref()?;
        self.pane_todos_in_display_order(panel.pane_id)
            .get(panel.selected)
            .map(|todo| (*todo).clone())
    }

    /// Resolve a todo's link. `None` is a dead link: either the stored target
    /// was already unresolvable at restore, or the pane has since closed.
    /// One definition, so the dimmed chip and the inert click agree.
    pub(crate) fn pane_todo_link_target(
        &self,
        todo: &crate::terminal::todo::PaneTodo,
    ) -> Option<(usize, PaneId)> {
        let pane_id = todo.link.as_ref()?.pane?;
        let ws_idx = self
            .workspaces
            .iter()
            .position(|workspace| workspace.pane_state(pane_id).is_some())?;
        Some((ws_idx, pane_id))
    }
```

Invariants — in `assert_invariants_for_test` (`:2392`), with the empty-state block near `rename_pane_target` (`:2427`):

```rust
            assert!(
                self.pane_todos.is_none(),
                "empty app state must not keep a pane todo panel"
            );
```

Deliberately **no** live-pane assertion for `pane_todos`: the panel resolves its pane on every read and renders nothing once it is gone, so asserting liveness would encode a stronger contract than the code keeps. `rename_pane_target` is asserted because a save consumes it.

**3b. `src/ui/todo_panel.rs`** — prepend above the test module:

```rust
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::text::{display_width, display_width_u16, truncate_end};
use super::widgets::{
    action_button_row_rects, action_button_width, panel_contrast_fg, render_action_button,
    render_panel_shell, ActionButtonSpec,
};
use crate::app::state::{AppState, PaneTodoPanelButton};
use crate::terminal::todo::{PaneTodo, TodoPriority};

/// Footer buttons in the notification center's language: the shortcut hint
/// inside the filled box, in render order.
const TOGGLE_BUTTON: (&str, &str) = ("spc", "toggle");
const CLEAR_DONE_BUTTON: (&str, &str) = ("c", "clear done");
const CLOSE_BUTTON: (&str, &str) = ("esc", "close");

fn button_spec(button: (&'static str, &'static str)) -> ActionButtonSpec<'static> {
    ActionButtonSpec {
        hint: Some(button.0),
        label: button.1,
    }
}

/// Footer button rects; the mouse layer and the render agree on this geometry.
/// `toggle` is dropped first when the panel is too narrow for all three boxes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PaneTodoPanelButtonRects {
    pub toggle: Option<Rect>,
    pub clear_done: Rect,
    pub close: Rect,
}

impl PaneTodoPanelButtonRects {
    pub(crate) fn hit(&self, col: u16, row: u16) -> Option<PaneTodoPanelButton> {
        let contains = |rect: Rect| col >= rect.x && col < rect.x + rect.width && row == rect.y;
        if self.toggle.is_some_and(contains) {
            return Some(PaneTodoPanelButton::Toggle);
        }
        if contains(self.clear_done) {
            return Some(PaneTodoPanelButton::ClearDone);
        }
        if contains(self.close) {
            return Some(PaneTodoPanelButton::Close);
        }
        None
    }

    pub(crate) fn row_y(&self) -> u16 {
        self.clear_done.y
    }
}

pub(crate) fn pane_todo_panel_button_rects(inner: Rect) -> Option<PaneTodoPanelButtonRects> {
    if inner.width == 0 || inner.height < 2 {
        return None;
    }
    let gap = 2u16;
    let all = [
        button_spec(TOGGLE_BUTTON),
        button_spec(CLEAR_DONE_BUTTON),
        button_spec(CLOSE_BUTTON),
    ];
    let all_width: u16 = all
        .iter()
        .map(|spec| action_button_width(spec.hint, spec.label))
        .sum::<u16>()
        + gap * 2;
    let row_offset = inner.height - 1;
    if all_width <= inner.width {
        let rects = action_button_row_rects(inner, &all, gap, row_offset);
        Some(PaneTodoPanelButtonRects {
            toggle: Some(rects[0]),
            clear_done: rects[1],
            close: rects[2],
        })
    } else {
        let rects = action_button_row_rects(inner, &all[1..], gap, row_offset);
        Some(PaneTodoPanelButtonRects {
            toggle: None,
            clear_done: rects[0],
            close: rects[1],
        })
    }
}

/// The `→ label` chip at a row's right edge, for a todo that carries a link.
/// One definition for the renderer and the mouse hit-test, so clicking the
/// chip and seeing the chip cannot drift apart.
pub(crate) fn pane_todo_link_chip(row: Rect, label: &str) -> Option<(Rect, String)> {
    if label.is_empty() || row.width < 16 {
        return None;
    }
    let budget = (row.width / 3) as usize;
    let text = format!(" → {} ", truncate_end(label, budget.saturating_sub(4)));
    let width = display_width_u16(&text);
    if width == 0 || width >= row.width {
        return None;
    }
    Some((
        Rect::new(row.x + row.width - width, row.y, width, 1),
        text,
    ))
}

/// Three-cell state block, mirroring the notification center's dot column.
fn todo_glyph(todo: &PaneTodo) -> &'static str {
    if todo.done {
        return " ✓ ";
    }
    match todo.priority {
        TodoPriority::High => " ▲ ",
        TodoPriority::Normal => " ● ",
        TodoPriority::Low => " ▼ ",
    }
}

pub(super) fn render_pane_todo_panel(app: &AppState, frame: &mut Frame) {
    let Some(rect) = app.pane_todo_panel_rect() else {
        return;
    };
    let p = &app.palette;
    let Some(inner) = render_panel_shell(frame, rect, p.accent, p.panel_bg) else {
        return;
    };
    let Some(panel) = app.pane_todos.as_ref() else {
        return;
    };
    let todos = app.pane_todos_in_display_order(panel.pane_id);

    if todos.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " no todos",
                Style::default().fg(p.overlay0),
            ))),
            Rect::new(inner.x, inner.y, inner.width, inner.height.min(1)),
        );
        return;
    }

    let Some((list, start)) = app.pane_todo_panel_list_window() else {
        return;
    };

    for (row, todo) in todos
        .iter()
        .skip(start)
        .take(list.height as usize)
        .enumerate()
    {
        let idx = start + row;
        let row_rect = Rect::new(list.x, list.y + row as u16, list.width, 1);
        let is_selected = idx == panel.selected;
        let chip = todo
            .link
            .as_ref()
            .and_then(|link| pane_todo_link_chip(row_rect, &link.label));
        let chip_width = chip.as_ref().map(|(rect, _)| rect.width).unwrap_or(0) as usize;

        let (glyph_style, text_style, row_style) = if is_selected {
            // The band alone marks selection; the glyph keeps signalling
            // priority and done state so a selected row stays legible.
            let base = Style::default().fg(panel_contrast_fg(p)).bg(p.accent);
            (base, base, base)
        } else if todo.done {
            (
                Style::default().fg(p.overlay0),
                Style::default()
                    .fg(p.overlay0)
                    .add_modifier(Modifier::CROSSED_OUT),
                Style::default(),
            )
        } else {
            (
                Style::default().fg(app.pane_todo_indicator_color(Some(todo.priority))),
                Style::default().fg(p.text),
                Style::default(),
            )
        };

        let text_budget = (list.width as usize).saturating_sub(3 + chip_width);
        let text = truncate_end(&todo.text, text_budget);
        let pad = text_budget.saturating_sub(display_width(&text));
        let line = Line::from(vec![
            Span::styled(todo_glyph(todo), glyph_style),
            Span::styled(text, text_style),
            Span::styled(" ".repeat(pad), row_style),
        ]);
        frame.render_widget(Paragraph::new(line).style(row_style), row_rect);

        if let Some((chip_rect, chip_text)) = chip {
            // A dead link keeps its captured label but reads as inert.
            let chip_style = if is_selected {
                Style::default().fg(panel_contrast_fg(p)).bg(p.accent)
            } else if app.pane_todo_link_target(todo).is_some() {
                Style::default().fg(p.blue)
            } else {
                Style::default().fg(p.overlay0).add_modifier(Modifier::DIM)
            };
            frame.render_widget(Paragraph::new(chip_text).style(chip_style), chip_rect);
        }
    }

    if let Some(buttons) = app.pane_todo_panel_buttons() {
        let hovered = panel.hovered_button;
        let style_for = |button: PaneTodoPanelButton| {
            if hovered == Some(button) {
                Style::default()
                    .fg(panel_contrast_fg(p))
                    .bg(p.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(p.text)
                    .bg(p.surface0)
                    .add_modifier(Modifier::BOLD)
            }
        };
        if let Some(toggle) = buttons.toggle {
            render_action_button(
                frame,
                toggle,
                Some(TOGGLE_BUTTON.0),
                TOGGLE_BUTTON.1,
                style_for(PaneTodoPanelButton::Toggle),
            );
        }
        render_action_button(
            frame,
            buttons.clear_done,
            Some(CLEAR_DONE_BUTTON.0),
            CLEAR_DONE_BUTTON.1,
            style_for(PaneTodoPanelButton::ClearDone),
        );
        render_action_button(
            frame,
            buttons.close,
            Some(CLOSE_BUTTON.0),
            CLOSE_BUTTON.1,
            style_for(PaneTodoPanelButton::Close),
        );
    }
}
```

**3c. `src/app/input/mouse.rs`** — a second panel constant next to `NOTIFICATION_PANEL_MAX_ROWS` (`:28`) and the geometry block next to the notification center's (`:1350-1476`):

```rust
const PANE_TODO_PANEL_MAX_ROWS: u16 = 12;
```

```rust
    /// Panel rect, hanging off the pane's top border at its right edge so it
    /// drops out of the indicator. `None` unless the panel is open, so render
    /// and hit-test go quiet together.
    pub(crate) fn pane_todo_panel_rect(&self) -> Option<Rect> {
        let panel = self.pane_todos.as_ref()?;
        // A panel whose pane has gone renders nothing and hit-tests to
        // nothing; the next Esc closes it.
        self.pane_terminal(panel.pane_id)?;
        let screen = self.screen_rect();
        if screen.width == 0 || screen.height == 0 {
            return None;
        }
        let anchor = self
            .pane_info_by_id(panel.pane_id)
            .map(|info| info.rect)
            .unwrap_or(self.view.terminal_area);
        let todos = self.pane_todos_in_display_order(panel.pane_id);
        let content_width = todos
            .iter()
            .take(PANE_TODO_PANEL_MAX_ROWS as usize)
            .map(|todo| {
                let link_width = todo
                    .link
                    .as_ref()
                    .map(|link| crate::ui::text::display_width(&link.label) + 4)
                    .unwrap_or(0);
                crate::ui::text::display_width(&todo.text) + link_width
            })
            .max()
            .unwrap_or(16);
        // borders + state glyph block + trailing space
        let panel_w = ((content_width + 2 + 3 + 1) as u16)
            .clamp(30, 60)
            .min(screen.width.max(1));
        let rows = (todos.len().max(1) as u16).min(PANE_TODO_PANEL_MAX_ROWS);
        let footer = if todos.is_empty() { 0 } else { 1 };
        let panel_h = (rows + 2 + footer).min(screen.height.max(1));
        let right = anchor.x.saturating_add(anchor.width);
        let x = right.saturating_sub(panel_w).max(screen.x);
        let bottom_y = screen.y + screen.height.saturating_sub(panel_h);
        let y = anchor.y.saturating_add(1).min(bottom_y).max(screen.y);
        Some(Rect::new(x, y, panel_w, panel_h))
    }

    /// Full inner rect (panel minus borders), covering list and footer.
    fn pane_todo_panel_inner(&self) -> Option<Rect> {
        let rect = self.pane_todo_panel_rect()?;
        Some(Rect::new(
            rect.x + 1,
            rect.y + 1,
            rect.width.saturating_sub(2),
            rect.height.saturating_sub(2),
        ))
    }

    /// The footer button row, present only when there is something to act on.
    pub(crate) fn pane_todo_panel_buttons(&self) -> Option<crate::ui::PaneTodoPanelButtonRects> {
        let panel = self.pane_todos.as_ref()?;
        if self.pane_todos_in_display_order(panel.pane_id).is_empty() {
            return None;
        }
        crate::ui::pane_todo_panel_button_rects(self.pane_todo_panel_inner()?)
    }

    /// Y of the footer row. Clicks in this row but outside a button are inert
    /// rather than closing, so a near-miss does not dismiss the panel.
    fn pane_todo_panel_footer_row_y(&self) -> Option<u16> {
        self.pane_todo_panel_buttons().map(|buttons| buttons.row_y())
    }

    /// The panel's list rect (footer excluded) and the first visible index.
    /// Shared by render and hit-testing so they agree on which row is where.
    pub(crate) fn pane_todo_panel_list_window(&self) -> Option<(Rect, usize)> {
        let inner = self.pane_todo_panel_inner()?;
        let footer = if self.pane_todo_panel_buttons().is_some() {
            1
        } else {
            0
        };
        let list = Rect::new(
            inner.x,
            inner.y,
            inner.width,
            inner.height.saturating_sub(footer),
        );
        let selected = self.pane_todos.as_ref()?.selected;
        let visible = list.height as usize;
        let start = selected.saturating_sub(visible.saturating_sub(1));
        Some((list, start))
    }
```

**3d. `src/ui.rs`** — the `mod todo_panel;` line already landed in Step 1; here add the re-export for the mouse layer and the render arm in the `match app.mode` (`:523`):

```rust
pub(crate) use self::todo_panel::{pane_todo_link_chip, pane_todo_panel_button_rects,
    PaneTodoPanelButtonRects};
```

```rust
        Mode::PaneTodos => render_pane_todo_panel(app, frame),
```

with `use self::todo_panel::render_pane_todo_panel;` alongside the other private render imports.

**3e. Inert key arms** (Task 3 replaces both):
- `src/app/input/mod.rs:96` inner match: `Mode::PaneTodos => {}`
- `src/app/mod.rs:1966`: `Mode::PaneTodos => {}`

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run --locked --no-fail-fast ui::todo_panel`
Run: `cargo nextest run --locked --no-fail-fast app::state::tests app::mod::tests::mode_wants_ascii`
Run: `cargo clippy --all-targets -- -D warnings`
Expected: green.

- [ ] **Step 5: Commit**

```bash
git add src/ui/todo_panel.rs src/ui.rs src/app/state.rs src/app/mod.rs src/app/input/mouse.rs src/app/input/mod.rs
git commit -m "feat: render the pane todo panel"
```

---

### Task 3: Panel input — keys, mouse, `keys.open_pane_todos`, follow-link

**Files:**
- Modify: `src/app/runtime_mutations.rs` (`runtime_todo_add/update/remove/clear`)
- Modify: `src/app/input/modal.rs` (`PaneTodoAction`, `App::handle_pane_todos_key_via_api`, `App::apply_pane_todo_action`)
- Modify: `src/app/input/mouse.rs` (`pane_todo_indicator_at`, the indicator toggle, the `Mode::PaneTodos` block, `MouseAction::PaneTodo`)
- Modify: `src/app/input/mod.rs` (real key arm, `MouseAction::PaneTodo` dispatch)
- Modify: `src/app/mod.rs` (real headless key arm)
- Modify: `src/app/input/navigate.rs` (`NavigateAction::OpenPaneTodos`, binding table, both executors, `App::open_focused_pane_todos`)
- Modify: `src/config/model.rs`, `src/config/keybinds.rs`, `src/main.rs` (`keys.open_pane_todos` = `prefix+ctrl+t`)
- Modify: `src/ui/keybind_help.rs` (`panes` group entry)

**Interfaces:**
- Consumes: Task 2's panel state and geometry; `App::public_pane_id(usize, PaneId) -> Option<String>` (`src/app/ids.rs:27`); `App::focus_pane_internal_via_api(usize, PaneId)` (`src/app/input/navigate.rs:526`) — the **same** focus path a notification jump uses; `leave_modal(&mut AppState)` (`src/app/input/modal.rs:451`); `rect_contains(Rect, u16, u16)` (`src/app/input/mouse.rs:2109`); `App::dispatch_runtime_mutation` (`src/app/runtime_mutations.rs:12`); `Method::TodoAdd/TodoUpdate/TodoRemove/TodoClear` and their params (`src/api/schema/todos.rs`).
- Produces, relied on by Tasks 4–5:
  - `pub(super) enum PaneTodoAction { Edit, ToggleDone, Remove, ClearDone, FollowLink }`
  - `App::apply_pane_todo_action(&mut self, action: PaneTodoAction)` — one funnel for keys and clicks
  - `App::handle_pane_todos_key_via_api(&mut self, key: KeyEvent)`
  - `MouseAction::PaneTodo(PaneTodoAction)`
  - `App::runtime_todo_add/update/remove/clear(&mut self, id: &'static str, params) -> String`
  - `NavigateAction::OpenPaneTodos`, `keys.open_pane_todos`

- [ ] **Step 1: Write the failing tests**

Add to `src/app/input/modal.rs`'s `#[cfg(test)] mod tests`. These drive `App`, not bare `AppState`: unlike the notification center (whose jump has a pure-state twin), every todo mutation only exists behind the API, so `App` is the honest level and no `#[cfg(test)]` twin is worth writing.

```rust
    /// App with one workspace, one pane, and todos on it. Builds on the
    /// module's existing `app_with_test_workspaces` (`src/app/input/modal.rs:1665`),
    /// which already wires `App::new`, `ensure_test_terminals`, and `active`.
    fn app_with_pane_todos(todos: &[(&str, bool, crate::terminal::todo::TodoPriority)]) -> App {
        let mut app = app_with_test_workspaces(&["todos"]);
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.state.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let terminal = app
            .state
            .terminals
            .get_mut(&terminal_id)
            .expect("test terminal should exist");
        for (text, done, priority) in todos {
            let todo = terminal
                .add_todo(text, *priority, None, 100)
                .expect("todo should be added");
            if *done {
                terminal
                    .update_todo(
                        todo.id,
                        crate::terminal::todo::TodoUpdate {
                            done: Some(true),
                            ..Default::default()
                        },
                        200,
                    )
                    .expect("todo should be updated");
            }
        }
        app.state.open_pane_todos(pane_id);
        app
    }

    fn panel_todo_texts(app: &App) -> Vec<String> {
        let pane_id = app
            .state
            .pane_todos
            .as_ref()
            .expect("panel should be open")
            .pane_id;
        app.state
            .pane_todos_in_display_order(pane_id)
            .into_iter()
            .map(|todo| todo.text.clone())
            .collect()
    }

    #[test]
    fn pane_todo_panel_selection_moves_with_arrows_and_j_k() {
        let mut app = app_with_pane_todos(&[
            ("first", false, crate::terminal::todo::TodoPriority::High),
            ("second", false, crate::terminal::todo::TodoPriority::Normal),
        ]);

        app.handle_pane_todos_key_via_api(key(KeyCode::Down));
        assert_eq!(app.state.pane_todos.as_ref().expect("panel").selected, 1);
        app.handle_pane_todos_key_via_api(key(KeyCode::Char('k')));
        assert_eq!(app.state.pane_todos.as_ref().expect("panel").selected, 0);
        app.handle_pane_todos_key_via_api(key(KeyCode::Char('j')));
        assert_eq!(app.state.pane_todos.as_ref().expect("panel").selected, 1);
    }

    #[test]
    fn space_toggles_the_selected_todo_through_the_api() {
        let mut app = app_with_pane_todos(&[(
            "toggle me",
            false,
            crate::terminal::todo::TodoPriority::Normal,
        )]);
        app.state.session_dirty = false;

        app.handle_pane_todos_key_via_api(key(KeyCode::Char(' ')));

        let todo = app
            .state
            .selected_pane_todo()
            .expect("a todo should still be selected");
        assert!(todo.done, "space marks the selected todo done");
        assert!(
            app.state.session_dirty,
            "the mutation went through the API, so the session is dirty"
        );

        app.handle_pane_todos_key_via_api(key(KeyCode::Char(' ')));
        assert!(
            !app.state
                .selected_pane_todo()
                .expect("a todo should still be selected")
                .done,
            "space toggles back"
        );
    }

    #[test]
    fn d_removes_and_c_clears_only_done_todos() {
        let mut app = app_with_pane_todos(&[
            ("keep me", false, crate::terminal::todo::TodoPriority::Normal),
            ("finished", true, crate::terminal::todo::TodoPriority::Normal),
        ]);

        app.handle_pane_todos_key_via_api(key(KeyCode::Char('c')));
        assert_eq!(panel_todo_texts(&app), vec!["keep me".to_string()]);

        app.handle_pane_todos_key_via_api(key(KeyCode::Char('d')));
        assert!(panel_todo_texts(&app).is_empty());
        assert_eq!(
            app.state.pane_todos.as_ref().expect("panel").selected,
            0,
            "the selection re-clamps once the list shrinks"
        );
    }

    /// Spec: "Following a link → focus moves to the linked pane" via the same
    /// focus path a notification jump uses.
    #[test]
    fn g_follows_a_live_link_and_closes_the_panel() {
        let mut app = app_with_pane_todos(&[(
            "look over there",
            false,
            crate::terminal::todo::TodoPriority::Normal,
        )]);
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.state.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let todo_id = app.state.terminals[&terminal_id].todos()[0].id;
        app.state
            .terminals
            .get_mut(&terminal_id)
            .expect("test terminal should exist")
            .update_todo(
                todo_id,
                crate::terminal::todo::TodoUpdate {
                    link: Some(Some(crate::terminal::todo::TodoLink {
                        pane: Some(pane_id),
                        label: "infra".into(),
                    })),
                    ..Default::default()
                },
                300,
            )
            .expect("todo should be updated");

        app.handle_pane_todos_key_via_api(key(KeyCode::Char('g')));

        assert!(app.state.pane_todos.is_none(), "the panel closes on a jump");
        assert_eq!(app.state.mode, Mode::Terminal);
    }

    #[test]
    fn g_on_a_dead_link_is_inert() {
        let mut app = app_with_pane_todos(&[(
            "gone",
            false,
            crate::terminal::todo::TodoPriority::Normal,
        )]);
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.state.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let todo_id = app.state.terminals[&terminal_id].todos()[0].id;
        app.state
            .terminals
            .get_mut(&terminal_id)
            .expect("test terminal should exist")
            .update_todo(
                todo_id,
                crate::terminal::todo::TodoUpdate {
                    link: Some(Some(crate::terminal::todo::TodoLink {
                        pane: None,
                        label: "infra".into(),
                    })),
                    ..Default::default()
                },
                300,
            )
            .expect("todo should be updated");

        app.handle_pane_todos_key_via_api(key(KeyCode::Char('g')));

        assert!(
            app.state.pane_todos.is_some(),
            "a dead link changes nothing at all"
        );
        assert_eq!(app.state.mode, Mode::PaneTodos);
    }

    #[test]
    fn esc_and_q_close_the_panel() {
        for code in [KeyCode::Esc, KeyCode::Char('q')] {
            let mut app = app_with_pane_todos(&[(
                "still here",
                false,
                crate::terminal::todo::TodoPriority::Normal,
            )]);
            app.handle_pane_todos_key_via_api(key(code));
            assert!(app.state.pane_todos.is_none());
            assert_ne!(app.state.mode, Mode::PaneTodos);
        }
    }
```

Add to `src/app/input/mouse.rs`'s `#[cfg(test)] mod tests` (uses `app_for_mouse_test()` from `src/app/input/mod.rs:753` and the local `mouse(...)` helper). Call `app.state.handle_mouse(&mut app.terminal_runtimes, ...)`, **not** `App::handle_mouse` (`src/app/input/mod.rs:269`): only the `AppState` method returns the `Option<MouseAction>` these tests assert on — the `App` wrapper consumes it. Borrowing `app.state` and `app.terminal_runtimes` mutably at once is fine, they are disjoint fields.

```rust
    /// Give the mouse-test app a bordered pane with one outstanding todo.
    fn app_for_pane_todo_indicator() -> App {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![crate::workspace::Workspace::test_new("todos")];
        app.state.active = Some(0);
        app.state.ensure_test_terminals();
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.state.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&terminal_id)
            .expect("test terminal should exist")
            .add_todo(
                "ship it",
                crate::terminal::todo::TodoPriority::High,
                None,
                100,
            )
            .expect("todo should be added");
        app.state.view.pane_infos = vec![PaneInfo {
            id: pane_id,
            rect: Rect::new(26, 0, 40, 10),
            inner_rect: Rect::new(27, 1, 38, 8),
            scrollbar_rect: None,
            borders: ratatui::widgets::Borders::ALL,
            is_focused: true,
        }];
        app
    }

    /// Spec: "the cells that respond to a click are exactly the cells drawn".
    #[test]
    fn clicking_the_pane_todo_indicator_toggles_the_panel() {
        let mut app = app_for_pane_todo_indicator();
        let indicator = crate::ui::pane_todo_indicator(&app.state, &app.state.view.pane_infos[0])
            .expect("indicator should exist");

        for col in indicator.rect.x..indicator.rect.x + indicator.rect.width {
            app.state.handle_mouse(
                &mut app.terminal_runtimes,
                mouse(
                    crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
                    col,
                    indicator.rect.y,
                ),
            );
            assert_eq!(
                app.state.mode,
                Mode::PaneTodos,
                "column {col} of the indicator must open the panel"
            );
            app.state.handle_mouse(
                &mut app.terminal_runtimes,
                mouse(
                    crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
                    col,
                    indicator.rect.y,
                ),
            );
            assert!(
                app.state.pane_todos.is_none(),
                "a second click on the indicator closes it"
            );
        }
    }

    #[test]
    fn a_border_click_beside_the_indicator_still_starts_a_split_drag_or_focus() {
        let mut app = app_for_pane_todo_indicator();
        let indicator = crate::ui::pane_todo_indicator(&app.state, &app.state.view.pane_infos[0])
            .expect("indicator should exist");

        app.state.handle_mouse(
            &mut app.terminal_runtimes,
            mouse(
                crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
                indicator.rect.x - 1,
                indicator.rect.y,
            ),
        );

        assert!(
            app.state.pane_todos.is_none(),
            "only the drawn cells open the panel"
        );
    }

    #[test]
    fn clicking_a_panel_row_opens_the_edit_view_and_the_chip_follows_the_link() {
        let mut app = app_for_pane_todo_indicator();
        let pane_id = app.state.view.pane_infos[0].id;
        app.state.open_pane_todos(pane_id);
        let (list, _) = app
            .state
            .pane_todo_panel_list_window()
            .expect("panel list window should exist");

        let action = app.state.handle_mouse(
            &mut app.terminal_runtimes,
            mouse(
                crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
                list.x + 4,
                list.y,
            ),
        );

        assert!(
            matches!(action, Some(MouseAction::PaneTodo(PaneTodoAction::Edit))),
            "a row click edits, mirroring Enter"
        );
    }

    #[test]
    fn a_near_miss_on_the_panel_footer_row_is_inert() {
        let mut app = app_for_pane_todo_indicator();
        let pane_id = app.state.view.pane_infos[0].id;
        app.state.open_pane_todos(pane_id);
        let buttons = app
            .state
            .pane_todo_panel_buttons()
            .expect("footer buttons should exist");

        // Aim at the gap between two boxes rather than at a column guessed
        // from the panel's edge: `centered_button_row` centres the row, so
        // `panel.x + 1` lands *on* the leftmost box at this panel width and
        // the test would pass without ever reaching the inert path.
        let gap_col = buttons.clear_done.x + buttons.clear_done.width;
        assert_eq!(
            buttons.hit(gap_col, buttons.row_y()),
            None,
            "the near-miss column must really miss every button"
        );

        let action = app.state.handle_mouse(
            &mut app.terminal_runtimes,
            mouse(
                crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
                gap_col,
                buttons.row_y(),
            ),
        );

        assert!(action.is_none(), "a near-miss triggers no action at all");
        assert!(
            app.state.pane_todos.is_some(),
            "missing a footer button must not dismiss the panel"
        );
    }
```

Add to `src/config/keybinds.rs`'s tests and `src/ui.rs`'s tests:

```rust
    // src/config/keybinds.rs
    #[test]
    fn open_pane_todos_defaults_to_prefix_ctrl_t() {
        let kb = Config::default().keybinds();
        assert_eq!(
            binding_triggers(&kb.open_pane_todos),
            // `KeyCombo` is the bare `(KeyCode, KeyModifiers)` tuple, not a
            // `TerminalKey` — see `new_worktree_defaults_to_prefix_shift_g`.
            vec![BindingTrigger::Prefix((
                KeyCode::Char('t'),
                KeyModifiers::CONTROL
            ))]
        );
        assert!(
            Config::default().collect_diagnostics().is_empty(),
            "prefix+ctrl+t must not collide with an existing default"
        );
    }
```

```rust
    // src/ui.rs
    #[test]
    fn keybind_help_lists_the_pane_todo_panel_action() {
        let app = crate::app::state::AppState::test_new();
        let groups = keybind_help_groups(&app);
        let panes = groups
            .iter()
            .find(|(name, _)| *name == "panes")
            .expect("panes group")
            .1
            .clone();

        assert!(panes
            .iter()
            .any(|(key, label)| key == "prefix+ctrl+t" && label.as_ref() == "pane todos"));
    }
```

And the action-resolution test in `src/app/input/navigate.rs`'s tests, modelled on `default_pane_move_bindings_map_to_their_actions_without_collisions` (`:2363`):

```rust
    #[test]
    fn open_pane_todos_maps_to_its_action_in_prefix_mode() {
        let mut state = state_with_workspaces(&["test"]);
        state.keybinds = crate::config::Config::default().keybinds();

        assert_eq!(
            action_for_key(
                &state,
                TerminalKey::new(KeyCode::Char('t'), KeyModifiers::CONTROL),
                BindingDispatch::Prefix,
            ),
            Some(NavigateAction::OpenPaneTodos)
        );
    }

    #[test]
    fn open_pane_todos_opens_the_panel_on_the_focused_pane() {
        let mut app = app_with_test_workspaces(&["main"]);
        let pane_id = app.state.workspaces[0]
            .focused_pane_id()
            .expect("a focused pane");

        app.execute_tui_navigate_action(NavigateAction::OpenPaneTodos, ActionContext::Prefix);

        assert_eq!(app.state.mode, Mode::PaneTodos);
        assert_eq!(
            app.state.pane_todos.as_ref().expect("panel").pane_id,
            pane_id
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run --locked --no-fail-fast pane_todo`
Run: `cargo nextest run --locked --no-fail-fast open_pane_todos`
Expected: compile failures — `handle_pane_todos_key_via_api`, `MouseAction::PaneTodo`, `NavigateAction::OpenPaneTodos`, `kb.open_pane_todos` do not exist.

- [ ] **Step 3: Write the implementation**

**3a. `src/app/runtime_mutations.rs`** — extend the `crate::api::schema` import with `TodoAddParams, TodoClearParams, TodoRemoveParams, TodoUpdateParams` and append next to `runtime_pane_clear_scrollback`:

```rust
    pub(crate) fn runtime_todo_add(&mut self, id: &'static str, params: TodoAddParams) -> String {
        self.dispatch_runtime_mutation(id, Method::TodoAdd(params))
    }

    pub(crate) fn runtime_todo_update(
        &mut self,
        id: &'static str,
        params: TodoUpdateParams,
    ) -> String {
        self.dispatch_runtime_mutation(id, Method::TodoUpdate(params))
    }

    pub(crate) fn runtime_todo_remove(
        &mut self,
        id: &'static str,
        params: TodoRemoveParams,
    ) -> String {
        self.dispatch_runtime_mutation(id, Method::TodoRemove(params))
    }

    pub(crate) fn runtime_todo_clear(
        &mut self,
        id: &'static str,
        params: TodoClearParams,
    ) -> String {
        self.dispatch_runtime_mutation(id, Method::TodoClear(params))
    }
```

**3b. `src/app/input/modal.rs`** — the action enum next to `ModalAction` (`:17`):

```rust
/// What the todo panel is being asked to do. Keys and clicks both funnel
/// through `App::apply_pane_todo_action`, so a shortcut and its button can
/// never diverge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PaneTodoAction {
    Edit,
    ToggleDone,
    Remove,
    ClearDone,
    FollowLink,
}
```

and, in the `impl App` block next to `handle_notification_center_key_via_api` (`:1037`):

```rust
    pub(crate) fn handle_pane_todos_key_via_api(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.state.pane_todos_move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.state.pane_todos_move_selection(1),
            // Enter edits rather than jumps: todos are authored, notifications
            // are not. Jumping is on the link chip and `g`.
            KeyCode::Enter => self.apply_pane_todo_action(PaneTodoAction::Edit),
            KeyCode::Char(' ') => self.apply_pane_todo_action(PaneTodoAction::ToggleDone),
            KeyCode::Char('g') => self.apply_pane_todo_action(PaneTodoAction::FollowLink),
            KeyCode::Char('d') => self.apply_pane_todo_action(PaneTodoAction::Remove),
            KeyCode::Char('c') => self.apply_pane_todo_action(PaneTodoAction::ClearDone),
            KeyCode::Esc | KeyCode::Char('q') => {
                self.state.close_pane_todos();
                leave_modal(&mut self.state);
            }
            _ => {}
        }
    }

    /// Apply a panel action to the selected todo. Every mutation goes back
    /// through the `todo.*` API, so the panel, the CLI, and subscribers all
    /// move the same state and `todo.changed` is emitted for free.
    pub(super) fn apply_pane_todo_action(&mut self, action: PaneTodoAction) {
        let Some(pane_id) = self.state.pane_todos.as_ref().map(|panel| panel.pane_id) else {
            return;
        };
        let Some(ws_idx) = self.state.active else {
            return;
        };
        let Some(public_pane_id) = self.public_pane_id(ws_idx, pane_id) else {
            return;
        };

        if action == PaneTodoAction::ClearDone {
            self.runtime_todo_clear(
                "tui.todo.clear",
                crate::api::schema::TodoClearParams {
                    pane_id: public_pane_id,
                    done_only: true,
                },
            );
            // The list just shrank under the cursor.
            self.state.pane_todos_move_selection(0);
            return;
        }

        let Some(todo) = self.state.selected_pane_todo() else {
            return;
        };
        match action {
            PaneTodoAction::Edit => self.state.open_pane_todo_edit(pane_id, todo.id),
            PaneTodoAction::ToggleDone => {
                self.runtime_todo_update(
                    "tui.todo.update",
                    crate::api::schema::TodoUpdateParams {
                        pane_id: public_pane_id,
                        id: todo.id,
                        done: Some(!todo.done),
                        ..Default::default()
                    },
                );
                // Toggling re-sorts the list (done sinks), so re-clamp.
                self.state.pane_todos_move_selection(0);
            }
            PaneTodoAction::Remove => {
                self.runtime_todo_remove(
                    "tui.todo.remove",
                    crate::api::schema::TodoRemoveParams {
                        pane_id: public_pane_id,
                        id: todo.id,
                    },
                );
                self.state.pane_todos_move_selection(0);
            }
            PaneTodoAction::FollowLink => {
                // A dead link is inert: it keeps its label and never resolves
                // to some other pane.
                let Some((target_ws_idx, target_pane_id)) =
                    self.state.pane_todo_link_target(&todo)
                else {
                    return;
                };
                self.state.close_pane_todos();
                self.focus_pane_internal_via_api(target_ws_idx, target_pane_id);
                self.state.mode = Mode::Terminal;
            }
            PaneTodoAction::ClearDone => {}
        }
    }
```

`open_pane_todo_edit` lands in Task 4; until then stub the `Edit` arm as `PaneTodoAction::Edit => {}` **with a `// Task 4` comment** so this task compiles and commits on its own.

**3c. `src/app/input/mouse.rs`** — the indicator hit-test next to `pane_info_by_id` (`:1693`):

```rust
    /// The pane whose todo indicator covers this cell. Reads the very rect the
    /// renderer draws into, so the click target cannot drift from the glyph.
    fn pane_todo_indicator_at(&self, col: u16, row: u16) -> Option<crate::layout::PaneId> {
        self.view.pane_infos.iter().find_map(|info| {
            let indicator = crate::ui::pane_todo_indicator(self, info)?;
            rect_contains(indicator.rect, col, row).then_some(info.id)
        })
    }
```

The toggle block goes **immediately after** the notification indicator block (`:194`) and **before** the `Mode::NotificationCenter` block. Placement is the whole trick: the indicator sits on a pane's top border, and for every pane below the top of the layout `find_border_at` (`:1631`) treats that row as a split-drag hitbox. Checking here returns before the `Down(Left)` arm ever reaches line 587, so the drag never starts.

```rust
        let pane_todo_indicator_hit = matches!(
            self.mode,
            Mode::Terminal | Mode::Navigate | Mode::Resize | Mode::PaneTodos
        )
        .then(|| self.pane_todo_indicator_at(mouse.column, mouse.row))
        .flatten();
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            if let Some(pane_id) = pane_todo_indicator_hit {
                if self.mode == Mode::PaneTodos {
                    self.close_pane_todos();
                    leave_modal(self);
                } else {
                    self.open_pane_todos(pane_id);
                }
                return None;
            }
        }
```

The panel block goes after the `Mode::NotificationCenter` block (`:245`):

```rust
        if self.mode == Mode::PaneTodos {
            match mouse.kind {
                MouseEventKind::Moved => {
                    let over_button = self
                        .pane_todo_panel_buttons()
                        .and_then(|buttons| buttons.hit(mouse.column, mouse.row));
                    if let Some(idx) = self.pane_todo_panel_row_at(mouse.column, mouse.row) {
                        if let Some(panel) = self.pane_todos.as_mut() {
                            panel.selected = idx;
                        }
                    }
                    if let Some(panel) = self.pane_todos.as_mut() {
                        panel.hovered_button = over_button;
                    }
                }
                MouseEventKind::ScrollUp => self.pane_todos_move_selection(-1),
                MouseEventKind::ScrollDown => self.pane_todos_move_selection(1),
                MouseEventKind::Down(MouseButton::Left) => {
                    if let Some(idx) = self.pane_todo_panel_row_at(mouse.column, mouse.row) {
                        let on_chip = self.pane_todo_link_chip_at(mouse.column, mouse.row);
                        if let Some(panel) = self.pane_todos.as_mut() {
                            panel.selected = idx;
                        }
                        return Some(MouseAction::PaneTodo(if on_chip {
                            PaneTodoAction::FollowLink
                        } else {
                            PaneTodoAction::Edit
                        }));
                    }
                    match self
                        .pane_todo_panel_buttons()
                        .and_then(|buttons| buttons.hit(mouse.column, mouse.row))
                    {
                        Some(PaneTodoPanelButton::Toggle) => {
                            return Some(MouseAction::PaneTodo(PaneTodoAction::ToggleDone));
                        }
                        Some(PaneTodoPanelButton::ClearDone) => {
                            return Some(MouseAction::PaneTodo(PaneTodoAction::ClearDone));
                        }
                        Some(PaneTodoPanelButton::Close) => {
                            self.close_pane_todos();
                            leave_modal(self);
                            return None;
                        }
                        None => {}
                    }
                    // A near-miss elsewhere on the buttons' row is inert.
                    if self.pane_todo_panel_footer_row_y() == Some(mouse.row) {
                        return None;
                    }
                    self.close_pane_todos();
                    leave_modal(self);
                }
                _ => {}
            }
            return None;
        }
```

with the two remaining hit-tests next to the other panel geometry:

```rust
    fn pane_todo_panel_row_at(&self, col: u16, row: u16) -> Option<usize> {
        let panel = self.pane_todos.as_ref()?;
        let (list, start) = self.pane_todo_panel_list_window()?;
        let len = self.pane_todos_in_display_order(panel.pane_id).len();
        if len == 0 || !rect_contains(list, col, row) {
            return None;
        }
        let idx = start + (row - list.y) as usize;
        (idx < len).then_some(idx)
    }

    /// Whether the cell is on a row's link chip. Uses the same chip geometry
    /// the renderer draws, so what looks clickable is clickable.
    fn pane_todo_link_chip_at(&self, col: u16, row: u16) -> bool {
        let Some(idx) = self.pane_todo_panel_row_at(col, row) else {
            return false;
        };
        let Some(panel) = self.pane_todos.as_ref() else {
            return false;
        };
        let Some((list, _)) = self.pane_todo_panel_list_window() else {
            return false;
        };
        let todos = self.pane_todos_in_display_order(panel.pane_id);
        let Some(todo) = todos.get(idx) else {
            return false;
        };
        let Some(link) = todo.link.as_ref() else {
            return false;
        };
        crate::ui::pane_todo_link_chip(Rect::new(list.x, row, list.width, 1), &link.label)
            .is_some_and(|(chip, _)| rect_contains(chip, col, row))
    }
```

Extend the `MouseAction` enum (`:30`) with `PaneTodo(PaneTodoAction)`, and the `use super::modal::{...}` import (`:19`) with `PaneTodoAction`; add `PaneTodoPanelButton` to the `crate::app::state::{...}` import (`:7`).

**3d. `src/app/input/mod.rs`** — replace the inert arm with `Mode::PaneTodos => self.handle_pane_todos_key_via_api(key_event),` and add the dispatch arm next to `MouseAction::ClearNotifications` (`:369`):

```rust
                    MouseAction::PaneTodo(action) => self.apply_pane_todo_action(action),
```

**3e. `src/app/mod.rs`** — replace the inert headless arm with:

```rust
            Mode::PaneTodos => {
                self.handle_pane_todos_key_via_api(key_event);
            }
```

**3f. `src/app/input/navigate.rs`** — variant in `pub(crate) enum NavigateAction` (`:1690` area): `OpenPaneTodos,`; row in `non_indexed_action_for_key`'s table (the fn is at `:1778`; put it next to the `open_notification_center` row at `:1841`): `(&kb.open_pane_todos, NavigateAction::OpenPaneTodos),`; the `App` arm next to `OpenNotificationCenter` (`:438`):

```rust
            NavigateAction::OpenPaneTodos => self.open_focused_pane_todos(),
```

the pure-state twin arm (`:2168` area):

```rust
        NavigateAction::OpenPaneTodos => {
            if let Some(pane_id) = state
                .active
                .and_then(|ws_idx| state.workspaces.get(ws_idx))
                .and_then(crate::workspace::Workspace::focused_pane_id)
            {
                state.open_pane_todos(pane_id);
            }
        }
```

and the helper next to `focused_pane_target` (`:749`):

```rust
    pub(crate) fn open_focused_pane_todos(&mut self) {
        let Some((_, pane_id)) = self.focused_pane_target() else {
            return;
        };
        self.state.open_pane_todos(pane_id);
    }
```

Neither arm calls `leave_navigate_mode`: `finish_action_context` (`:2215`) only leaves command mode when the mode did not change, and setting `Mode::PaneTodos` is exactly that change.

**3g. Keybinding registration** — all six sites, in order:
1. `src/config/model.rs:558` area, in `KeysConfig`: `/// Open the focused pane's todo panel. Default: "prefix+ctrl+t".` + `pub open_pane_todos: BindingConfig,`
2. `src/config/model.rs:701` area, in `KeysConfigOverlay`: `#[serde(skip_serializing_if = "Option::is_none")] open_pane_todos: Option<BindingConfig>,`
3. `src/config/model.rs:815` area: `apply_field!(open_pane_todos);` — without this a user's TOML is silently ignored
4. `src/config/model.rs:924` area: `copy_effective_action_field!(open_pane_todos, keybinds.open_pane_todos);` — without this the binding never reaches a `herdr --remote` client
5. `src/config/model.rs:1299` area, in `impl Default for KeysConfig`: `open_pane_todos: BindingConfig::one("prefix+ctrl+t"),`
6. `src/config/keybinds.rs`: `pub open_pane_todos: ActionKeybinds,` (`:359` area), `open_pane_todos: empty_action!(),` (`:532` area), `apply_action!(keybinds.open_pane_todos, open_pane_todos, source);` (`:688` area)

`prefix+ctrl+t` is free: `grep -rn "ctrl+t" src` is empty, and the only ctrl-modified prefix chords in use are `ctrl+n`, `ctrl+u`, `ctrl+k`.

`src/main.rs` `DEFAULT_CONFIG`, after `# open_notification_center = "prefix+ctrl+n"` (`:178`) — enforced by `default_config_documents_every_binding_action` (`:1017`):

```text
# open_pane_todos = "prefix+ctrl+t"
```

**3h. `src/ui/keybind_help.rs`** — in the `panes` group vec, after `clear scrollback` (`:166`):

```rust
        help_entry(keybind_label(&kb.open_pane_todos), "pane todos"),
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run --locked --no-fail-fast pane_todo`
Run: `cargo nextest run --locked --no-fail-fast keybind`
Run: `cargo nextest run --locked --no-fail-fast config::`
Run: `cargo clippy --all-targets -- -D warnings`
Expected: green.

- [ ] **Step 5: Commit**

```bash
git add src/app src/config src/main.rs src/ui/keybind_help.rs
git commit -m "feat: drive the pane todo panel from the keyboard and mouse"
```

---

### Task 4: Edit modal and `keys.add_pane_todo`

**Files:**
- Modify: `src/app/state.rs` (`Mode::PaneTodoEdit`, `PaneTodoEditLink`, `PaneTodoEditState`, the field, lifecycle + cycling helpers, `pane_link_candidates`, invariants, mode test lists)
- Modify: `src/ui/dialogs.rs` (`PaneTodoEditRects`, `pane_todo_edit_rects`, `render_pane_todo_edit_overlay`)
- Modify: `src/ui.rs` (re-export + render arm)
- Modify: `src/app/input/modal.rs` (key handling, save, the `PaneTodoAction::Edit` arm, `delete_last_word` extraction)
- Modify: `src/app/input/overlays.rs` (`pane_todo_edit_inner`)
- Modify: `src/app/input/mouse.rs` (`Mode::PaneTodoEdit` block, `MouseAction::PaneTodoEditModal`)
- Modify: `src/app/input/mod.rs` (key arm, both paste hooks, mouse dispatch)
- Modify: `src/app/mod.rs` (headless key arm)
- Modify: `src/app/input/navigate.rs` (`NavigateAction::AddPaneTodo`)
- Modify: `src/config/model.rs`, `src/config/keybinds.rs`, `src/main.rs` (`keys.add_pane_todo`, unbound)
- Modify: `src/ui/keybind_help.rs` (`add pane todo` entry)

**Interfaces:**
- Consumes: `runtime_todo_add` / `runtime_todo_update` (Task 3); `TerminalState::border_label(bool)` (`src/terminal/state.rs:1996`); `Workspace::public_pane_number(PaneId) -> Option<usize>` (`src/workspace.rs:1049`); `TileLayout::pane_ids(&self) -> Vec<PaneId>` (`src/layout.rs:310`) — reached as `tab.layout.pane_ids()`, because `Tab::layout` is `pub layout: TileLayout` (`src/workspace/tab.rs:43`); there is no type named `Layout` in the crate; `render_modal_shell` / `render_modal_header` / `render_action_button` / `action_button_row_rects` (`src/ui/widgets.rs`); `AppState::onboarding_modal_inner(u16, u16)` (`src/app/input/overlays.rs:356`); `MAX_TODO_TEXT_LEN` (`src/terminal/todo.rs:16`).
- Produces, relied on by Task 5:
  - `pub enum PaneTodoEditLink { Keep, Clear, Set(PaneId) }`
  - `pub struct PaneTodoEditState { pub pane_id: PaneId, pub todo_id: Option<u64>, pub text: String, pub priority: TodoPriority, pub link: PaneTodoEditLink }`
  - `AppState::pane_todo_edit: Option<PaneTodoEditState>`
  - `AppState::open_pane_todo_edit(&mut self, pane_id: PaneId, todo_id: u64)`, `open_new_pane_todo(&mut self, pane_id: PaneId)`
  - `AppState::cycle_pane_todo_edit_priority(&mut self)`, `cycle_pane_todo_edit_link(&mut self)`, `pane_todo_edit_link_label(&self) -> String`
  - `AppState::pane_link_candidates(&self, pane_id: PaneId) -> Vec<(PaneId, String)>`
  - `pub(crate) struct PaneTodoEditRects { pub input: Rect, pub priority: Rect, pub link: Rect, pub save: Rect, pub cancel: Rect }`
  - `pub(crate) fn pane_todo_edit_rects(inner: Rect) -> Option<PaneTodoEditRects>`
  - `NavigateAction::AddPaneTodo`, `keys.add_pane_todo` (unbound by default)

- [ ] **Step 1: Write the failing tests**

Add to `src/app/input/modal.rs`'s tests (reusing `app_with_pane_todos` from Task 3):

```rust
    /// Spec: "its text changed and the change saved → the todo's text and
    /// updated timestamp change while its id, done state, and creation
    /// timestamp are preserved".
    #[test]
    fn saving_an_edit_changes_text_and_keeps_id_done_and_created_at() {
        let mut app = app_with_pane_todos(&[(
            "draft",
            true,
            crate::terminal::todo::TodoPriority::Normal,
        )]);
        let before = app.state.selected_pane_todo().expect("a selected todo");

        app.handle_pane_todos_key_via_api(key(KeyCode::Enter));
        assert_eq!(app.state.mode, Mode::PaneTodoEdit);
        assert_eq!(
            app.state
                .pane_todo_edit
                .as_ref()
                .expect("edit state")
                .text,
            "draft",
            "the modal opens prefilled"
        );

        for _ in 0..5 {
            app.handle_pane_todo_edit_key_via_api(key(KeyCode::Backspace));
        }
        for ch in "final".chars() {
            app.handle_pane_todo_edit_key_via_api(key(KeyCode::Char(ch)));
        }
        app.handle_pane_todo_edit_key_via_api(key(KeyCode::Enter));

        let after = app.state.selected_pane_todo().expect("a selected todo");
        assert_eq!(after.text, "final");
        assert_eq!(after.id, before.id, "id is preserved");
        assert!(after.done, "done state is untouched by a text edit");
        assert_eq!(
            after.created_at_unix, before.created_at_unix,
            "created_at is preserved"
        );
        assert!(after.updated_at_unix >= before.updated_at_unix);
        assert!(app.state.pane_todo_edit.is_none());
        assert_eq!(
            app.state.mode,
            Mode::PaneTodos,
            "saving returns to the panel it was opened from"
        );
    }

    #[test]
    fn tab_cycles_priority_and_cancel_discards_the_buffer() {
        let mut app = app_with_pane_todos(&[(
            "keep me",
            false,
            crate::terminal::todo::TodoPriority::Normal,
        )]);
        app.handle_pane_todos_key_via_api(key(KeyCode::Enter));

        app.handle_pane_todo_edit_key_via_api(key(KeyCode::Tab));
        assert_eq!(
            app.state.pane_todo_edit.as_ref().expect("edit state").priority,
            crate::terminal::todo::TodoPriority::High
        );
        app.handle_pane_todo_edit_key_via_api(key(KeyCode::Char('!')));
        app.handle_pane_todo_edit_key_via_api(key(KeyCode::Esc));

        let todo = app.state.selected_pane_todo().expect("a selected todo");
        assert_eq!(todo.text, "keep me", "cancel writes nothing");
        assert_eq!(todo.priority, crate::terminal::todo::TodoPriority::Normal);
        assert_eq!(app.state.mode, Mode::PaneTodos);
    }

    #[test]
    fn an_empty_buffer_never_saves() {
        let mut app = app_with_pane_todos(&[(
            "keep me",
            false,
            crate::terminal::todo::TodoPriority::Normal,
        )]);
        app.handle_pane_todos_key_via_api(key(KeyCode::Enter));
        app.handle_pane_todo_edit_key_via_api(key(KeyCode::Char('u')).with_control());

        app.handle_pane_todo_edit_key_via_api(key(KeyCode::Enter));

        assert_eq!(
            app.state.mode,
            Mode::PaneTodoEdit,
            "the store rejects empty text, so the modal stays open instead of dropping the edit"
        );
        assert_eq!(
            app.state.selected_pane_todo().expect("a todo").text,
            "keep me"
        );
    }

    #[test]
    fn the_add_action_creates_a_todo_on_the_focused_pane() {
        let mut app = app_with_pane_todos(&[]);
        app.state.close_pane_todos();
        app.state.mode = Mode::Terminal;
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;

        app.state.open_new_pane_todo(pane_id);
        for ch in "write it down".chars() {
            app.handle_pane_todo_edit_key_via_api(key(KeyCode::Char(ch)));
        }
        app.handle_pane_todo_edit_key_via_api(key(KeyCode::Enter));

        let todos = app.state.pane_todos_in_display_order(pane_id);
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].text, "write it down");
        assert!(
            app.state.pane_todos.is_none(),
            "opened without a panel, it returns to the terminal rather than opening one"
        );
    }
```

`key(...)` is the existing `fn key(code: KeyCode) -> KeyEvent` (`src/app/input/modal.rs:1471`); add a tiny local helper next to it rather than reaching for a new dependency:

```rust
    trait WithControl {
        fn with_control(self) -> KeyEvent;
    }

    impl WithControl for KeyEvent {
        fn with_control(mut self) -> KeyEvent {
            self.modifiers |= KeyModifiers::CONTROL;
            self
        }
    }
```

Add to `src/app/state.rs`'s tests:

```rust
    #[test]
    fn the_edit_link_control_cycles_keep_clear_then_each_candidate() {
        let mut state = AppState::test_new();
        state.workspaces = vec![crate::workspace::Workspace::test_new("links")];
        state.active = Some(0);
        state.ensure_test_terminals();
        let pane_id = state.workspaces[0].tabs[0].root_pane;

        state.open_new_pane_todo(pane_id);
        assert_eq!(
            state.pane_todo_edit.as_ref().expect("edit state").link,
            PaneTodoEditLink::Keep
        );

        state.cycle_pane_todo_edit_link();
        assert_eq!(
            state.pane_todo_edit.as_ref().expect("edit state").link,
            PaneTodoEditLink::Clear
        );
        assert_eq!(state.pane_todo_edit_link_label(), "none");

        // The only pane is the todo's own, so there is nothing to link to and
        // the cycle wraps straight back.
        assert!(state.pane_link_candidates(pane_id).is_empty());
        state.cycle_pane_todo_edit_link();
        assert_eq!(
            state.pane_todo_edit.as_ref().expect("edit state").link,
            PaneTodoEditLink::Keep
        );
    }
```

Add to `src/ui/dialogs.rs`'s tests, mirroring `new_worktree_hit_test_geometry_matches_modal_size` (`:1051`). Extend that module's `use super::{confirm_close_overlay_text, render_new_linked_worktree_overlay};` (`:892`) with `pane_todo_edit_rects, render_pane_todo_edit_overlay, PANE_TODO_EDIT_POPUP_HEIGHT, PANE_TODO_EDIT_POPUP_WIDTH` — `TestBackend`, `Terminal`, and `Rect` are already imported there:

```rust
    #[test]
    fn pane_todo_edit_hit_test_geometry_matches_what_is_drawn() {
        let mut app = AppState::test_new();
        app.workspaces = vec![Workspace::test_new("todos")];
        app.active = Some(0);
        app.ensure_test_terminals();
        app.view.terminal_area = Rect::new(0, 0, 80, 24);
        let pane_id = app.workspaces[0].tabs[0].root_pane;
        app.open_new_pane_todo(pane_id);
        if let Some(edit) = app.pane_todo_edit.as_mut() {
            edit.text = "rerun the deploy".into();
        }

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| render_pane_todo_edit_overlay(&app, frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();

        let inner = crate::ui::centered_popup_rect(
            Rect::new(0, 0, 80, 24),
            PANE_TODO_EDIT_POPUP_WIDTH,
            PANE_TODO_EDIT_POPUP_HEIGHT,
        )
        .map(|popup| {
            Rect::new(
                popup.x + 1,
                popup.y + 1,
                popup.width - 2,
                popup.height - 2,
            )
        })
        .expect("popup should fit");
        let rects = pane_todo_edit_rects(inner).expect("edit rects should exist");

        let input: String = (rects.input.x..rects.input.x + rects.input.width)
            .map(|x| buffer[(x, rects.input.y)].symbol())
            .collect();
        assert!(input.contains("rerun the deploy"));
        assert!(input.contains('█'), "the fake cursor sits after the text");

        let priority: String = (rects.priority.x..rects.priority.x + rects.priority.width)
            .map(|x| buffer[(x, rects.priority.y)].symbol())
            .collect();
        assert!(priority.contains("priority"));
        assert!(priority.contains("normal"));

        let link: String = (rects.link.x..rects.link.x + rects.link.width)
            .map(|x| buffer[(x, rects.link.y)].symbol())
            .collect();
        assert!(link.contains("link"));

        let save: String = (rects.save.x..rects.save.x + rects.save.width)
            .map(|x| buffer[(x, rects.save.y)].symbol())
            .collect();
        assert!(save.contains("save"));
    }
```

Add to `src/config/keybinds.rs`'s tests and `src/ui.rs`'s tests:

```rust
    // src/config/keybinds.rs
    #[test]
    fn add_pane_todo_is_unset_by_default_and_maps_when_bound() {
        assert!(Config::default().keybinds().add_pane_todo.bindings.is_empty());

        let config: Config = toml::from_str(
            r#"
[keys]
add_pane_todo = "prefix+ctrl+y"
"#,
        )
        .unwrap();
        assert_eq!(
            binding_triggers(&config.keybinds().add_pane_todo),
            vec![BindingTrigger::Prefix((
                KeyCode::Char('y'),
                KeyModifiers::CONTROL
            ))]
        );
    }
```

```rust
    // src/ui.rs
    #[test]
    fn keybind_help_shows_unset_for_the_add_pane_todo_action() {
        let app = crate::app::state::AppState::test_new();
        let groups = keybind_help_groups(&app);
        let panes = groups
            .iter()
            .find(|(name, _)| *name == "panes")
            .expect("panes group")
            .1
            .clone();

        assert!(
            panes
                .iter()
                .any(|(key, label)| key == "unset" && label.as_ref() == "add pane todo"),
            "an unbound action is still discoverable in the help panel"
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run --locked --no-fail-fast pane_todo_edit`
Run: `cargo nextest run --locked --no-fail-fast add_pane_todo`
Expected: compile failures — `Mode::PaneTodoEdit`, `open_new_pane_todo`, `pane_todo_edit_rects`, `kb.add_pane_todo` do not exist.

- [ ] **Step 3: Write the implementation**

**3a. `src/app/state.rs`** — `Mode::PaneTodoEdit,` after `PaneTodos`. It is **text entry**, so it stays **out** of the `wants_ascii_input` allowlist; add it to the *second* array of `mode_wants_ascii_input_classification` (`src/app/mod.rs:2206`) and to the array in `honors_key_repeat_allowlists_terminal_and_copy` (`src/app/state.rs:2718`).

```rust
/// The edit modal's link control. `Keep` leaves whatever the todo already has
/// — including a dead link, which the store preserves — untouched; the other
/// two are explicit choices. These map exactly onto `todo.update`'s
/// `link_pane_id` / `clear_link` pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneTodoEditLink {
    Keep,
    Clear,
    Set(PaneId),
}

/// TUI-only state for the pane todo edit modal: the in-progress buffer until
/// save. Deliberately its own `text` rather than the shared `name_input`, so a
/// cancelled rename can never leak into a todo save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneTodoEditState {
    pub pane_id: PaneId,
    /// `None` while composing a brand-new todo.
    pub todo_id: Option<u64>,
    pub text: String,
    pub priority: crate::terminal::todo::TodoPriority,
    pub link: PaneTodoEditLink,
}
```

Field next to `pane_todos`, plus `pane_todo_edit: None,` in `test_new` and `App::new`:

```rust
    /// TUI-only edit buffer; `Some` while `Mode::PaneTodoEdit`.
    pub pane_todo_edit: Option<PaneTodoEditState>,
```

Helpers next to the panel's:

```rust
    /// Open the edit modal on an existing todo, prefilled from the store.
    pub(crate) fn open_pane_todo_edit(&mut self, pane_id: PaneId, todo_id: u64) {
        let Some(todo) = self
            .pane_terminal(pane_id)
            .and_then(|terminal| terminal.todos().iter().find(|todo| todo.id == todo_id))
            .cloned()
        else {
            return;
        };
        self.pane_todo_edit = Some(PaneTodoEditState {
            pane_id,
            todo_id: Some(todo.id),
            text: todo.text,
            priority: todo.priority,
            link: PaneTodoEditLink::Keep,
        });
        self.mode = Mode::PaneTodoEdit;
    }

    /// Open the edit modal on a brand-new todo for a pane.
    pub(crate) fn open_new_pane_todo(&mut self, pane_id: PaneId) {
        self.pane_todo_edit = Some(PaneTodoEditState {
            pane_id,
            todo_id: None,
            text: String::new(),
            priority: crate::terminal::todo::TodoPriority::default(),
            link: PaneTodoEditLink::Keep,
        });
        self.mode = Mode::PaneTodoEdit;
    }

    pub(crate) fn close_pane_todo_edit(&mut self) {
        self.pane_todo_edit = None;
    }

    /// Panes a todo can link to: every other pane of its own workspace, in tab
    /// then layout order, labelled the way that pane's border is.
    pub(crate) fn pane_link_candidates(&self, pane_id: PaneId) -> Vec<(PaneId, String)> {
        let Some(workspace) = self
            .workspaces
            .iter()
            .find(|workspace| workspace.pane_state(pane_id).is_some())
        else {
            return Vec::new();
        };
        workspace
            .tabs
            .iter()
            .flat_map(|tab| tab.layout.pane_ids())
            .filter(|candidate| *candidate != pane_id)
            .map(|candidate| {
                let label = self
                    .pane_terminal(candidate)
                    .and_then(|terminal| terminal.border_label(true))
                    .or_else(|| {
                        workspace
                            .public_pane_number(candidate)
                            .map(|number| format!("pane {number}"))
                    })
                    .unwrap_or_else(|| "pane".to_string());
                (candidate, label)
            })
            .collect()
    }

    pub(crate) fn cycle_pane_todo_edit_priority(&mut self) {
        let Some(edit) = self.pane_todo_edit.as_mut() else {
            return;
        };
        edit.priority = match edit.priority {
            crate::terminal::todo::TodoPriority::Low => {
                crate::terminal::todo::TodoPriority::Normal
            }
            crate::terminal::todo::TodoPriority::Normal => {
                crate::terminal::todo::TodoPriority::High
            }
            crate::terminal::todo::TodoPriority::High => crate::terminal::todo::TodoPriority::Low,
        };
    }

    /// Cycle the link control: keep → clear → each linkable pane → keep.
    pub(crate) fn cycle_pane_todo_edit_link(&mut self) {
        let Some(pane_id) = self.pane_todo_edit.as_ref().map(|edit| edit.pane_id) else {
            return;
        };
        let candidates = self.pane_link_candidates(pane_id);
        let Some(edit) = self.pane_todo_edit.as_mut() else {
            return;
        };
        edit.link = match edit.link {
            PaneTodoEditLink::Keep => PaneTodoEditLink::Clear,
            PaneTodoEditLink::Clear => match candidates.first() {
                Some((candidate, _)) => PaneTodoEditLink::Set(*candidate),
                None => PaneTodoEditLink::Keep,
            },
            PaneTodoEditLink::Set(current) => candidates
                .iter()
                .position(|(candidate, _)| *candidate == current)
                .and_then(|idx| candidates.get(idx + 1))
                .map(|(candidate, _)| PaneTodoEditLink::Set(*candidate))
                .unwrap_or(PaneTodoEditLink::Keep),
        };
    }

    /// What the modal's link row shows for the current choice.
    pub(crate) fn pane_todo_edit_link_label(&self) -> String {
        let Some(edit) = self.pane_todo_edit.as_ref() else {
            return String::new();
        };
        match edit.link {
            PaneTodoEditLink::Clear => "none".to_string(),
            PaneTodoEditLink::Keep => edit
                .todo_id
                .and_then(|todo_id| {
                    let terminal = self.pane_terminal(edit.pane_id)?;
                    let todo = terminal.todos().iter().find(|todo| todo.id == todo_id)?;
                    todo.link.as_ref().map(|link| link.label.clone())
                })
                .unwrap_or_else(|| "none".to_string()),
            PaneTodoEditLink::Set(target) => self
                .pane_link_candidates(edit.pane_id)
                .into_iter()
                .find(|(candidate, _)| *candidate == target)
                .map(|(_, label)| label)
                .unwrap_or_else(|| "pane".to_string()),
        }
    }
```

Invariant, next to the panel's:

```rust
            assert!(
                self.pane_todo_edit.is_none(),
                "empty app state must not keep a pane todo edit buffer"
            );
```

**3b. `src/ui/dialogs.rs`** — next to `rename_button_rects` (`:20`):

```rust
pub(crate) const PANE_TODO_EDIT_POPUP_WIDTH: u16 = 60;
pub(crate) const PANE_TODO_EDIT_POPUP_HEIGHT: u16 = 11;

/// The modal's interactive regions. One definition, read by the renderer and
/// by the mouse layer, so clicking "priority" always lands on the row that
/// says "priority".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PaneTodoEditRects {
    pub input: Rect,
    pub priority: Rect,
    pub link: Rect,
    pub save: Rect,
    pub cancel: Rect,
}

pub(crate) fn pane_todo_edit_rects(inner: Rect) -> Option<PaneTodoEditRects> {
    if inner.width == 0 || inner.height < 8 {
        return None;
    }
    let row = |offset: u16| Rect::new(inner.x, inner.y + offset, inner.width, 1);
    let buttons = action_button_row_rects(
        inner,
        &[
            ActionButtonSpec {
                hint: Some("↵"),
                label: "save",
            },
            ActionButtonSpec {
                hint: Some("esc"),
                label: "cancel",
            },
        ],
        2,
        inner.height - 1,
    );
    Some(PaneTodoEditRects {
        input: row(2),
        priority: row(4),
        link: row(5),
        save: buttons[0],
        cancel: buttons[1],
    })
}

pub(super) fn render_pane_todo_edit_overlay(app: &AppState, frame: &mut Frame, area: Rect) {
    let Some(edit) = app.pane_todo_edit.as_ref() else {
        return;
    };
    super::dim_background(frame, area);

    let title = if edit.todo_id.is_some() {
        "edit todo"
    } else {
        "new todo"
    };
    let Some(inner) = render_modal_shell(
        frame,
        area,
        PANE_TODO_EDIT_POPUP_WIDTH,
        PANE_TODO_EDIT_POPUP_HEIGHT,
        &app.palette,
    ) else {
        return;
    };
    let Some(rects) = pane_todo_edit_rects(inner) else {
        return;
    };

    render_modal_header(frame, Rect::new(inner.x, inner.y, inner.width, 1), title, &app.palette);

    frame.render_widget(Clear, rects.input);
    frame.render_widget(
        Paragraph::new(format!(
            " {}█",
            truncate_end(&edit.text, rects.input.width.saturating_sub(3) as usize)
        ))
        .style(
            Style::default()
                .fg(app.palette.text)
                .bg(app.palette.surface0),
        ),
        rects.input,
    );

    let priority_label = match edit.priority {
        crate::terminal::todo::TodoPriority::High => "high",
        crate::terminal::todo::TodoPriority::Normal => "normal",
        crate::terminal::todo::TodoPriority::Low => "low",
    };
    let field = |name: &str, hint: &str, value: String, value_style: Style| {
        Line::from(vec![
            Span::styled(
                format!(" {name:<10}"),
                Style::default().fg(app.palette.overlay0),
            ),
            Span::styled(
                format!("{hint:<5}"),
                Style::default().fg(app.palette.overlay1),
            ),
            Span::styled(value, value_style),
        ])
    };
    frame.render_widget(
        Paragraph::new(field(
            "priority",
            "⇥",
            priority_label.to_string(),
            Style::default().fg(app.pane_todo_indicator_color(Some(edit.priority))),
        )),
        rects.priority,
    );
    frame.render_widget(
        Paragraph::new(field(
            "link",
            "^l",
            app.pane_todo_edit_link_label(),
            Style::default().fg(app.palette.blue),
        )),
        rects.link,
    );

    render_action_button(
        frame,
        rects.save,
        Some("↵"),
        "save",
        Style::default()
            .fg(panel_contrast_fg(&app.palette))
            .bg(app.palette.accent)
            .add_modifier(Modifier::BOLD),
    );
    render_action_button(
        frame,
        rects.cancel,
        Some("esc"),
        "cancel",
        Style::default()
            .fg(app.palette.text)
            .bg(app.palette.surface0)
            .add_modifier(Modifier::BOLD),
    );
}
```

**3c. `src/ui.rs`** — add `pane_todo_edit_rects`, `PaneTodoEditRects`, `PANE_TODO_EDIT_POPUP_WIDTH`, `PANE_TODO_EDIT_POPUP_HEIGHT` to the `dialogs::{...}` re-export block (`:76`), import `render_pane_todo_edit_overlay` privately, and add the render arm:

```rust
        Mode::PaneTodoEdit => render_pane_todo_edit_overlay(app, frame, frame.area()),
```

**3d. `src/app/input/overlays.rs`** — next to `rename_modal_inner` (`:398`):

```rust
    pub(super) fn pane_todo_edit_inner(&self) -> Option<Rect> {
        self.onboarding_modal_inner(
            crate::ui::PANE_TODO_EDIT_POPUP_WIDTH,
            crate::ui::PANE_TODO_EDIT_POPUP_HEIGHT,
        )
    }
```

**3e. `src/app/input/modal.rs`** — extract the word-delete so both modals share it. Rename the private `RenameWordDeleteClass` / `rename_word_delete_class` to `WordDeleteClass` / `word_delete_class` and replace the body of `delete_rename_input_word` (`:641`):

```rust
fn delete_rename_input_word(state: &mut AppState) {
    if state.name_input_replace_on_type {
        clear_rename_input(state);
        return;
    }
    delete_last_word(&mut state.name_input);
}

/// Delete trailing whitespace, then the run of like-classed characters before
/// it. Shared by the rename modal and the todo edit modal.
fn delete_last_word(buffer: &mut String) {
    while buffer.chars().last().is_some_and(char::is_whitespace) {
        buffer.pop();
    }
    let Some(class) = buffer.chars().last().map(word_delete_class) else {
        return;
    };
    while buffer
        .chars()
        .last()
        .is_some_and(|ch| !ch.is_whitespace() && word_delete_class(ch) == class)
    {
        buffer.pop();
    }
}
```

Text keymap — same shape as `handle_rename_edit_key`, on the modal's own buffer:

```rust
fn handle_pane_todo_edit_text_key(state: &mut AppState, key: KeyEvent) {
    let Some(edit) = state.pane_todo_edit.as_mut() else {
        return;
    };
    match key.code {
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => edit.text.clear(),
        KeyCode::Backspace if key.modifiers.contains(KeyModifiers::SUPER) => edit.text.clear(),
        KeyCode::Backspace
            if key.modifiers.contains(KeyModifiers::CONTROL)
                || key.modifiers.contains(KeyModifiers::ALT) =>
        {
            delete_last_word(&mut edit.text);
        }
        KeyCode::Char('h' | 'w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            delete_last_word(&mut edit.text);
        }
        KeyCode::Backspace => {
            edit.text.pop();
        }
        KeyCode::Char(c) if key.modifiers.difference(KeyModifiers::SHIFT).is_empty() => {
            // Stop at the store's limit so the modal cannot compose a todo the
            // server will reject.
            if edit.text.chars().count() < crate::terminal::todo::MAX_TODO_TEXT_LEN {
                edit.text.push(c);
            }
        }
        _ => {}
    }
}
```

In the `impl App` block:

```rust
    pub(crate) fn handle_pane_todo_edit_key_via_api(&mut self, key: KeyEvent) {
        // Commands before text, like `handle_rename_key_via_api`. Anything
        // carrying CTRL/ALT/SUPER can never be swallowed by the text field.
        match key.code {
            KeyCode::Enter => {
                self.save_pane_todo_edit_via_api();
                return;
            }
            KeyCode::Esc => {
                self.close_pane_todo_edit_and_return();
                return;
            }
            KeyCode::Tab => {
                self.state.cycle_pane_todo_edit_priority();
                return;
            }
            KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.state.cycle_pane_todo_edit_link();
                return;
            }
            _ => {}
        }
        handle_pane_todo_edit_text_key(&mut self.state, key);
    }

    /// Leave the modal back to the panel it was opened from, or to the
    /// terminal when it was opened straight from a keybinding.
    pub(super) fn close_pane_todo_edit_and_return(&mut self) {
        self.state.close_pane_todo_edit();
        if self.state.pane_todos.is_some() {
            self.state.mode = Mode::PaneTodos;
        } else {
            leave_modal(&mut self.state);
        }
    }

    fn save_pane_todo_edit_via_api(&mut self) {
        let Some((pane_id, todo_id, text, priority, link)) =
            self.state.pane_todo_edit.as_ref().map(|edit| {
                (
                    edit.pane_id,
                    edit.todo_id,
                    edit.text.trim().to_string(),
                    edit.priority,
                    edit.link,
                )
            })
        else {
            return;
        };
        if text.is_empty() {
            // The store rejects empty text; keep the modal open rather than
            // silently dropping what was typed.
            return;
        }
        let Some(ws_idx) = self.state.active else {
            return;
        };
        let Some(public_pane_id) = self.public_pane_id(ws_idx, pane_id) else {
            return;
        };
        let link_pane_id = match link {
            crate::app::state::PaneTodoEditLink::Set(target) => {
                self.public_pane_id(ws_idx, target)
            }
            _ => None,
        };
        let clear_link = matches!(link, crate::app::state::PaneTodoEditLink::Clear);

        match todo_id {
            Some(id) => {
                self.runtime_todo_update(
                    "tui.todo.update",
                    crate::api::schema::TodoUpdateParams {
                        pane_id: public_pane_id,
                        id,
                        text: Some(text),
                        done: None,
                        priority: Some(priority),
                        link_pane_id,
                        clear_link,
                    },
                );
            }
            None => {
                self.runtime_todo_add(
                    "tui.todo.add",
                    crate::api::schema::TodoAddParams {
                        pane_id: public_pane_id,
                        text,
                        priority: Some(priority),
                        link_pane_id,
                    },
                );
            }
        }
        self.close_pane_todo_edit_and_return();
        self.state.pane_todos_move_selection(0);
    }

    pub(super) fn apply_pane_todo_edit_action_via_api(&mut self, action: ModalAction) {
        match action {
            ModalAction::Save => self.save_pane_todo_edit_via_api(),
            ModalAction::Cancel => self.close_pane_todo_edit_and_return(),
            _ => {}
        }
    }
```

and replace Task 3's stub with `PaneTodoAction::Edit => self.state.open_pane_todo_edit(pane_id, todo.id),`.

**3f. `src/app/input/mouse.rs`** — inside the `Down(MouseButton::Left)` arm, next to the rename modal block (`:537`):

```rust
                if self.mode == Mode::PaneTodoEdit {
                    let rects = self
                        .pane_todo_edit_inner()
                        .and_then(crate::ui::pane_todo_edit_rects);
                    let Some(rects) = rects else {
                        return None;
                    };
                    if rect_contains(rects.priority, mouse.column, mouse.row) {
                        self.cycle_pane_todo_edit_priority();
                        return None;
                    }
                    if rect_contains(rects.link, mouse.column, mouse.row) {
                        self.cycle_pane_todo_edit_link();
                        return None;
                    }
                    // Anything else cancels, matching the rename modal.
                    let action = modal_action_from_buttons(
                        mouse.column,
                        mouse.row,
                        &[
                            (rects.save, ModalAction::Save),
                            (rects.cancel, ModalAction::Cancel),
                        ],
                    )
                    .unwrap_or(ModalAction::Cancel);
                    return Some(MouseAction::PaneTodoEditModal(action));
                }
```

plus `PaneTodoEditModal(ModalAction),` on the `MouseAction` enum.

**3g. `src/app/input/mod.rs`**
- key arm: `Mode::PaneTodoEdit => self.handle_pane_todo_edit_key_via_api(key_event),`
- mouse dispatch: `MouseAction::PaneTodoEditModal(action) => self.apply_pane_todo_edit_action_via_api(action),`
- `modal_paste_target_active` (`:654`): `Mode::PaneTodoEdit => true,`
- `paste_into_active_text_input` (`:151`) — **both** hooks are required or paste silently does nothing:

```rust
            Mode::PaneTodoEdit => {
                let Some(edit) = self.state.pane_todo_edit.as_mut() else {
                    return false;
                };
                let room = crate::terminal::todo::MAX_TODO_TEXT_LEN
                    .saturating_sub(edit.text.chars().count());
                edit.text
                    .extend(text.chars().filter(|ch| !ch.is_control()).take(room));
                true
            }
```

**3h. `src/app/mod.rs`** — headless arm: `Mode::PaneTodoEdit => { self.handle_pane_todo_edit_key_via_api(key_event); }`

**3i. `src/app/input/navigate.rs`** — `AddPaneTodo,` variant; `(&kb.add_pane_todo, NavigateAction::AddPaneTodo),` in the table; the `App` arm `NavigateAction::AddPaneTodo => self.open_new_pane_todo_for_focused_pane(),`; the pure-state twin arm mirroring `OpenPaneTodos` but calling `state.open_new_pane_todo(pane_id)`; and:

```rust
    pub(crate) fn open_new_pane_todo_for_focused_pane(&mut self) {
        let Some((_, pane_id)) = self.focused_pane_target() else {
            return;
        };
        self.state.open_new_pane_todo(pane_id);
    }
```

**3j. Keybinding registration for `add_pane_todo`** — the same six sites as Task 3, with `BindingConfig::empty()` as the default and the doc comment `/// Open the todo editor on a new todo for the focused pane. Unbound by default.` `src/main.rs` `DEFAULT_CONFIG`, next to the `open_pane_todos` line:

```text
# add_pane_todo = ""      # optional, unset by default; compose a new todo for the focused pane
```

**3k. `src/ui/keybind_help.rs`** — after the `pane todos` entry:

```rust
        help_entry(keybind_label(&kb.add_pane_todo), "add pane todo"),
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run --locked --no-fail-fast pane_todo`
Run: `cargo nextest run --locked --no-fail-fast dialogs`
Run: `cargo nextest run --locked --no-fail-fast keybind config::`
Run: `cargo clippy --all-targets -- -D warnings`
Expected: green.

- [ ] **Step 5: Commit**

```bash
git add src/app src/ui src/config src/main.rs
git commit -m "feat: edit pane todos from the tui"
```

---

### Task 5: Pane-close confirmation, docs, validation

This task closes OpenSpec task 2.3 — the one scenario Phase 1 deliberately deferred because it needs UI — and then stages the docs. It produces **two** commits.

**Files:**
- Modify: `src/app/state.rs` (`confirm_close_pane` field + invariant, `forget_pane_todo_ui`)
- Modify: `src/app/actions.rs` (`pane_has_outstanding_todos`, `confirm_pane_close_with_todos`, gate in the `#[cfg(test)]` `close_pane` twin)
- Modify: `src/app/api/panes.rs` (the gate in `App::close_pane`, plus the cleanup call)
- Modify: `src/app/input/modal.rs` (`confirm_close_accept_via_api` branch, `confirm_close_cancel` clears the token)
- Modify: `src/ui/dialogs.rs` (`confirm_close_overlay_text` pane branch)
- Modify: `docs/next/CHANGELOG.md`
- Modify: `docs/next/website/src/content/docs/keyboard.mdx` (English only — see Step 6 for why the `ja/` and `zh-cn/` copies are left alone)
- Modify: `docs/next/website/src/data/config-reference.json`
- Modify: `openspec/changes/pane-todos/tasks.md`

**Interfaces:**
- Consumes: `TerminalState::outstanding_todo_count()`; `AppState::pane_terminal` (Task 1); `AppState::confirm_implicit_worktree_group_close(usize) -> bool` (`src/app/actions.rs:2013`) as the shape to mirror; `App::runtime_pane_close(&'static str, String)`; `encode_error(String, &str, impl Into<String>)` (`src/app/api/responses.rs:7`).
- Produces:
  - `AppState::confirm_close_pane: Option<PaneId>` — the pending-confirmation token
  - `AppState::pane_has_outstanding_todos(&self, pane_id: PaneId) -> bool`
  - `AppState::confirm_pane_close_with_todos(&mut self, ws_idx: usize, pane_id: PaneId) -> bool`
  - `AppState::forget_pane_todo_ui(&mut self, pane_id: PaneId)`

- [ ] **Step 1: Write the failing tests**

These go in `src/app/input/modal.rs`'s `#[cfg(test)] mod tests`, **not** in `src/app/api/`: `confirm_close_accept_via_api` and `confirm_close_cancel` are `pub(super)` in `app::input::modal`, so only that module's own tests can call them without widening visibility or adding a shim. `App::handle_api_request` is `pub(crate)` (`src/app/api.rs:986`), so the close request is still issued through the real API path. Reuses `app_with_pane_todos` from Task 3, which seeds the todos directly on the terminal.

```rust
    fn close_pane_via_api(app: &mut App, pane_id: crate::layout::PaneId) -> serde_json::Value {
        let public_pane_id = app
            .public_pane_id(0, pane_id)
            .expect("pane should have a public id");
        let raw = app.handle_api_request(crate::api::schema::Request {
            id: "test".into(),
            method: crate::api::schema::Method::PaneClose(crate::api::schema::PaneTarget {
                pane_id: public_pane_id,
            }),
        });
        serde_json::from_str(&raw).expect("response should be json")
    }

    /// Spec: "a pane with at least one not-done todo is closed → a
    /// confirmation is requested before the pane is destroyed".
    #[test]
    fn closing_a_pane_with_outstanding_todos_asks_first() {
        let mut app = app_with_pane_todos(&[(
            "unfinished",
            false,
            crate::terminal::todo::TodoPriority::Normal,
        )]);
        app.state.close_pane_todos();
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;

        let response = close_pane_via_api(&mut app, pane_id);

        assert_eq!(response["error"]["code"], "confirmation_required");
        assert_eq!(app.state.mode, Mode::ConfirmClose);
        assert_eq!(app.state.confirm_close_pane, Some(pane_id));
        assert!(
            !app.state.workspaces.is_empty(),
            "nothing is destroyed before the answer"
        );
    }

    #[test]
    fn accepting_the_confirmation_closes_the_pane() {
        let mut app = app_with_pane_todos(&[(
            "unfinished",
            false,
            crate::terminal::todo::TodoPriority::Normal,
        )]);
        app.state.close_pane_todos();
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        close_pane_via_api(&mut app, pane_id);

        app.confirm_close_accept_via_api();

        assert!(
            app.state.confirm_close_pane.is_none(),
            "the pending token is consumed, so the retry goes through"
        );
        assert!(
            app.state.workspaces.is_empty(),
            "the last pane closing takes its workspace with it"
        );
    }

    #[test]
    fn cancelling_the_confirmation_keeps_the_pane_and_drops_the_token() {
        let mut app = app_with_pane_todos(&[(
            "unfinished",
            false,
            crate::terminal::todo::TodoPriority::Normal,
        )]);
        app.state.close_pane_todos();
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        close_pane_via_api(&mut app, pane_id);

        confirm_close_cancel(&mut app.state);

        assert!(app.state.confirm_close_pane.is_none());
        assert!(!app.state.workspaces.is_empty());
    }

    /// Spec: "every todo on the pane is done → the pane closes without
    /// additional confirmation".
    #[test]
    fn a_pane_whose_todos_are_all_done_closes_without_a_prompt() {
        let mut app = app_with_pane_todos(&[(
            "finished",
            true,
            crate::terminal::todo::TodoPriority::Normal,
        )]);
        app.state.close_pane_todos();
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;

        let response = close_pane_via_api(&mut app, pane_id);

        assert!(response["result"].is_object(), "no prompt: {response:?}");
        assert!(app.state.confirm_close_pane.is_none());
    }
```

Add the pure-state twin test to `src/app/actions.rs`'s tests:

```rust
    #[test]
    fn close_pane_defers_to_confirmation_while_todos_remain() {
        let mut state = AppState::test_new();
        state.workspaces = vec![crate::workspace::Workspace::test_new("todos")];
        state.active = Some(0);
        state.ensure_test_terminals();
        let pane_id = state.workspaces[0].tabs[0].root_pane;
        let terminal_id = state.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        state
            .terminals
            .get_mut(&terminal_id)
            .expect("test terminal should exist")
            .add_todo(
                "unfinished",
                crate::terminal::todo::TodoPriority::Normal,
                None,
                100,
            )
            .expect("todo should be added");

        assert!(state.close_pane(), "the close is deferred to confirmation");
        assert_eq!(state.mode, Mode::ConfirmClose);
        assert_eq!(state.confirm_close_pane, Some(pane_id));

        assert!(
            !state.close_pane(),
            "answering yes consumes the token and the close proceeds"
        );
    }
```

And the overlay copy test in `src/ui/dialogs.rs`'s tests, next to the existing `confirm_close_overlay_text` tests (`:900`):

```rust
    #[test]
    fn confirm_close_text_names_the_unfinished_todos() {
        let mut app = AppState::test_new();
        app.workspaces = vec![Workspace::test_new("current")];
        app.active = Some(0);
        app.selected = 0;
        app.ensure_test_terminals();
        let pane_id = app.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        let terminal = app
            .terminals
            .get_mut(&terminal_id)
            .expect("test terminal should exist");
        for text in ["one", "two"] {
            terminal
                .add_todo(text, crate::terminal::todo::TodoPriority::Normal, None, 100)
                .expect("todo should be added");
        }
        app.confirm_close_pane = Some(pane_id);

        let terminal_runtimes = crate::terminal::TerminalRuntimeRegistry::new();
        let (title, detail) = confirm_close_overlay_text(&app, &terminal_runtimes);

        assert_eq!(title, "Close pane with unfinished todos?");
        assert!(detail.contains("2 outstanding"));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run --locked --no-fail-fast confirm`
Expected: compile failure — `AppState::confirm_close_pane` does not exist.

- [ ] **Step 3: Write the implementation**

**3a. `src/app/state.rs`** — field next to `rename_pane_target` (`:1635`), plus `confirm_close_pane: None,` in `test_new` and `App::new`:

```rust
    /// Pane whose close is waiting on the confirmation modal because it still
    /// has outstanding todos. Doubles as the "the user said yes" token: the
    /// close path consumes it.
    pub confirm_close_pane: Option<PaneId>,
```

Cleanup helper next to `close_pane_todos`:

```rust
    /// Drop the TUI todo surfaces that pointed at a pane which is going away,
    /// so no panel, modal, or pending confirmation outlives its pane.
    pub(crate) fn forget_pane_todo_ui(&mut self, pane_id: PaneId) {
        if self
            .pane_todos
            .as_ref()
            .is_some_and(|panel| panel.pane_id == pane_id)
        {
            self.pane_todos = None;
            if self.mode == Mode::PaneTodos {
                self.mode = Mode::Terminal;
            }
        }
        if self
            .pane_todo_edit
            .as_ref()
            .is_some_and(|edit| edit.pane_id == pane_id)
        {
            self.pane_todo_edit = None;
            if self.mode == Mode::PaneTodoEdit {
                self.mode = Mode::Terminal;
            }
        }
        if self.confirm_close_pane == Some(pane_id) {
            self.confirm_close_pane = None;
        }
    }
```

Empty-state invariant next to the other two:

```rust
            assert!(
                self.confirm_close_pane.is_none(),
                "empty app state must not keep a pending pane close confirmation"
            );
```

**3b. `src/app/actions.rs`** — next to `confirm_implicit_worktree_group_close` (`:2013`):

```rust
    /// Whether closing this pane would discard unfinished work.
    pub(crate) fn pane_has_outstanding_todos(&self, pane_id: PaneId) -> bool {
        self.pane_terminal(pane_id)
            .is_some_and(|terminal| terminal.outstanding_todo_count() > 0)
    }

    /// Ask before destroying a pane that still has outstanding todos. Returns
    /// true when the close was deferred to the confirmation modal.
    ///
    /// A pending confirmation for the same pane *is* the user's answer: it is
    /// consumed here, so the retry that the modal issues goes straight
    /// through. That keeps the whole gate in one place and needs no `force`
    /// flag on the wire.
    ///
    /// Deliberately not gated on `ui.confirm_close`: that option is documented
    /// as "ask before closing a workspace", and the requirement here has no
    /// escape hatch — the prompt only ever appears when work is genuinely
    /// unfinished.
    pub(crate) fn confirm_pane_close_with_todos(&mut self, ws_idx: usize, pane_id: PaneId) -> bool {
        if self.confirm_close_pane == Some(pane_id) {
            self.confirm_close_pane = None;
            return false;
        }
        if !self.pane_has_outstanding_todos(pane_id) {
            return false;
        }
        self.selected = ws_idx;
        self.confirm_close_pane = Some(pane_id);
        self.mode = Mode::ConfirmClose;
        true
    }
```

and the same gate in the `#[cfg(test)]` `close_pane` twin (`:2044`), right after the existing worktree-group block:

```rust
        if let Some(ws_idx) = active {
            if let Some(pane_id) = self
                .workspaces
                .get(ws_idx)
                .and_then(crate::workspace::Workspace::focused_pane_id)
            {
                if self.confirm_pane_close_with_todos(ws_idx, pane_id) {
                    return true;
                }
            }
        }
```

**3c. `src/app/api/panes.rs`** — in `close_pane` (`:1543`), immediately after the existing worktree-group check (`:1552-1560`), so the bigger warning still wins when both apply:

```rust
        if self.state.confirm_pane_close_with_todos(ws_idx, pane_id) {
            return Err(encode_error(
                id,
                "confirmation_required",
                "this pane still has outstanding todos",
            ));
        }
```

and next to `self.state.remove_plugin_pane_records([pane_id]);` (`:1569`):

```rust
        self.state.forget_pane_todo_ui(pane_id);
```

Putting the gate here rather than in the TUI is deliberate: it is the same place the worktree-group confirmation lives, so an external `pane.close` over the socket gets the same `confirmation_required` answer the TUI does.

**3d. `src/app/input/modal.rs`** — `confirm_close_accept_via_api` (`:1169`) gains a leading branch, and `confirm_close_cancel` (`:749`) drops the token:

```rust
    pub(super) fn confirm_close_accept_via_api(&mut self) {
        // A pending pane confirmation is what is on screen; the retry
        // re-enters `close_pane`, which consumes the token and proceeds.
        // Without this branch the modal would close the whole workspace.
        if let Some(pane_id) = self.state.confirm_close_pane {
            let ws_idx = self.state.selected;
            match self.public_pane_id(ws_idx, pane_id) {
                Some(public_pane_id) => {
                    self.runtime_pane_close("tui.pane.close", public_pane_id);
                }
                None => self.state.confirm_close_pane = None,
            }
            self.state.mode = if self.state.active.is_some() {
                Mode::Terminal
            } else {
                Mode::Navigate
            };
            return;
        }

        let ws_idx = self.state.selected;
        if ws_idx < self.state.workspaces.len() {
            self.close_workspace_idx_via_api(ws_idx);
        }
        self.state.mode = if self.state.active.is_some() {
            Mode::Terminal
        } else {
            Mode::Navigate
        };
    }
```

```rust
pub(super) fn confirm_close_cancel(state: &mut AppState) {
    state.confirm_close_pane = None;
    state.mode = Mode::Navigate;
}
```

The `#[cfg(test)]` accept twin `confirm_close_accept` (`:740`) is deliberately **not** changed — see the sharp edge at the end of this plan for why, and do not "fix" it on the way past.

**3e. `src/ui/dialogs.rs`** — at the top of `confirm_close_overlay_text` (`:717`):

```rust
    if let Some(pane_id) = app.confirm_close_pane {
        let outstanding = app
            .pane_terminal(pane_id)
            .map(|terminal| terminal.outstanding_todo_count())
            .unwrap_or(0);
        let label = app
            .pane_terminal(pane_id)
            .and_then(|terminal| terminal.border_label(true))
            .unwrap_or_else(|| "this pane".to_string());
        let todo_text = if outstanding == 1 {
            "1 outstanding todo".to_string()
        } else {
            format!("{outstanding} outstanding todos")
        };
        return (
            "Close pane with unfinished todos?".to_string(),
            format!("{label} — {todo_text}"),
        );
    }
```

- [ ] **Step 4: Run the confirmation tests**

Run: `cargo nextest run --locked --no-fail-fast confirm`
Run: `cargo nextest run --locked --no-fail-fast app::api::todos app::api::panes`
Run: `cargo clippy --all-targets -- -D warnings`
Expected: green.

- [ ] **Step 5: Commit the confirmation**

```bash
git add src/app src/ui/dialogs.rs
git commit -m "feat: confirm closing a pane that still has outstanding todos"
```

- [ ] **Step 6: Stage the docs**

`docs/next/CHANGELOG.md`, a new bullet under `## Unreleased` → `### Added`, directly after the existing pane-todos bullet:

```markdown
- Pane todos are now visible and editable in the TUI. A split pane with todos shows a `▾ N` indicator at the far right of its top border carrying its outstanding count — a bare `▾` once everything is done, and nothing at all for a pane with no todos — coloured by the highest outstanding priority. Clicking it or pressing `keys.open_pane_todos` (`prefix+ctrl+t`) opens a panel hanging off that pane listing its todos not-done first, then priority, then creation order. `Up`/`Down` or `j`/`k` move the selection, `Enter` or a row click opens the todo for editing, `space` toggles done, `d` removes the selected todo, `c` (or the footer button) clears the done ones, `g` or a click on the `→` chip jumps to a todo's linked pane, and `Esc`/`q` closes. Links whose target is gone keep their label, render dimmed, and are inert. The edit modal changes text, cycles priority with `Tab`, and cycles the link with `Ctrl+L`, with explicit save and cancel; `keys.add_pane_todo` (unbound by default) opens it on a new todo for the focused pane. Closing a pane that still has outstanding todos now asks for confirmation first — over the socket API too, where `pane.close` answers `confirmation_required`. New options: `ui.show_pane_todo_indicator` (default `true`) and `ui.pane_todo_color`.
```

`docs/next/website/src/content/docs/keyboard.mdx` — two rows in the **Panes** table (header at `:36`, rows `:38-48`), directly after the "Close pane" row (`:39`) so the pane-scoped actions stay together and above the layout/copy-mode rows:

```markdown
| Pane todos | `prefix+ctrl+t` |
| Add pane todo | unset |
```

and one paragraph immediately after the notification-center paragraph (`:73`) and before *"The full keymap and the binding syntax live in…"* (`:75`), so the two panel explanations sit together:

```markdown
The pane todo panel (`prefix+ctrl+t`, or click the `▾ N` indicator at the far right of a split pane's top border) lists that pane's todos, not-done first, then by priority, then in creation order. `Up`/`Down` or `j`/`k` move the selection, `Enter` (or a row click) opens the todo for editing, `space` toggles done, `d` removes it, `c` clears the done ones, `g` (or a click on the `→` chip) jumps to the todo's linked pane, and `Esc`/`q` closes. A link whose target pane is gone keeps its label but is dimmed and inert. In the edit modal, `Tab` cycles priority, `Ctrl+L` cycles the link target, `Enter` saves, and `Esc` cancels back to the panel. `keys.add_pane_todo` is unbound by default; bind it to compose a new todo for the focused pane without opening the panel. Panes with no todos show no indicator, and a pane with no top border (a single-pane tab, or `ui.pane_borders = false`) is reached through the keybinding.
```

**Do not touch `ja/keyboard.mdx` or `zh-cn/keyboard.mdx`, and do not touch `configuration.mdx` at all.** Both are deliberate, and both were checked against the tooling rather than assumed:

- Translation parity is *file set + heading outline*, nothing more: `scripts/docs_translation_parity.py` compares the `*.mdx` names per locale and the list of heading levels (`heading_outline`), and `just release-docs-check` runs it over both `docs/next/website/src/content/docs` and `website/src/content/docs`. Adding table rows and a paragraph under existing headings changes no heading, so the check stays green. The localized keyboard pages already omit the whole notification-center row and paragraph, so half-translating one new feature into them would make them *less* consistent, not more. (The "extend existing pages" rule in the constraints still holds for the opposite reason: a **new** `.mdx` would need `ja/` and `zh-cn/` files with an identical heading outline, which is real work.)
- `configuration.mdx` does not enumerate `ui.pane_*` keys. Its "UI and sidebar" section (`:255-257`) is one paragraph that says *"Search `ui.` in the [Config reference](/docs/config-reference/) for sizing, collapsed mode, Agent panel ordering, mouse behavior, pane borders, and other presentation settings."* — `ui.pane_borders`, `ui.pane_title_active_color`, and the rest live only in `config-reference.json`. The two new `ui.*` options follow that pattern, so their reference rows (shipped in Task 1) are their documentation, and there is no configuration.mdx prose to translate.

`docs/next/website/src/data/config-reference.json` — two rows here, or `python3 scripts/config_reference_check.py` fails. That script is **not** release-only: `scripts/test_config_reference_check.py` wraps it and both `just test` and `just check` run it in their `python3 -m unittest` line, so a key without its row breaks the tree on the commit that adds the key. The two `ui.*` rows therefore ship with Task 1, and only the two `keys.*` rows are left for here (`keys.open_pane_todos` lands in Task 3 and `keys.add_pane_todo` in Task 4 — add each row in the same commit as its key if that commit is the one that makes the check fail).

The file is **not** a flat array: it is `{"sections": [{"id", "title", "keys": [...]}, …]}`, and each row goes in the `keys` array of its section. `reference_entries` flattens every section, so placement does not affect the check — it affects where the row renders on `/docs/config-reference/`, so put each one where its neighbours are:

- `keys.open_pane_todos` and `keys.add_pane_todo` → the `"id": "keys"` section ("Keybindings"), after the existing `keys.close_pane` row.

```json
{
  "key": "keys.open_pane_todos",
  "type": "keybinding",
  "default": "\"prefix+ctrl+t\"",
  "description": "Open the focused pane's todo panel."
},
{
  "key": "keys.add_pane_todo",
  "type": "keybinding",
  "default": "unset",
  "description": "Open the todo editor on a new todo for the focused pane. Unset by default."
}
```

Match the `"type"` string the existing keybinding rows use (check `keys.close_pane`) rather than inventing one.

- [ ] **Step 7: Tick the OpenSpec tasks**

In `openspec/changes/pane-todos/tasks.md`, tick 2.3, all of 5.x, all of 6.x, and 7.1–7.3. Leave **4.4** unticked and leave its deferral note intact.

- [ ] **Step 8: Full validation**

Run: `just check`
Expected: green apart from the known macOS `live_handoff_keeps_unmanaged_agent_name_bound_to_saved_session` failure, which reproduces on clean `upstream/master` (fork issue #33). `tests/cli` reporting "0 tests run" on macOS is expected (issue #30).

Run: `python3 scripts/config_reference_check.py`
Expected: no missing/stale keys.

Run: `openspec validate pane-todos --strict`
Expected: `Change 'pane-todos' is valid`.

Dogfood in a live build before calling it done — build the debug binary and drive the real surfaces, clearing inherited socket overrides so it talks to the debug server:

```bash
env -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH cargo run -- server start
```

Check by hand: the indicator appears on a split pane after `herdr todo add`, clicking it opens the panel, `space`/`d`/`c` move the same state the CLI sees, `g` jumps, and closing a pane with an outstanding todo prompts.

- [ ] **Step 9: Commit the docs**

```bash
git add docs/next openspec/changes/pane-todos/tasks.md
git commit -m "docs: document the pane todo indicator, panel, and keybindings"
```

---

## Self-Review Notes

**Spec coverage.** *Pane todo indicator* → Task 1 (outstanding count, all-done bare glyph, quiet pane unchanged, priority colour, `ui.show_pane_todo_indicator`, single shared rect) and Task 3 (the click target, asserted cell-by-cell against the drawn rect). *Pane todo panel and editing* → Task 2 (anchored panel, presentation order, dimmed dead links) + Task 3 (selection, toggle, remove, clear done, follow link, close, `keys.open_pane_todos`) + Task 4 (edit modal, `keys.add_pane_todo`, both `help_entry` rows). *Todos persist with their pane* → Task 5, the close-confirmation scenario only; the persistence scenarios landed in Phase 1.

**Requirement → test map.**

| Scenario | Test | Task |
|---|---|---|
| Outstanding count is shown | `pane_todo_indicator_counts_only_outstanding_todos` | 1 |
| A quiet pane is unchanged | `a_pane_with_no_todos_renders_the_border_it_has_today` | 1 |
| Click target matches what is drawn | `pane_todo_indicator_draws_exactly_the_cells_it_claims` + `clicking_the_pane_todo_indicator_toggles_the_panel` | 1, 3 |
| Opening the panel | `open_pane_todos_opens_the_panel_on_the_focused_pane`, `rows_render_in_presentation_order` | 2, 3 |
| Editing a todo | `saving_an_edit_changes_text_and_keeps_id_done_and_created_at` | 4 |
| Following a link | `g_follows_a_live_link_and_closes_the_panel` | 3 |
| Dead link is inert | `g_on_a_dead_link_is_inert`, `a_dead_link_chip_renders_dimmed_and_a_live_one_does_not` | 2, 3 |
| Actions are discoverable | `keybind_help_lists_the_pane_todo_panel_action`, `keybind_help_shows_unset_for_the_add_pane_todo_action` | 3, 4 |
| Closing a pane with outstanding todos is confirmed | `closing_a_pane_with_outstanding_todos_asks_first`, `a_pane_whose_todos_are_all_done_closes_without_a_prompt` | 5 |

**Existing symbols verified against the working tree at `50c83a1f`** — every one is referenced by this plan and confirmed to exist with the receiver shown:

| Symbol | Location |
|---|---|
| `fn render_pane_border_titles(&AppState, &Workspace, &[PaneInfo], &mut Frame)` | `src/ui/panes.rs:624` (private) |
| `fn pane_border_title(&str, u16, bool) -> Option<String>` | `src/ui/panes.rs:26` |
| `fn render_panes(...)` resolves `ws` from `app.active` | `src/ui/panes.rs:304-309` |
| `pub struct PaneInfo { id, rect, inner_rect, scrollbar_rect, borders, is_focused }` | `src/layout.rs:34` |
| `Workspace::pane_state(PaneId) -> Option<&PaneState>` | `src/workspace.rs:1175` |
| `Workspace::public_pane_number(PaneId) -> Option<usize>` | `src/workspace.rs:1049` |
| `TileLayout::pane_ids(&self) -> Vec<PaneId>` (the struct is `TileLayout`, `src/layout.rs:106`; `Tab::layout` is `pub layout: TileLayout`, `src/workspace/tab.rs:43`) | `src/layout.rs:310` |
| `TerminalState::border_label(bool) -> Option<String>` | `src/terminal/state.rs:1996` |
| `TerminalState::{todos, todos_in_display_order, outstanding_todo_count, highest_outstanding_todo_priority}` | `src/terminal/todo.rs:113/191/206/211` |
| `AppState::pane_title_color(bool) -> Color` | `src/app/state.rs:1959` |
| `AppState::ensure_test_terminals()` | `src/app/state.rs:2366` |
| `AppState::assert_invariants_for_test()` | `src/app/state.rs:2392` |
| `AppState::screen_rect() -> Rect` | `src/app/input/mouse.rs:1340` |
| `AppState::pane_info_by_id(PaneId) -> Option<&PaneInfo>` | `src/app/input/mouse.rs:1693` |
| `fn rect_contains(Rect, u16, u16) -> bool` | `src/app/input/mouse.rs:2109` |
| `AppState::find_border_at(u16, u16) -> Option<&SplitBorder>` | `src/app/input/mouse.rs:1631` |
| `notification_indicator_hit` block (placement model) | `src/app/input/mouse.rs:179-194` |
| `leave_modal(&mut AppState)` | `src/app/input/modal.rs:451` |
| `confirm_close_cancel(&mut AppState)` | `src/app/input/modal.rs:749` |
| `App::confirm_close_accept_via_api()` | `src/app/input/modal.rs:1169` |
| `App::focus_pane_internal_via_api(usize, PaneId)` | `src/app/input/navigate.rs:526` |
| `App::focused_pane_target() -> Option<(usize, PaneId)>` | `src/app/input/navigate.rs:749` |
| `App::public_pane_id(usize, PaneId) -> Option<String>` | `src/app/ids.rs:27` |
| `App::close_pane(String, &PaneTarget) -> Result<(), String>` | `src/app/api/panes.rs:1543` |
| `AppState::confirm_implicit_worktree_group_close(usize) -> bool` | `src/app/actions.rs:2013` |
| `AppState::close_pane() -> bool` (`#[cfg(test)]`) | `src/app/actions.rs:2044` |
| `confirm_close_overlay_text(&AppState, &TerminalRuntimeRegistry) -> (String, String)` | `src/ui/dialogs.rs:717` |
| `rename_button_rects(Rect) -> (Rect, Rect, Rect)` / `render_rename_overlay` | `src/ui/dialogs.rs:20`/`:43` |
| `render_panel_shell` / `render_modal_shell` / `render_modal_header` / `render_action_button` / `action_button_row_rects` / `action_button_width` / `panel_contrast_fg` / `centered_popup_rect` | `src/ui/widgets.rs:11/51/62/164/151/142/32/39` |
| `notification_center_button_rects(Rect) -> Option<NotificationCenterButtonRects>` (shape model) | `src/ui/notification_center.rs:62` |
| `AppState::onboarding_modal_inner(u16, u16) -> Option<Rect>` | `src/app/input/overlays.rs:356` |
| `modal_action_from_buttons(u16, u16, &[(Rect, A)]) -> Option<A>` | `src/app/input/modal.rs:64` |
| `keybind_help_groups(&AppState) -> Vec<HelpGroup>`, `help_entry`, `keybind_label` | `src/ui/keybind_help.rs:69/22/26` |
| `default_config_documents_every_binding_action` | `src/main.rs:1017` |
| `App::dispatch_runtime_mutation(&'static str, Method) -> String` | `src/app/runtime_mutations.rs:12` |
| `TodoAddParams / TodoUpdateParams / TodoRemoveParams / TodoClearParams` | `src/api/schema/todos.rs:39/51/67/74` |
| `app_for_mouse_test()`, `state_with_workspaces(&[&str])` | `src/app/input/mod.rs:753`/`:739` |
| `test_app_with_pane()`, `request_json(&mut App, Method)` | `src/app/api/todos.rs:269`/`:287` |

**Exhaustive `Mode` matches** (verified by adding a throwaway variant and reading rustc's `E0004` list): `src/app/input/mod.rs:96`, `src/app/mod.rs:1966`, `src/ui.rs:523`. Nothing outside `src/` names `crate::app::Mode`. Two more sites are **not** compile-enforced and are the easy misses: the `wants_ascii_input` allowlist (`src/app/state.rs:851`) and the hand-written arrays in `honors_key_repeat_allowlists_terminal_and_copy` (`src/app/state.rs:2718`) and `mode_wants_ascii_input_classification` (`src/app/mod.rs:2191`). Tasks 2 and 4 name all five.

**Where the mouse check must go, and why.** The indicator sits on a pane's top border, and for every pane that is not at the top of the layout `find_border_at` claims that whole row as a split-drag hitbox (`src/app/input/mouse.rs:1649/1652` — `b.pos` *is* the lower pane's top-border row). So the indicator hit-test goes high in `handle_mouse`, right after the notification-indicator block, not at the `!in_sidebar` branch at `:587`. Same reason the notification indicator's allowlist includes its own panel mode: that is what makes the glyph a toggle.

**No `PROTOCOL_VERSION` bump, no schema edits.** Phase 2 only *calls* the `todo.*` methods Phase 1 shipped. `src/api/schema/` and `docs/next/api/herdr-api.schema.json` stay byte-identical; the protocol expectations in `tests/cli/sessions.rs`, `tests/api_ping.rs`, and `tests/support/mod.rs` stay at 19.

**Runtime/client boundary.** Everything Phase 2 adds to `AppState` (`pane_todos`, `pane_todo_edit`, `confirm_close_pane`, `show_pane_todo_indicator`, `pane_todo_color`) is client-side. Nothing new is persisted, evented, or named on the wire, and no API vocabulary gains `panel`, `row`, `chip`, or `indicator`.

**Deliberate divergences from the notification center, and why.** `Enter` edits instead of jumping — todos are authored, notifications are not — so jumping moves onto the `→` chip and `g`, which keeps the mouse-first path matching what is visible. The panel has no pure-state `#[cfg(test)]` key twin: the notification center has one because `AppState::focus_pane_in_workspace` exists as pure state, whereas every todo mutation only exists behind the API, so `App`-level tests are the honest level rather than a second implementation to keep in sync.

**Known sharp edges, called out rather than hidden.**
- If a pane close would *both* close a worktree group and discard outstanding todos, the worktree prompt wins and accepting it closes the workspace without a second todo prompt. That mirrors the existing precedence, and the user has already been warned about the larger destruction.
- `confirm_close_cancel` returns to `Mode::Navigate` even when a pane was focused. That is pre-existing behaviour, left alone.
- Only the `via_api` accept path learns about `confirm_close_pane`. Its `#[cfg(test)]` pure-state twin `confirm_close_accept` (`src/app/input/modal.rs:740`) still calls `state.close_selected_workspace()` unconditionally, and the `#[cfg(test)]` `handle_confirm_close_key` (`:754`) routes Enter there, so a *test* that answers a pending pane confirmation through that twin closes the whole workspace instead of the pane. Nothing in production reaches it — the real key path is `handle_confirm_close_key_via_api` (`:1215`) → `confirm_close_accept_via_api` (`:1169`) — and Task 5's tests deliberately use `App::confirm_close_accept_via_api` and `AppState::close_pane` instead. Teaching the twin about the token would mean duplicating `public_pane_id` + `runtime_pane_close` in pure state, which is exactly the second implementation this plan avoids elsewhere.
- Clicking inside the edit modal but off a control cancels it, matching `render_rename_overlay`'s `unwrap_or(ModalAction::Cancel)`. Parity with the existing modal was chosen over a local fix.
- `pane_todos` / `pane_todo_edit` / `confirm_close_pane` get empty-state invariants but **not** live-pane assertions: all three resolve their pane on every read and go quiet when it is gone. `rename_pane_target` is asserted because a save consumes it. `App::close_pane` additionally calls `forget_pane_todo_ui` so the common single-pane close cleans up eagerly.
