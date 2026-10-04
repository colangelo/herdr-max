use std::borrow::Cow;

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};

use super::release_notes::release_notes_close_button_rect;
use super::scrollbar::{release_notes_scrollbar_rect, render_scrollbar};
use super::widgets::{
    modal_stack_areas, panel_contrast_fg, render_action_button, render_modal_header,
    render_modal_shell,
};
use crate::app::state::OverlayKind;
use crate::app::AppState;

pub(super) type HelpEntry = (String, Cow<'static, str>);
pub(super) type HelpGroup = (&'static str, Vec<HelpEntry>);

fn help_entry(key: impl Into<String>, label: &'static str) -> HelpEntry {
    (key.into(), Cow::Borrowed(label))
}

fn keybind_label(bindings: &crate::config::ActionKeybinds) -> String {
    bindings.label().unwrap_or_else(|| "unset".to_string())
}

fn indexed_label(bindings: &[crate::config::IndexedKeybind]) -> String {
    if bindings.is_empty() {
        return "unset".to_string();
    }

    let mut parts = Vec::new();
    let mut index = 0;
    while index < bindings.len() {
        if let Some(prefix) = indexed_range_prefix(&bindings[index..], b'1', 9) {
            parts.push(format!("{prefix}1..9"));
            index += 9;
        } else if let Some(prefix) = indexed_range_prefix(&bindings[index..], b'a', 26) {
            parts.push(format!("{prefix}a..z"));
            index += 26;
        } else {
            parts.push(bindings[index].label.clone());
            index += 1;
        }
    }

    parts.join(" / ")
}

fn indexed_range_prefix(
    bindings: &[crate::config::IndexedKeybind],
    start: u8,
    len: usize,
) -> Option<&str> {
    let run = bindings.get(..len)?;
    let prefix = run[0].label.strip_suffix(char::from(start))?;
    for (offset, binding) in run.iter().enumerate() {
        let symbol = char::from(start + offset as u8);
        if binding.label.strip_suffix(symbol) != Some(prefix) {
            return None;
        }
    }
    Some(prefix)
}

/// What an overlay contributes to the keybinding help panel.
///
/// The match below is exhaustive over [`OverlayKind`], so a new overlay does
/// not compile until it says what it puts in the panel — which is the
/// `AGENTS.md` rule "new keybindings must be discoverable in the help panel",
/// enforced by the compiler instead of by remembering it.
pub(super) enum OverlayHelp {
    /// The entries this overlay contributes, in the order the panel shows
    /// them. Never empty.
    Entries(Vec<HelpEntry>),
    /// This overlay has no keybinding to document, and why. The panel renders
    /// `unset` for an unbound action, so this is only for surfaces that are
    /// not reached by a key at all.
    // The reason is the point of the variant — it is what makes "no entry" a
    // recorded decision rather than an omission — and the guard test is what
    // reads it.
    #[allow(dead_code)]
    NoKeybinding(&'static str),
}

impl OverlayHelp {
    fn entries(self) -> Vec<HelpEntry> {
        match self {
            Self::Entries(entries) => entries,
            Self::NoKeybinding(_) => Vec::new(),
        }
    }
}

pub(super) fn overlay_help(kind: OverlayKind, kb: &crate::config::Keybinds) -> OverlayHelp {
    use OverlayHelp::{Entries, NoKeybinding};
    match kind {
        OverlayKind::KeybindHelp => Entries(vec![help_entry(keybind_label(&kb.help), "keybinds")]),
        OverlayKind::Settings => Entries(vec![help_entry(keybind_label(&kb.settings), "settings")]),
        OverlayKind::NotificationCenter => Entries(vec![help_entry(
            keybind_label(&kb.open_notification_center),
            "notification center",
        )]),
        OverlayKind::Navigator => Entries(vec![help_entry(
            keybind_label(&kb.goto),
            "session navigator",
        )]),
        OverlayKind::NewLinkedWorktree => Entries(vec![help_entry(
            keybind_label(&kb.new_worktree),
            "new worktree",
        )]),
        OverlayKind::OpenExistingWorktree => Entries(vec![help_entry(
            keybind_label(&kb.open_worktree),
            "open worktree",
        )]),
        OverlayKind::ConfirmRemoveWorktree => Entries(vec![help_entry(
            keybind_label(&kb.remove_worktree),
            "delete worktree checkout",
        )]),
        OverlayKind::PaneMoveTargetPicker => Entries(vec![help_entry(
            keybind_label(&kb.move_pane_to_tab),
            "move pane to tab or space",
        )]),
        OverlayKind::PaneTodos => Entries(vec![help_entry(
            keybind_label(&kb.open_pane_todos),
            "pane todos",
        )]),
        OverlayKind::PaneTodoEdit => Entries(vec![help_entry(
            keybind_label(&kb.add_pane_todo),
            "add pane todo",
        )]),
        OverlayKind::TodoBoard => Entries(vec![help_entry(
            keybind_label(&kb.open_todo_board),
            "session todo board",
        )]),
        OverlayKind::DisplayPanes => Entries(vec![help_entry(
            keybind_label(&kb.display_panes),
            "show pane numbers and sizes",
        )]),
        OverlayKind::GlobalMenu => {
            NoKeybinding("opened from the launcher glyph in the tab bar, by mouse only")
        }
        OverlayKind::ContextMenu => NoKeybinding("opened by right-clicking a sidebar row"),
        OverlayKind::ReleaseNotes => {
            NoKeybinding("opened from the global menu's what's-new entry after an update")
        }
        OverlayKind::ProductAnnouncement => {
            NoKeybinding("shown once at startup when a release has something to announce")
        }
    }
}

fn overlay_entries(kind: OverlayKind, kb: &crate::config::Keybinds) -> Vec<HelpEntry> {
    overlay_help(kind, kb).entries()
}

pub(super) fn keybind_help_groups(app: &AppState) -> Vec<HelpGroup> {
    let kb = &app.keybinds;
    let mut groups = Vec::new();

    let mut global = vec![help_entry(app.prefix_label(), "prefix mode")];
    global.extend(overlay_entries(OverlayKind::KeybindHelp, kb));
    global.extend(overlay_entries(OverlayKind::Settings, kb));
    global.extend([
        help_entry(keybind_label(&kb.detach), "detach"),
        help_entry(keybind_label(&kb.reload_config), "reload config"),
        help_entry(
            keybind_label(&kb.open_notification_target),
            "open notification target",
        ),
    ]);
    global.extend(overlay_entries(OverlayKind::NotificationCenter, kb));
    groups.push(("global", global));

    groups.push((
        "navigation",
        vec![
            help_entry("esc", "back"),
            help_entry(
                format!(
                    "{} / {}",
                    keybind_label(&kb.navigate.workspace_up),
                    keybind_label(&kb.navigate.workspace_down)
                ),
                "workspace list",
            ),
            help_entry(
                format!(
                    "{} / {} / {} / {} / left / right",
                    keybind_label(&kb.navigate.pane_left),
                    keybind_label(&kb.navigate.pane_down),
                    keybind_label(&kb.navigate.pane_up),
                    keybind_label(&kb.navigate.pane_right)
                ),
                "move focus",
            ),
            help_entry("tab / shift+tab", "cycle pane"),
            help_entry("enter", "open workspace"),
            help_entry("1..9", "switch workspace"),
        ],
    ));

    let mut workspace_tab = vec![help_entry(
        keybind_label(&kb.workspace_picker),
        "workspace navigation",
    )];
    workspace_tab.extend(overlay_entries(OverlayKind::Navigator, kb));
    workspace_tab.push(help_entry(
        keybind_label(&kb.new_workspace),
        "new workspace",
    ));
    workspace_tab.extend(overlay_entries(OverlayKind::NewLinkedWorktree, kb));
    workspace_tab.extend(overlay_entries(OverlayKind::OpenExistingWorktree, kb));
    workspace_tab.extend(overlay_entries(OverlayKind::ConfirmRemoveWorktree, kb));
    workspace_tab.extend([
        help_entry(keybind_label(&kb.rename_workspace), "rename workspace"),
        help_entry(keybind_label(&kb.toggle_pin_workspace), "pin workspace"),
        help_entry(keybind_label(&kb.toggle_pin_agent), "pin agent"),
        help_entry(keybind_label(&kb.close_workspace), "close workspace"),
        help_entry(keybind_label(&kb.previous_workspace), "previous workspace"),
        help_entry(keybind_label(&kb.next_workspace), "next workspace"),
        help_entry(indexed_label(&kb.switch_workspace), "switch workspace 1-9"),
        help_entry(keybind_label(&kb.previous_agent), "previous agent"),
        help_entry(keybind_label(&kb.next_agent), "next agent"),
        help_entry(indexed_label(&kb.focus_agent), "focus agent 1-9"),
        help_entry(keybind_label(&kb.new_tab), "new tab"),
        help_entry(keybind_label(&kb.rename_tab), "rename tab"),
        help_entry(keybind_label(&kb.previous_tab), "previous tab"),
        help_entry(keybind_label(&kb.next_tab), "next tab"),
        help_entry(keybind_label(&kb.move_tab_previous), "move tab left"),
        help_entry(keybind_label(&kb.move_tab_next), "move tab right"),
        help_entry(indexed_label(&kb.switch_tab), "switch tab 1-9"),
        help_entry(keybind_label(&kb.close_tab), "close tab"),
    ]);
    groups.push(("workspaces / tabs", workspace_tab));

    let mut panes = vec![
        help_entry(keybind_label(&kb.split_vertical), "split vertical"),
        help_entry(keybind_label(&kb.split_horizontal), "split horizontal"),
        help_entry(keybind_label(&kb.close_pane), "close pane"),
        help_entry(keybind_label(&kb.respawn_pane), "respawn pane"),
        help_entry(keybind_label(&kb.rename_pane), "rename pane"),
        help_entry(keybind_label(&kb.break_pane), "break pane to new tab"),
    ];
    panes.extend(overlay_entries(OverlayKind::PaneMoveTargetPicker, kb));
    panes.extend([
        help_entry(
            keybind_label(&kb.move_pane_next_tab),
            "move pane to next tab",
        ),
        help_entry(
            keybind_label(&kb.move_pane_prev_tab),
            "move pane to previous tab",
        ),
        help_entry(keybind_label(&kb.edit_scrollback), "edit scrollback"),
        help_entry(keybind_label(&kb.clear_scrollback), "clear scrollback"),
    ]);
    panes.extend(overlay_entries(OverlayKind::PaneTodos, kb));
    panes.extend(overlay_entries(OverlayKind::PaneTodoEdit, kb));
    panes.extend(overlay_entries(OverlayKind::TodoBoard, kb));
    panes.extend(overlay_entries(OverlayKind::DisplayPanes, kb));
    panes.extend([
        help_entry(keybind_label(&kb.copy_mode), "copy mode"),
        help_entry(keybind_label(&kb.copy_mode_page_up), "copy mode + page up"),
        help_entry(
            keybind_label(&kb.copy_mode_half_page_up),
            "copy mode + half page up",
        ),
        help_entry(keybind_label(&kb.copy_mode_line_up), "copy mode + line up"),
        help_entry(keybind_label(&kb.copy_mode_page_down), "scroll page down"),
        help_entry(
            keybind_label(&kb.copy_mode_half_page_down),
            "scroll half page down",
        ),
        help_entry(keybind_label(&kb.copy_mode_line_down), "scroll line down"),
        help_entry(keybind_label(&kb.zoom), "zoom pane"),
        help_entry(keybind_label(&kb.toggle_sync_panes), "sync panes"),
        help_entry(keybind_label(&kb.resize_mode), "resize mode"),
        help_entry(keybind_label(&kb.resize_pane_left), "resize pane left"),
        help_entry(keybind_label(&kb.resize_pane_down), "resize pane down"),
        help_entry(keybind_label(&kb.resize_pane_up), "resize pane up"),
        help_entry(keybind_label(&kb.resize_pane_right), "resize pane right"),
        help_entry(keybind_label(&kb.balance_panes), "balance panes"),
        help_entry(keybind_label(&kb.next_layout), "cycle layout"),
        help_entry(keybind_label(&kb.toggle_sidebar), "toggle sidebar"),
        help_entry(keybind_label(&kb.focus_pane_left), "focus pane left"),
        help_entry(keybind_label(&kb.focus_pane_down), "focus pane down"),
        help_entry(keybind_label(&kb.focus_pane_up), "focus pane up"),
        help_entry(keybind_label(&kb.focus_pane_right), "focus pane right"),
        help_entry(keybind_label(&kb.swap_pane_left), "swap pane left"),
        help_entry(keybind_label(&kb.swap_pane_down), "swap pane down"),
        help_entry(keybind_label(&kb.swap_pane_up), "swap pane up"),
        help_entry(keybind_label(&kb.swap_pane_right), "swap pane right"),
        help_entry(keybind_label(&kb.cycle_pane_next), "cycle pane next"),
        help_entry(
            keybind_label(&kb.cycle_pane_previous),
            "cycle pane previous",
        ),
        help_entry(keybind_label(&kb.last_pane), "last pane"),
    ]);
    groups.push(("panes", panes));

    // Fixed chords rather than `KeysConfig` actions — the todo panel and its
    // edit modal own their keymaps — but a shortcut absent from this panel is
    // a shortcut nobody finds, so they are listed the same way the navigation
    // group lists `esc` and `enter`.
    groups.push((
        "pane todos",
        vec![
            help_entry("enter", "edit selected todo"),
            help_entry("a", "add todo"),
            help_entry("spc", "toggle done"),
            help_entry("d", "remove todo"),
            help_entry("c", "clear done todos"),
            help_entry("g", "follow todo link"),
            help_entry("esc / q", "close panel"),
        ],
    ));

    groups.push((
        "todo edit modal",
        vec![
            help_entry("ctrl+s / alt+enter", "save todo"),
            help_entry("esc", "cancel edit"),
            help_entry("tab", "cycle priority"),
            help_entry("ctrl+l", "choose link target"),
            help_entry("ctrl+g", "save and follow the link"),
            help_entry("ctrl+t", "toggle done"),
            help_entry("enter", "insert newline"),
            help_entry("ctrl+a / ctrl+e", "line start / end"),
            help_entry("ctrl+b / ctrl+f", "character back / forward"),
            help_entry("alt+b / alt+f", "word back / forward"),
            help_entry("ctrl+d", "delete forward"),
            help_entry("ctrl+k / ctrl+u", "kill to line end / start"),
            help_entry("ctrl+w", "kill word back"),
            help_entry("ctrl+y", "yank last kill"),
            help_entry("ctrl+_ / ctrl+- / ctrl+/", "undo"),
        ],
    ));

    if !kb.custom_commands.is_empty() {
        groups.push((
            "custom",
            kb.custom_commands
                .iter()
                .map(|binding| {
                    (
                        binding.label.clone(),
                        binding
                            .description
                            .clone()
                            .map(Cow::Owned)
                            .unwrap_or(Cow::Borrowed("custom command")),
                    )
                })
                .collect(),
        ));
    }

    groups
}

/// Put the host cursor on a `" / "`-prefixed search row's insertion point.
fn filter_keybind_help_groups(groups: Vec<HelpGroup>, query: &str) -> Vec<HelpGroup> {
    if query.is_empty() {
        return groups;
    }

    let query = query.to_lowercase();
    groups
        .into_iter()
        .filter_map(|(group, entries)| {
            let entries = entries
                .into_iter()
                .filter(|(key, label)| {
                    key.to_lowercase().contains(&query) || label.to_lowercase().contains(&query)
                })
                .collect::<Vec<_>>();
            (!entries.is_empty()).then_some((group, entries))
        })
        .collect()
}

pub(crate) fn keybind_help_lines(app: &AppState) -> Vec<(usize, Line<'static>)> {
    let heading_style = super::widgets::heading_style(&app.palette);
    let key_style = super::widgets::sub_heading_style(&app.palette);
    let label_style = Style::default().fg(app.palette.text);

    let groups = filter_keybind_help_groups(keybind_help_groups(app), app.keybind_help_query());
    let key_width = groups
        .iter()
        .flat_map(|(_, entries)| entries.iter().map(|(key, _)| key.chars().count()))
        .max()
        .unwrap_or(8);

    let mut lines = Vec::new();

    if groups.is_empty() {
        let message = " no matching keybinds";
        return vec![(
            message.chars().count(),
            Line::from(Span::styled(
                message,
                Style::default().fg(app.palette.overlay1),
            )),
        )];
    }

    for (group, entries) in groups {
        lines.push((
            group.len() + 1,
            Line::from(vec![Span::styled(format!(" {group}"), heading_style)]),
        ));
        for (key, label) in entries {
            let padded_key = format!(" {:<width$} ", key, width = key_width);
            let width = padded_key.chars().count() + label.chars().count();
            lines.push((
                width,
                Line::from(vec![
                    Span::styled(padded_key, key_style),
                    Span::styled(label.into_owned(), label_style),
                ]),
            ));
        }
        lines.push((0, Line::raw("")));
    }

    lines
}

pub(super) fn render_keybind_help_overlay(app: &AppState, frame: &mut Frame) {
    let Some(help) = app.keybind_help() else {
        return;
    };
    super::dim_background(frame, frame.area());

    let Some(inner) = render_modal_shell(frame, frame.area(), 76, 22, &app.palette) else {
        return;
    };
    if inner.height < 6 || inner.width < 20 {
        return;
    }

    let stack = modal_stack_areas(inner, 2, 1, 0, 1);
    let header_rows =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas::<2>(stack.header);

    render_modal_header(frame, header_rows[0], "keybinds", &app.palette);
    render_action_button(
        frame,
        release_notes_close_button_rect(header_rows[0]),
        Some("esc"),
        if help.search_focused { "back" } else { "close" },
        Style::default()
            .fg(panel_contrast_fg(&app.palette))
            .bg(app.palette.accent)
            .add_modifier(Modifier::BOLD),
    );
    let search_line = if help.search_focused {
        Line::from(vec![
            Span::styled(
                " / ",
                Style::default()
                    .fg(app.palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                help.query.text(),
                Style::default()
                    .fg(app.palette.text)
                    .add_modifier(Modifier::BOLD),
            ),
        ])
    } else {
        Line::from(Span::styled(
            " press / to filter by command or shortcut",
            Style::default().fg(app.palette.overlay0),
        ))
    };
    frame.render_widget(Paragraph::new(search_line), header_rows[1]);
    if help.search_focused {
        // The search box has an insertion point now, so the host cursor goes
        // where it is — an IME composes at the host cursor, and a caret the
        // user cannot see is a caret they cannot use.
        crate::ui::overlay::set_search_caret(frame, header_rows[1], &help.query);
    }

    let body_area = stack.content;
    let metrics = crate::pane::ScrollMetrics {
        offset_from_bottom: app.keybind_help_max_scroll().saturating_sub(help.scroll) as usize,
        max_offset_from_bottom: app.keybind_help_max_scroll() as usize,
        viewport_rows: body_area.height.max(1) as usize,
    };
    let track = release_notes_scrollbar_rect(body_area, metrics);
    let text_area = track
        .map(|_| {
            Rect::new(
                body_area.x,
                body_area.y,
                body_area.width.saturating_sub(1),
                body_area.height,
            )
        })
        .unwrap_or(body_area);

    let body = Paragraph::new(
        keybind_help_lines(app)
            .into_iter()
            .map(|(_, line)| line)
            .collect::<Vec<_>>(),
    )
    .wrap(Wrap { trim: false })
    .scroll((help.scroll, 0));
    frame.render_widget(body, text_area);
    if let Some(track) = track {
        render_scrollbar(
            frame,
            metrics,
            track,
            app.palette.overlay0,
            app.palette.overlay1,
            "▐",
        );
    }

    let footer = if help.search_focused {
        Line::from(vec![
            Span::styled(" filter ", Style::default().fg(app.palette.overlay0)),
            Span::styled("type/backspace", Style::default().fg(app.palette.text)),
            Span::styled(" · ", Style::default().fg(app.palette.overlay0)),
            Span::styled("clear ", Style::default().fg(app.palette.overlay0)),
            Span::styled("ctrl+u", Style::default().fg(app.palette.text)),
            Span::styled(" · ", Style::default().fg(app.palette.overlay0)),
            Span::styled("scroll ", Style::default().fg(app.palette.overlay0)),
            Span::styled("↑↓/pgup/pgdn", Style::default().fg(app.palette.text)),
            Span::styled(" · ", Style::default().fg(app.palette.overlay0)),
            Span::styled("back ", Style::default().fg(app.palette.overlay0)),
            Span::styled("esc", Style::default().fg(app.palette.text)),
        ])
    } else {
        Line::from(vec![
            Span::styled(" search ", Style::default().fg(app.palette.overlay0)),
            Span::styled("/", Style::default().fg(app.palette.text)),
            Span::styled(" · ", Style::default().fg(app.palette.overlay0)),
            Span::styled("scroll ", Style::default().fg(app.palette.overlay0)),
            Span::styled("j/k/↑↓/pgup/pgdn", Style::default().fg(app.palette.text)),
            Span::styled(" · ", Style::default().fg(app.palette.overlay0)),
            Span::styled("close ", Style::default().fg(app.palette.overlay0)),
            Span::styled("esc/enter", Style::default().fg(app.palette.text)),
        ])
    };
    frame.render_widget(Paragraph::new(footer), stack.footer.unwrap_or_default());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn groups() -> Vec<HelpGroup> {
        vec![
            (
                "workspaces / tabs",
                vec![
                    help_entry("w", "workspace navigation"),
                    help_entry("c", "new tab"),
                ],
            ),
            (
                "panes",
                vec![
                    help_entry("v", "split vertical"),
                    help_entry("x", "close pane"),
                ],
            ),
        ]
    }

    #[test]
    fn keybind_help_filter_matches_labels_case_insensitively() {
        let filtered = filter_keybind_help_groups(groups(), "WoRk");

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, "workspaces / tabs");
        assert_eq!(filtered[0].1.len(), 1);
        assert_eq!(filtered[0].1[0].1, "workspace navigation");
    }

    #[test]
    fn keybind_help_filter_matches_shortcuts_without_matching_group_headings() {
        let filtered = filter_keybind_help_groups(groups(), "x");

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, "panes");
        assert_eq!(filtered[0].1.len(), 1);
        assert_eq!(filtered[0].1[0].1, "close pane");

        assert!(filter_keybind_help_groups(groups(), "panes").is_empty());
    }

    #[test]
    fn the_downward_scroll_gestures_are_discoverable_in_the_help_panel() {
        let state = crate::app::state::AppState::test_new();

        let panes = keybind_help_groups(&state)
            .into_iter()
            .find(|(group, _)| *group == "panes")
            .expect("the panes group should exist")
            .1;

        for (label, expected_key) in [
            ("scroll page down", "prefix+pagedown"),
            ("scroll half page down", "prefix+ctrl+d"),
            ("scroll line down", "prefix+ctrl+j"),
        ] {
            let entry = panes
                .iter()
                .find(|(_, entry_label)| entry_label == label)
                .unwrap_or_else(|| panic!("{label} should appear in the help panel"));
            assert_eq!(entry.0, expected_key);
        }
    }

    /// A shortcut that works but is absent from `prefix+?` is incomplete work.
    #[test]
    fn respawn_pane_is_discoverable_in_the_help_panel() {
        let state = crate::app::state::AppState::test_new();

        let entry = keybind_help_groups(&state)
            .into_iter()
            .find(|(group, _)| *group == "panes")
            .expect("the panes group should exist")
            .1
            .into_iter()
            .find(|(_, label)| label == "respawn pane")
            .expect("respawn pane should appear in the help panel");

        assert_eq!(entry.0, "prefix+ctrl+x");
    }

    /// Opened the way `open_keybind_help` opens it — the fields it resets are
    /// the fields the render reads.
    #[test]
    fn snapshot_keybind_help() {
        crate::ui::test_support::overlay_snapshot_of(|app| {
            app.open_overlay(crate::app::state::Overlay::KeybindHelp(
                crate::app::state::KeybindHelpState::default(),
            ));
        })
        .assert(
            Rect::new(2, 1, 76, 22),
            &[
                "┌──────────────────────────────────────────────────────────────────────────┐",
                "│ keybinds                                                       esc close │",
                "│ press / to filter by command or shortcut                                 │",
                "│                                                                          │",
                "│ global                                                                  ▐│",
                "│ ctrl+b                       prefix mode                                ▐│",
                "│ prefix+?                     keybinds                                   ▕│",
                "│ prefix+s                     settings                                   ▕│",
                "│ prefix+q                     detach                                     ▕│",
                "│ prefix+shift+r               reload config                              ▕│",
                "│ prefix+o                     open notification target                   ▕│",
                "│ prefix+ctrl+n                notification center                        ▕│",
                "│                                                                         ▕│",
                "│ navigation                                                              ▕│",
                "│ esc                          back                                       ▕│",
                "│ up / down                    workspace list                             ▕│",
                "│ h / j / k / l / left / right move focus                                 ▕│",
                "│ tab / shift+tab              cycle pane                                 ▕│",
                "│ enter                        open workspace                             ▕│",
                "│                                                                          │",
                "│ search / · scroll j/k/↑↓/pgup/pgdn · close esc/enter                     │",
                "└──────────────────────────────────────────────────────────────────────────┘",
            ],
        );
    }

    /// The second half of the help-panel rule. The `overlay_help` match is
    /// exhaustive, so a new overlay cannot compile without saying what it puts
    /// in the panel; this checks that what it claims is actually on screen,
    /// and that an overlay claiming nothing says why.
    #[test]
    fn every_overlay_is_accounted_for_in_the_help_panel() {
        let app = AppState::test_new();
        let groups = keybind_help_groups(&app);
        let labels: Vec<String> = groups
            .iter()
            .flat_map(|(_, entries)| entries.iter().map(|(_, label)| label.to_string()))
            .collect();

        for kind in OverlayKind::ALL {
            match overlay_help(*kind, &app.keybinds) {
                OverlayHelp::Entries(entries) => {
                    assert!(!entries.is_empty(), "{kind:?} claims entries but has none");
                    for (_, label) in entries {
                        assert!(
                            labels.contains(&label.to_string()),
                            "{kind:?} contributes {label:?}, which the panel does not show"
                        );
                    }
                }
                OverlayHelp::NoKeybinding(reason) => assert!(
                    !reason.trim().is_empty(),
                    "{kind:?} must say why it has no keybinding to document"
                ),
            }
        }
    }

    /// Every overlay's input-source answer comes from its own variant rather
    /// than from a list on `Mode` that a new overlay can fall off.
    #[test]
    fn every_overlay_mode_takes_its_ascii_answer_from_its_variant() {
        for kind in crate::app::state::OverlayKind::ALL {
            assert_eq!(
                kind.mode().wants_ascii_input(),
                kind.wants_ascii_input(),
                "{kind:?} and its mode disagree about the input source"
            );
        }
    }
}
