//! The pane todo panel (fork 7c41f3c2, c88046c2, 0bac78f1, 611316dc,
//! 28576f8d/6903a103/5a326d81, 857670ca): a list of one pane's todos hanging
//! from the pane's top border, with a detail box for text a row cannot show and
//! a footer of actions.
//!
//! The todos are the server's: the panel lists them with `todo.list`, changes
//! them with `todo.update`/`todo.remove`/`todo.clear`, and fetches the list
//! again when the pane's todo revision in the snapshot facts moves. What the
//! client owns is the selection, the scroll and the hovered button.
//!
//! Draw and hit-test share one layout, [`todo_panel_layout`], so a cell that
//! is drawn as a row, a chip or a button is the cell the mouse hits.

use super::render::{display_width, put_text};
use super::todo_edit::SuspendedTodoSurface;
use super::*;
use crate::api::schema::{
    Method, PaneTarget, TodoClearParams, TodoInfo, TodoListParams, TodoRemoveParams,
    TodoUpdateParams,
};
use crate::terminal::todo::TodoPriority;
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::style::Color;

/// Rows the list shows before it scrolls; also how many todos size its width.
const MAX_ROWS: usize = 12;
const MIN_WIDTH: u16 = 30;
const MAX_WIDTH: u16 = 60;
/// Content width an empty panel sizes itself to.
const EMPTY_CONTENT_WIDTH: usize = 16;
/// The blank row above the footer and the footer row itself.
pub(super) const FOOTER_ROWS: u16 = 2;
/// Text rows a detail box draws at most.
const DETAIL_MAX_TEXT_ROWS: u16 = 6;
/// A detail box shorter than this (its two borders and one text row) is not
/// drawn at all.
pub(super) const DETAIL_MIN_ROWS: u16 = 3;
/// Columns the todo's own text keeps whatever its link is called.
const CHIP_MIN_TEXT_COLUMNS: usize = 8;
pub(super) const BUTTON_GAP: u16 = 2;
/// What the detail box says for a selection that hides nothing.
const NOTHING_HIDDEN: &str = "full text in the row";

#[derive(Debug)]
pub(super) struct ClientTodoPanelOverlay {
    pub(super) pane_id: String,
    pub(super) boot_id: String,
    /// In display order, as `todo.list` answers.
    pub(super) todos: Vec<TodoInfo>,
    /// The pane's todo revision the list was fetched at; `None` until the
    /// first list lands.
    pub(super) loaded_revision: Option<u64>,
    /// The snapshot's pane set the list was fetched against: a link's liveness
    /// and public id follow it.
    pub(super) loaded_panes: u64,
    pub(super) list_in_flight: bool,
    /// An index into `todos`, not a todo: after a toggle or a save the list
    /// re-sorts and the index is only re-clamped, as in the fork.
    pub(super) selected: usize,
    pub(super) scroll: usize,
    pub(super) hovered_button: Option<TodoPanelButton>,
}

impl ClientTodoPanelOverlay {
    pub(super) fn new(pane_id: String, boot_id: String) -> Self {
        Self {
            pane_id,
            boot_id,
            todos: Vec::new(),
            loaded_revision: None,
            loaded_panes: 0,
            list_in_flight: false,
            selected: 0,
            scroll: 0,
            hovered_button: None,
        }
    }

    fn selected_todo(&self) -> Option<&TodoInfo> {
        self.todos.get(self.selected)
    }

    fn clamp_selection(&mut self) {
        self.selected = self.selected.min(self.todos.len().saturating_sub(1));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TodoPanelButton {
    Add,
    Toggle,
    Go,
    ClearDone,
    Close,
}

/// Where everything in the panel is, for one frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct TodoPanelLayout {
    pub(super) outer: Rect,
    pub(super) inner: Rect,
    pub(super) detail: Option<Rect>,
    pub(super) list: Rect,
    /// The first todo the list shows.
    pub(super) start: usize,
    pub(super) footer_row: Option<Rect>,
    pub(super) buttons: Vec<(Rect, TodoPanelButton)>,
}

impl TodoPanelLayout {
    fn row_rect(&self, index: usize) -> Option<Rect> {
        let offset = index.checked_sub(self.start)?;
        (offset < usize::from(self.list.height))
            .then(|| Rect::new(self.list.x, self.list.y + offset as u16, self.list.width, 1))
    }

    fn row_at(&self, x: u16, y: u16, len: usize) -> Option<usize> {
        if !super::contains(self.list, (x, y)) {
            return None;
        }
        let index = self.start + usize::from(y - self.list.y);
        (index < len).then_some(index)
    }
}

/// The mark an open todo's glyph is drawn in: `ui.pane_todo_color` when set,
/// else its priority's colour.
pub(super) fn todo_priority_color(
    priority: TodoPriority,
    override_color: Option<Color>,
    p: &Palette,
) -> Color {
    override_color.unwrap_or(match priority {
        TodoPriority::High => p.red,
        TodoPriority::Normal => p.yellow,
        TodoPriority::Low => p.blue,
    })
}

pub(super) fn contrast(p: &Palette) -> Color {
    match p.panel_bg {
        Color::Reset => p.surface_dim,
        c => c,
    }
}

pub(super) fn width_of(text: &str) -> usize {
    usize::from(display_width(text))
}

// -- one todo row (fork src/ui/todo_panel.rs) ---------------------------------

pub(super) fn row_id_text(todo_id: u64) -> String {
    format!("#{todo_id}")
}

/// The part of a row the link chip may occupy: everything left of the id and
/// the space before it.
pub(super) fn row_chip_area(row: Rect, todo_id: u64) -> Rect {
    let reserved = display_width(&row_id_text(todo_id)).saturating_add(1);
    Rect::new(row.x, row.y, row.width.saturating_sub(reserved), row.height)
}

/// The public id a live link leads with; `None` for a dead link.
pub(super) fn live_link_id(todo: &TodoInfo) -> Option<&str> {
    todo.link_alive
        .then_some(todo.link_pane_id.as_deref())
        .flatten()
}

pub(super) fn has_link(todo: &TodoInfo) -> bool {
    todo.link_pane_id.is_some() || todo.link_label.is_some()
}

/// The chip's text before any truncation.
pub(super) fn link_chip_text(public_id: Option<&str>, label: &str) -> String {
    match public_id {
        Some(id) if label.is_empty() => format!(" → {id} "),
        Some(id) => format!(" → {id} · {label} "),
        None => format!(" → {label} "),
    }
}

/// The link chip at the right of `chip_area`, for a todo that carries a link.
pub(super) fn link_chip(chip_area: Rect, todo: &TodoInfo) -> Option<(Rect, String)> {
    if !has_link(todo) {
        return None;
    }
    let public_id = live_link_id(todo);
    let label = todo.link_label.as_deref().unwrap_or("");
    if (label.is_empty() && public_id.is_none()) || chip_area.width < 16 {
        return None;
    }
    let budget = usize::from(chip_area.width).saturating_sub(3 + CHIP_MIN_TEXT_COLUMNS);
    let content = budget.saturating_sub(4);
    let label = match public_id {
        Some(id) => crate::ui::text::truncate_end(label, content.saturating_sub(width_of(id) + 3)),
        None => crate::ui::text::truncate_end(label, content),
    };
    let text = link_chip_text(public_id, &label);
    let width = display_width(&text);
    if width == 0 || width >= chip_area.width {
        return None;
    }
    Some((
        Rect::new(chip_area.x + chip_area.width - width, chip_area.y, width, 1),
        text,
    ))
}

/// What a todo shows on its single row: its first line, with a marker when it
/// holds more.
fn row_text(text: &str, budget: usize) -> String {
    let Some((first, _)) = text.split_once('\n') else {
        return crate::ui::text::truncate_end(text, budget);
    };
    let marker = " ⏎";
    let first = crate::ui::text::truncate_end(first, budget.saturating_sub(width_of(marker)));
    format!("{first}{marker}")
}

fn row_text_budget(row: Rect, todo: &TodoInfo) -> usize {
    let id_width = width_of(&row_id_text(todo.id));
    let chip_width = link_chip(row_chip_area(row, todo.id), todo)
        .map(|(rect, _)| usize::from(rect.width))
        .unwrap_or(0);
    usize::from(row.width).saturating_sub(3 + chip_width + id_width + 1)
}

/// Whether a todo's row hides some of its text: a second line, or a first
/// line wider than the row gives it.
pub(super) fn row_hides_text(row: Rect, todo: &TodoInfo) -> bool {
    todo.text.contains('\n') || width_of(&todo.text) > row_text_budget(row, todo)
}

fn detail_box_rows(text: &str, width: u16) -> u16 {
    let text_width = usize::from(width.saturating_sub(4));
    if text_width == 0 {
        return 0;
    }
    let wrapped = super::todo_text::wrap_layout(text, text_width).len() as u16;
    2 + wrapped.clamp(1, DETAIL_MAX_TEXT_ROWS)
}

/// Rows of detail box the todos need so any of them can be selected without
/// the panel changing height. `row_width` is what a todo's row is drawn in
/// (the panel's list width; the board's, indented), `box_width` the box's.
pub(super) fn detail_rows<'a>(
    todos: impl IntoIterator<Item = &'a TodoInfo>,
    row_width: u16,
    box_width: u16,
) -> u16 {
    let most = 2 + DETAIL_MAX_TEXT_ROWS;
    let row = Rect::new(0, 0, row_width, 1);
    let mut rows = 0;
    for todo in todos {
        if row_hides_text(row, todo) {
            rows = rows.max(detail_box_rows(&todo.text, box_width));
            if rows >= most {
                break;
            }
        }
    }
    rows
}

fn todo_glyph(todo: &TodoInfo) -> &'static str {
    if todo.done {
        return " ✓ ";
    }
    match todo.priority {
        TodoPriority::High => " ▲ ",
        TodoPriority::Normal => " ● ",
        TodoPriority::Low => " ▼ ",
    }
}

pub(super) fn render_row(
    b: &mut Buffer,
    row: Rect,
    todo: &TodoInfo,
    selected: bool,
    todo_color: Option<Color>,
    p: &Palette,
) {
    let band = Style::default().fg(contrast(p)).bg(p.accent);
    let (glyph_style, text_style) = if selected {
        b.set_style(row, band);
        (band, band)
    } else if todo.done {
        (
            Style::default().fg(p.overlay0),
            Style::default()
                .fg(p.overlay0)
                .add_modifier(Modifier::CROSSED_OUT),
        )
    } else {
        (
            Style::default().fg(todo_priority_color(todo.priority, todo_color, p)),
            Style::default().fg(p.text),
        )
    };
    let budget = row_text_budget(row, todo);
    let text = row_text(&todo.text, budget);
    let x = row.x;
    put_text(b, x, row.y, 3.min(row.width), todo_glyph(todo), glyph_style);
    put_text(
        b,
        x.saturating_add(3),
        row.y,
        row.width.saturating_sub(3).min(budget as u16),
        &text,
        text_style,
    );
    let id_text = row_id_text(todo.id);
    let id_width = display_width(&id_text);
    if id_width < row.width {
        let id_style = if selected {
            band
        } else {
            Style::default().fg(p.overlay0)
        };
        put_text(
            b,
            row.x + row.width - id_width,
            row.y,
            id_width,
            &id_text,
            id_style,
        );
    }
    if let Some((chip, text)) = link_chip(row_chip_area(row, todo.id), todo) {
        let style = if selected {
            band
        } else if live_link_id(todo).is_some() {
            Style::default().fg(p.blue)
        } else {
            Style::default().fg(p.overlay0).add_modifier(Modifier::DIM)
        };
        put_text(b, chip.x, chip.y, chip.width, &text, style);
    }
}

// -- geometry -----------------------------------------------------------------

fn button_text(button: TodoPanelButton) -> &'static str {
    match button {
        TodoPanelButton::Add => " a add ",
        TodoPanelButton::Toggle => " spc toggle ",
        TodoPanelButton::Go => " g go ",
        TodoPanelButton::ClearDone => " c clear done ",
        TodoPanelButton::Close => " esc close ",
    }
}

fn drop_rank(button: TodoPanelButton) -> Option<u8> {
    match button {
        TodoPanelButton::Toggle => Some(0),
        TodoPanelButton::ClearDone => Some(1),
        TodoPanelButton::Go => Some(2),
        TodoPanelButton::Add | TodoPanelButton::Close => None,
    }
}

/// Lay out the footer: every button that fits, dropping the lowest drop rank
/// first while the row is too narrow; `add` and `close` never drop.
fn footer_buttons(row: Rect, has_todos: bool, has_live_link: bool) -> Vec<(Rect, TodoPanelButton)> {
    let mut buttons = vec![TodoPanelButton::Add];
    if has_todos {
        buttons.push(TodoPanelButton::Toggle);
    }
    if has_live_link {
        buttons.push(TodoPanelButton::Go);
    }
    if has_todos {
        buttons.push(TodoPanelButton::ClearDone);
    }
    buttons.push(TodoPanelButton::Close);
    let total = |buttons: &[TodoPanelButton]| {
        buttons
            .iter()
            .map(|button| display_width(button_text(*button)))
            .sum::<u16>()
            + BUTTON_GAP * buttons.len().saturating_sub(1) as u16
    };
    while total(&buttons) > row.width {
        let Some(lowest) = buttons.iter().filter_map(|button| drop_rank(*button)).min() else {
            break;
        };
        buttons.retain(|button| drop_rank(*button) != Some(lowest));
    }
    if row.width == 0 {
        return Vec::new();
    }
    let mut x = row.x + row.width.saturating_sub(total(&buttons)) / 2;
    buttons
        .into_iter()
        .map(|button| {
            let width = display_width(button_text(button)).min(row.right().saturating_sub(x));
            let rect = Rect::new(x, row.y, width, 1);
            x = x.saturating_add(width + BUTTON_GAP).min(row.right());
            (rect, button)
        })
        .collect()
}

/// The panel's layout, hanging inside the top of `anchor` (the pane's outer
/// rect) and right-aligned to it, within `screen`.
pub(super) fn todo_panel_layout(
    panel: &ClientTodoPanelOverlay,
    anchor: Rect,
    screen: Rect,
) -> Option<TodoPanelLayout> {
    if screen.width == 0 || screen.height == 0 {
        return None;
    }
    let content = panel
        .todos
        .iter()
        .take(MAX_ROWS)
        .map(|todo| {
            let first = todo.text.split('\n').next().unwrap_or("");
            let chip = if has_link(todo) {
                width_of(&link_chip_text(
                    live_link_id(todo),
                    todo.link_label.as_deref().unwrap_or(""),
                ))
            } else {
                0
            };
            width_of(first) + chip + width_of(&row_id_text(todo.id)) + 1
        })
        .max()
        .unwrap_or(EMPTY_CONTENT_WIDTH)
        + 2
        + 3
        + 1;
    let width = (content.clamp(usize::from(MIN_WIDTH), usize::from(MAX_WIDTH)) as u16)
        .min(screen.width.max(1));
    let list_width = width.saturating_sub(2);
    let requested_detail = detail_rows(&panel.todos, list_width, list_width);
    let rows = panel.todos.len().clamp(1, MAX_ROWS) as u16;
    let height = (rows + 2 + FOOTER_ROWS + requested_detail).min(screen.height.max(1));
    let x = anchor.right().saturating_sub(width).max(screen.x);
    let bottom = screen.bottom().saturating_sub(height);
    let y = anchor.y.saturating_add(1).min(bottom).max(screen.y);
    let outer = Rect::new(x, y, width, height);
    let inner = Rect::new(
        outer.x.saturating_add(1),
        outer.y.saturating_add(1),
        outer.width.saturating_sub(2),
        outer.height.saturating_sub(2),
    );
    let (mut list, footer_row) = if inner.width > 0 && inner.height >= FOOTER_ROWS {
        (
            Rect::new(inner.x, inner.y, inner.width, inner.height - FOOTER_ROWS),
            Some(Rect::new(inner.x, inner.bottom() - 1, inner.width, 1)),
        )
    } else {
        (inner, None)
    };
    let detail_height = requested_detail.min(list.height.saturating_sub(1));
    let detail = (requested_detail > 0 && detail_height >= DETAIL_MIN_ROWS).then(|| {
        let detail = Rect::new(list.x, list.y, list.width, detail_height);
        list = Rect::new(
            list.x,
            list.y + detail_height,
            list.width,
            list.height - detail_height,
        );
        detail
    });
    let visible = usize::from(list.height);
    let start = panel.scroll.min(panel.todos.len().saturating_sub(visible));
    let has_live_link = panel
        .selected_todo()
        .is_some_and(|todo| live_link_id(todo).is_some());
    let buttons = footer_row
        .map(|row| footer_buttons(row, !panel.todos.is_empty(), has_live_link))
        .unwrap_or_default();
    Some(TodoPanelLayout {
        outer,
        inner,
        detail,
        list,
        start,
        footer_row,
        buttons,
    })
}

// -- render -------------------------------------------------------------------

fn render_shell(b: &mut Buffer, rect: Rect, border: Color, bg: Color) -> Option<Rect> {
    if rect.width < 2 || rect.height < 2 {
        return None;
    }
    let fill = Style::default().bg(bg).fg(Color::Reset);
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            b[(x, y)].reset();
            b[(x, y)].set_symbol(" ").set_style(fill);
        }
    }
    let style = Style::default().fg(border).bg(bg);
    for x in rect.x..rect.right() {
        let (top, bottom) = if x == rect.x {
            ("┌", "└")
        } else if x + 1 == rect.right() {
            ("┐", "┘")
        } else {
            ("─", "─")
        };
        b[(x, rect.y)].set_symbol(top).set_style(style);
        b[(x, rect.bottom() - 1)]
            .set_symbol(bottom)
            .set_style(style);
    }
    for y in rect.y + 1..rect.bottom() - 1 {
        b[(rect.x, y)].set_symbol("│").set_style(style);
        b[(rect.right() - 1, y)].set_symbol("│").set_style(style);
    }
    Some(Rect::new(
        rect.x + 1,
        rect.y + 1,
        rect.width - 2,
        rect.height - 2,
    ))
}

/// The panel's border and fill, shared with the todo editor's shell.
pub(super) fn render_panel_shell(
    b: &mut Buffer,
    rect: Rect,
    border: Color,
    bg: Color,
) -> Option<Rect> {
    render_shell(b, rect.intersection(b.area), border, bg)
}

pub(super) fn render_detail_box(b: &mut Buffer, area: Rect, text: Option<&str>, p: &Palette) {
    let Some(inner) = render_panel_shell(b, area, p.overlay0, p.panel_bg) else {
        return;
    };
    let text_area = Rect::new(
        inner.x + 1,
        inner.y,
        inner.width.saturating_sub(2),
        inner.height,
    );
    render_detail_text(b, text_area, text, p);
}

/// The detail's text in `text_area`, wrapped, or a note that the row shows
/// all of it.
pub(super) fn render_detail_text(b: &mut Buffer, text_area: Rect, text: Option<&str>, p: &Palette) {
    if text_area.width == 0 || text_area.height == 0 {
        return;
    }
    let Some(text) = text else {
        put_text(
            b,
            text_area.x,
            text_area.y,
            text_area.width,
            NOTHING_HIDDEN,
            Style::default()
                .fg(p.overlay0)
                .add_modifier(Modifier::ITALIC),
        );
        return;
    };
    let lines: Vec<&str> = text.split('\n').collect();
    let rows = super::todo_text::wrap_layout(text, usize::from(text_area.width));
    for (row, wrapped) in rows.iter().take(usize::from(text_area.height)).enumerate() {
        let Some(line) = lines.get(wrapped.line) else {
            continue;
        };
        put_text(
            b,
            text_area.x,
            text_area.y + row as u16,
            text_area.width,
            &line[wrapped.start..wrapped.end],
            Style::default().fg(p.subtext0),
        );
    }
}

/// Draw the panel and return where everything landed.
pub(super) fn render_todo_panel(
    b: &mut Buffer,
    panel: &ClientTodoPanelOverlay,
    anchor: Rect,
    todo_color: Option<Color>,
    p: &Palette,
) -> Option<TodoPanelLayout> {
    // Nothing until the first list lands, so an empty "no todos" never
    // flashes up in front of a pane that has todos.
    panel.loaded_revision?;
    let layout = todo_panel_layout(panel, anchor, b.area)?;
    render_panel_shell(b, layout.outer, p.accent, p.panel_bg)?;
    let inner = layout.inner;
    if panel.todos.is_empty() {
        put_text(
            b,
            inner.x,
            inner.y,
            inner.width,
            " no todos",
            Style::default().fg(p.overlay0),
        );
    }
    for (index, todo) in panel.todos.iter().enumerate() {
        if let Some(row) = layout.row_rect(index) {
            render_row(b, row, todo, index == panel.selected, todo_color, p);
        }
    }
    if let Some(detail) = layout.detail {
        let row = Rect::new(detail.x, detail.y, detail.width, 1);
        let text = panel
            .selected_todo()
            .filter(|todo| row_hides_text(row, todo))
            .map(|todo| todo.text.as_str());
        render_detail_box(b, detail, text, p);
    }
    for (rect, button) in &layout.buttons {
        let style = if panel.hovered_button == Some(*button) {
            Style::default()
                .fg(contrast(p))
                .bg(p.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(p.text)
                .bg(p.surface0)
                .add_modifier(Modifier::BOLD)
        };
        b.set_style(*rect, style);
        let text = button_text(*button);
        let width = display_width(text).min(rect.width);
        put_text(
            b,
            rect.x + (rect.width - width) / 2,
            rect.y,
            width,
            text,
            style,
        );
    }
    Some(layout)
}

// -- behaviour ----------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PanelAction {
    Add,
    Edit,
    ToggleDone,
    Remove,
    ClearDone,
    FollowLink,
    Close,
}

/// A cheap fingerprint of the snapshot's panes: a link's liveness and public
/// id move with it.
pub(super) fn pane_set_fingerprint(snapshot: &ClientShellSnapshot) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for pane in &snapshot.panes {
        pane.pane_id.hash(&mut hasher);
    }
    hasher.finish()
}

/// The pane's todo revision in the snapshot facts, when the endpoint sends one.
pub(super) fn pane_todo_revision(snapshot: &ClientShellSnapshot, pane_id: &str) -> Option<u64> {
    snapshot
        .resource_facts
        .as_ref()?
        .pane_todos
        .as_ref()?
        .get(pane_id)
        .map(|summary| summary.revision)
}

impl ClientShellState {
    fn todo_panel_mut(&mut self) -> Option<&mut ClientTodoPanelOverlay> {
        match self.overlay.as_mut() {
            Some(ClientShellOverlay::TodoPanel(panel)) => Some(panel),
            _ => None,
        }
    }

    /// Open the panel on `pane_id`, or close it when it is already open — the
    /// pane-border indicator's toggle. Returns whether anything changed.
    pub(super) fn toggle_todo_panel(&mut self, pane_id: &str, outcome: &mut ClientShellInput) {
        if matches!(self.overlay, Some(ClientShellOverlay::TodoPanel(_))) {
            self.close_todo_panel(outcome);
            return;
        }
        self.open_todo_panel(pane_id.to_owned(), outcome);
    }

    pub(super) fn open_todo_panel(&mut self, pane_id: String, outcome: &mut ClientShellInput) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        if !snapshot.panes.iter().any(|pane| pane.pane_id == pane_id) {
            return;
        }
        let panel = ClientTodoPanelOverlay::new(pane_id, snapshot.boot_id.clone());
        self.overlay = Some(ClientShellOverlay::TodoPanel(panel));
        self.request_todo_list(outcome);
        outcome.repaint = true;
    }

    /// Put a panel the editor suspended back, and fetch its list again: the
    /// save that closed the editor changed it.
    pub(super) fn resume_todo_panel(
        &mut self,
        mut panel: ClientTodoPanelOverlay,
        outcome: &mut ClientShellInput,
    ) {
        panel.list_in_flight = false;
        self.overlay = Some(ClientShellOverlay::TodoPanel(panel));
        self.request_todo_list(outcome);
        outcome.repaint = true;
    }

    fn close_todo_panel(&mut self, outcome: &mut ClientShellInput) {
        self.overlay = None;
        outcome.repaint = true;
    }

    fn request_todo_list(&mut self, outcome: &mut ClientShellInput) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        let panes = pane_set_fingerprint(snapshot);
        let Some(ClientShellOverlay::TodoPanel(panel)) = self.overlay.as_ref() else {
            return;
        };
        if panel.list_in_flight {
            return;
        }
        let pane_id = panel.pane_id.clone();
        // An endpoint without todo facts reads as revision 0, so the list is
        // fetched once and then on the panel's own changes.
        let revision = Some(pane_todo_revision(snapshot, &pane_id).unwrap_or(0));
        if self.push_endpoint_method_with_kind(
            Method::TodoList(TodoListParams {
                pane_id: Some(pane_id.clone()),
            }),
            PendingEndpointKind::TodoList {
                pane_id,
                revision,
                panes,
            },
            outcome,
        ) {
            if let Some(panel) = self.todo_panel_mut() {
                panel.list_in_flight = true;
            }
        }
    }

    /// Keep an open panel in step with the server: close it when its pane or
    /// its server boot is gone, fetch the list again when the pane's todos or
    /// the pane set changed. Runs from the client's timer, never from render.
    pub(crate) fn tick_todo_panel(&mut self, outcome: &mut ClientShellInput) {
        let Some(ClientShellOverlay::TodoPanel(panel)) = self.overlay.as_ref() else {
            return;
        };
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        if snapshot.boot_id != panel.boot_id
            || !snapshot
                .panes
                .iter()
                .any(|pane| pane.pane_id == panel.pane_id)
        {
            self.close_todo_panel(outcome);
            return;
        }
        if panel.list_in_flight {
            return;
        }
        let revision = pane_todo_revision(snapshot, &panel.pane_id);
        let stale = panel.loaded_revision.is_none()
            || revision.is_some_and(|revision| Some(revision) != panel.loaded_revision)
            || pane_set_fingerprint(snapshot) != panel.loaded_panes;
        if stale {
            self.request_todo_list(outcome);
        }
    }

    pub(super) fn handle_todo_list_result(
        &mut self,
        pane_id: &str,
        revision: Option<u64>,
        panes: u64,
        result: Result<crate::api::schema::ResponseResult, ClientShellEndpointError>,
    ) -> bool {
        let Some(panel) = self.todo_panel_mut() else {
            return false;
        };
        if panel.pane_id != pane_id {
            return false;
        }
        panel.list_in_flight = false;
        let Ok(crate::api::schema::ResponseResult::TodoList { todos }) = result else {
            // Keep what is on screen; the next tick asks again only when the
            // revision moves, so a failing list is not retried in a loop.
            panel.loaded_revision = revision;
            panel.loaded_panes = panes;
            return false;
        };
        panel.todos = todos;
        panel.loaded_revision = revision;
        panel.loaded_panes = panes;
        panel.clamp_selection();
        true
    }

    fn todo_panel_visible_rows(&self) -> usize {
        self.hits
            .todo_panel
            .as_ref()
            .map(|layout| usize::from(layout.list.height))
            .unwrap_or(MAX_ROWS)
    }

    fn move_todo_panel_selection(&mut self, target: impl FnOnce(usize, usize, usize) -> usize) {
        let visible = self.todo_panel_visible_rows();
        let Some(panel) = self.todo_panel_mut() else {
            return;
        };
        let len = panel.todos.len();
        if len == 0 {
            panel.selected = 0;
            panel.scroll = 0;
            return;
        }
        panel.selected = target(panel.selected, len - 1, visible).min(len - 1);
        // Reveal by the minimum, never recentre.
        if panel.selected < panel.scroll {
            panel.scroll = panel.selected;
        } else if visible > 0 && panel.selected >= panel.scroll + visible {
            panel.scroll = panel.selected + 1 - visible;
        }
    }

    fn apply_todo_panel_action(&mut self, action: PanelAction, outcome: &mut ClientShellInput) {
        outcome.repaint = true;
        let Some(ClientShellOverlay::TodoPanel(panel)) = self.overlay.as_ref() else {
            return;
        };
        let pane_id = panel.pane_id.clone();
        let selected = panel.selected_todo().cloned();
        match action {
            PanelAction::Close => self.close_todo_panel(outcome),
            PanelAction::Add => {
                let Some(ClientShellOverlay::TodoPanel(panel)) = self.overlay.take() else {
                    return;
                };
                self.open_todo_editor(pane_id, None, Some(SuspendedTodoSurface::Panel(panel)));
            }
            PanelAction::Edit => {
                let Some(todo) = selected else {
                    return;
                };
                let Some(ClientShellOverlay::TodoPanel(panel)) = self.overlay.take() else {
                    return;
                };
                self.open_todo_editor(
                    pane_id,
                    Some(todo),
                    Some(SuspendedTodoSurface::Panel(panel)),
                );
            }
            PanelAction::ToggleDone => {
                let Some(todo) = selected else {
                    return;
                };
                self.push_todo_mutation(
                    Method::TodoUpdate(TodoUpdateParams {
                        pane_id: pane_id.clone(),
                        id: todo.id,
                        text: None,
                        done: Some(!todo.done),
                        priority: None,
                        link_pane_id: None,
                        clear_link: false,
                    }),
                    pane_id,
                    outcome,
                );
            }
            PanelAction::Remove => {
                let Some(todo) = selected else {
                    return;
                };
                self.push_todo_mutation(
                    Method::TodoRemove(TodoRemoveParams {
                        pane_id: pane_id.clone(),
                        id: todo.id,
                    }),
                    pane_id,
                    outcome,
                );
            }
            PanelAction::ClearDone => {
                self.push_todo_mutation(
                    Method::TodoClear(TodoClearParams {
                        pane_id: pane_id.clone(),
                        done_only: true,
                    }),
                    pane_id,
                    outcome,
                );
            }
            PanelAction::FollowLink => {
                let Some(target) = selected.as_ref().and_then(live_link_id) else {
                    return;
                };
                let target = target.to_owned();
                self.close_todo_panel(outcome);
                self.push_endpoint_method(
                    Method::PaneFocus(PaneTarget { pane_id: target }),
                    outcome,
                );
            }
        }
    }

    pub(super) fn push_todo_mutation(
        &mut self,
        method: Method,
        pane_id: String,
        outcome: &mut ClientShellInput,
    ) {
        self.push_endpoint_method_with_kind(
            method,
            PendingEndpointKind::TodoMutation { pane_id },
            outcome,
        );
    }

    /// After a change lands, fetch the list at once rather than waiting for
    /// the snapshot that carries the new revision.
    pub(super) fn handle_todo_mutation_result(
        &mut self,
        pane_id: &str,
        ok: bool,
        outcome: &mut ClientShellInput,
    ) {
        if !ok {
            return;
        }
        // The board lists every pane, so a change to any of them moves it.
        if matches!(self.overlay, Some(ClientShellOverlay::TodoBoard(_))) {
            self.request_todo_board_list(outcome);
            return;
        }
        if self
            .todo_panel_mut()
            .is_some_and(|panel| panel.pane_id == pane_id)
        {
            self.request_todo_list(outcome);
            if let Some(panel) = self.todo_panel_mut() {
                panel.clamp_selection();
            }
        }
    }

    /// Keys while the panel is open (fork `handle_pane_todos_key_via_api`).
    pub(super) fn route_todo_panel_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) {
        if key.kind == crossterm::event::KeyEventKind::Release {
            return;
        }
        // A shifted letter does what its lowercase form does.
        let (code, modifiers) = match key.code {
            KeyCode::Char(c) if key.modifiers == KeyModifiers::SHIFT && c.is_ascii_alphabetic() => {
                (KeyCode::Char(c.to_ascii_lowercase()), KeyModifiers::NONE)
            }
            code => (code, key.modifiers),
        };
        let bare = modifiers.is_empty();
        let ctrl = modifiers == KeyModifiers::CONTROL;
        let action = match code {
            KeyCode::Enter => Some(PanelAction::Edit),
            KeyCode::Esc => Some(PanelAction::Close),
            KeyCode::Char('a') if bare => Some(PanelAction::Add),
            KeyCode::Char(' ') if bare => Some(PanelAction::ToggleDone),
            KeyCode::Char('g') if bare => Some(PanelAction::FollowLink),
            KeyCode::Char('d') if bare => Some(PanelAction::Remove),
            KeyCode::Char('c') if bare => Some(PanelAction::ClearDone),
            KeyCode::Char('q') if bare => Some(PanelAction::Close),
            _ => None,
        };
        if let Some(action) = action {
            self.apply_todo_panel_action(action, outcome);
            return;
        }
        let half = |visible: usize| (visible / 2).max(1);
        let moved = match code {
            KeyCode::Up if bare => {
                self.move_todo_panel_selection(|selected, _, _| selected.saturating_sub(1));
                true
            }
            KeyCode::Char('k' | 'p') if ctrl => {
                self.move_todo_panel_selection(|selected, _, _| selected.saturating_sub(1));
                true
            }
            KeyCode::Char('k') if bare => {
                self.move_todo_panel_selection(|selected, _, _| selected.saturating_sub(1));
                true
            }
            KeyCode::Down if bare => {
                self.move_todo_panel_selection(|selected, last, _| (selected + 1).min(last));
                true
            }
            KeyCode::Char('j' | 'n') if ctrl => {
                self.move_todo_panel_selection(|selected, last, _| (selected + 1).min(last));
                true
            }
            KeyCode::Char('j') if bare => {
                self.move_todo_panel_selection(|selected, last, _| (selected + 1).min(last));
                true
            }
            KeyCode::Char('u') if ctrl => {
                self.move_todo_panel_selection(|selected, _, visible| {
                    selected.saturating_sub(half(visible))
                });
                true
            }
            KeyCode::Char('d') if ctrl => {
                self.move_todo_panel_selection(|selected, last, visible| {
                    (selected + half(visible)).min(last)
                });
                true
            }
            KeyCode::Home if bare => {
                self.move_todo_panel_selection(|_, _, _| 0);
                true
            }
            KeyCode::End if bare => {
                self.move_todo_panel_selection(|_, last, _| last);
                true
            }
            _ => false,
        };
        outcome.repaint |= moved;
    }

    /// Mouse while the panel is open (fork mouse.rs, `Mode::PaneTodos`).
    /// Every event stops here: nothing reaches the panes beneath.
    pub(super) fn route_todo_panel_mouse(
        &mut self,
        mouse: MouseEvent,
        outcome: &mut ClientShellInput,
    ) {
        let Some(layout) = self.hits.todo_panel.clone() else {
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                self.close_todo_panel(outcome);
            }
            return;
        };
        let len = self.todo_panel_mut().map_or(0, |panel| panel.todos.len());
        let point = (mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::Moved => {
                let row = layout.row_at(mouse.column, mouse.row, len);
                let hovered = layout
                    .buttons
                    .iter()
                    .find(|(rect, _)| super::contains(*rect, point))
                    .map(|(_, button)| *button);
                if let Some(panel) = self.todo_panel_mut() {
                    if let Some(row) = row {
                        outcome.repaint |= panel.selected != row;
                        panel.selected = row;
                    }
                    outcome.repaint |= panel.hovered_button != hovered;
                    panel.hovered_button = hovered;
                }
            }
            MouseEventKind::ScrollUp => {
                self.move_todo_panel_selection(|selected, _, _| selected.saturating_sub(1));
                outcome.repaint = true;
            }
            MouseEventKind::ScrollDown => {
                self.move_todo_panel_selection(|selected, last, _| (selected + 1).min(last));
                outcome.repaint = true;
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(index) = layout.row_at(mouse.column, mouse.row, len) {
                    let on_chip = self.todo_panel_mut().and_then(|panel| {
                        panel.selected = index;
                        let row = layout.row_rect(index)?;
                        let todo = panel.todos.get(index)?;
                        let (chip, _) = link_chip(row_chip_area(row, todo.id), todo)?;
                        Some(super::contains(chip, point))
                    });
                    let action = if on_chip == Some(true) {
                        PanelAction::FollowLink
                    } else {
                        PanelAction::Edit
                    };
                    self.apply_todo_panel_action(action, outcome);
                    return;
                }
                if let Some((_, button)) = layout
                    .buttons
                    .iter()
                    .find(|(rect, _)| super::contains(*rect, point))
                {
                    let action = match button {
                        TodoPanelButton::Add => PanelAction::Add,
                        TodoPanelButton::Toggle => PanelAction::ToggleDone,
                        TodoPanelButton::Go => PanelAction::FollowLink,
                        TodoPanelButton::ClearDone => PanelAction::ClearDone,
                        TodoPanelButton::Close => PanelAction::Close,
                    };
                    self.apply_todo_panel_action(action, outcome);
                    return;
                }
                // A click on the footer's row that lands on no button is a near
                // miss, inert even outside the panel.
                if layout.footer_row.is_some_and(|row| mouse.row == row.y) {
                    return;
                }
                if !super::contains(layout.outer, point) {
                    self.close_todo_panel(outcome);
                }
            }
            _ => {}
        }
    }
}
