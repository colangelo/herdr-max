//! The todo compose/edit modal (fork 88f45de8, e02a7ece, 58aeb518, f26ac476,
//! 22a493d0, 611316dc): a multi-line text block with priority, link and done
//! rows, saved with ctrl+s or alt+enter.
//!
//! The save is the server's: the modal sends `todo.add` or `todo.update` and
//! closes only when the answer is a success, so a refused save keeps every
//! typed character on screen. The panel it was opened from is carried inside
//! it and put back when it closes.

use super::render::{display_width, put_text};
use super::todo_panel::{render_panel_shell, todo_priority_color, ClientTodoPanelOverlay};
use super::todo_text::{
    apply_text_key, caret_visual_position, visual_to_logical, wrap_layout, Shape, TextField,
};
use super::*;
use crate::api::schema::{Method, PaneTarget, TodoAddParams, TodoInfo, TodoUpdateParams};
use crate::terminal::todo::TodoPriority;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::style::Color;

const POPUP_WIDTH: u16 = 84;
const POPUP_HEIGHT: u16 = 20;
const INPUT_ROWS: u16 = 8;
/// Below this many inner rows the modal draws only its frame.
const MIN_INNER_HEIGHT: u16 = 16;
/// The store's limit, enforced while composing.
const MAX_TEXT_CHARS: usize = 500;
const SAVE_TEXT: &str = " ^s save ";
const CANCEL_TEXT: &str = " esc cancel ";

/// What the save does to the todo's link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum TodoEditLink {
    /// Leave the stored link as it is, dead or alive.
    Keep,
    Clear,
    /// Link to this pane, by its public id.
    Set {
        pane_id: String,
        label: String,
    },
}

/// The link a todo already carries, as the server reported it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StoredTodoLink {
    /// The target's public id while it still exists.
    pub(super) live_pane_id: Option<String>,
    pub(super) label: String,
}

#[derive(Debug)]
pub(super) struct ClientTodoEditOverlay {
    pub(super) pane_id: String,
    pub(super) boot_id: String,
    pub(super) todo_id: Option<u64>,
    pub(super) text: TextField,
    pub(super) priority: TodoPriority,
    pub(super) done: bool,
    pub(super) link: TodoEditLink,
    pub(super) stored_link: Option<StoredTodoLink>,
    /// The panel this modal was opened from, put back when it closes.
    pub(super) suspended: Option<ClientTodoPanelOverlay>,
    /// A save is in flight; another save waits for its answer.
    pub(super) saving: bool,
}

impl ClientTodoEditOverlay {
    /// The pane a save-and-follow would travel to: the staged link, else the
    /// stored one while it is alive.
    fn link_target(&self) -> Option<&str> {
        match &self.link {
            TodoEditLink::Clear => None,
            TodoEditLink::Set { pane_id, .. } => Some(pane_id),
            TodoEditLink::Keep => self
                .stored_link
                .as_ref()
                .and_then(|link| link.live_pane_id.as_deref()),
        }
    }

    fn link_label(&self) -> String {
        let label = |id: Option<&str>, label: &str| match id {
            Some(id) if label.is_empty() => id.to_owned(),
            Some(id) => format!("{id} · {label}"),
            None => label.to_owned(),
        };
        match &self.link {
            TodoEditLink::Clear => "none".to_owned(),
            TodoEditLink::Set {
                pane_id,
                label: text,
            } => label(Some(pane_id), text),
            TodoEditLink::Keep => match &self.stored_link {
                None => "none".to_owned(),
                Some(link) => label(link.live_pane_id.as_deref(), &link.label),
            },
        }
    }
}

/// Where everything in the modal is, for one frame. `rows` is `None` when the
/// screen is too short for the modal's controls: it then draws its frame only
/// and the mouse is inert.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct TodoEditLayout {
    pub(super) popup: Rect,
    pub(super) rows: Option<TodoEditRows>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct TodoEditRows {
    pub(super) inner: Rect,
    pub(super) input: Rect,
    pub(super) text_area: Rect,
    pub(super) priority: Rect,
    pub(super) link: Rect,
    pub(super) done: Rect,
    pub(super) save: Rect,
    pub(super) cancel: Rect,
}

pub(super) fn todo_edit_layout(area: Rect) -> Option<TodoEditLayout> {
    let width = POPUP_WIDTH.min(area.width.saturating_sub(4));
    let height = POPUP_HEIGHT.min(area.height.saturating_sub(2));
    if width < 4 || height < 4 {
        return None;
    }
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    let inner = Rect::new(popup.x + 1, popup.y + 1, popup.width - 2, popup.height - 2);
    if inner.width == 0 || inner.height < MIN_INNER_HEIGHT {
        return Some(TodoEditLayout { popup, rows: None });
    }
    let row = |offset: u16| Rect::new(inner.x, inner.y + offset, inner.width, 1);
    let input = Rect::new(inner.x, inner.y + 2, inner.width, INPUT_ROWS);
    let text_area = Rect::new(
        input.x + 1,
        input.y,
        input.width.saturating_sub(2),
        input.height,
    );
    let below_input = 2 + INPUT_ROWS;
    let save_width = display_width(SAVE_TEXT);
    let cancel_width = display_width(CANCEL_TEXT);
    let total = save_width + 2 + cancel_width;
    let buttons_y = inner.bottom() - 1;
    let x = inner.x + inner.width.saturating_sub(total) / 2;
    let save = Rect::new(x, buttons_y, save_width.min(inner.right() - x), 1);
    let cancel_x = (x + save_width + 2).min(inner.right());
    let cancel = Rect::new(
        cancel_x,
        buttons_y,
        cancel_width.min(inner.right() - cancel_x),
        1,
    );
    Some(TodoEditLayout {
        popup,
        rows: Some(TodoEditRows {
            inner,
            input,
            text_area,
            priority: row(below_input + 1),
            link: row(below_input + 2),
            done: row(below_input + 3),
            save,
            cancel,
        }),
    })
}

/// The wrapped rows of the text and the first one the block shows: the view
/// scrolls only once the caret passes its last row, and then keeps the caret
/// on that row.
fn wrapped_view(
    text: &TextField,
    text_area: Rect,
) -> (Vec<super::todo_text::WrappedRow>, usize, (usize, usize)) {
    let rows = wrap_layout(text.text(), usize::from(text_area.width));
    let caret = caret_visual_position(&rows, text.cursor_line(), text.cursor_column());
    let scroll = caret
        .0
        .saturating_sub(usize::from(text_area.height).saturating_sub(1));
    (rows, scroll, caret)
}

fn priority_name(priority: TodoPriority) -> &'static str {
    match priority {
        TodoPriority::High => "high",
        TodoPriority::Normal => "normal",
        TodoPriority::Low => "low",
    }
}

fn contrast(p: &Palette) -> Color {
    match p.panel_bg {
        Color::Reset => p.surface_dim,
        c => c,
    }
}

fn put_field(
    b: &mut Buffer,
    row: Rect,
    name: &str,
    hint: &str,
    value: &[(&str, Style)],
    p: &Palette,
) {
    let mut x = row.x;
    let mut put = |text: &str, style: Style| {
        let width = display_width(text).min(row.right().saturating_sub(x));
        put_text(b, x, row.y, width, text, style);
        x = x.saturating_add(width);
    };
    put(&format!(" {name:<10}"), Style::default().fg(p.overlay0));
    put(&format!("{hint:<5}"), Style::default().fg(p.overlay1));
    for (text, style) in value {
        put(text, *style);
    }
}

pub(super) fn render_todo_edit(
    b: &mut Buffer,
    edit: &ClientTodoEditOverlay,
    todo_color: Option<Color>,
    p: &Palette,
) -> Option<TodoEditLayout> {
    for y in b.area.y..b.area.bottom() {
        for x in b.area.x..b.area.right() {
            let cell = &mut b[(x, y)];
            cell.set_style(cell.style().add_modifier(Modifier::DIM));
        }
    }
    let layout = todo_edit_layout(b.area)?;
    render_panel_shell(b, layout.popup, p.accent, p.panel_bg)?;
    let Some(rows) = layout.rows.as_ref() else {
        return Some(layout);
    };
    let inner = rows.inner;
    let title = match edit.todo_id {
        Some(id) => format!("edit todo/note #{id}"),
        None => "new todo/note".to_owned(),
    };
    put_text(
        b,
        inner.x + 1,
        inner.y,
        inner.width.saturating_sub(2),
        &title,
        Style::default().fg(p.text).add_modifier(Modifier::BOLD),
    );

    let block = Style::default().fg(p.text).bg(p.surface0);
    b.set_style(rows.input, block);
    for y in rows.input.y..rows.input.bottom() {
        for x in rows.input.x..rows.input.right() {
            b[(x, y)].set_symbol(" ");
        }
    }
    let text_area = rows.text_area;
    let (wrapped, scroll, caret) = wrapped_view(&edit.text, text_area);
    let lines: Vec<&str> = edit.text.text().split('\n').collect();
    for (offset, row) in wrapped
        .iter()
        .skip(scroll)
        .take(usize::from(text_area.height))
        .enumerate()
    {
        let Some(line) = lines.get(row.line) else {
            continue;
        };
        put_text(
            b,
            text_area.x,
            text_area.y + offset as u16,
            text_area.width,
            &line[row.start..row.end],
            block,
        );
    }
    // The caret is drawn, not the terminal cursor: the cell under it, or the
    // blank cell after a row's text, in the accent.
    let caret_row = caret.0.saturating_sub(scroll);
    if caret_row < usize::from(text_area.height) && caret.1 < usize::from(text_area.width) {
        let x = text_area.x + caret.1 as u16;
        let y = text_area.y + caret_row as u16;
        if b.area.contains(ratatui::layout::Position::new(x, y)) {
            b[(x, y)].set_style(Style::default().fg(p.surface0).bg(p.accent));
        }
    }

    put_field(
        b,
        rows.priority,
        "priority",
        "⇥",
        &[(
            priority_name(edit.priority),
            Style::default().fg(todo_priority_color(edit.priority, todo_color, p)),
        )],
        p,
    );
    let link_label = edit.link_label();
    let mut link_value = vec![(link_label.as_str(), Style::default().fg(p.blue))];
    if edit.link_target().is_some() {
        link_value.push(("   ^g go", Style::default().fg(p.overlay1)));
    }
    put_field(b, rows.link, "link", "^l", &link_value, p);
    if edit.todo_id.is_some() {
        let done = if edit.done {
            ("yes", Style::default().fg(p.green))
        } else {
            ("no", Style::default().fg(p.overlay1))
        };
        put_field(b, rows.done, "done", "^t", &[done], p);
    }

    for (rect, text, style) in [
        (
            rows.save,
            SAVE_TEXT,
            Style::default()
                .fg(contrast(p))
                .bg(p.accent)
                .add_modifier(Modifier::BOLD),
        ),
        (
            rows.cancel,
            CANCEL_TEXT,
            Style::default()
                .fg(p.text)
                .bg(p.surface0)
                .add_modifier(Modifier::BOLD),
        ),
    ] {
        b.set_style(rect, style);
        put_text(b, rect.x, rect.y, rect.width, text, style);
    }
    Some(layout)
}

fn next_priority(priority: TodoPriority) -> TodoPriority {
    match priority {
        TodoPriority::Low => TodoPriority::Normal,
        TodoPriority::Normal => TodoPriority::High,
        TodoPriority::High => TodoPriority::Low,
    }
}

impl ClientShellState {
    fn todo_edit_mut(&mut self) -> Option<&mut ClientTodoEditOverlay> {
        match self.overlay.as_mut() {
            Some(ClientShellOverlay::TodoEdit(edit)) => Some(edit),
            _ => None,
        }
    }

    /// Open the editor on `pane_id`: composing when `todo` is `None`, else
    /// editing it with its text, cursor at the end.
    pub(super) fn open_todo_editor(
        &mut self,
        pane_id: String,
        todo: Option<TodoInfo>,
        suspended: Option<ClientTodoPanelOverlay>,
    ) {
        let Some(boot_id) = self
            .snapshot
            .as_deref()
            .map(|snapshot| snapshot.boot_id.clone())
        else {
            return;
        };
        let stored_link = todo.as_ref().and_then(|todo| {
            (todo.link_pane_id.is_some() || todo.link_label.is_some()).then(|| StoredTodoLink {
                live_pane_id: todo.link_alive.then(|| todo.link_pane_id.clone()).flatten(),
                label: todo.link_label.clone().unwrap_or_default(),
            })
        });
        self.overlay = Some(ClientShellOverlay::TodoEdit(ClientTodoEditOverlay {
            pane_id,
            boot_id,
            todo_id: todo.as_ref().map(|todo| todo.id),
            text: todo
                .as_ref()
                .map(|todo| TextField::from_text(&todo.text, MAX_TEXT_CHARS))
                .unwrap_or_else(|| TextField::new(MAX_TEXT_CHARS)),
            priority: todo
                .as_ref()
                .map_or(TodoPriority::Normal, |todo| todo.priority),
            done: todo.as_ref().is_some_and(|todo| todo.done),
            link: TodoEditLink::Keep,
            stored_link,
            suspended,
            saving: false,
        }));
    }

    /// Close the editor without writing anything and put back what it was
    /// opened from.
    fn close_todo_editor(&mut self, outcome: &mut ClientShellInput) {
        outcome.repaint = true;
        let Some(ClientShellOverlay::TodoEdit(edit)) = self.overlay.take() else {
            return;
        };
        if let Some(panel) = edit.suspended {
            self.resume_todo_panel(panel, outcome);
        }
    }

    /// Close the editor and everything it suspended when its pane or its
    /// server boot is gone.
    pub(crate) fn tick_todo_editor(&mut self, outcome: &mut ClientShellInput) {
        let Some(ClientShellOverlay::TodoEdit(edit)) = self.overlay.as_ref() else {
            return;
        };
        let alive = self.snapshot.as_deref().is_some_and(|snapshot| {
            snapshot.boot_id == edit.boot_id
                && snapshot
                    .panes
                    .iter()
                    .any(|pane| pane.pane_id == edit.pane_id)
        });
        if !alive {
            self.overlay = None;
            outcome.repaint = true;
        }
    }

    fn save_todo_edit(&mut self, follow: bool, outcome: &mut ClientShellInput) {
        let Some(edit) = self.todo_edit_mut() else {
            return;
        };
        if edit.saving {
            return;
        }
        let text = edit.text.text().trim().to_owned();
        // An empty todo never saves; the modal stays as it is.
        if text.is_empty() {
            return;
        }
        let follow = if follow {
            match edit.link_target() {
                Some(target) => Some(target.to_owned()),
                // Save-and-follow with nowhere to go does nothing at all.
                None => return,
            }
        } else {
            None
        };
        let link_pane_id = match &edit.link {
            TodoEditLink::Set { pane_id, .. } => Some(pane_id.clone()),
            TodoEditLink::Keep | TodoEditLink::Clear => None,
        };
        let method = match edit.todo_id {
            None => Method::TodoAdd(TodoAddParams {
                pane_id: edit.pane_id.clone(),
                text,
                priority: Some(edit.priority),
                link_pane_id,
            }),
            Some(id) => Method::TodoUpdate(TodoUpdateParams {
                pane_id: edit.pane_id.clone(),
                id,
                text: Some(text),
                done: Some(edit.done),
                priority: Some(edit.priority),
                link_pane_id,
                clear_link: edit.link == TodoEditLink::Clear,
            }),
        };
        let kind = PendingEndpointKind::TodoSave {
            pane_id: edit.pane_id.clone(),
            todo_id: edit.todo_id,
            follow,
        };
        if self.push_endpoint_method_with_kind(method, kind, outcome) {
            if let Some(edit) = self.todo_edit_mut() {
                edit.saving = true;
            }
        }
    }

    /// A save landed. Success closes the editor and returns to the panel, or
    /// travels to the link for a save-and-follow; a refusal keeps the editor
    /// and its text open (the notice says why).
    pub(super) fn handle_todo_save_result(
        &mut self,
        pane_id: &str,
        todo_id: Option<u64>,
        follow: Option<String>,
        ok: bool,
        outcome: &mut ClientShellInput,
    ) {
        let Some(edit) = self.todo_edit_mut() else {
            return;
        };
        if edit.pane_id != pane_id || edit.todo_id != todo_id || !edit.saving {
            return;
        }
        edit.saving = false;
        outcome.repaint = true;
        if !ok {
            return;
        }
        match follow {
            Some(target) => {
                self.overlay = None;
                self.push_endpoint_method(
                    Method::PaneFocus(PaneTarget { pane_id: target }),
                    outcome,
                );
            }
            None => self.close_todo_editor(outcome),
        }
    }

    /// Text arriving as text (a paste, an input method's commit): newlines
    /// stay, other control characters go, and only what fits is taken.
    pub(super) fn insert_todo_edit_text(&mut self, text: &str) -> bool {
        let Some(edit) = self.todo_edit_mut() else {
            return false;
        };
        let text: String = text
            .chars()
            .filter(|c| *c == '\n' || !c.is_control())
            .collect();
        edit.text.insert_str(&text);
        true
    }

    /// Keys while the editor is open (fork `handle_pane_todo_edit_key_via_api`):
    /// the modal's own commands first, then the shared multi-line editing set.
    pub(super) fn route_todo_edit_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) {
        if key.kind == crossterm::event::KeyEventKind::Release {
            return;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        outcome.repaint = true;
        match key.code {
            KeyCode::Char('s' | 'S') if ctrl => return self.save_todo_edit(false, outcome),
            KeyCode::Enter if alt => return self.save_todo_edit(false, outcome),
            KeyCode::Esc => return self.close_todo_editor(outcome),
            KeyCode::Tab => {
                if let Some(edit) = self.todo_edit_mut() {
                    edit.priority = next_priority(edit.priority);
                }
                return;
            }
            KeyCode::Char('g' | 'G') if ctrl => return self.save_todo_edit(true, outcome),
            KeyCode::Char('t' | 'T') if ctrl => {
                if let Some(edit) = self.todo_edit_mut() {
                    if edit.todo_id.is_some() {
                        edit.done = !edit.done;
                    }
                }
                return;
            }
            KeyCode::Char('l' | 'L') if ctrl => {
                self.open_todo_link_picker();
                return;
            }
            _ => {}
        }
        let Some(edit) = self.todo_edit_mut() else {
            return;
        };
        // Text the host composed is authoritative (AltGr, dead keys).
        if let Some(text) = key
            .generated_text
            .as_deref()
            .filter(|text| !text.is_empty() && !ctrl && !alt)
        {
            if !matches!(key.code, KeyCode::Enter | KeyCode::Tab) {
                let text: String = text.chars().filter(|c| !c.is_control()).collect();
                for ch in text.chars() {
                    edit.text.insert_char(ch);
                }
                return;
            }
        }
        if let KeyCode::Char(_) = key.code {
            if key.modifiers.difference(KeyModifiers::SHIFT).is_empty() {
                if let Some(ch) = crate::input::keybind_help_text_char(key) {
                    edit.text.insert_char(ch);
                }
                return;
            }
        }
        apply_text_key(
            &mut edit.text,
            KeyEvent::new(key.code, key.modifiers),
            Shape::Multiline,
        );
    }

    /// Mouse while the editor is open: only a left press acts. A press on no
    /// control, inside the modal or out, cancels.
    pub(super) fn route_todo_edit_mouse(
        &mut self,
        mouse: MouseEvent,
        outcome: &mut ClientShellInput,
    ) {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        let Some(rows) = self
            .hits
            .todo_edit
            .as_ref()
            .and_then(|layout| layout.rows.clone())
        else {
            return;
        };
        let point = (mouse.column, mouse.row);
        outcome.repaint = true;
        if super::contains(rows.input, point) {
            let Some(edit) = self.todo_edit_mut() else {
                return;
            };
            let (wrapped, scroll, _) = wrapped_view(&edit.text, rows.text_area);
            let visual_row = scroll + usize::from(mouse.row.saturating_sub(rows.text_area.y));
            let column = usize::from(mouse.column.saturating_sub(rows.text_area.x));
            let (line, column) = visual_to_logical(&wrapped, visual_row, column);
            edit.text.place_cursor(line, column);
        } else if super::contains(rows.priority, point) {
            if let Some(edit) = self.todo_edit_mut() {
                edit.priority = next_priority(edit.priority);
            }
        } else if super::contains(rows.link, point) {
            self.open_todo_link_picker();
        } else if super::contains(rows.done, point) {
            if let Some(edit) = self.todo_edit_mut() {
                if edit.todo_id.is_some() {
                    edit.done = !edit.done;
                }
            }
        } else if super::contains(rows.save, point) {
            self.save_todo_edit(false, outcome);
        } else {
            self.close_todo_editor(outcome);
        }
    }

    /// ctrl+l or a click on the link row: the navigator opens as the link
    /// picker with this editor parked inside it (fork bc502abd).
    fn open_todo_link_picker(&mut self) {
        match self.overlay.take() {
            Some(ClientShellOverlay::TodoEdit(edit)) => {
                self.open_navigator_for(ClientNavigatorPurpose::TodoLink, Some(Box::new(edit)));
            }
            other => self.overlay = other,
        }
    }

    /// The link picker's Enter: a pane row stages that pane, the "no link"
    /// row stages clearing it, and either returns to the editor. Space and
    /// machine rows link nothing and leave the picker open.
    pub(super) fn accept_todo_link(&mut self, target: ClientNavigatorTarget) {
        let link = match target {
            ClientNavigatorTarget::ClearLink => TodoEditLink::Clear,
            ClientNavigatorTarget::Pane { pane_id, .. } => {
                let label = match self.overlay.as_ref() {
                    Some(ClientShellOverlay::Navigator(navigator)) => {
                        render::client_navigator_rows(
                            &self.endpoints,
                            &self.active_endpoint_id,
                            navigator,
                        )
                        .into_iter()
                        .find(|row| {
                            matches!(&row.target, ClientNavigatorTarget::Pane { pane_id: id, .. } if *id == pane_id)
                        })
                        .map(|row| row.label)
                        .unwrap_or_default()
                    }
                    _ => String::new(),
                };
                TodoEditLink::Set { pane_id, label }
            }
            ClientNavigatorTarget::Machine { .. } | ClientNavigatorTarget::Workspace { .. } => {
                return
            }
        };
        if let Some(ClientShellOverlay::Navigator(navigator)) = self.overlay.as_mut() {
            if let Some(edit) = navigator.suspended_todo_edit.as_mut() {
                edit.link = link;
            }
        }
        self.dismiss_navigator();
    }
}
