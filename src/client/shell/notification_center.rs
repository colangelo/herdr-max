//! The notification center (fork 1b621bb3, cb44a453, 80bd43fd, bc7f367e,
//! e80de3f8, 397e8f23, e37c637d, b61c92cc): the server's retained
//! notification log as a panel under the tab bar (or above a bottom-right
//! indicator), newest first, with per-entry read state.
//!
//! The log is the server's: the panel lists it with `notification.list`,
//! marks entries read with `notification.mark_seen` and empties it with
//! `notification.clear`, and fetches it again when the log summary in the
//! snapshot facts moves. Opening the panel reads nothing; only activating an
//! entry or `r` does.

use super::render::{display_width, put_text};
use super::todo_panel::render_panel_shell;
use super::*;
use crate::api::schema::{
    EmptyParams, Method, NotificationInfo, NotificationKind, NotificationMarkSeenParams, PaneTarget,
};
use crate::config::NotificationCenterPositionConfig;
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::style::Color;

/// Rows the list shows before it scrolls; also how many entries size its width.
const MAX_ROWS: usize = 12;
const MIN_WIDTH: u16 = 30;
const MAX_WIDTH: u16 = 60;
const EMPTY_CONTENT_WIDTH: usize = 16;
const FOOTER_ROWS: u16 = 2;
const BUTTON_GAP: u16 = 2;

#[derive(Debug)]
pub(super) struct ClientNotificationCenterOverlay {
    pub(super) boot_id: String,
    /// Newest first, as `notification.list` answers.
    pub(super) entries: Vec<NotificationInfo>,
    /// The log summary the list was fetched at; `None` until it lands.
    pub(super) loaded: Option<(usize, usize, u64)>,
    pub(super) list_in_flight: bool,
    /// An index, not an entry: a post while the panel is open moves what it
    /// points at, as in the fork.
    pub(super) selected: usize,
    pub(super) scroll: usize,
    pub(super) hovered_button: Option<NotificationButton>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NotificationButton {
    MarkRead,
    Clear,
    Close,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct NotificationCenterLayout {
    pub(super) outer: Rect,
    pub(super) inner: Rect,
    pub(super) list: Rect,
    pub(super) start: usize,
    pub(super) footer_row: Option<Rect>,
    pub(super) buttons: Vec<(Rect, NotificationButton)>,
}

impl NotificationCenterLayout {
    fn row_at(&self, x: u16, y: u16, len: usize) -> Option<usize> {
        if !super::contains(self.list, (x, y)) {
            return None;
        }
        let index = self.start + usize::from(y - self.list.y);
        (index < len).then_some(index)
    }
}

fn button_text(button: NotificationButton) -> &'static str {
    match button {
        NotificationButton::MarkRead => " r mark read ",
        NotificationButton::Clear => " c clear all ",
        NotificationButton::Close => " esc close ",
    }
}

fn footer_width() -> u16 {
    [
        NotificationButton::MarkRead,
        NotificationButton::Clear,
        NotificationButton::Close,
    ]
    .iter()
    .map(|button| display_width(button_text(*button)))
    .sum::<u16>()
        + 2 * BUTTON_GAP
}

/// `mark read` drops first when the row is too narrow; the other two never.
fn footer_buttons(row: Rect) -> Vec<(Rect, NotificationButton)> {
    let mut buttons = vec![
        NotificationButton::MarkRead,
        NotificationButton::Clear,
        NotificationButton::Close,
    ];
    let total = |buttons: &[NotificationButton]| {
        buttons
            .iter()
            .map(|button| display_width(button_text(*button)))
            .sum::<u16>()
            + BUTTON_GAP * buttons.len().saturating_sub(1) as u16
    };
    if total(&buttons) > row.width {
        buttons.retain(|button| *button != NotificationButton::MarkRead);
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

/// The panel's layout: right-aligned to `anchor` (the tab bar), below it at
/// the top-right, or above the floating `indicator` at the bottom-right.
pub(super) fn notification_center_layout(
    center: &ClientNotificationCenterOverlay,
    anchor: Rect,
    indicator: Rect,
    position: NotificationCenterPositionConfig,
    screen: Rect,
) -> Option<NotificationCenterLayout> {
    if screen.width == 0 || screen.height == 0 {
        return None;
    }
    let anchor = if anchor.width > 0 {
        anchor
    } else {
        Rect::new(screen.x, screen.y, screen.width, 0)
    };
    let content = center
        .entries
        .iter()
        .take(MAX_ROWS)
        .map(|entry| {
            let context = if entry.context.is_empty() {
                0
            } else {
                usize::from(display_width(&entry.context)) + 3
            };
            usize::from(display_width(&entry.title)) + context
        })
        .max()
        .unwrap_or(EMPTY_CONTENT_WIDTH);
    let rows_width = content + 2 + 3 + 5;
    let footer_rows = if center.entries.is_empty() {
        0
    } else {
        FOOTER_ROWS
    };
    let wanted = if footer_rows > 0 {
        rows_width.max(usize::from(footer_width()) + 2)
    } else {
        rows_width
    };
    let width = (wanted.clamp(usize::from(MIN_WIDTH), usize::from(MAX_WIDTH)) as u16)
        .min(screen.width.max(1));
    let rows = center.entries.len().clamp(1, MAX_ROWS) as u16;
    let height = (rows + 2 + footer_rows).min(screen.height.max(1));
    let x = anchor.right().saturating_sub(width).max(screen.x);
    let bottom = screen.bottom().saturating_sub(height);
    let top = match position {
        NotificationCenterPositionConfig::TopRight => anchor.bottom(),
        NotificationCenterPositionConfig::BottomRight if indicator.width > 0 => {
            indicator.y.saturating_sub(height)
        }
        NotificationCenterPositionConfig::BottomRight => bottom,
    };
    let y = top.min(bottom).max(screen.y);
    let outer = Rect::new(x, y, width, height);
    let inner = Rect::new(
        outer.x.saturating_add(1),
        outer.y.saturating_add(1),
        outer.width.saturating_sub(2),
        outer.height.saturating_sub(2),
    );
    let (list, footer_row) = if footer_rows > 0 && inner.width > 0 && inner.height >= FOOTER_ROWS {
        (
            Rect::new(inner.x, inner.y, inner.width, inner.height - FOOTER_ROWS),
            Some(Rect::new(inner.x, inner.bottom() - 1, inner.width, 1)),
        )
    } else {
        (inner, None)
    };
    let visible = usize::from(list.height);
    let start = center
        .scroll
        .min(center.entries.len().saturating_sub(visible));
    Some(NotificationCenterLayout {
        outer,
        inner,
        list,
        start,
        footer_row,
        buttons: footer_row.map(footer_buttons).unwrap_or_default(),
    })
}

fn contrast(p: &Palette) -> Color {
    match p.panel_bg {
        Color::Reset => p.surface_dim,
        c => c,
    }
}

fn render_entry(
    b: &mut Buffer,
    row: Rect,
    entry: &NotificationInfo,
    selected: bool,
    now_unix: u64,
    p: &Palette,
) {
    let age = crate::ui::text::relative_time_label(now_unix, entry.posted_at_unix);
    let age_width = usize::from(display_width(&age));
    let text_budget = usize::from(row.width).saturating_sub(3 + age_width + 1);
    let title = crate::ui::text::truncate_end(&entry.title, text_budget);
    let context_budget = text_budget.saturating_sub(usize::from(display_width(&title)));
    let context = if entry.context.is_empty() || context_budget < 6 {
        String::new()
    } else {
        crate::ui::text::truncate_end(&format!(" · {}", entry.context), context_budget)
    };
    let band = Style::default().fg(contrast(p)).bg(p.accent);
    let bold_if_unread = |style: Style| {
        if entry.read {
            style
        } else {
            style.add_modifier(Modifier::BOLD)
        }
    };
    let (dot_style, title_style, dim_style) = if selected {
        b.set_style(row, band);
        (band, bold_if_unread(band), band)
    } else if entry.read {
        (
            Style::default(),
            Style::default().fg(p.overlay0),
            Style::default().fg(p.overlay0),
        )
    } else {
        let dot = match entry.kind {
            NotificationKind::NeedsAttention => p.red,
            NotificationKind::Finished => p.blue,
            NotificationKind::UpdateInstalled => p.accent,
        };
        (
            Style::default().fg(dot),
            Style::default().fg(p.text).add_modifier(Modifier::BOLD),
            Style::default().fg(p.overlay0),
        )
    };
    let dot = if entry.read { "   " } else { " ● " };
    let mut x = row.x;
    let mut put = |b: &mut Buffer, text: &str, style: Style| {
        let width = display_width(text).min(row.right().saturating_sub(x));
        put_text(b, x, row.y, width, text, style);
        x = x.saturating_add(width);
    };
    put(b, dot, dot_style);
    put(b, &title, title_style);
    put(b, &context, dim_style);
    let age_x = row.right().saturating_sub(age_width as u16 + 1).max(row.x);
    put_text(b, age_x, row.y, age_width as u16, &age, dim_style);
}

pub(super) fn render_notification_center(
    b: &mut Buffer,
    center: &ClientNotificationCenterOverlay,
    anchor: Rect,
    indicator: Rect,
    position: NotificationCenterPositionConfig,
    now_unix: u64,
    p: &Palette,
) -> Option<NotificationCenterLayout> {
    // Nothing until the first list lands.
    center.loaded?;
    let layout = notification_center_layout(center, anchor, indicator, position, b.area)?;
    render_panel_shell(b, layout.outer, p.accent, p.panel_bg)?;
    if center.entries.is_empty() {
        put_text(
            b,
            layout.inner.x,
            layout.inner.y,
            layout.inner.width,
            " no notifications",
            Style::default().fg(p.overlay0),
        );
        return Some(layout);
    }
    for (offset, entry) in center
        .entries
        .iter()
        .enumerate()
        .skip(layout.start)
        .take(usize::from(layout.list.height))
    {
        let row = Rect::new(
            layout.list.x,
            layout.list.y + (offset - layout.start) as u16,
            layout.list.width,
            1,
        );
        render_entry(b, row, entry, offset == center.selected, now_unix, p);
    }
    for (rect, button) in &layout.buttons {
        let style = if center.hovered_button == Some(*button) {
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

/// The log summary in the snapshot facts: total, unread and newest id.
fn log_summary(snapshot: &ClientShellSnapshot) -> Option<(usize, usize, u64)> {
    let summary = snapshot.resource_facts.as_ref()?.notifications.as_ref()?;
    Some((summary.total, summary.unread, summary.latest_id))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CenterAction {
    Activate,
    MarkAllRead,
    Clear,
    Close,
}

impl ClientShellState {
    fn notification_center_mut(&mut self) -> Option<&mut ClientNotificationCenterOverlay> {
        match self.overlay.as_mut() {
            Some(ClientShellOverlay::NotificationCenter(center)) => Some(center),
            _ => None,
        }
    }

    /// The indicator's toggle: open the center, or close it when it is open.
    pub(super) fn toggle_notification_center(&mut self, outcome: &mut ClientShellInput) {
        if matches!(
            self.overlay,
            Some(ClientShellOverlay::NotificationCenter(_))
        ) {
            self.overlay = None;
            outcome.repaint = true;
            return;
        }
        self.open_notification_center(outcome);
    }

    pub(super) fn open_notification_center(&mut self, outcome: &mut ClientShellInput) {
        let Some(boot_id) = self
            .snapshot
            .as_deref()
            .map(|snapshot| snapshot.boot_id.clone())
        else {
            return;
        };
        self.overlay = Some(ClientShellOverlay::NotificationCenter(
            ClientNotificationCenterOverlay {
                boot_id,
                entries: Vec::new(),
                loaded: None,
                list_in_flight: false,
                selected: 0,
                scroll: 0,
                hovered_button: None,
            },
        ));
        self.request_notification_list(outcome);
        outcome.repaint = true;
    }

    fn request_notification_list(&mut self, outcome: &mut ClientShellInput) {
        let Some(summary) = self.snapshot.as_deref().map(log_summary) else {
            return;
        };
        if self
            .notification_center_mut()
            .is_none_or(|center| center.list_in_flight)
        {
            return;
        }
        if self.push_endpoint_method_with_kind(
            Method::NotificationList(EmptyParams::default()),
            PendingEndpointKind::NotificationList {
                summary: summary.unwrap_or_default(),
            },
            outcome,
        ) {
            if let Some(center) = self.notification_center_mut() {
                center.list_in_flight = true;
            }
        }
    }

    /// Close the center when its server boot is gone; fetch the log again when
    /// its summary moved. Runs from the client's timer, never from render.
    pub(crate) fn tick_notification_center(&mut self, outcome: &mut ClientShellInput) {
        let Some(ClientShellOverlay::NotificationCenter(center)) = self.overlay.as_ref() else {
            return;
        };
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        if snapshot.boot_id != center.boot_id {
            self.overlay = None;
            outcome.repaint = true;
            return;
        }
        if center.list_in_flight {
            return;
        }
        let summary = log_summary(snapshot).unwrap_or_default();
        if center.loaded != Some(summary) {
            self.request_notification_list(outcome);
        }
    }

    pub(super) fn handle_notification_list_result(
        &mut self,
        summary: (usize, usize, u64),
        result: Result<crate::api::schema::ResponseResult, ClientShellEndpointError>,
    ) -> bool {
        let Some(center) = self.notification_center_mut() else {
            return false;
        };
        center.list_in_flight = false;
        center.loaded = Some(summary);
        let Ok(crate::api::schema::ResponseResult::NotificationList { notifications, .. }) = result
        else {
            return false;
        };
        center.entries = notifications;
        true
    }

    /// After a change lands, fetch the log at once.
    pub(super) fn handle_notification_mutation_result(
        &mut self,
        ok: bool,
        outcome: &mut ClientShellInput,
    ) {
        if ok && self.notification_center_mut().is_some() {
            self.request_notification_list(outcome);
        }
    }

    fn visible_notification_rows(&self) -> usize {
        self.hits
            .notification_center
            .as_ref()
            .map(|layout| usize::from(layout.list.height))
            .unwrap_or(MAX_ROWS)
    }

    fn move_notification_selection(&mut self, target: impl FnOnce(usize, usize, usize) -> usize) {
        let visible = self.visible_notification_rows();
        let Some(center) = self.notification_center_mut() else {
            return;
        };
        let len = center.entries.len();
        if len == 0 {
            center.selected = 0;
            center.scroll = 0;
            return;
        }
        center.selected = target(center.selected, len - 1, visible).min(len - 1);
        if center.selected < center.scroll {
            center.scroll = center.selected;
        } else if visible > 0 && center.selected >= center.scroll + visible {
            center.scroll = center.selected + 1 - visible;
        }
    }

    fn apply_notification_action(&mut self, action: CenterAction, outcome: &mut ClientShellInput) {
        outcome.repaint = true;
        match action {
            CenterAction::Close => self.overlay = None,
            CenterAction::MarkAllRead => {
                self.push_endpoint_method_with_kind(
                    Method::NotificationMarkSeen(NotificationMarkSeenParams { id: None }),
                    PendingEndpointKind::NotificationMutation,
                    outcome,
                );
            }
            CenterAction::Clear => {
                if let Some(center) = self.notification_center_mut() {
                    center.selected = 0;
                    center.scroll = 0;
                    center.hovered_button = None;
                }
                self.push_endpoint_method_with_kind(
                    Method::NotificationClear(EmptyParams::default()),
                    PendingEndpointKind::NotificationMutation,
                    outcome,
                );
            }
            CenterAction::Activate => self.activate_notification(outcome),
        }
    }

    /// The fork's activate: an entry with no target, or whose workspace is
    /// gone, does nothing. Otherwise it is marked read, the panel closes, and
    /// the pane is focused when it still exists.
    fn activate_notification(&mut self, outcome: &mut ClientShellInput) {
        let Some(entry) = self
            .notification_center_mut()
            .and_then(|center| center.entries.get(center.selected).cloned())
        else {
            return;
        };
        let Some(workspace_id) = entry.workspace_id.as_deref() else {
            return;
        };
        let workspace_alive = self.snapshot.as_deref().is_some_and(|snapshot| {
            snapshot
                .workspaces
                .iter()
                .any(|workspace| workspace.workspace_id == workspace_id)
        });
        if !workspace_alive {
            return;
        }
        self.push_endpoint_method_with_kind(
            Method::NotificationMarkSeen(NotificationMarkSeenParams { id: Some(entry.id) }),
            PendingEndpointKind::NotificationMutation,
            outcome,
        );
        self.overlay = None;
        self.mode = ClientShellMode::Terminal;
        if let Some(pane_id) = entry.pane_id {
            self.push_endpoint_method(Method::PaneFocus(PaneTarget { pane_id }), outcome);
        }
    }

    pub(super) fn route_notification_center_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) {
        if key.kind == crossterm::event::KeyEventKind::Release {
            return;
        }
        let (code, modifiers) = match key.code {
            KeyCode::Char(c) if key.modifiers == KeyModifiers::SHIFT && c.is_ascii_alphabetic() => {
                (KeyCode::Char(c.to_ascii_lowercase()), KeyModifiers::NONE)
            }
            code => (code, key.modifiers),
        };
        let bare = modifiers.is_empty();
        let ctrl = modifiers == KeyModifiers::CONTROL;
        let action = match code {
            KeyCode::Enter => Some(CenterAction::Activate),
            KeyCode::Esc => Some(CenterAction::Close),
            KeyCode::Char('c') if bare => Some(CenterAction::Clear),
            KeyCode::Char('r') if bare => Some(CenterAction::MarkAllRead),
            KeyCode::Char('q') if bare => Some(CenterAction::Close),
            _ => None,
        };
        if let Some(action) = action {
            self.apply_notification_action(action, outcome);
            return;
        }
        let half = |visible: usize| (visible / 2).max(1);
        let moved = match code {
            KeyCode::Up if bare => Some(-1),
            KeyCode::Char('k') if bare => Some(-1),
            KeyCode::Char('k' | 'p') if ctrl => Some(-1),
            KeyCode::Down if bare => Some(1),
            KeyCode::Char('j') if bare => Some(1),
            KeyCode::Char('j' | 'n') if ctrl => Some(1),
            _ => None,
        };
        if let Some(delta) = moved {
            self.move_notification_selection(|selected, last, _| {
                if delta < 0 {
                    selected.saturating_sub(1)
                } else {
                    (selected + 1).min(last)
                }
            });
            outcome.repaint = true;
            return;
        }
        match code {
            KeyCode::Char('u') if ctrl => {
                self.move_notification_selection(|selected, _, visible| {
                    selected.saturating_sub(half(visible))
                });
            }
            KeyCode::Char('d') if ctrl => {
                self.move_notification_selection(|selected, last, visible| {
                    (selected + half(visible)).min(last)
                });
            }
            KeyCode::Home if bare => self.move_notification_selection(|_, _, _| 0),
            KeyCode::End if bare => self.move_notification_selection(|_, last, _| last),
            _ => return,
        }
        outcome.repaint = true;
    }

    /// Mouse while the center is open. Every event stops here; unlike the
    /// todo panel, a press inside the panel on no row or button closes it.
    pub(super) fn route_notification_center_mouse(
        &mut self,
        mouse: MouseEvent,
        outcome: &mut ClientShellInput,
    ) {
        let point = (mouse.column, mouse.row);
        let Some(layout) = self.hits.notification_center.clone() else {
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                self.overlay = None;
                outcome.repaint = true;
            }
            return;
        };
        let len = self
            .notification_center_mut()
            .map_or(0, |center| center.entries.len());
        match mouse.kind {
            MouseEventKind::Moved => {
                let row = layout.row_at(mouse.column, mouse.row, len);
                let hovered = layout
                    .buttons
                    .iter()
                    .find(|(rect, _)| super::contains(*rect, point))
                    .map(|(_, button)| *button);
                if let Some(center) = self.notification_center_mut() {
                    if let Some(row) = row {
                        outcome.repaint |= center.selected != row;
                        center.selected = row;
                    }
                    outcome.repaint |= center.hovered_button != hovered;
                    center.hovered_button = hovered;
                }
            }
            MouseEventKind::ScrollUp => {
                self.move_notification_selection(|selected, _, _| selected.saturating_sub(1));
                outcome.repaint = true;
            }
            MouseEventKind::ScrollDown => {
                self.move_notification_selection(|selected, last, _| (selected + 1).min(last));
                outcome.repaint = true;
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(index) = layout.row_at(mouse.column, mouse.row, len) {
                    if let Some(center) = self.notification_center_mut() {
                        center.selected = index;
                    }
                    self.apply_notification_action(CenterAction::Activate, outcome);
                    return;
                }
                if let Some((_, button)) = layout
                    .buttons
                    .iter()
                    .find(|(rect, _)| super::contains(*rect, point))
                {
                    let action = match button {
                        NotificationButton::MarkRead => CenterAction::MarkAllRead,
                        NotificationButton::Clear => CenterAction::Clear,
                        NotificationButton::Close => CenterAction::Close,
                    };
                    self.apply_notification_action(action, outcome);
                    return;
                }
                if layout.footer_row.is_some_and(|row| mouse.row == row.y) {
                    return;
                }
                self.overlay = None;
                outcome.repaint = true;
            }
            _ => {}
        }
    }
}
