use super::*;

/// The fork's popup for "new worktree": 68 by 13.
const CREATE_SIZE: (u16, u16) = (68, 13);
/// The fork's popup for "delete worktree checkout?": 72 by 11.
const REMOVE_SIZE: (u16, u16) = (72, 11);
const OPEN_WIDTH: u16 = 96;
/// Rows the open picker spends outside its entries: title, blank, search and
/// rule above them; the status row and the button row below.
const OPEN_CHROME_ROWS: u16 = 6;

/// How many 2-row entries the open picker shows in an inner area this tall.
pub(super) fn open_visible_entries(inner_height: u16) -> usize {
    usize::from(inner_height.saturating_sub(OPEN_CHROME_ROWS) / 2)
}

/// The open picker's popup height: two rows per entry plus the chrome, kept
/// between 13 and 27 rows. One formula for drawing and hit-testing.
fn open_popup_height(entries: usize) -> u16 {
    (entries.saturating_mul(2) + usize::from(OPEN_CHROME_ROWS) + 2).clamp(13, 27) as u16
}

/// The count at the right of the search row: every checkout, or "shown/all"
/// once the query is non-blank.
fn open_count_text(open: &ClientWorktreeOpenOverlay, shown: usize) -> String {
    if open.query.trim().is_empty() {
        format!("{} checkouts", open.entries.len())
    } else {
        format!("{shown}/{} checkouts", open.entries.len())
    }
}

pub(super) fn render_worktree_create_overlay(
    b: &mut Buffer,
    create: &ClientWorktreeCreateOverlay,
    p: &Palette,
) -> Option<OverlayRender> {
    use ratatui::widgets::{Paragraph, Widget, Wrap};

    let popup = popup(b.area, CREATE_SIZE.0, CREATE_SIZE.1)?;
    let inner = panel(b, popup, p.accent, p.panel_bg)?;
    if inner.height < 9 {
        return Some(OverlayRender {
            area: popup,
            ..OverlayRender::default()
        });
    }
    render_modal_header(
        b,
        Rect::new(inner.x, inner.y, inner.width, 1),
        "new worktree",
        p,
    );
    put_text(
        b,
        inner.x,
        inner.y + 2,
        inner.width,
        " branch",
        Style::default().fg(p.overlay0).bg(p.panel_bg),
    );
    let input = Rect::new(inner.x, inner.y + 3, inner.width, 1);
    b.set_style(input, Style::default().fg(p.text).bg(p.surface0));
    let cursor = text_editor::render(
        b,
        Rect::new(input.x + 1, input.y, input.width.saturating_sub(1), 1),
        &create.branch,
        Style::default().fg(p.text).bg(p.surface0),
    );
    put_text(
        b,
        inner.x,
        inner.y + 4,
        inner.width,
        " checkout",
        Style::default().fg(p.overlay0).bg(p.panel_bg),
    );
    put_text(
        b,
        inner.x,
        inner.y + 5,
        inner.width,
        &format!(" {}", create.checkout_path),
        Style::default().fg(p.subtext0).bg(p.panel_bg),
    );
    // Three rows for the status: git's errors run to several lines.
    let status = Rect::new(inner.x, inner.y + 6, inner.width, 3);
    if create.creating {
        put_text(
            b,
            status.x,
            status.y,
            status.width,
            " creating…",
            Style::default().fg(p.overlay0).bg(p.panel_bg),
        );
    } else if let Some(error) = create.error.as_deref() {
        Widget::render(
            Paragraph::new(format!(" {error}"))
                .style(Style::default().fg(p.red).bg(p.panel_bg))
                .wrap(Wrap { trim: false }),
            status,
            b,
        );
    }
    let buttons = render_button_row(
        b,
        inner,
        &[
            ModalButton::primary("↵", "create and open", p.accent),
            ModalButton::secondary("esc", "cancel"),
        ],
        BUTTON_GAP,
        inner.height.saturating_sub(1),
        p,
    );
    let [primary, cancel] = buttons.as_slice() else {
        return None;
    };
    Some(OverlayRender {
        area: popup,
        primary: *primary,
        cancel: *cancel,
        cursor: cursor.filter(|_| !create.creating),
        ..OverlayRender::default()
    })
}

pub(super) fn render_worktree_open_overlay(
    b: &mut Buffer,
    open: &ClientWorktreeOpenOverlay,
    p: &Palette,
) -> Option<OverlayRender> {
    let popup = popup(b.area, OPEN_WIDTH, open_popup_height(open.entries.len()))?;
    let inner = panel(b, popup, p.accent, p.panel_bg)?;
    if inner.height < 9 {
        return Some(OverlayRender {
            area: popup,
            ..OverlayRender::default()
        });
    }
    render_modal_header(
        b,
        Rect::new(inner.x, inner.y, inner.width, 1),
        "open worktree",
        p,
    );

    // Search row: " / ", the query or its placeholder, the count at the right.
    let search = Rect::new(inner.x, inner.y + 2, inner.width, 1);
    let filtered = open.filtered_indices();
    let count = open_count_text(open, filtered.len());
    let count_width = display_width(&count);
    put_text(
        b,
        search.x,
        search.y,
        search.width.min(3),
        " / ",
        if open.search_focused {
            Style::default()
                .fg(p.accent)
                .bg(p.panel_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(p.overlay0).bg(p.panel_bg)
        },
    );
    // What the query may use before it would reach the count.
    let text_room = search.width.saturating_sub(4 + count_width);
    let cursor = if open.search_focused {
        text_editor::render(
            b,
            Rect::new(search.x + 3, search.y, text_room, 1),
            &open.query,
            Style::default().fg(p.text).bg(p.panel_bg),
        )
    } else {
        if open.query.trim().is_empty() {
            put_text(
                b,
                search.x + 3,
                search.y,
                text_room,
                "filter worktrees",
                Style::default().fg(p.overlay0).bg(p.panel_bg),
            );
        } else {
            put_text(
                b,
                search.x + 3,
                search.y,
                text_room,
                &open.query,
                Style::default().fg(p.text).bg(p.panel_bg),
            );
        }
        None
    };
    put_right_text(
        b,
        search,
        search.y,
        &count,
        Style::default().fg(p.overlay0).bg(p.panel_bg),
    );
    put_text(
        b,
        inner.x,
        inner.y + 3,
        inner.width,
        &"─".repeat(inner.width as usize),
        Style::default().fg(p.surface1).bg(p.panel_bg),
    );

    let max_rows = open_visible_entries(inner.height);
    let selected_entry = open.selected_entry_index();
    let selected_position = filtered
        .iter()
        .position(|index| Some(*index) == selected_entry)
        .unwrap_or(0);
    let start = crate::client::shell::move_picker::reveal_scroll(
        0,
        selected_position,
        max_rows,
        filtered.len(),
    );
    let list_y = inner.y + 4;
    let mut row_hits = Vec::new();
    for (visible, entry_index) in filtered
        .iter()
        .copied()
        .skip(start)
        .take(max_rows)
        .enumerate()
    {
        let entry = &open.entries[entry_index];
        let rect = Rect::new(inner.x, list_y + visible as u16 * 2, inner.width, 2);
        row_hits.push((rect, entry_index));
        let selected = Some(entry_index) == selected_entry;
        let (name_style, path_style) = if selected {
            (
                Style::default()
                    .fg(p.text)
                    .bg(p.surface0)
                    .add_modifier(Modifier::BOLD),
                Style::default().fg(p.subtext0).bg(p.surface0),
            )
        } else {
            (
                Style::default().fg(p.subtext0).bg(p.panel_bg),
                Style::default().fg(p.overlay0).bg(p.panel_bg),
            )
        };
        b.set_style(rect, name_style);
        let status = entry.status_label();
        let marker = if selected { "›" } else { " " };
        let name_room = usize::from(
            inner
                .width
                .saturating_sub(display_width(status))
                .saturating_sub(4),
        );
        put_text(
            b,
            rect.x,
            rect.y,
            rect.width,
            &format!(
                "{marker} {}",
                crate::ui::text::truncate_end(&entry.label, name_room)
            ),
            name_style,
        );
        if !status.is_empty() {
            put_right_text(b, rect, rect.y, status, name_style);
        }
        put_text(
            b,
            rect.x,
            rect.y + 1,
            rect.width,
            &crate::ui::text::truncate_end(&format!("  {}", entry.path), usize::from(rect.width)),
            path_style,
        );
    }
    if filtered.is_empty() {
        // On the first list row; the fork drew it over the rule.
        put_text(
            b,
            inner.x,
            list_y,
            inner.width,
            " no matching worktrees",
            Style::default().fg(p.overlay0).bg(p.panel_bg),
        );
    }
    let status_y = inner.bottom().saturating_sub(2);
    if open.opening {
        put_text(
            b,
            inner.x,
            status_y,
            inner.width,
            " opening…",
            Style::default().fg(p.overlay0).bg(p.panel_bg),
        );
    } else if let Some(error) = open.error.as_deref() {
        put_text(
            b,
            inner.x,
            status_y,
            inner.width,
            &format!(" {error}"),
            Style::default().fg(p.red).bg(p.panel_bg),
        );
    }
    let buttons = render_button_row(
        b,
        inner,
        &[
            ModalButton::primary("↵", "open", p.accent),
            ModalButton::secondary("esc", "cancel"),
        ],
        BUTTON_GAP,
        inner.height.saturating_sub(1),
        p,
    );
    let [primary, cancel] = buttons.as_slice() else {
        return None;
    };
    Some(OverlayRender {
        area: popup,
        primary: *primary,
        cancel: *cancel,
        worktree_search: search,
        worktree_rows: row_hits,
        cursor: cursor.filter(|_| !open.opening),
        ..OverlayRender::default()
    })
}

pub(super) fn render_worktree_remove_overlay(
    b: &mut Buffer,
    remove: &ClientWorktreeRemoveOverlay,
    p: &Palette,
) -> Option<OverlayRender> {
    let popup = popup(b.area, REMOVE_SIZE.0, REMOVE_SIZE.1)?;
    let inner = panel(b, popup, p.red, p.panel_bg)?;
    if inner.height < 7 {
        return Some(OverlayRender {
            area: popup,
            ..OverlayRender::default()
        });
    }
    put_text(
        b,
        inner.x,
        inner.y,
        inner.width,
        " delete worktree checkout?",
        Style::default()
            .fg(p.red)
            .bg(p.panel_bg)
            .add_modifier(Modifier::BOLD),
    );
    put_text(
        b,
        inner.x,
        inner.y + 1,
        inner.width,
        " This removes the checkout folder:",
        Style::default().fg(p.overlay0).bg(p.panel_bg),
    );
    put_text(
        b,
        inner.x,
        inner.y + 2,
        inner.width,
        &format!(" {}", remove.path),
        Style::default().fg(p.text).bg(p.panel_bg),
    );
    put_text(
        b,
        inner.x,
        inner.y + 3,
        inner.width,
        " The branch is not deleted. The Herdr workspace will close.",
        Style::default().fg(p.overlay0).bg(p.panel_bg),
    );
    if remove.force_confirmation {
        put_text(
            b,
            inner.x,
            inner.y + 4,
            inner.width,
            " Dirty or untracked files will be permanently deleted.",
            Style::default().fg(p.red).bg(p.panel_bg),
        );
    }
    if remove.removing {
        put_text(
            b,
            inner.x,
            inner.y + 5,
            inner.width,
            " removing…",
            Style::default().fg(p.overlay0).bg(p.panel_bg),
        );
    } else if let Some(error) = remove.error.as_deref() {
        put_text(
            b,
            inner.x,
            inner.y + 5,
            inner.width,
            &format!(" {error}"),
            Style::default().fg(p.red).bg(p.panel_bg),
        );
    }
    let buttons = render_button_row(
        b,
        inner,
        &[
            ModalButton::primary(
                "↵",
                if remove.force_confirmation {
                    "delete anyway"
                } else {
                    "remove"
                },
                p.red,
            ),
            ModalButton::secondary("esc", "cancel"),
        ],
        BUTTON_GAP,
        inner.height.saturating_sub(1),
        p,
    );
    let [primary, cancel] = buttons.as_slice() else {
        return None;
    };
    Some(OverlayRender {
        area: popup,
        primary: *primary,
        cancel: *cancel,
        ..OverlayRender::default()
    })
}
