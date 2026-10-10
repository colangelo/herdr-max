//! The command palette (fork issue 182, part 1): one bar that lists every
//! herdr command with its shortcut. Typing filters, `enter` runs the selected
//! command exactly as its key does (through `record_binding`, the path mouse
//! clicks already use).
//!
//! The rows come from the command table in `input::keybind_help`, the same
//! table the help panel is built from, so a command added there is in the bar
//! without a second list.

use super::move_picker::{list_chord, render_rule, render_scrollbar, reveal_scroll, Chord};
use super::render::{display_width, put_text};
use super::todo_panel::render_panel_shell;
use super::todo_text::{apply_text_key, Shape, TextField};
use super::*;
use crate::api::schema::{
    AgentMessageBusy, AgentMessageMode, AgentMessageParams, AgentStatus, Method, PaneTarget,
    ResponseResult,
};
use crate::input::{command_table, CommandAction};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::style::Color;

const WIDTH: u16 = 82;
const MAX_LIST_ROWS: usize = 16;
const TOP_MARGIN: u16 = 2;
/// Title, query, rule.
const HEADER_ROWS: u16 = 3;
/// A blank row and the key bar.
const FOOTER_ROWS: u16 = 2;
const QUERY_MAX_CHARS: usize = 256;
const PROMPT: &str = " : ";
const PLACEHOLDER: &str = "type a command, or send <session> <text>";
/// The words that turn the bar into a send: `send reviewer`.
const SEND_WORD: &str = "send ";
const RECEIPT_CAVEAT: &str = "This means it arrived, not that it was read or done.";

/// What choosing a row does.
#[derive(Clone, Copy, Debug)]
pub(super) enum PaletteAction {
    Command(CommandAction),
    /// The `send to a session…` row: switch to the list of sessions.
    OpenSend,
    /// A row of the session list: an index into the overlay's targets.
    Target(usize),
}

/// A row of the bar: a command, or a session to send to.
#[derive(Clone, Debug)]
pub(super) struct PaletteCommand {
    pub(super) group: &'static str,
    /// Drawn dim at the row's end: a shortcut, or a session's state.
    pub(super) key: String,
    pub(super) label: String,
    pub(super) action: PaletteAction,
}

impl PaletteCommand {
    fn haystack(&self) -> String {
        format!("{} {}", self.group, self.label)
    }

    fn is_indexed(&self) -> bool {
        matches!(
            self.action,
            PaletteAction::Command(CommandAction::Indexed(_))
        )
    }
}

/// A session the bar can send to, read from the snapshot when it opens.
#[derive(Clone, Debug)]
pub(super) struct SendTarget {
    pub(super) pane_id: String,
    /// The agent's herdr name; a note is addressed by it.
    pub(super) name: Option<String>,
    pub(super) label: String,
    pub(super) status: AgentStatus,
}

/// A message being written to one session.
#[derive(Debug)]
pub(super) struct SendDraft {
    pub(super) target: SendTarget,
    pub(super) mode: AgentMessageMode,
    /// Typing as you needs a local endpoint and a session that is not blocked
    /// on a prompt: free text is never typed into a prompt.
    pub(super) typed_possible: bool,
    /// A note is addressed by the agent's name.
    pub(super) note_possible: bool,
    /// Waiting for "queue or interrupt?" (`palette.send.busy = "ask"`).
    pub(super) asking: bool,
    pub(super) sending: bool,
    pub(super) notice: Option<String>,
}

#[derive(Debug)]
pub(super) enum PaletteStage {
    Commands,
    Targets,
    Compose(SendDraft),
    Receipt(Vec<String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PaletteItem {
    Heading(&'static str),
    Gap,
    /// An index into the overlay's commands.
    Command(usize),
}

#[derive(Debug)]
pub(super) struct ClientPaletteOverlay {
    pub(super) commands: Vec<PaletteCommand>,
    pub(super) items: Vec<PaletteItem>,
    /// The row count with nothing typed, so the box keeps its height while
    /// the list is filtered.
    full_rows: usize,
    /// An index into `items`; always a `Command` row when there is one.
    pub(super) selected: usize,
    pub(super) scroll: usize,
    pub(super) query: TextField,
    /// The digit typed after an indexed command's words (zero-based).
    pub(super) argument: Option<usize>,
    pub(super) stage: PaletteStage,
    pub(super) targets: Vec<SendTarget>,
    /// The command rows, kept while the session list takes their place.
    saved: Vec<PaletteCommand>,
}

/// Subsequence match of `query` in `text`, ignoring case and the query's
/// spaces. Higher is better: word starts and runs of adjacent characters
/// score, and the query appearing whole scores most. `None` when a query
/// character is missing.
pub(super) fn fuzzy_score(query: &str, text: &str) -> Option<i64> {
    let needle: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    if needle.is_empty() {
        return Some(0);
    }
    let hay: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    // The query appearing whole beats any scattered match, most of all at the
    // start of a word (`spl` is `split`, not the middle of `display`).
    let whole = query.trim().to_lowercase();
    let hay_text: String = hay.iter().collect();
    if let Some(at) = hay_text.find(&whole) {
        let word_start = hay_text[..at]
            .chars()
            .next_back()
            .is_none_or(|before| !before.is_alphanumeric());
        return Some(1000 + if word_start { 200 } else { 0 });
    }
    let mut score = 0i64;
    let mut next = 0;
    let mut last: Option<usize> = None;
    for (index, ch) in hay.iter().enumerate() {
        if next < needle.len() && *ch == needle[next] {
            score += 10;
            if index == 0 || !hay[index - 1].is_alphanumeric() {
                score += 20;
            }
            if last.is_some_and(|previous| previous + 1 == index) {
                score += 15;
            }
            last = Some(index);
            next += 1;
        }
    }
    if next < needle.len() {
        return None;
    }
    Some(score)
}

/// `switch workspace 3` -> (`switch workspace`, Some(2)): a single digit
/// after the words, zero-based.
fn split_argument(query: &str) -> (&str, Option<usize>) {
    let trimmed = query.trim_end();
    if let Some((rest, last)) = trimmed.rsplit_once(' ') {
        let mut chars = last.chars();
        if let (Some(digit), None) = (chars.next(), chars.next()) {
            if let Some(number) = digit.to_digit(10).filter(|n| *n >= 1) {
                if !rest.trim().is_empty() {
                    return (rest.trim(), Some(number as usize - 1));
                }
            }
        }
    }
    (query, None)
}

impl ClientPaletteOverlay {
    pub(super) fn new(commands: Vec<PaletteCommand>, targets: Vec<SendTarget>) -> Self {
        let mut palette = Self {
            commands,
            items: Vec::new(),
            full_rows: 0,
            selected: 0,
            scroll: 0,
            query: TextField::new(QUERY_MAX_CHARS),
            argument: None,
            stage: PaletteStage::Commands,
            targets,
            saved: Vec::new(),
        };
        palette.refilter();
        palette.full_rows = palette.items.len();
        palette
    }

    pub(super) fn total(&self) -> usize {
        self.commands.len()
    }

    pub(super) fn shown(&self) -> usize {
        self.items
            .iter()
            .filter(|item| matches!(item, PaletteItem::Command(_)))
            .count()
    }

    /// Rebuild `items` from the query: grouped in help order when it is
    /// empty, best match first when it is not. The selection returns to the
    /// first command.
    pub(super) fn refilter(&mut self) {
        let query = self.query.text().to_owned();
        self.argument = None;
        self.items.clear();
        if query.trim().is_empty() {
            let mut group = None;
            for (index, command) in self.commands.iter().enumerate() {
                if group != Some(command.group) {
                    if group.is_some() {
                        self.items.push(PaletteItem::Gap);
                    }
                    self.items.push(PaletteItem::Heading(command.group));
                    group = Some(command.group);
                }
                self.items.push(PaletteItem::Command(index));
            }
        } else {
            // `switch workspace 3`: the digit is the argument of an indexed
            // command, when the words before it name one.
            let (base, argument) = split_argument(&query);
            let by_argument = argument.map(|_| self.scored(base, true));
            let mut scored = match by_argument {
                Some(rows) if !rows.is_empty() => {
                    self.argument = argument;
                    rows
                }
                _ => self.scored(&query, false),
            };
            scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            self.items.extend(
                scored
                    .into_iter()
                    .map(|(index, _)| PaletteItem::Command(index)),
            );
        }
        self.selected = self.first_command().unwrap_or(0);
        self.scroll = 0;
    }

    fn scored(&self, query: &str, indexed_only: bool) -> Vec<(usize, i64)> {
        self.commands
            .iter()
            .enumerate()
            .filter(|(_, command)| !indexed_only || command.is_indexed())
            .filter_map(|(index, command)| {
                fuzzy_score(query, &command.haystack()).map(|score| (index, score))
            })
            .collect()
    }

    fn first_command(&self) -> Option<usize> {
        self.items
            .iter()
            .position(|item| matches!(item, PaletteItem::Command(_)))
    }

    pub(super) fn selected_command(&self) -> Option<&PaletteCommand> {
        match self.items.get(self.selected) {
            Some(PaletteItem::Command(index)) => self.commands.get(*index),
            _ => None,
        }
    }

    /// The command rows' labels in the order drawn, for tests.
    #[cfg(test)]
    pub(super) fn labels(&self) -> Vec<&str> {
        self.items
            .iter()
            .filter_map(|item| match item {
                PaletteItem::Command(index) => Some(self.commands[*index].label.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Move to the next (`delta` 1) or previous (-1) command row, skipping
    /// headings and gaps; stay put at the ends.
    fn step(&mut self, delta: isize) -> bool {
        let mut index = self.selected as isize + delta;
        while index >= 0 && (index as usize) < self.items.len() {
            if matches!(self.items[index as usize], PaletteItem::Command(_)) {
                let moved = self.selected != index as usize;
                self.selected = index as usize;
                return moved;
            }
            index += delta;
        }
        false
    }

    fn reveal(&mut self, visible: usize) {
        self.scroll = reveal_scroll(self.scroll, self.selected, visible, self.items.len());
        // A heading just above the first row stays in view with its commands.
        if self.scroll > 0
            && self.scroll + 1 == self.selected
            && matches!(self.items.get(self.scroll), Some(PaletteItem::Heading(_)))
        {
            self.scroll -= 1;
        }
    }

    /// The words `tab` puts in the query for the selected command.
    fn completion(&self) -> Option<String> {
        let command = self.selected_command()?;
        Some(match command.action {
            PaletteAction::Command(CommandAction::Indexed(_)) => {
                format!("{} ", command.label.trim_end_matches(" 1-9"))
            }
            _ => command.label.clone(),
        })
    }

    /// Switch to the list of sessions, filtered by `filter`.
    pub(super) fn enter_targets(&mut self, filter: &str) {
        if matches!(self.stage, PaletteStage::Commands) {
            self.saved = std::mem::take(&mut self.commands);
        }
        self.commands = self
            .targets
            .iter()
            .enumerate()
            .map(|(index, target)| PaletteCommand {
                group: "sessions",
                key: status_word(target.status).to_owned(),
                label: target.label.clone(),
                action: PaletteAction::Target(index),
            })
            .collect();
        self.stage = PaletteStage::Targets;
        self.query.clear();
        self.query.insert_str(filter);
        self.refilter();
    }

    /// Back to the commands.
    pub(super) fn leave_targets(&mut self) {
        self.commands = std::mem::take(&mut self.saved);
        self.stage = PaletteStage::Commands;
        self.query.clear();
        self.refilter();
    }

    /// Whether the query is a command list or a session list being filtered
    /// (not a message being written).
    pub(super) fn filtering(&self) -> bool {
        matches!(self.stage, PaletteStage::Commands | PaletteStage::Targets)
    }
}

fn status_word(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Idle => "idle",
        AgentStatus::Working => "working",
        AgentStatus::Blocked => "blocked",
        AgentStatus::Done => "done",
        AgentStatus::Unknown => "unknown",
    }
}

// -- layout and render ---------------------------------------------------------

#[derive(Clone, Debug)]
pub(super) struct PaletteLayout {
    pub(super) outer: Rect,
    pub(super) title: Rect,
    pub(super) query: Rect,
    pub(super) rule: Rect,
    pub(super) list: Rect,
    pub(super) footer: Option<Rect>,
    pub(super) start: usize,
}

impl PaletteLayout {
    fn row_at(&self, x: u16, y: u16, len: usize) -> Option<usize> {
        if x < self.list.x || x >= self.list.right() || y < self.list.y || y >= self.list.bottom() {
            return None;
        }
        let index = self.start + usize::from(y - self.list.y);
        (index < len).then_some(index)
    }
}

pub(super) fn palette_layout(
    palette: &ClientPaletteOverlay,
    screen: Rect,
) -> Option<PaletteLayout> {
    if screen.width < 4 || screen.height < 4 {
        return None;
    }
    let width = WIDTH.min(screen.width);
    let rows = palette.full_rows.clamp(1, MAX_LIST_ROWS);
    let height = (usize::from(HEADER_ROWS) + rows + usize::from(FOOTER_ROWS) + 2)
        .min(usize::from(screen.height)) as u16;
    let x = screen.x + (screen.width - width) / 2;
    let y = (screen.y + TOP_MARGIN).min(screen.bottom() - height);
    let outer = Rect::new(x, y, width, height);
    let inner = Rect::new(
        outer.x + 1,
        outer.y + 1,
        outer.width.saturating_sub(2),
        outer.height.saturating_sub(2),
    );
    let footer = (inner.height > FOOTER_ROWS + HEADER_ROWS)
        .then(|| Rect::new(inner.x, inner.bottom() - 1, inner.width, 1));
    let body_bottom = inner
        .bottom()
        .saturating_sub(if footer.is_some() { FOOTER_ROWS } else { 0 });
    let header_rows = HEADER_ROWS.min(inner.height);
    let list_y = inner.y + header_rows;
    let list = Rect::new(
        inner.x,
        list_y,
        inner.width,
        body_bottom.saturating_sub(list_y),
    );
    let visible = usize::from(list.height);
    Some(PaletteLayout {
        outer,
        title: Rect::new(inner.x, inner.y, inner.width, 1),
        query: Rect::new(inner.x, inner.y + 1, inner.width, 1),
        rule: Rect::new(inner.x, inner.y + 2, inner.width, 1),
        list,
        footer,
        start: palette
            .scroll
            .min(palette.items.len().saturating_sub(visible)),
    })
}

fn contrast(p: &Palette) -> Color {
    super::panel_contrast_fg(p)
}

fn render_title(b: &mut Buffer, row: Rect, palette: &ClientPaletteOverlay, p: &Palette) {
    let (title, noun) = match &palette.stage {
        PaletteStage::Commands => ("commands".to_owned(), "commands"),
        PaletteStage::Targets => ("send to a session".to_owned(), "sessions"),
        PaletteStage::Compose(draft) => (format!("send to {}", draft.target.label), ""),
        PaletteStage::Receipt(_) => ("sent".to_owned(), ""),
    };
    put_text(
        b,
        row.x + 1,
        row.y,
        row.width.saturating_sub(2),
        &title,
        Style::default()
            .fg(p.text)
            .bg(p.panel_bg)
            .add_modifier(Modifier::BOLD),
    );
    let count = if !palette.filtering() {
        String::new()
    } else if palette.query.text().trim().is_empty() {
        format!("{} {noun}", palette.total())
    } else {
        format!("{} of {}", palette.shown(), palette.total())
    };
    let width = display_width(&count);
    if width + 12 < row.width {
        put_text(
            b,
            row.right() - 1 - width,
            row.y,
            width,
            &count,
            Style::default().fg(p.overlay0).bg(p.panel_bg),
        );
    }
}

/// The query row; returns the caret's cell.
fn render_query(
    b: &mut Buffer,
    row: Rect,
    palette: &ClientPaletteOverlay,
    p: &Palette,
) -> Option<crate::protocol::CursorState> {
    if row.width == 0 {
        return None;
    }
    put_text(
        b,
        row.x,
        row.y,
        3.min(row.width),
        PROMPT,
        Style::default()
            .fg(p.accent)
            .bg(p.panel_bg)
            .add_modifier(Modifier::BOLD),
    );
    let text_x = row.x + 3.min(row.width);
    let room = row.right().saturating_sub(text_x);
    let query = palette.query.text();
    if matches!(palette.stage, PaletteStage::Receipt(_)) {
        return None;
    }
    if query.is_empty() {
        let placeholder = match palette.stage {
            PaletteStage::Targets => "pick a session",
            PaletteStage::Compose(_) => "a message, one line",
            _ => PLACEHOLDER,
        };
        put_text(
            b,
            text_x,
            row.y,
            room,
            placeholder,
            Style::default().fg(p.overlay0).bg(p.panel_bg),
        );
    } else {
        put_text(
            b,
            text_x,
            row.y,
            room,
            query,
            Style::default().fg(p.text).bg(p.panel_bg),
        );
    }
    Some(crate::protocol::CursorState {
        x: (usize::from(text_x) + palette.query.cursor_column())
            .min(usize::from(row.right().saturating_sub(1))) as u16,
        y: row.y,
        visible: true,
        shape: 0,
    })
}

/// `gutter` is the column the scrollbar takes at the row's end: the selection
/// bar still runs under it, the text stops before it.
fn render_item(
    b: &mut Buffer,
    row: Rect,
    gutter: u16,
    palette: &ClientPaletteOverlay,
    index: usize,
    key_width: usize,
    p: &Palette,
) {
    let text_end = row.right().saturating_sub(gutter);
    let text_width = row.width.saturating_sub(gutter);
    let selected = index == palette.selected;
    let bar = Style::default()
        .fg(contrast(p))
        .bg(p.accent)
        .add_modifier(Modifier::BOLD);
    let plain = Style::default().fg(p.text).bg(p.panel_bg);
    match palette.items[index] {
        PaletteItem::Gap => {}
        PaletteItem::Heading(group) => put_text(
            b,
            row.x + 1,
            row.y,
            text_width.saturating_sub(1),
            group,
            Style::default()
                .fg(p.accent)
                .bg(p.panel_bg)
                .add_modifier(Modifier::BOLD),
        ),
        PaletteItem::Command(command_index) => {
            let command = &palette.commands[command_index];
            if selected {
                b.set_style(row, bar);
            }
            let key_style = if selected {
                bar
            } else {
                Style::default().fg(p.overlay0).bg(p.panel_bg)
            };
            let key_room = key_width.min(usize::from(text_width).saturating_sub(8));
            let key = crate::ui::text::truncate_end(&command.key, key_room);
            let key_cells = usize::from(display_width(&key));
            let label_room = usize::from(text_width).saturating_sub(key_cells + 4);
            let label = crate::ui::text::truncate_end(&command.label, label_room);
            put_text(
                b,
                row.x + 2,
                row.y,
                text_width.saturating_sub(2),
                &label,
                if selected { bar } else { plain },
            );
            put_text(
                b,
                text_end.saturating_sub(1 + key_cells as u16),
                row.y,
                key_cells as u16,
                &key,
                key_style,
            );
        }
    }
}

/// `key label` pairs: the key bold accent, its label dim.
fn put_key_bar(b: &mut Buffer, row: Rect, pairs: &[(&str, &str)], p: &Palette) {
    let mut x = row.x + 1;
    for (key, label) in pairs {
        for (text, style) in [
            (
                *key,
                Style::default()
                    .fg(p.accent)
                    .bg(p.panel_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            (
                &format!(" {label}  ")[..],
                Style::default().fg(p.overlay0).bg(p.panel_bg),
            ),
        ] {
            let width = display_width(text);
            if x + width > row.right() {
                return;
            }
            put_text(b, x, row.y, width, text, style);
            x += width;
        }
    }
}

fn render_rows(
    b: &mut Buffer,
    list: Rect,
    start: usize,
    palette: &ClientPaletteOverlay,
    p: &Palette,
) {
    let key_width = palette
        .commands
        .iter()
        .map(|command| usize::from(display_width(&command.key)))
        .max()
        .unwrap_or(0);
    // The scrollbar takes the last column of the list when it is drawn.
    let gutter = u16::from(palette.items.len() > usize::from(list.height));
    for (offset, index) in (start..palette.items.len())
        .take(usize::from(list.height))
        .enumerate()
    {
        let row = Rect::new(list.x, list.y + offset as u16, list.width, 1);
        render_item(b, row, gutter, palette, index, key_width, p);
    }
    render_scrollbar(b, list, start, palette.items.len(), p);
    if palette.items.is_empty() && list.height > 0 {
        put_text(
            b,
            list.x,
            list.y,
            list.width,
            " no match",
            Style::default().fg(p.overlay0).bg(p.panel_bg),
        );
    }
}

/// The body of a message being written: who it goes to, in what way, and what
/// the target's state means for it.
fn render_compose(b: &mut Buffer, list: Rect, draft: &SendDraft, config: &ClientShellConfig) {
    let p = &config.palette;
    let dim = Style::default().fg(p.overlay0).bg(p.panel_bg);
    let text = Style::default().fg(p.text).bg(p.panel_bg);
    let status = draft.target.status;
    let mut lines: Vec<Vec<(String, Style)>> = Vec::new();
    lines.push(vec![
        (" to   ".to_owned(), dim),
        (
            draft.target.label.clone(),
            Style::default()
                .fg(p.accent)
                .bg(p.panel_bg)
                .add_modifier(Modifier::BOLD),
        ),
        ("  ".to_owned(), dim),
        (
            format!("{} {}", config.state_icon(status), status_word(status)),
            Style::default()
                .fg(config.state_color(status))
                .bg(p.panel_bg),
        ),
    ]);
    lines.push(Vec::new());
    let (how, meaning) = match draft.mode {
        AgentMessageMode::Typed => ("you", "typed into the pane, as if you were at it"),
        AgentMessageMode::Note => (
            "a note",
            "a peer message it may act on; it approves nothing",
        ),
    };
    let mut as_line = vec![
        (" as   ".to_owned(), dim),
        (how.to_owned(), text.add_modifier(Modifier::BOLD)),
        (format!("  {meaning}"), dim),
    ];
    if draft.typed_possible && draft.note_possible {
        as_line.push(("   tab switches".to_owned(), dim));
    }
    lines.push(as_line);
    if !draft.typed_possible && !draft.note_possible {
        lines.push(vec![(
            " blocked on a prompt: free text is not typed into it. enter opens its pane."
                .to_owned(),
            Style::default().fg(p.yellow).bg(p.panel_bg),
        )]);
    } else if status == AgentStatus::Blocked {
        lines.push(vec![(
            " blocked on a prompt: only a note can go to it. enter sends the note.".to_owned(),
            Style::default().fg(p.yellow).bg(p.panel_bg),
        )]);
    } else if draft.asking {
        lines.push(vec![(
            " working now: enter queues it, alt+enter interrupts first, esc cancels.".to_owned(),
            Style::default().fg(p.yellow).bg(p.panel_bg),
        )]);
    } else if status == AgentStatus::Working {
        lines.push(vec![(
            " working now: enter queues it, alt+enter interrupts first.".to_owned(),
            dim,
        )]);
    }
    if draft.sending {
        lines.push(vec![(" sending…".to_owned(), dim)]);
    }
    if let Some(notice) = &draft.notice {
        lines.push(vec![(
            format!(" {notice}"),
            Style::default().fg(p.red).bg(p.panel_bg),
        )]);
    }
    for (offset, spans) in lines.iter().take(usize::from(list.height)).enumerate() {
        let mut x = list.x;
        for (part, style) in spans {
            let room = list.right().saturating_sub(x);
            let width = display_width(part).min(room);
            put_text(b, x, list.y + offset as u16, width, part, *style);
            x += width;
        }
    }
}

/// What a send reported, in its own words, and what that does not mean.
fn render_receipt(b: &mut Buffer, list: Rect, lines: &[String], p: &Palette) {
    for (offset, line) in lines.iter().take(usize::from(list.height)).enumerate() {
        let style = if offset == 0 {
            Style::default()
                .fg(p.green)
                .bg(p.panel_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(p.overlay0).bg(p.panel_bg)
        };
        put_text(
            b,
            list.x,
            list.y + offset as u16,
            list.width,
            &format!(" {line}"),
            style,
        );
    }
}

fn footer_keys(stage: &PaletteStage) -> &'static [(&'static str, &'static str)] {
    match stage {
        PaletteStage::Commands => &[
            ("enter", "run"),
            ("↑↓ ^j/^k", "move"),
            ("tab", "complete"),
            ("esc", "close"),
        ],
        PaletteStage::Targets => &[("enter", "pick"), ("↑↓ ^j/^k", "move"), ("esc", "back")],
        PaletteStage::Receipt(_) => &[("enter", "close"), ("esc", "close")],
        PaletteStage::Compose(draft) => {
            let both = draft.typed_possible && draft.note_possible;
            let working = draft.target.status == AgentStatus::Working;
            if !draft.typed_possible && !draft.note_possible {
                &[("enter", "open pane"), ("esc", "back")]
            } else if draft.asking {
                &[
                    ("enter", "queue"),
                    ("alt+enter", "interrupt"),
                    ("esc", "cancel"),
                ]
            } else {
                match (both, working) {
                    (true, true) => &[
                        ("enter", "send"),
                        ("alt+enter", "interrupt first"),
                        ("tab", "you / note"),
                        ("esc", "back"),
                    ],
                    (true, false) => &[("enter", "send"), ("tab", "you / note"), ("esc", "back")],
                    (false, true) => &[
                        ("enter", "send"),
                        ("alt+enter", "interrupt first"),
                        ("esc", "back"),
                    ],
                    (false, false) => &[("enter", "send"), ("esc", "back")],
                }
            }
        }
    }
}

/// Dim the screen, then draw the bar near the top. Returns its layout and the
/// caret.
pub(super) fn render_palette(
    b: &mut Buffer,
    palette: &ClientPaletteOverlay,
    config: &ClientShellConfig,
) -> Option<(PaletteLayout, Option<crate::protocol::CursorState>)> {
    let p = &config.palette;
    for y in b.area.y..b.area.bottom() {
        for x in b.area.x..b.area.right() {
            let cell = &mut b[(x, y)];
            cell.set_style(cell.style().add_modifier(Modifier::DIM));
        }
    }
    let layout = palette_layout(palette, b.area)?;
    render_panel_shell(b, layout.outer, p.accent, p.panel_bg)?;
    render_title(b, layout.title, palette, p);
    let cursor = render_query(b, layout.query, palette, p);
    render_rule(b, layout.rule.x, layout.rule.y, layout.rule.width, p);

    let list = layout.list;
    match &palette.stage {
        PaletteStage::Commands | PaletteStage::Targets => {
            render_rows(b, list, layout.start, palette, p);
        }
        PaletteStage::Compose(draft) => render_compose(b, list, draft, config),
        PaletteStage::Receipt(lines) => render_receipt(b, list, lines, p),
    }
    if let Some(footer) = layout.footer {
        put_key_bar(b, footer, footer_keys(&palette.stage), p);
    }
    Some((layout, cursor))
}

// -- input -----------------------------------------------------------------------

impl ClientShellState {
    fn palette_mut(&mut self) -> Option<&mut ClientPaletteOverlay> {
        match self.overlay.as_mut() {
            Some(ClientShellOverlay::Palette(palette)) => Some(palette),
            _ => None,
        }
    }

    pub(super) fn open_palette(&mut self, outcome: &mut ClientShellInput) {
        let mut commands: Vec<PaletteCommand> =
            command_table(&self.config.keybinds.keybinds, &self.config.keybinds.prefix)
                .into_iter()
                .flat_map(|(group, entries)| {
                    entries.into_iter().filter_map(move |entry| {
                        let action = entry.action?;
                        // Opening the bar from the bar is a no-op row.
                        if matches!(
                            action,
                            CommandAction::Run(crate::input::KeybindAction::CommandPalette)
                        ) {
                            return None;
                        }
                        Some(PaletteCommand {
                            group,
                            key: entry.key,
                            label: entry.label.into_owned(),
                            action: PaletteAction::Command(action),
                        })
                    })
                })
                .collect();
        // The way into a send, after the global commands.
        let at = commands
            .iter()
            .position(|command| command.group != "global")
            .unwrap_or(commands.len());
        commands.insert(
            at,
            PaletteCommand {
                group: "sessions",
                key: "send".to_owned(),
                label: "send to a session…".to_owned(),
                action: PaletteAction::OpenSend,
            },
        );
        let targets = self
            .snapshot
            .as_deref()
            .map(|snapshot| {
                snapshot
                    .agents
                    .iter()
                    .map(|agent| SendTarget {
                        pane_id: agent.pane_id.clone(),
                        name: agent.name.clone(),
                        label: agent
                            .name
                            .clone()
                            .or_else(|| agent.display_agent.clone())
                            .or_else(|| agent.agent.clone())
                            .or_else(|| agent.title.clone())
                            .unwrap_or_else(|| agent.pane_id.clone()),
                        status: agent.agent_status,
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.overlay = Some(ClientShellOverlay::Palette(ClientPaletteOverlay::new(
            commands, targets,
        )));
        self.mode = ClientShellMode::Terminal;
        outcome.repaint = true;
    }

    fn visible_palette_rows(&self) -> usize {
        self.hits
            .palette
            .as_ref()
            .map_or(0, |layout| usize::from(layout.list.height))
    }

    fn close_palette(&mut self, outcome: &mut ClientShellInput) {
        self.overlay = None;
        self.mode = ClientShellMode::Terminal;
        outcome.repaint = true;
    }

    fn move_palette_selection(&mut self, chord: Chord) {
        let visible = self.visible_palette_rows();
        let Some(palette) = self.palette_mut() else {
            return;
        };
        if !palette.filtering() {
            return;
        }
        let half = (visible / 2).max(1);
        let (delta, count) = match chord {
            Chord::Prev => (-1, 1),
            Chord::Next => (1, 1),
            Chord::HalfPageUp => (-1, half),
            Chord::HalfPageDown => (1, half),
            Chord::First => (-1, palette.items.len()),
            Chord::Last => (1, palette.items.len()),
        };
        for _ in 0..count {
            if !palette.step(delta) {
                break;
            }
        }
        palette.reveal(visible);
    }

    /// The query changed: filter again. `send reviewer` typed over the commands
    /// turns the bar into the list of sessions, filtered by what follows.
    fn refilter_palette(&mut self) {
        let visible = self.visible_palette_rows();
        let Some(palette) = self.palette_mut() else {
            return;
        };
        if !palette.filtering() {
            return;
        }
        if matches!(palette.stage, PaletteStage::Commands) {
            let typed = palette.query.text().to_owned();
            if typed.len() >= SEND_WORD.len()
                && typed[..SEND_WORD.len()].eq_ignore_ascii_case(SEND_WORD)
            {
                palette.enter_targets(typed[SEND_WORD.len()..].trim_start());
                palette.reveal(visible);
                return;
            }
        }
        palette.refilter();
        palette.reveal(visible);
    }

    /// Choose the selected row: run the command as its key would (the bar
    /// closes first, then the same path a key press takes), open the session
    /// list, or start a message to the session. An indexed command without a
    /// digit completes its words instead.
    fn submit_palette(&mut self, outcome: &mut ClientShellInput) {
        let local = self.active_endpoint_id.is_local();
        let default_mode = self.config.palette_send_default_mode;
        let Some(palette) = self.palette_mut() else {
            return;
        };
        let Some(action) = palette.selected_command().map(|command| command.action) else {
            return;
        };
        let argument = palette.argument;
        let command = match action {
            PaletteAction::OpenSend => {
                palette.enter_targets("");
                outcome.repaint = true;
                return;
            }
            PaletteAction::Target(index) => {
                palette.begin_compose(index, default_mode, local);
                outcome.repaint = true;
                return;
            }
            PaletteAction::Command(command) => command,
        };
        let action = match command {
            CommandAction::Run(action) => action,
            CommandAction::Indexed(make) => match argument {
                Some(index) => make(index),
                None => {
                    self.complete_palette();
                    outcome.repaint = true;
                    return;
                }
            },
        };
        self.close_palette(outcome);
        self.record_binding(crate::input::KeybindMatch::Action(action), outcome);
    }

    fn complete_palette(&mut self) {
        let Some(palette) = self.palette_mut() else {
            return;
        };
        let Some(text) = palette.completion() else {
            return;
        };
        let keep = palette
            .selected_command()
            .map(|command| command.label.clone());
        palette.query.clear();
        palette.query.insert_str(&text);
        palette.refilter();
        if let Some(keep) = keep {
            let position = palette.items.iter().position(|item| {
                matches!(item, PaletteItem::Command(index)
                    if palette.commands[*index].label == keep)
            });
            if let Some(position) = position {
                palette.selected = position;
            }
        }
        let visible = self.visible_palette_rows();
        if let Some(palette) = self.palette_mut() {
            palette.reveal(visible);
        }
    }

    /// Text arriving as text (a paste): into the query.
    pub(super) fn insert_palette_text(&mut self, text: &str) -> bool {
        let Some(palette) = self.palette_mut() else {
            return false;
        };
        if matches!(palette.stage, PaletteStage::Receipt(_)) {
            return true;
        }
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        if palette.query.insert_str(&text) {
            self.refilter_palette();
        }
        true
    }

    /// Edit the query with a key. Returns whether the key was an edit.
    fn edit_palette_query(&mut self, key: &crate::input::TerminalKey) -> bool {
        let (code, modifiers) = (key.code, key.modifiers);
        let Some(palette) = self.palette_mut() else {
            return false;
        };
        let before = palette.query.text().to_owned();
        // Text the host composed is authoritative (AltGr, dead keys).
        let handled = match key.generated_text.as_deref().filter(|text| {
            !text.is_empty() && !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        }) {
            Some(text) => {
                let text: String = text.chars().filter(|c| !c.is_control()).collect();
                palette.query.insert_str(&text);
                true
            }
            None => apply_text_key(
                &mut palette.query,
                KeyEvent::new(code, modifiers),
                Shape::SingleLine,
            ),
        };
        if handled && palette.query.text() != before {
            self.refilter_palette();
        }
        handled
    }

    pub(super) fn route_palette_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) {
        if key.kind == crossterm::event::KeyEventKind::Release {
            return;
        }
        let Some(palette) = self.palette_mut() else {
            return;
        };
        let (code, modifiers) = (key.code, key.modifiers);
        outcome.repaint = true;
        match &mut palette.stage {
            PaletteStage::Receipt(_) => {
                if matches!(code, KeyCode::Esc | KeyCode::Enter) {
                    self.close_palette(outcome);
                } else {
                    outcome.repaint = false;
                }
                return;
            }
            PaletteStage::Compose(draft) => {
                match code {
                    KeyCode::Esc if draft.asking => draft.asking = false,
                    KeyCode::Esc => {
                        palette.enter_targets("");
                        let visible = self.visible_palette_rows();
                        if let Some(palette) = self.palette_mut() {
                            palette.reveal(visible);
                        }
                    }
                    KeyCode::Enter => {
                        self.submit_send(modifiers.contains(KeyModifiers::ALT), outcome);
                    }
                    KeyCode::Tab if modifiers.is_empty() => {
                        if draft.typed_possible && draft.note_possible {
                            draft.mode = match draft.mode {
                                AgentMessageMode::Typed => AgentMessageMode::Note,
                                AgentMessageMode::Note => AgentMessageMode::Typed,
                            };
                        }
                    }
                    _ => {
                        draft.notice = None;
                        if !self.edit_palette_query(key) {
                            outcome.repaint = false;
                        }
                    }
                }
                return;
            }
            PaletteStage::Commands | PaletteStage::Targets => {}
        }
        match code {
            KeyCode::Esc if !palette.query.text().is_empty() => {
                palette.query.clear();
                self.refilter_palette();
                return;
            }
            KeyCode::Esc if matches!(palette.stage, PaletteStage::Targets) => {
                palette.leave_targets();
                return;
            }
            KeyCode::Esc => {
                self.close_palette(outcome);
                return;
            }
            KeyCode::Enter => {
                self.submit_palette(outcome);
                return;
            }
            KeyCode::Tab if modifiers.is_empty() => {
                self.complete_palette();
                return;
            }
            _ => {}
        }
        if let Some(chord) = list_chord(code, modifiers, true) {
            self.move_palette_selection(chord);
            return;
        }
        if !self.edit_palette_query(key) {
            outcome.repaint = false;
        }
    }

    /// Send the message: through `agent.message`, the one method every client of
    /// a send uses. A blocked session with nothing to send to is focused
    /// instead; a working one waits for "queue or interrupt?" when the config
    /// says to ask.
    fn submit_send(&mut self, interrupt: bool, outcome: &mut ClientShellInput) {
        let ask_when_busy = self.config.palette_send_busy == crate::config::PaletteBusyPolicy::Ask;
        let Some(palette) = self.palette_mut() else {
            return;
        };
        let text = palette.query.text().trim().to_owned();
        let PaletteStage::Compose(draft) = &mut palette.stage else {
            return;
        };
        if draft.sending {
            return;
        }
        if !draft.typed_possible && !draft.note_possible {
            let pane_id = draft.target.pane_id.clone();
            self.close_palette(outcome);
            self.push_endpoint_method_with_kind(
                Method::PaneFocus(PaneTarget { pane_id }),
                PendingEndpointKind::Generic,
                outcome,
            );
            return;
        }
        if text.is_empty() {
            draft.notice = Some("type a message first".to_owned());
            return;
        }
        let working = draft.target.status == AgentStatus::Working;
        if working && !interrupt && ask_when_busy && !draft.asking {
            draft.asking = true;
            return;
        }
        draft.asking = false;
        draft.sending = true;
        draft.notice = None;
        let params = AgentMessageParams {
            target: draft.target.pane_id.clone(),
            text,
            mode: draft.mode,
            busy: if interrupt && working {
                AgentMessageBusy::Interrupt
            } else {
                AgentMessageBusy::Queue
            },
            answer: None,
        };
        self.push_endpoint_method_with_kind(
            Method::AgentMessage(params),
            PendingEndpointKind::AgentMessage,
            outcome,
        );
    }

    /// The answer to `agent.message`: a receipt in the bar, or the refusal under
    /// the message so it can be fixed.
    pub(super) fn handle_agent_send_result(
        &mut self,
        result: &Result<ResponseResult, ClientShellEndpointError>,
        outcome: &mut ClientShellInput,
    ) {
        let Some(palette) = self.palette_mut() else {
            return;
        };
        let PaletteStage::Compose(draft) = &mut palette.stage else {
            return;
        };
        outcome.repaint = true;
        match result {
            Ok(ResponseResult::AgentMessaged {
                delivery,
                pieces,
                interrupted,
                note,
                ..
            }) => {
                let who = draft.target.label.clone();
                let mut lines = Vec::new();
                match (delivery, note) {
                    (AgentMessageMode::Note, Some(note)) => {
                        lines.push(format!("note to {who}: {}", note.state));
                        if !note.detail.is_empty() {
                            lines.push(note.detail.clone());
                        }
                    }
                    _ => {
                        let first = if *pieces == 0 {
                            format!("answered {who}")
                        } else {
                            let plural = if *pieces == 1 { "piece" } else { "pieces" };
                            format!("typed to {who} in {pieces} {plural}, enter pressed")
                        };
                        lines.push(first);
                    }
                }
                if *interrupted {
                    lines.push("It was interrupted first.".to_owned());
                }
                lines.push(RECEIPT_CAVEAT.to_owned());
                palette.stage = PaletteStage::Receipt(lines);
            }
            Ok(_) => {
                draft.sending = false;
                draft.notice = Some("unexpected runtime response".to_owned());
            }
            Err(error) => {
                draft.sending = false;
                draft.notice = Some(error.message.clone());
            }
        }
    }

    /// Mouse while the bar is open: a click on a row chooses it, a click
    /// outside closes the bar, the wheel moves the selection.
    pub(super) fn route_palette_mouse(
        &mut self,
        mouse: MouseEvent,
        outcome: &mut ClientShellInput,
    ) {
        let Some(layout) = self.hits.palette.clone() else {
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                self.close_palette(outcome);
            }
            return;
        };
        let visible = usize::from(layout.list.height);
        let Some(palette) = self.palette_mut() else {
            return;
        };
        let filtering = palette.filtering();
        let hovered = layout
            .row_at(mouse.column, mouse.row, palette.items.len())
            .filter(|_| filtering)
            .filter(|index| matches!(palette.items.get(*index), Some(PaletteItem::Command(_))));
        match mouse.kind {
            MouseEventKind::Moved => {
                if let Some(index) = hovered {
                    outcome.repaint |= palette.selected != index;
                    palette.selected = index;
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown if filtering => {
                palette.step(if mouse.kind == MouseEventKind::ScrollUp {
                    -1
                } else {
                    1
                });
                palette.reveal(visible);
                outcome.repaint = true;
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(index) = hovered {
                    palette.selected = index;
                    self.submit_palette(outcome);
                    return;
                }
                if !super::contains(layout.outer, (mouse.column, mouse.row)) {
                    self.close_palette(outcome);
                }
            }
            _ => {}
        }
    }
}

impl ClientPaletteOverlay {
    /// Start a message to `targets[index]`. Typing as you needs this client's
    /// own machine (cross-machine typing is not built) and a session that is
    /// not blocked on a prompt; a note needs the agent's name.
    fn begin_compose(
        &mut self,
        index: usize,
        default_mode: crate::config::PaletteSendMode,
        local: bool,
    ) {
        let Some(target) = self.targets.get(index).cloned() else {
            return;
        };
        let typed_possible = local && target.status != AgentStatus::Blocked;
        let note_possible = target.name.is_some();
        let wants_note = default_mode == crate::config::PaletteSendMode::Note;
        let mode = match (wants_note, typed_possible, note_possible) {
            (true, _, true) | (false, false, true) => AgentMessageMode::Note,
            _ => AgentMessageMode::Typed,
        };
        self.query.clear();
        self.stage = PaletteStage::Compose(SendDraft {
            target,
            mode,
            typed_possible,
            note_possible,
            asking: false,
            sending: false,
            notice: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::KeybindAction;

    fn commands(rows: &[(&'static str, &str, CommandAction)]) -> Vec<PaletteCommand> {
        rows.iter()
            .map(|(group, label, action)| PaletteCommand {
                group,
                key: "k".into(),
                label: (*label).into(),
                action: PaletteAction::Command(*action),
            })
            .collect()
    }

    fn sample() -> ClientPaletteOverlay {
        use CommandAction::{Indexed, Run};
        ClientPaletteOverlay::new(
            commands(&[
                ("global", "settings", Run(KeybindAction::Settings)),
                (
                    "panes",
                    "display pane labels",
                    Run(KeybindAction::DisplayPanes),
                ),
                ("panes", "split vertical", Run(KeybindAction::SplitVertical)),
                (
                    "panes",
                    "split horizontal",
                    Run(KeybindAction::SplitHorizontal),
                ),
                ("panes", "zoom pane", Run(KeybindAction::Zoom)),
                (
                    "workspaces / tabs",
                    "next workspace",
                    Run(KeybindAction::NextWorkspace),
                ),
                (
                    "workspaces / tabs",
                    "switch workspace 1-9",
                    Indexed(KeybindAction::SwitchWorkspace),
                ),
            ]),
            vec![
                target("w1:p1", Some("reviewer"), AgentStatus::Idle),
                target("w1:p2", Some("builder"), AgentStatus::Working),
                target("w1:p3", None, AgentStatus::Blocked),
            ],
        )
    }

    fn target(pane_id: &str, name: Option<&str>, status: AgentStatus) -> SendTarget {
        SendTarget {
            pane_id: pane_id.into(),
            name: name.map(str::to_owned),
            label: name.unwrap_or(pane_id).to_owned(),
            status,
        }
    }

    fn draft(palette: &ClientPaletteOverlay) -> &SendDraft {
        match &palette.stage {
            PaletteStage::Compose(draft) => draft,
            other => panic!("expected a message being written, got {other:?}"),
        }
    }

    #[test]
    fn the_session_list_replaces_the_commands_and_leaving_it_brings_them_back() {
        let mut palette = sample();
        let commands = palette.total();
        palette.enter_targets("");
        assert_eq!(palette.labels(), ["reviewer", "builder", "w1:p3"]);
        assert!(matches!(palette.stage, PaletteStage::Targets));

        typed(&mut palette, "bui");
        assert_eq!(palette.labels(), ["builder"]);

        palette.leave_targets();
        assert!(matches!(palette.stage, PaletteStage::Commands));
        assert_eq!(palette.total(), commands);
    }

    #[test]
    fn a_session_filter_typed_after_send_filters_the_sessions() {
        let mut palette = sample();
        palette.enter_targets("rev");
        assert_eq!(palette.labels(), ["reviewer"]);
        assert_eq!(palette.query.text(), "rev");
    }

    #[test]
    fn typed_is_the_default_way_to_an_idle_local_session() {
        use crate::config::PaletteSendMode;
        let mut palette = sample();
        palette.begin_compose(0, PaletteSendMode::Typed, true);
        assert_eq!(draft(&palette).mode, AgentMessageMode::Typed);
        assert!(draft(&palette).typed_possible && draft(&palette).note_possible);

        palette.begin_compose(0, PaletteSendMode::Note, true);
        assert_eq!(
            draft(&palette).mode,
            AgentMessageMode::Note,
            "the config switch"
        );
    }

    #[test]
    fn a_session_on_another_machine_is_a_note_only() {
        use crate::config::PaletteSendMode;
        let mut palette = sample();
        palette.begin_compose(0, PaletteSendMode::Typed, false);
        assert_eq!(draft(&palette).mode, AgentMessageMode::Note);
        assert!(!draft(&palette).typed_possible);
    }

    #[test]
    fn a_blocked_session_is_never_typed_into() {
        use crate::config::PaletteSendMode;
        let mut palette = sample();
        palette.begin_compose(2, PaletteSendMode::Typed, true);
        let blocked = draft(&palette);
        assert!(!blocked.typed_possible);
        assert!(!blocked.note_possible, "this one has no name to address");

        let mut named = sample();
        named.targets[2].name = Some("gate".into());
        named.begin_compose(2, PaletteSendMode::Typed, true);
        assert_eq!(draft(&named).mode, AgentMessageMode::Note);
    }

    fn typed(palette: &mut ClientPaletteOverlay, text: &str) {
        palette.query.clear();
        palette.query.insert_str(text);
        palette.refilter();
    }

    #[test]
    fn an_empty_query_lists_every_command_under_its_group_in_help_order() {
        let palette = sample();
        assert_eq!(palette.shown(), 7);
        assert_eq!(
            palette.items[..3],
            [
                PaletteItem::Heading("global"),
                PaletteItem::Command(0),
                PaletteItem::Gap
            ]
        );
        assert_eq!(palette.items[3], PaletteItem::Heading("panes"));
        assert_eq!(palette.selected, 1, "the first command, not the heading");
    }

    #[test]
    fn spl_lists_the_split_commands_before_looser_matches() {
        let mut palette = sample();
        typed(&mut palette, "spl");
        let labels = palette.labels();
        assert_eq!(labels[..2], ["split vertical", "split horizontal"]);
        assert!(labels.contains(&"display pane labels"), "{labels:?}");
        assert!(!labels.contains(&"zoom pane"));
    }

    #[test]
    fn zoom_finds_the_zoom_command_first() {
        let mut palette = sample();
        typed(&mut palette, "zoom");
        assert_eq!(palette.labels(), ["zoom pane"]);
    }

    #[test]
    fn ws_matches_the_workspace_commands_and_the_filter_keeps_help_order_on_ties() {
        let mut palette = sample();
        typed(&mut palette, "ws");
        let labels = palette.labels();
        assert!(!labels.is_empty());
        assert!(labels[0].contains("workspace"), "{labels:?}");
    }

    #[test]
    fn a_digit_after_an_indexed_commands_words_is_its_argument() {
        let mut palette = sample();
        typed(&mut palette, "switch workspace 3");
        assert_eq!(palette.labels(), ["switch workspace 1-9"]);
        assert_eq!(palette.argument, Some(2));

        typed(&mut palette, "switch workspace");
        assert_eq!(palette.argument, None);
    }

    #[test]
    fn a_digit_after_words_that_match_no_indexed_command_stays_text() {
        let mut palette = sample();
        typed(&mut palette, "zoom 3");
        assert_eq!(palette.argument, None);
    }

    #[test]
    fn stepping_skips_headings_and_gaps_and_stops_at_the_ends() {
        let mut palette = sample();
        assert!(!palette.step(-1), "already on the first command");
        palette.selected = 1;
        assert!(palette.step(1));
        assert!(matches!(
            palette.items[palette.selected],
            PaletteItem::Command(1)
        ));
        for _ in 0..20 {
            palette.step(1);
        }
        assert!(matches!(
            palette.items[palette.selected],
            PaletteItem::Command(6)
        ));
    }

    #[test]
    fn tab_completes_an_indexed_command_up_to_its_digit() {
        let mut palette = sample();
        typed(&mut palette, "switch");
        assert_eq!(
            palette.completion().as_deref(),
            Some("switch workspace "),
            "the range is left for the user to type"
        );
    }
}
