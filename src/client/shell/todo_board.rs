//! The session todo board, `todos/notes` (fork 6e9d11ef, f55bfc4c, 7458d911,
//! ef8adeac, 611316dc, 73c2b2a0, 18d15145, 5a326d81, 51f40966, 385eac83): every
//! pane's todos in one centred modal, grouped by the pane that owns them, with
//! a search row, a preview for text a row cannot show and a footer of
//! actions. Its look is docs/ui-style.md's (fork issue 174): the count on the
//! title row, the search as the subtitle, a rule, the list, the preview under
//! the list after a rule with no frame of its own, and `↵ open pane` as the
//! primary chip.
//!
//! A todo is written in a pane but read across panes, when deciding what to
//! pick up next. The board is the pane panel widened: its rows are the panel's
//! rows, its footer speaks the panel's language, and `enter` goes to the todo's
//! owner pane instead of editing it (`e` edits).
//!
//! The todos are the server's: the board lists them all with `todo.list` and
//! no pane, changes them with `todo.update`/`todo.remove`/`todo.clear` against
//! each todo's owner, and fetches the list again when the snapshot's todo
//! revisions move. What the client owns is the grouping, the filter, the
//! selection, the scroll and the hovered button.
//!
//! Draw and hit-test share one layout, [`todo_board_layout`], so a cell that is
//! drawn as a row, a chip or a button is the cell the mouse hits.

use super::move_picker::{
    list_chord, matches_query, pane_display_label, pane_id_label, render_search_row, reveal_scroll,
    Chord,
};
use super::render::{display_width, put_text};
use super::todo_edit::SuspendedTodoSurface;
use super::todo_panel::{
    contrast, detail_rows, has_link, link_chip, link_chip_text, live_link_id, pane_set_fingerprint,
    render_detail_text, render_panel_shell, render_row, row_chip_area, row_hides_text, row_id_text,
    width_of, BUTTON_GAP, DETAIL_MIN_ROWS, FOOTER_ROWS,
};
use super::todo_text::{apply_text_key, Shape, TextField};
use super::*;
use crate::api::schema::{
    Method, PaneTarget, ResponseResult, TodoClearParams, TodoInfo, TodoListParams,
    TodoRemoveParams, TodoUpdateParams,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::style::Color;

/// Columns the board asks for before its own content has a say.
const MIN_WIDTH: u16 = 80;
/// The widest a centred list dialog grows.
const MAX_WIDTH: u16 = 120;
/// Columns a group's todos are drawn in from its heading. Applied by
/// narrowing the rect a row is drawn into, so the row renderer is the
/// panel's, untouched; headings are not indented.
const TODO_INDENT: u16 = 2;
/// Title, search, and the rule under them.
const HEADER_ROWS: u16 = 3;
const SEARCH_MAX_CHARS: usize = 256;

/// One row of the board. Pane headings are drawn but never selected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum TodoBoardItem {
    PaneHeading {
        /// The owning space's name: where the work lives is the first thing
        /// to decide between.
        space: String,
        label: String,
    },
    /// An index into the board's `todos`.
    Todo(usize),
    /// The blank row between one group and the next. A row like any other,
    /// so the scroll window, the selection and the hit-test all rest on list
    /// row n being item n.
    GroupGap,
}

#[derive(Debug)]
pub(super) struct ClientTodoBoardOverlay {
    pub(super) boot_id: String,
    /// Every pane's todos as `todo.list` answers: consecutive by pane, each
    /// pane in its panel order.
    pub(super) todos: Vec<TodoInfo>,
    /// Every group. The board is sized from this, so a query never resizes it.
    pub(super) all_items: Vec<TodoBoardItem>,
    /// `all_items` narrowed by the search.
    pub(super) items: Vec<TodoBoardItem>,
    /// An index into `items`, always on a todo while there is one.
    pub(super) selected: usize,
    pub(super) scroll: usize,
    pub(super) hovered_button: Option<TodoBoardButton>,
    pub(super) search: TextField,
    pub(super) search_focused: bool,
    /// The snapshot's todo state the list was fetched against; `None` until
    /// the first list lands.
    pub(super) loaded_key: Option<u64>,
    pub(super) list_in_flight: bool,
    /// A change landed while a list was on its way: ask again when it does.
    pub(super) refetch_wanted: bool,
}

impl ClientTodoBoardOverlay {
    pub(super) fn new(boot_id: String) -> Self {
        Self {
            boot_id,
            todos: Vec::new(),
            all_items: Vec::new(),
            items: Vec::new(),
            selected: 0,
            scroll: 0,
            hovered_button: None,
            search: TextField::new(SEARCH_MAX_CHARS),
            search_focused: false,
            loaded_key: None,
            list_in_flight: false,
            refetch_wanted: false,
        }
    }

    /// The query as matched: surrounding blanks do not count.
    fn query(&self) -> &str {
        self.search.text().trim()
    }

    pub(super) fn todo_at(&self, index: usize) -> Option<&TodoInfo> {
        match self.items.get(index) {
            Some(TodoBoardItem::Todo(todo)) => self.todos.get(*todo),
            _ => None,
        }
    }

    pub(super) fn selected_todo(&self) -> Option<&TodoInfo> {
        self.todo_at(self.selected)
    }

    /// Point the selection at `index` only when it holds a todo, so a click
    /// cannot park it on a heading.
    fn select_todo(&mut self, index: usize) -> bool {
        if self.todo_at(index).is_none() {
            return false;
        }
        self.selected = index;
        true
    }

    fn has_todos(&self) -> bool {
        self.items
            .iter()
            .any(|item| matches!(item, TodoBoardItem::Todo(_)))
    }

    /// The todos on the board, whatever the query: the count the search row
    /// shows.
    fn todo_count(&self) -> usize {
        self.all_items
            .iter()
            .filter(|item| matches!(item, TodoBoardItem::Todo(_)))
            .count()
    }

    /// The nearest todo at or past `target`, searching the direction of
    /// travel first and falling back the other way at either end, so no
    /// movement parks on a heading or a gap.
    fn nearest_todo(&self, target: usize, forward: bool) -> Option<usize> {
        let len = self.items.len();
        let ahead = if forward {
            (target..len).find(|index| self.todo_at(*index).is_some())
        } else {
            (0..=target.min(len.saturating_sub(1)))
                .rev()
                .find(|index| self.todo_at(*index).is_some())
        };
        ahead.or_else(|| {
            if forward {
                (0..target.min(len))
                    .rev()
                    .find(|index| self.todo_at(*index).is_some())
            } else {
                (target.saturating_add(1)..len).find(|index| self.todo_at(*index).is_some())
            }
        })
    }

    /// The heading that labels the item at `index`: the nearest one at or
    /// before it. Scrolling keeps it on screen with the selection, because a
    /// selected row whose heading has scrolled away does not say which pane
    /// it belongs to.
    fn group_heading_index(&self, index: usize) -> usize {
        (0..=index.min(self.items.len().saturating_sub(1)))
            .rev()
            .find(|index| {
                matches!(
                    self.items.get(*index),
                    Some(TodoBoardItem::PaneHeading { .. })
                )
            })
            .unwrap_or(0)
    }

    /// Move the selection by `delta` rows, landing on a todo rather than on a
    /// heading, and keep it (and its heading, when both fit) in view.
    fn move_selection_by(&mut self, delta: isize, visible: usize) {
        let len = self.items.len();
        let target = self
            .selected
            .saturating_add_signed(delta)
            .min(len.saturating_sub(1));
        if let Some(index) = self.nearest_todo(target, delta >= 0) {
            self.selected = index;
        }
        let heading = self.group_heading_index(self.selected);
        self.scroll = reveal_scroll_with_context(self.scroll, heading, self.selected, visible, len);
    }

    /// Move the visible window without moving the selection: the wheel on a
    /// list long enough to scan.
    fn scroll_by(&mut self, delta: isize, visible: usize) {
        let max = self.items.len().saturating_sub(visible);
        self.scroll = self.scroll.saturating_add_signed(delta).min(max);
    }

    /// `all_items` narrowed by the query. A todo matches on its pane heading,
    /// its text and its `#id` together, so `infra deploy` finds infra's deploy
    /// todo; a heading stays while any of its todos match and a matching
    /// heading keeps all of them. Gaps separate the groups that survive.
    fn filtered(&self) -> Vec<TodoBoardItem> {
        let query = self.query();
        if query.is_empty() {
            return self.all_items.clone();
        }
        let mut items = Vec::new();
        let mut index = 0;
        while index < self.all_items.len() {
            let TodoBoardItem::PaneHeading { space, label } = &self.all_items[index] else {
                index += 1;
                continue;
            };
            let heading = heading_text(space, label);
            let mut end = index + 1;
            let mut kept = Vec::new();
            while let Some(TodoBoardItem::Todo(todo)) = self.all_items.get(end) {
                let matched = self.todos.get(*todo).is_some_and(|todo| {
                    matches_query(&format!("{heading} {} #{}", todo.text, todo.id), query)
                });
                if matched {
                    kept.push(self.all_items[end].clone());
                }
                end += 1;
            }
            if !kept.is_empty() {
                if !items.is_empty() {
                    items.push(TodoBoardItem::GroupGap);
                }
                items.push(self.all_items[index].clone());
                items.append(&mut kept);
            }
            index = end;
        }
        items
    }

    /// Narrow the list after the query changed and select the first match.
    fn refilter(&mut self, visible: usize) {
        self.items = self.filtered();
        let first = self.nearest_todo(0, true).unwrap_or(0);
        let heading = self.group_heading_index(first);
        self.selected = first;
        self.scroll = reveal_scroll_with_context(0, heading, first, visible, self.items.len());
    }

    /// Take a fresh list. The first one selects the first todo; later ones
    /// keep the selection on the same todo, else on the nearest one from where
    /// it was, and leave the scroll alone.
    fn apply_list(&mut self, todos: Vec<TodoInfo>, snapshot: &ClientShellSnapshot) {
        let first = self.loaded_key.is_none();
        let previous = self
            .selected_todo()
            .map(|todo| (todo.pane_id.clone(), todo.id));
        let index = self.selected;
        self.todos = todos;
        self.all_items = board_items(&self.todos, snapshot);
        self.items = self.filtered();
        if first {
            self.selected = self.nearest_todo(0, true).unwrap_or(0);
            self.scroll = 0;
            return;
        }
        let same = previous.and_then(|(pane_id, id)| {
            self.items.iter().position(|item| {
                matches!(item, TodoBoardItem::Todo(todo)
                    if self.todos.get(*todo).is_some_and(|todo| todo.pane_id == pane_id && todo.id == id))
            })
        });
        self.selected = same.or_else(|| self.nearest_todo(index, true)).unwrap_or(0);
    }
}

/// The scroll that keeps `index` in a `visible`-row window and `context`
/// (its group's heading) too when the two fit together. The selection always
/// wins: a group taller than the window shows the selection rather than the
/// heading above it. Without the context, a list whose first row is never
/// selectable can never be scrolled back to it.
fn reveal_scroll_with_context(
    scroll: usize,
    context: usize,
    index: usize,
    visible: usize,
    len: usize,
) -> usize {
    let scroll = reveal_scroll(scroll, index, visible, len);
    if visible == 0 || context >= scroll {
        return scroll;
    }
    if index.saturating_sub(context) < visible {
        return context;
    }
    scroll
}

/// How far a chord moves the selection, in rows (headings and gaps count).
fn chord_delta(chord: Chord, selected: usize, visible: usize, len: usize) -> isize {
    let half = (visible / 2).max(1);
    let last = len.saturating_sub(1);
    let target = match chord {
        Chord::Prev => selected.saturating_sub(1),
        Chord::Next => selected.saturating_add(1).min(last),
        Chord::HalfPageUp => selected.saturating_sub(half),
        Chord::HalfPageDown => selected.saturating_add(half).min(last),
        Chord::First => 0,
        Chord::Last => last,
    };
    target as isize - selected as isize
}

/// A group heading's text: the space, then the pane's label, as
/// `infra · imap-jmap-mcp`. No addressable identifier: what a heading is for is
/// recognising the pane, which the space and the label do.
pub(super) fn heading_text(space: &str, label: &str) -> String {
    match (space.is_empty(), label.is_empty()) {
        (false, false) => format!(" {space} · {label}"),
        (false, true) => format!(" {space}"),
        (true, false) => format!(" {label}"),
        (true, true) => String::new(),
    }
}

/// The space and pane names a heading shows. A pane the snapshot does not
/// hold (the list and the snapshot raced) keeps its todos under its id's
/// fallback name and no space.
fn heading_parts(snapshot: &ClientShellSnapshot, pane_id: &str) -> (String, String) {
    let Some(pane) = snapshot.panes.iter().find(|pane| pane.pane_id == pane_id) else {
        return (String::new(), pane_id_label(pane_id));
    };
    let space = snapshot
        .workspaces
        .iter()
        .find(|workspace| workspace.workspace_id == pane.workspace_id)
        .map(|workspace| workspace.label.clone())
        .unwrap_or_default();
    (space, pane_display_label(snapshot, pane))
}

/// The board's rows for a list: per pane with todos, a gap (never above the
/// first group), its heading, then its todos.
fn board_items(todos: &[TodoInfo], snapshot: &ClientShellSnapshot) -> Vec<TodoBoardItem> {
    let mut items = Vec::new();
    let mut index = 0;
    while index < todos.len() {
        let pane_id = &todos[index].pane_id;
        let mut end = index;
        while end < todos.len() && todos[end].pane_id == *pane_id {
            end += 1;
        }
        if !items.is_empty() {
            items.push(TodoBoardItem::GroupGap);
        }
        let (space, label) = heading_parts(snapshot, pane_id);
        items.push(TodoBoardItem::PaneHeading { space, label });
        items.extend((index..end).map(TodoBoardItem::Todo));
        index = end;
    }
    items
}

/// What the snapshot says about every pane's todos: moves whenever any
/// pane's todos or the pane set do.
fn board_key(snapshot: &ClientShellSnapshot) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    pane_set_fingerprint(snapshot).hash(&mut hasher);
    if let Some(todos) = snapshot
        .resource_facts
        .as_ref()
        .and_then(|facts| facts.pane_todos.as_ref())
    {
        for (pane_id, summary) in todos {
            pane_id.hash(&mut hasher);
            summary.revision.hash(&mut hasher);
        }
    }
    hasher.finish()
}

// -- footer ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TodoBoardButton {
    Open,
    Toggle,
    Go,
    ClearDone,
    Close,
}

fn button_text(button: TodoBoardButton) -> &'static str {
    match button {
        TodoBoardButton::Open => " ↵ open pane ",
        TodoBoardButton::Toggle => " spc toggle ",
        TodoBoardButton::Go => " g go ",
        TodoBoardButton::ClearDone => " c clear done ",
        TodoBoardButton::Close => " esc close ",
    }
}

/// Dropped in ascending order while the row is too narrow; `None` never drops:
/// `open` is why the board exists over the CLI's flat listing, and an overlay
/// that could not be dismissed is the dead end a footer prevents.
fn drop_rank(button: TodoBoardButton) -> Option<u8> {
    match button {
        TodoBoardButton::Toggle => Some(0),
        TodoBoardButton::ClearDone => Some(1),
        TodoBoardButton::Go => Some(2),
        TodoBoardButton::Open | TodoBoardButton::Close => None,
    }
}

/// The boxes the footer would like, in render order. `toggle` and
/// `clear done` are absent when no pane holds a todo, `go` unless the
/// selected todo's link resolves. There is no `add`: adding is pane-scoped.
fn footer_set(has_todos: bool, has_live_link: bool) -> Vec<TodoBoardButton> {
    let mut buttons = vec![TodoBoardButton::Open];
    if has_todos {
        buttons.push(TodoBoardButton::Toggle);
    }
    if has_live_link {
        buttons.push(TodoBoardButton::Go);
    }
    if has_todos {
        buttons.push(TodoBoardButton::ClearDone);
    }
    buttons.push(TodoBoardButton::Close);
    buttons
}

fn buttons_width(buttons: &[TodoBoardButton]) -> u16 {
    buttons
        .iter()
        .map(|button| display_width(button_text(*button)))
        .sum::<u16>()
        + BUTTON_GAP * buttons.len().saturating_sub(1) as u16
}

/// Lay the footer out centred on `row`, dropping the lowest drop rank first
/// while it is too narrow; the boxes that never drop are laid out anyway.
fn footer_buttons(row: Rect, buttons: Vec<TodoBoardButton>) -> Vec<(Rect, TodoBoardButton)> {
    if row.width == 0 || row.height == 0 {
        return Vec::new();
    }
    let mut buttons = buttons;
    while buttons_width(&buttons) > row.width {
        let Some(lowest) = buttons.iter().filter_map(|button| drop_rank(*button)).min() else {
            break;
        };
        buttons.retain(|button| drop_rank(*button) != Some(lowest));
    }
    let mut x = row.x + row.width.saturating_sub(buttons_width(&buttons)) / 2;
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

// -- geometry -------------------------------------------------------------------

/// Where everything in the board is, for one frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct TodoBoardLayout {
    pub(super) outer: Rect,
    /// The title, the search and the rule.
    pub(super) header: Rect,
    /// The preview under the list: its rule, then the selected todo's text.
    pub(super) detail: Option<Rect>,
    pub(super) list: Rect,
    pub(super) footer_row: Option<Rect>,
    pub(super) buttons: Vec<(Rect, TodoBoardButton)>,
    /// The first item drawn on the list's first row.
    pub(super) start: usize,
}

impl TodoBoardLayout {
    /// Header row `index` (0 is the title).
    pub(super) fn header_row(&self, index: u16) -> Rect {
        Rect::new(self.header.x, self.header.y + index, self.header.width, 1)
    }

    pub(super) fn search_row(&self) -> Rect {
        self.header_row(1)
    }

    fn row_at(&self, x: u16, y: u16, len: usize) -> Option<usize> {
        if !super::contains(self.list, (x, y)) {
            return None;
        }
        let index = self.start + usize::from(y - self.list.y);
        (index < len).then_some(index)
    }
}

/// A board row narrowed to where its todo is drawn.
fn todo_rect(row: Rect) -> Rect {
    let indent = TODO_INDENT.min(row.width);
    Rect::new(row.x + indent, row.y, row.width - indent, row.height)
}

/// The widest row the board would like to draw, chrome included: a heading's
/// full text, or a todo's indent, state glyph, first line, link chip and id.
fn content_width(board: &ClientTodoBoardOverlay) -> usize {
    let widest = board
        .all_items
        .iter()
        .map(|item| match item {
            // A blank row asks for nothing.
            TodoBoardItem::GroupGap => 0,
            TodoBoardItem::PaneHeading { space, label } => width_of(&heading_text(space, label)),
            TodoBoardItem::Todo(todo) => {
                let Some(todo) = board.todos.get(*todo) else {
                    return 0;
                };
                let chip = if has_link(todo) {
                    width_of(&link_chip_text(
                        live_link_id(todo),
                        todo.link_label.as_deref().unwrap_or(""),
                    ))
                } else {
                    0
                };
                let first = todo.text.split('\n').next().unwrap_or("");
                usize::from(TODO_INDENT)
                    + 3
                    + width_of(first)
                    + chip
                    + width_of(&row_id_text(todo.id))
                    + 1
            }
        })
        .max()
        .unwrap_or(0);
    // Two borders and a trailing space.
    widest + 3
}

/// Centred on the screen and sized from every row, so a query never resizes
/// it; it grows to its content in both directions and lets the screen clamp.
pub(super) fn todo_board_layout(
    board: &ClientTodoBoardOverlay,
    screen: Rect,
) -> Option<TodoBoardLayout> {
    if screen.width == 0 || screen.height == 0 {
        return None;
    }
    let footer_floor = usize::from(buttons_width(&footer_set(true, true))) + 2;
    let width = (content_width(board)
        .max(footer_floor)
        .clamp(usize::from(MIN_WIDTH), usize::from(MAX_WIDTH)) as u16)
        .min(screen.width.max(1));
    // The preview is sized from every todo, so it neither appears nor
    // resizes with the query or the selection. It is a rule and the text
    // rows, inset one column: the rows of a box two columns wider, less one.
    let list_width = width.saturating_sub(2);
    let row_width = list_width.saturating_sub(TODO_INDENT);
    let requested_detail = detail_rows(
        board.all_items.iter().filter_map(|item| match item {
            TodoBoardItem::Todo(todo) => board.todos.get(*todo),
            _ => None,
        }),
        row_width,
        list_width.saturating_add(2),
    )
    .saturating_sub(1);
    let rows = board.all_items.len().clamp(1, usize::from(u16::MAX));
    let height = (usize::from(HEADER_ROWS)
        + rows
        + 2
        + usize::from(FOOTER_ROWS)
        + usize::from(requested_detail))
    .min(usize::from(screen.height.max(1))) as u16;
    let x = screen.x + screen.width.saturating_sub(width) / 2;
    let bottom_y = screen.y + screen.height.saturating_sub(height);
    let top = screen.y + screen.height.saturating_sub(height) / 2;
    let outer = Rect::new(x, top.min(bottom_y).max(screen.y), width, height);
    let inner = Rect::new(
        outer.x.saturating_add(1),
        outer.y.saturating_add(1),
        outer.width.saturating_sub(2),
        outer.height.saturating_sub(2),
    );
    let (list, footer_row) = if inner.width > 0 && inner.height >= FOOTER_ROWS {
        (
            Rect::new(inner.x, inner.y, inner.width, inner.height - FOOTER_ROWS),
            Some(Rect::new(inner.x, inner.bottom() - 1, inner.width, 1)),
        )
    } else {
        (inner, None)
    };
    // The header is carved off the top of the list.
    let header_height = HEADER_ROWS.min(list.height);
    let header = Rect::new(list.x, list.y, list.width, header_height);
    let list = Rect::new(
        list.x,
        list.y + header_height,
        list.width,
        list.height - header_height,
    );
    // The preview yields to the list: a board squeezed to nothing shows its
    // todos, and a preview with no room for a row of text is not drawn.
    let detail_height = requested_detail.min(list.height.saturating_sub(1));
    let (list, detail) = if detail_height >= DETAIL_MIN_ROWS - 1 {
        let shrunk = list.height - detail_height;
        (
            Rect::new(list.x, list.y, list.width, shrunk),
            Some(Rect::new(
                list.x,
                list.y + shrunk,
                list.width,
                detail_height,
            )),
        )
    } else {
        (list, None)
    };
    let has_live_link = board
        .selected_todo()
        .is_some_and(|todo| live_link_id(todo).is_some());
    let buttons = footer_row
        .map(|row| footer_buttons(row, footer_set(board.has_todos(), has_live_link)))
        .unwrap_or_default();
    Some(TodoBoardLayout {
        outer,
        header,
        detail,
        list,
        footer_row,
        buttons,
        start: board
            .scroll
            .min(board.items.len().saturating_sub(usize::from(list.height))),
    })
}

// -- render ---------------------------------------------------------------------

/// A heading as styled spans: the space in the heading style, ` · ` dim, the
/// label in the sub-heading style, the whole cut to the row first.
fn render_heading(b: &mut Buffer, row: Rect, space: &str, label: &str, p: &Palette) {
    let text = crate::ui::text::truncate_end(&heading_text(space, label), usize::from(row.width));
    let space_len = if space.is_empty() {
        0
    } else {
        space.chars().count() + 1
    };
    let sep_len = if !space.is_empty() && !label.is_empty() {
        3
    } else {
        0
    };
    let mut chars = text.chars();
    let mut take = |n: usize| chars.by_ref().take(n).collect::<String>();
    let space_part = take(space_len);
    let sep_part = take(sep_len);
    let label_part: String = chars.collect();
    let mut x = row.x;
    for (part, style) in [
        (
            space_part,
            Style::default().fg(p.accent).add_modifier(Modifier::BOLD),
        ),
        (sep_part, Style::default().fg(p.overlay0)),
        (
            label_part,
            Style::default().fg(p.mauve).add_modifier(Modifier::BOLD),
        ),
    ] {
        if part.is_empty() {
            continue;
        }
        let width = display_width(&part);
        put_text(b, x, row.y, width, &part, style);
        x = x.saturating_add(width);
    }
}

/// Dim the screen, then draw the board centred on it. Nothing is drawn until
/// the first list lands, so an empty board never flashes up in front of a
/// session that has todos. Returns its layout and, while the search is
/// focused, the caret.
pub(super) fn render_todo_board(
    b: &mut Buffer,
    board: &ClientTodoBoardOverlay,
    todo_color: Option<Color>,
    p: &Palette,
) -> Option<(TodoBoardLayout, Option<crate::protocol::CursorState>)> {
    board.loaded_key?;
    for y in b.area.y..b.area.bottom() {
        for x in b.area.x..b.area.right() {
            let cell = &mut b[(x, y)];
            cell.set_style(cell.style().add_modifier(Modifier::DIM));
        }
    }
    let layout = todo_board_layout(board, b.area)?;
    render_panel_shell(b, layout.outer, p.accent, p.panel_bg)?;
    let mut cursor = None;
    if layout.header.height >= 2 {
        // `todos/notes`, not `todos`: what a pane records is as often a note
        // to self as a task.
        let title = layout.header_row(0);
        let title_width = title.width.saturating_sub(2);
        if title_width > 0 {
            put_text(
                b,
                title.x + 1,
                title.y,
                title_width,
                "todos/notes",
                Style::default().fg(p.text).add_modifier(Modifier::BOLD),
            );
        }
        // The count sits right-aligned and dim on the title row.
        let count = board.todo_count();
        let count = format!("{count} {}", if count == 1 { "todo" } else { "todos" });
        let count_width = display_width(&count);
        if display_width(" todos/notes") + 1 + count_width < title.width {
            put_text(
                b,
                title.right() - 1 - count_width,
                title.y,
                count_width,
                &count,
                Style::default().fg(p.overlay0).bg(p.panel_bg),
            );
        }
        cursor = render_search_row(
            b,
            layout.search_row(),
            &board.search,
            board.search_focused,
            "search todos",
            "",
            p,
        );
    }
    if layout.header.height >= HEADER_ROWS {
        render_rule(b, layout.header_row(2), p);
    }

    let list = layout.list;
    // The empty state still falls through to the footer: opening the board on
    // a quiet session must say so rather than look like a broken keybinding.
    if board.items.is_empty() && list.height > 0 {
        let message = if board.all_items.is_empty() {
            " nothing outstanding"
        } else {
            " no match"
        };
        put_text(
            b,
            list.x,
            list.y,
            list.width,
            message,
            Style::default().fg(p.overlay0),
        );
    }
    for (offset, item) in board
        .items
        .iter()
        .enumerate()
        .skip(layout.start)
        .take(usize::from(list.height))
    {
        let row = Rect::new(
            list.x,
            list.y + (offset - layout.start) as u16,
            list.width,
            1,
        );
        match item {
            // The separation between groups is the row itself.
            TodoBoardItem::GroupGap => {}
            TodoBoardItem::PaneHeading { space, label } => {
                render_heading(b, row, space, label, p);
            }
            TodoBoardItem::Todo(todo) => {
                let Some(todo) = board.todos.get(*todo) else {
                    continue;
                };
                render_row(
                    b,
                    todo_rect(row),
                    todo,
                    offset == board.selected,
                    todo_color,
                    p,
                );
            }
        }
    }

    if let Some(detail) = layout.detail {
        let row = todo_rect(Rect::new(detail.x, detail.y, detail.width, 1));
        let text = board
            .selected_todo()
            .filter(|todo| row_hides_text(row, todo))
            .map(|todo| todo.text.as_str());
        render_rule(b, Rect::new(detail.x, detail.y, detail.width, 1), p);
        render_detail_text(
            b,
            Rect::new(
                detail.x + 1,
                detail.y + 1,
                detail.width.saturating_sub(2),
                detail.height - 1,
            ),
            text,
            p,
        );
    }

    for (rect, button) in &layout.buttons {
        // `↵ open pane` is what enter does: the primary chip, always accent.
        let style = if *button == TodoBoardButton::Open || board.hovered_button == Some(*button) {
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
    Some((layout, cursor))
}

/// A rule: `─` in `surface1`, inset one column on both sides.
fn render_rule(b: &mut Buffer, row: Rect, p: &Palette) {
    let width = row.width.saturating_sub(2);
    if width == 0 {
        return;
    }
    put_text(
        b,
        row.x + 1,
        row.y,
        width,
        &"─".repeat(usize::from(width)),
        Style::default().fg(p.surface1).bg(p.panel_bg),
    );
}

// -- behaviour ------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BoardAction {
    OpenOwner,
    Edit,
    ToggleDone,
    Remove,
    ClearDone,
    FollowLink,
}

impl ClientShellState {
    fn todo_board_mut(&mut self) -> Option<&mut ClientTodoBoardOverlay> {
        match self.overlay.as_mut() {
            Some(ClientShellOverlay::TodoBoard(board)) => Some(board),
            _ => None,
        }
    }

    pub(super) fn open_todo_board(&mut self, outcome: &mut ClientShellInput) {
        let Some(boot_id) = self
            .snapshot
            .as_deref()
            .map(|snapshot| snapshot.boot_id.clone())
        else {
            return;
        };
        self.overlay = Some(ClientShellOverlay::TodoBoard(ClientTodoBoardOverlay::new(
            boot_id,
        )));
        self.request_todo_board_list(outcome);
        outcome.repaint = true;
    }

    /// Open the board, or close it when it is already open: the tab-bar
    /// indicator's toggle.
    #[allow(dead_code)] // called by the chrome seat's tab-bar indicator
    pub(super) fn toggle_todo_board(&mut self, outcome: &mut ClientShellInput) {
        if matches!(self.overlay, Some(ClientShellOverlay::TodoBoard(_))) {
            self.close_todo_board(outcome);
            return;
        }
        self.open_todo_board(outcome);
    }

    fn close_todo_board(&mut self, outcome: &mut ClientShellInput) {
        self.overlay = None;
        self.mode = ClientShellMode::Terminal;
        outcome.repaint = true;
    }

    /// Put a board the editor suspended back, and fetch its list again: the
    /// save that closed the editor changed it.
    pub(super) fn resume_todo_board(
        &mut self,
        mut board: ClientTodoBoardOverlay,
        outcome: &mut ClientShellInput,
    ) {
        board.list_in_flight = false;
        board.refetch_wanted = false;
        self.overlay = Some(ClientShellOverlay::TodoBoard(board));
        self.request_todo_board_list(outcome);
        outcome.repaint = true;
    }

    /// Ask for every pane's todos. A request already on its way is not
    /// doubled; a change that lands meanwhile is asked for when it returns.
    pub(super) fn request_todo_board_list(&mut self, outcome: &mut ClientShellInput) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        let key = board_key(snapshot);
        let Some(ClientShellOverlay::TodoBoard(board)) = self.overlay.as_mut() else {
            return;
        };
        if board.list_in_flight {
            board.refetch_wanted = true;
            return;
        }
        if self.push_endpoint_method_with_kind(
            Method::TodoList(TodoListParams { pane_id: None }),
            PendingEndpointKind::TodoBoardList { key },
            outcome,
        ) {
            if let Some(board) = self.todo_board_mut() {
                board.list_in_flight = true;
            }
        }
    }

    /// Keep an open board in step with the server: close it when its server
    /// boot is gone, fetch the list again when any pane's todos or the pane
    /// set changed. Runs from the client's timer, never from render.
    pub(crate) fn tick_todo_board(&mut self, outcome: &mut ClientShellInput) {
        let Some(ClientShellOverlay::TodoBoard(board)) = self.overlay.as_ref() else {
            return;
        };
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        if snapshot.boot_id != board.boot_id {
            self.close_todo_board(outcome);
            return;
        }
        if !board.list_in_flight && board.loaded_key != Some(board_key(snapshot)) {
            self.request_todo_board_list(outcome);
        }
    }

    pub(super) fn handle_todo_board_list_result(
        &mut self,
        key: u64,
        result: Result<ResponseResult, ClientShellEndpointError>,
        outcome: &mut ClientShellInput,
    ) -> bool {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return false;
        };
        let Some(ClientShellOverlay::TodoBoard(board)) = self.overlay.as_mut() else {
            return false;
        };
        board.list_in_flight = false;
        let refetch = std::mem::take(&mut board.refetch_wanted);
        let Ok(ResponseResult::TodoList { todos }) = result else {
            // Keep what is on screen; the next tick asks again only when the
            // todo state moves, so a failing list is not retried in a loop.
            board.loaded_key = Some(key);
            return false;
        };
        board.apply_list(todos, snapshot);
        board.loaded_key = Some(key);
        if refetch {
            self.request_todo_board_list(outcome);
        }
        true
    }

    fn todo_board_visible_rows(&self) -> usize {
        self.hits
            .todo_board_layout
            .as_ref()
            .map_or(0, |layout| usize::from(layout.list.height))
    }

    fn move_todo_board_selection(&mut self, chord: Chord) {
        let visible = self.todo_board_visible_rows();
        let Some(board) = self.todo_board_mut() else {
            return;
        };
        let delta = chord_delta(chord, board.selected, visible, board.items.len());
        board.move_selection_by(delta, visible);
    }

    fn refilter_todo_board(&mut self) {
        let visible = self.todo_board_visible_rows();
        if let Some(board) = self.todo_board_mut() {
            board.refilter(visible);
        }
    }

    /// Apply a board action. Every mutation goes to the todo's owner pane
    /// through the `todo.*` API, which need not be the focused pane or even in
    /// the active space; the board refetches when each answer lands.
    fn apply_todo_board_action(&mut self, action: BoardAction, outcome: &mut ClientShellInput) {
        outcome.repaint = true;
        // Clearing acts on everything the board shows rather than on a
        // selection, so it runs before the selected-todo lookup.
        if action == BoardAction::ClearDone {
            let Some(ClientShellOverlay::TodoBoard(board)) = self.overlay.as_ref() else {
                return;
            };
            let mut panes: Vec<String> = Vec::new();
            for item in &board.items {
                let TodoBoardItem::Todo(todo) = item else {
                    continue;
                };
                if let Some(todo) = board.todos.get(*todo) {
                    if !panes.contains(&todo.pane_id) {
                        panes.push(todo.pane_id.clone());
                    }
                }
            }
            for pane_id in panes {
                self.push_todo_mutation(
                    Method::TodoClear(TodoClearParams {
                        pane_id: pane_id.clone(),
                        done_only: true,
                    }),
                    pane_id,
                    outcome,
                );
            }
            return;
        }
        let Some(ClientShellOverlay::TodoBoard(board)) = self.overlay.as_ref() else {
            return;
        };
        let Some(todo) = board.selected_todo().cloned() else {
            return;
        };
        let owner = todo.pane_id.clone();
        match action {
            BoardAction::OpenOwner => {
                self.close_todo_board(outcome);
                self.push_endpoint_method(
                    Method::PaneFocus(PaneTarget { pane_id: owner }),
                    outcome,
                );
            }
            BoardAction::Edit => {
                let Some(ClientShellOverlay::TodoBoard(board)) = self.overlay.take() else {
                    return;
                };
                self.open_todo_editor(
                    owner,
                    Some(todo),
                    Some(SuspendedTodoSurface::Board(Box::new(board))),
                );
            }
            BoardAction::ToggleDone => {
                self.push_todo_mutation(
                    Method::TodoUpdate(TodoUpdateParams {
                        pane_id: owner.clone(),
                        id: todo.id,
                        text: None,
                        done: Some(!todo.done),
                        priority: None,
                        link_pane_id: None,
                        clear_link: false,
                    }),
                    owner,
                    outcome,
                );
            }
            BoardAction::Remove => {
                self.push_todo_mutation(
                    Method::TodoRemove(TodoRemoveParams {
                        pane_id: owner.clone(),
                        id: todo.id,
                    }),
                    owner,
                    outcome,
                );
            }
            // Handled above, before the selected-todo lookup.
            BoardAction::ClearDone => {}
            BoardAction::FollowLink => {
                // A dead link is inert, and the *linked* pane is the
                // destination, never the owner.
                let Some(target) = live_link_id(&todo) else {
                    return;
                };
                let target = target.to_owned();
                self.close_todo_board(outcome);
                self.push_endpoint_method(
                    Method::PaneFocus(PaneTarget { pane_id: target }),
                    outcome,
                );
            }
        }
    }

    /// Text arriving as text: only while the search has focus.
    pub(super) fn insert_todo_board_text(&mut self, text: &str) -> bool {
        let Some(board) = self.todo_board_mut() else {
            return false;
        };
        if !board.search_focused {
            return false;
        }
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        if board.search.insert_str(&text) {
            self.refilter_todo_board();
        }
        true
    }

    /// Keys while the board is open (fork `handle_todo_board_key_via_api`):
    /// the panel's keys, with `enter` going to the owner pane and `e` editing.
    pub(super) fn route_todo_board_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) {
        if key.kind == crossterm::event::KeyEventKind::Release {
            return;
        }
        let Some(board) = self.todo_board_mut() else {
            return;
        };
        outcome.repaint = true;
        if board.search_focused {
            // While the search has the keyboard its letters are text, so none
            // of the board's letter commands can fire from a word being typed.
            let (code, modifiers) = (key.code, key.modifiers);
            if code == KeyCode::Esc {
                board.search_focused = false;
                return;
            }
            if code == KeyCode::Enter {
                self.apply_todo_board_action(BoardAction::OpenOwner, outcome);
                return;
            }
            if let Some(chord) = list_chord(code, modifiers, true) {
                self.move_todo_board_selection(chord);
                return;
            }
            let before = board.search.text().to_owned();
            // Text the host composed is authoritative (AltGr, dead keys).
            let handled = match key.generated_text.as_deref().filter(|text| {
                !text.is_empty() && !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            }) {
                Some(text) => {
                    let text: String = text.chars().filter(|c| !c.is_control()).collect();
                    board.search.insert_str(&text);
                    true
                }
                None => apply_text_key(
                    &mut board.search,
                    KeyEvent::new(code, modifiers),
                    Shape::SingleLine,
                ),
            };
            if handled && board.search.text() != before {
                self.refilter_todo_board();
            }
            return;
        }
        // A shifted letter does what its lowercase form does.
        let (code, modifiers) = match key.code {
            KeyCode::Char(c) if key.modifiers == KeyModifiers::SHIFT && c.is_ascii_uppercase() => {
                (KeyCode::Char(c.to_ascii_lowercase()), KeyModifiers::NONE)
            }
            code => (code, key.modifiers),
        };
        let bare = modifiers.is_empty();
        match code {
            KeyCode::Char('/') if bare => board.search_focused = true,
            // A set query is cleared before the board closes.
            KeyCode::Esc if !board.query().is_empty() => {
                board.search.clear();
                self.refilter_todo_board();
            }
            KeyCode::Enter => self.apply_todo_board_action(BoardAction::OpenOwner, outcome),
            KeyCode::Char('e') if bare => self.apply_todo_board_action(BoardAction::Edit, outcome),
            KeyCode::Char(' ') if bare => {
                self.apply_todo_board_action(BoardAction::ToggleDone, outcome)
            }
            KeyCode::Char('g') if bare => {
                self.apply_todo_board_action(BoardAction::FollowLink, outcome)
            }
            KeyCode::Char('d') if bare => {
                self.apply_todo_board_action(BoardAction::Remove, outcome)
            }
            KeyCode::Char('c') if bare => {
                self.apply_todo_board_action(BoardAction::ClearDone, outcome)
            }
            KeyCode::Esc => self.close_todo_board(outcome),
            KeyCode::Char('q') if bare => self.close_todo_board(outcome),
            _ => match list_chord(code, modifiers, false) {
                Some(chord) => self.move_todo_board_selection(chord),
                None => outcome.repaint = false,
            },
        }
    }

    /// Mouse while the board is open (fork mouse.rs, `Mode::TodoBoard`).
    /// Every event stops here: nothing reaches the panes beneath. The pointer
    /// moves the hovered button and nothing else; the wheel moves the window,
    /// not the selection; the first click on a row selects it and the second
    /// travels to its owner.
    pub(super) fn route_todo_board_mouse(
        &mut self,
        mouse: MouseEvent,
        outcome: &mut ClientShellInput,
    ) {
        let Some(layout) = self.hits.todo_board_layout.clone() else {
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                self.close_todo_board(outcome);
            }
            return;
        };
        let point = (mouse.column, mouse.row);
        let visible = usize::from(layout.list.height);
        match mouse.kind {
            MouseEventKind::Moved => {
                let hovered = layout
                    .buttons
                    .iter()
                    .find(|(rect, _)| super::contains(*rect, point))
                    .map(|(_, button)| *button);
                if let Some(board) = self.todo_board_mut() {
                    outcome.repaint |= board.hovered_button != hovered;
                    board.hovered_button = hovered;
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let delta = if mouse.kind == MouseEventKind::ScrollUp {
                    -1
                } else {
                    1
                };
                if let Some(board) = self.todo_board_mut() {
                    board.scroll_by(delta, visible);
                }
                outcome.repaint = true;
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let len = self.todo_board_mut().map_or(0, |board| board.items.len());
                if let Some(index) = layout.row_at(mouse.column, mouse.row, len) {
                    // A chip is an explicit target that names where it goes, so
                    // it acts on the first click wherever the selection is.
                    let Some(board) = self.todo_board_mut() else {
                        return;
                    };
                    let on_chip = board.todo_at(index).is_some_and(|todo| {
                        let row =
                            todo_rect(Rect::new(layout.list.x, mouse.row, layout.list.width, 1));
                        link_chip(row_chip_area(row, todo.id), todo)
                            .is_some_and(|(chip, _)| super::contains(chip, point))
                    });
                    if on_chip {
                        if board.select_todo(index) {
                            self.apply_todo_board_action(BoardAction::FollowLink, outcome);
                        }
                        return;
                    }
                    let already_selected = board.selected == index;
                    // A click on a heading or a gap selects nothing and does
                    // nothing, rather than acting on the old selection.
                    if !board.select_todo(index) {
                        return;
                    }
                    outcome.repaint = true;
                    // Travel closes the board and moves you elsewhere, so it
                    // takes a second click on the row you can now see chosen.
                    if already_selected {
                        self.apply_todo_board_action(BoardAction::OpenOwner, outcome);
                    }
                    return;
                }
                if let Some((_, button)) = layout
                    .buttons
                    .iter()
                    .find(|(rect, _)| super::contains(*rect, point))
                {
                    match button {
                        TodoBoardButton::Open => {
                            self.apply_todo_board_action(BoardAction::OpenOwner, outcome)
                        }
                        TodoBoardButton::Toggle => {
                            self.apply_todo_board_action(BoardAction::ToggleDone, outcome)
                        }
                        TodoBoardButton::Go => {
                            self.apply_todo_board_action(BoardAction::FollowLink, outcome)
                        }
                        TodoBoardButton::ClearDone => {
                            self.apply_todo_board_action(BoardAction::ClearDone, outcome)
                        }
                        TodoBoardButton::Close => self.close_todo_board(outcome),
                    }
                    return;
                }
                // A near miss elsewhere on the buttons' row is inert, even
                // outside the board.
                if layout.footer_row.is_some_and(|row| mouse.row == row.y) {
                    return;
                }
                if super::contains(layout.search_row(), point) {
                    if let Some(board) = self.todo_board_mut() {
                        board.search_focused = true;
                    }
                    outcome.repaint = true;
                    return;
                }
                // Inside the board but off its rows and buttons: inert. Only
                // a click outside it dismisses.
                if !super::contains(layout.outer, point) {
                    self.close_todo_board(outcome);
                }
            }
            _ => {}
        }
    }
}
