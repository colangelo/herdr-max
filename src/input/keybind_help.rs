use std::borrow::Cow;

use crossterm::event::{KeyCode, KeyModifiers};

use crate::{
    config::{ActionKeybinds, IndexedKeybind, Keybinds},
    input::{KeybindAction, TerminalKey},
};

pub(crate) type KeybindHelpEntry = (String, Cow<'static, str>);
pub(crate) type KeybindHelpGroup = (&'static str, Vec<KeybindHelpEntry>);

/// What running a command row does.
#[derive(Clone, Copy, Debug)]
pub(crate) enum CommandAction {
    Run(KeybindAction),
    /// A key range (`1..9`): the action takes the zero-based index the
    /// user typed after the command's words.
    Indexed(fn(usize) -> KeybindAction),
}

/// One row of the command table: the key as the help panel shows it, the
/// words, and the action when the row is a command (not a fixed chord of an
/// overlay).
#[derive(Clone, Debug)]
pub(crate) struct CommandEntry {
    pub(crate) key: String,
    pub(crate) label: Cow<'static, str>,
    pub(crate) action: Option<CommandAction>,
}

pub(crate) type CommandGroup = (&'static str, Vec<CommandEntry>);

/// The character a key types, for overlay shortcuts and text fields.
pub(crate) fn keybind_help_text_char(key: &TerminalKey) -> Option<char> {
    if !key.modifiers.difference(KeyModifiers::SHIFT).is_empty() {
        return None;
    }
    // Text the host says the key produced is authoritative: kitty hosts may
    // report Shift+/ as `/` with text `?` and no shifted alternate.
    if let Some(text) = key.generated_text.as_deref() {
        let mut chars = text.chars();
        if let (Some(character), None) = (chars.next(), chars.next()) {
            if !character.is_control() {
                return Some(character);
            }
        }
    }
    if let Some(character) = key.shifted_codepoint.and_then(char::from_u32) {
        return Some(character);
    }
    let KeyCode::Char(character) = key.code else {
        return None;
    };
    Some(character)
}

fn entry(key: impl Into<String>, label: &'static str) -> CommandEntry {
    CommandEntry {
        key: key.into(),
        label: Cow::Borrowed(label),
        action: None,
    }
}

fn act(action: KeybindAction, key: impl Into<String>, label: &'static str) -> CommandEntry {
    CommandEntry {
        action: Some(CommandAction::Run(action)),
        ..entry(key, label)
    }
}

fn indexed(
    make: fn(usize) -> KeybindAction,
    key: impl Into<String>,
    label: &'static str,
) -> CommandEntry {
    CommandEntry {
        action: Some(CommandAction::Indexed(make)),
        ..entry(key, label)
    }
}

fn binding_label(bindings: &ActionKeybinds) -> String {
    bindings.label().unwrap_or_else(|| "unset".to_owned())
}

fn indexed_label(bindings: &[IndexedKeybind]) -> String {
    if bindings.is_empty() {
        return "unset".to_owned();
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

fn indexed_range_prefix(bindings: &[IndexedKeybind], start: u8, len: usize) -> Option<&str> {
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

/// The command table: every help row, in help order, with the action its key
/// runs when it has one. The help panel reads it as (key, label) pairs and
/// the command palette reads the rows that name an action, so a command
/// added here is in both.
pub(crate) fn command_table(
    keybinds: &Keybinds,
    prefixes: &[crate::config::KeyCombo],
) -> Vec<CommandGroup> {
    let mut groups: Vec<CommandGroup> = vec![
        (
            "global",
            vec![
                entry(crate::config::format_prefix_combos(prefixes), "prefix mode"),
                act(
                    KeybindAction::Help,
                    binding_label(&keybinds.help),
                    "keybinds",
                ),
                act(
                    KeybindAction::Settings,
                    binding_label(&keybinds.settings),
                    "settings",
                ),
                act(
                    KeybindAction::CommandPalette,
                    binding_label(&keybinds.command_palette),
                    "command palette",
                ),
                act(
                    KeybindAction::Detach,
                    binding_label(&keybinds.detach),
                    "detach",
                ),
                act(
                    KeybindAction::ReloadConfig,
                    binding_label(&keybinds.reload_config),
                    "reload config",
                ),
                act(
                    KeybindAction::OpenNotificationTarget,
                    binding_label(&keybinds.open_notification_target),
                    "open notification target",
                ),
                act(
                    KeybindAction::OpenNotificationCenter,
                    binding_label(&keybinds.open_notification_center),
                    "notification center",
                ),
            ],
        ),
        (
            "navigation",
            vec![
                entry("esc", "back"),
                entry(
                    format!(
                        "{} / {}",
                        binding_label(&keybinds.navigate.workspace_up),
                        binding_label(&keybinds.navigate.workspace_down)
                    ),
                    "workspace list",
                ),
                entry(
                    format!(
                        "{} / {} / {} / {} / left / right",
                        binding_label(&keybinds.navigate.pane_left),
                        binding_label(&keybinds.navigate.pane_down),
                        binding_label(&keybinds.navigate.pane_up),
                        binding_label(&keybinds.navigate.pane_right)
                    ),
                    "move focus",
                ),
                entry("tab / shift+tab", "cycle pane"),
                entry("enter", "open workspace"),
                entry("1..9", "switch workspace"),
            ],
        ),
        (
            "workspaces / tabs",
            vec![
                act(
                    KeybindAction::WorkspacePicker,
                    binding_label(&keybinds.workspace_picker),
                    "workspace navigation",
                ),
                act(
                    KeybindAction::OpenNavigator,
                    binding_label(&keybinds.goto),
                    "session navigator",
                ),
                act(
                    KeybindAction::NewWorkspace,
                    binding_label(&keybinds.new_workspace),
                    "new workspace",
                ),
                act(
                    KeybindAction::TogglePinWorkspace,
                    binding_label(&keybinds.toggle_pin_workspace),
                    "pin / unpin space",
                ),
                act(
                    KeybindAction::TogglePinAgent,
                    binding_label(&keybinds.toggle_pin_agent),
                    "pin / unpin agent",
                ),
                act(
                    KeybindAction::NewWorktree,
                    binding_label(&keybinds.new_worktree),
                    "new worktree",
                ),
                act(
                    KeybindAction::OpenWorktree,
                    binding_label(&keybinds.open_worktree),
                    "open worktree",
                ),
                act(
                    KeybindAction::RemoveWorktree,
                    binding_label(&keybinds.remove_worktree),
                    "delete worktree checkout",
                ),
                act(
                    KeybindAction::RenameWorkspace,
                    binding_label(&keybinds.rename_workspace),
                    "rename workspace",
                ),
                act(
                    KeybindAction::CloseWorkspace,
                    binding_label(&keybinds.close_workspace),
                    "close workspace",
                ),
                act(
                    KeybindAction::PreviousWorkspace,
                    binding_label(&keybinds.previous_workspace),
                    "previous workspace",
                ),
                act(
                    KeybindAction::NextWorkspace,
                    binding_label(&keybinds.next_workspace),
                    "next workspace",
                ),
                indexed(
                    KeybindAction::SwitchWorkspace,
                    indexed_label(&keybinds.switch_workspace),
                    "switch workspace 1-9",
                ),
                act(
                    KeybindAction::PreviousAgent,
                    binding_label(&keybinds.previous_agent),
                    "previous agent",
                ),
                act(
                    KeybindAction::NextAgent,
                    binding_label(&keybinds.next_agent),
                    "next agent",
                ),
                indexed(
                    KeybindAction::FocusAgent,
                    indexed_label(&keybinds.focus_agent),
                    "focus agent 1-9",
                ),
                act(
                    KeybindAction::NewTab,
                    binding_label(&keybinds.new_tab),
                    "new tab",
                ),
                act(
                    KeybindAction::RenameTab,
                    binding_label(&keybinds.rename_tab),
                    "rename tab",
                ),
                act(
                    KeybindAction::PreviousTab,
                    binding_label(&keybinds.previous_tab),
                    "previous tab",
                ),
                act(
                    KeybindAction::NextTab,
                    binding_label(&keybinds.next_tab),
                    "next tab",
                ),
                act(
                    KeybindAction::MoveTabPrevious,
                    binding_label(&keybinds.move_tab_previous),
                    "move tab left",
                ),
                act(
                    KeybindAction::MoveTabNext,
                    binding_label(&keybinds.move_tab_next),
                    "move tab right",
                ),
                indexed(
                    KeybindAction::SwitchTab,
                    indexed_label(&keybinds.switch_tab),
                    "switch tab 1-9",
                ),
                act(
                    KeybindAction::CloseTab,
                    binding_label(&keybinds.close_tab),
                    "close tab",
                ),
            ],
        ),
        (
            "panes",
            vec![
                act(
                    KeybindAction::DisplayPanes,
                    binding_label(&keybinds.display_panes),
                    "display pane labels",
                ),
                act(
                    KeybindAction::SplitVertical,
                    binding_label(&keybinds.split_vertical),
                    "split vertical",
                ),
                act(
                    KeybindAction::SplitHorizontal,
                    binding_label(&keybinds.split_horizontal),
                    "split horizontal",
                ),
                act(
                    KeybindAction::ClosePane,
                    binding_label(&keybinds.close_pane),
                    "close pane",
                ),
                act(
                    KeybindAction::RespawnPane,
                    binding_label(&keybinds.respawn_pane),
                    "respawn pane",
                ),
                act(
                    KeybindAction::RenamePane,
                    binding_label(&keybinds.rename_pane),
                    "rename pane",
                ),
                act(
                    KeybindAction::BreakPane,
                    binding_label(&keybinds.break_pane),
                    "break pane to new tab",
                ),
                act(
                    KeybindAction::MovePaneToTab,
                    binding_label(&keybinds.move_pane_to_tab),
                    "move pane to tab or space",
                ),
                act(
                    KeybindAction::MovePaneNextTab,
                    binding_label(&keybinds.move_pane_next_tab),
                    "move pane to next tab",
                ),
                act(
                    KeybindAction::MovePanePrevTab,
                    binding_label(&keybinds.move_pane_prev_tab),
                    "move pane to previous tab",
                ),
                act(
                    KeybindAction::EditScrollback,
                    binding_label(&keybinds.edit_scrollback),
                    "edit scrollback",
                ),
                act(
                    KeybindAction::ClearPane,
                    binding_label(&keybinds.clear_pane),
                    "clear pane",
                ),
                act(
                    KeybindAction::ClearScrollback,
                    binding_label(&keybinds.clear_scrollback),
                    "clear scrollback",
                ),
                act(
                    KeybindAction::BalancePanes,
                    binding_label(&keybinds.balance_panes),
                    "balance panes",
                ),
                act(
                    KeybindAction::NextLayout,
                    binding_label(&keybinds.next_layout),
                    "cycle layout",
                ),
                act(
                    KeybindAction::OpenPaneTodos,
                    binding_label(&keybinds.open_pane_todos),
                    "pane todos",
                ),
                act(
                    KeybindAction::AddPaneTodo,
                    binding_label(&keybinds.add_pane_todo),
                    "add pane todo",
                ),
                act(
                    KeybindAction::OpenTodoBoard,
                    binding_label(&keybinds.open_todo_board),
                    "session todo board",
                ),
                act(
                    KeybindAction::CopyMode,
                    binding_label(&keybinds.copy_mode),
                    "copy mode",
                ),
                act(
                    KeybindAction::CopyModePageUp,
                    binding_label(&keybinds.copy_mode_page_up),
                    "scroll page up",
                ),
                act(
                    KeybindAction::CopyModeHalfPageUp,
                    binding_label(&keybinds.copy_mode_half_page_up),
                    "scroll half page up",
                ),
                act(
                    KeybindAction::CopyModeLineUp,
                    binding_label(&keybinds.copy_mode_line_up),
                    "scroll line up",
                ),
                act(
                    KeybindAction::CopyModePageDown,
                    binding_label(&keybinds.copy_mode_page_down),
                    "scroll page down",
                ),
                act(
                    KeybindAction::CopyModeHalfPageDown,
                    binding_label(&keybinds.copy_mode_half_page_down),
                    "scroll half page down",
                ),
                act(
                    KeybindAction::CopyModeLineDown,
                    binding_label(&keybinds.copy_mode_line_down),
                    "scroll line down",
                ),
                act(
                    KeybindAction::Zoom,
                    binding_label(&keybinds.zoom),
                    "zoom pane",
                ),
                act(
                    KeybindAction::ToggleSyncPanes,
                    binding_label(&keybinds.toggle_sync_panes),
                    "sync panes",
                ),
                act(
                    KeybindAction::EnterResizeMode,
                    binding_label(&keybinds.resize_mode),
                    "resize mode",
                ),
                act(
                    KeybindAction::ResizePaneLeft,
                    binding_label(&keybinds.resize_pane_left),
                    "resize pane left",
                ),
                act(
                    KeybindAction::ResizePaneDown,
                    binding_label(&keybinds.resize_pane_down),
                    "resize pane down",
                ),
                act(
                    KeybindAction::ResizePaneUp,
                    binding_label(&keybinds.resize_pane_up),
                    "resize pane up",
                ),
                act(
                    KeybindAction::ResizePaneRight,
                    binding_label(&keybinds.resize_pane_right),
                    "resize pane right",
                ),
                act(
                    KeybindAction::ToggleSidebar,
                    binding_label(&keybinds.toggle_sidebar),
                    "toggle sidebar",
                ),
                act(
                    KeybindAction::FocusPaneLeft,
                    binding_label(&keybinds.focus_pane_left),
                    "focus pane left",
                ),
                act(
                    KeybindAction::FocusPaneDown,
                    binding_label(&keybinds.focus_pane_down),
                    "focus pane down",
                ),
                act(
                    KeybindAction::FocusPaneUp,
                    binding_label(&keybinds.focus_pane_up),
                    "focus pane up",
                ),
                act(
                    KeybindAction::FocusPaneRight,
                    binding_label(&keybinds.focus_pane_right),
                    "focus pane right",
                ),
                act(
                    KeybindAction::SwapPaneLeft,
                    binding_label(&keybinds.swap_pane_left),
                    "swap pane left",
                ),
                act(
                    KeybindAction::SwapPaneDown,
                    binding_label(&keybinds.swap_pane_down),
                    "swap pane down",
                ),
                act(
                    KeybindAction::SwapPaneUp,
                    binding_label(&keybinds.swap_pane_up),
                    "swap pane up",
                ),
                act(
                    KeybindAction::SwapPaneRight,
                    binding_label(&keybinds.swap_pane_right),
                    "swap pane right",
                ),
                act(
                    KeybindAction::CyclePaneNext,
                    binding_label(&keybinds.cycle_pane_next),
                    "cycle pane next",
                ),
                act(
                    KeybindAction::CyclePanePrevious,
                    binding_label(&keybinds.cycle_pane_previous),
                    "cycle pane previous",
                ),
                act(
                    KeybindAction::LastPane,
                    binding_label(&keybinds.last_pane),
                    "last pane",
                ),
            ],
        ),
    ];

    // Fixed chords rather than `KeysConfig` actions — the todo panel and its
    // edit modal own their keymaps — but a shortcut absent from this panel is
    // a shortcut nobody finds.
    groups.push((
        "pane todos",
        vec![
            entry("enter", "edit selected todo"),
            entry("a", "add todo"),
            entry("spc", "toggle done"),
            entry("d", "remove todo"),
            entry("c", "clear done todos"),
            entry("g", "follow todo link"),
            entry("esc / q", "close panel"),
        ],
    ));
    groups.push((
        "notification center",
        vec![
            entry("enter", "jump to notification"),
            entry("r", "mark all read"),
            entry("c", "clear all"),
            entry("esc / q", "close panel"),
        ],
    ));
    groups.push((
        "move pane picker",
        vec![
            entry("enter / click", "move pane there"),
            entry("/", "search destinations"),
            entry("up / down / j / k", "previous / next destination"),
            entry("ctrl+u / ctrl+d", "half page up / down"),
            entry("home / end", "first / last destination"),
            entry("esc", "leave search / clear search / close"),
        ],
    ));
    groups.push((
        "command palette",
        vec![
            entry("enter", "run the selected command"),
            entry("tab", "complete the command's words"),
            entry("up / down / ctrl+j / ctrl+k", "previous / next command"),
            entry("ctrl+d / ctrl+u", "half page down / up"),
            entry("esc", "clear the query / close"),
        ],
    ));
    groups.push((
        "todo board",
        vec![
            entry("enter / click twice", "open owner pane"),
            entry("e", "edit todo"),
            entry("spc", "toggle done"),
            entry("g", "follow link"),
            entry("d", "remove todo"),
            entry("c", "clear done"),
            entry("/", "search todos"),
            entry("esc / q", "close board"),
        ],
    ));
    groups.push((
        "todo edit modal",
        vec![
            entry("ctrl+s / alt+enter", "save todo"),
            entry("esc", "cancel edit"),
            entry("tab", "cycle priority"),
            entry("ctrl+l", "choose link target"),
            entry("ctrl+g", "save and follow the link"),
            entry("ctrl+t", "toggle done"),
            entry("enter", "insert newline"),
            entry("ctrl+a / ctrl+e", "line start / end"),
            entry("ctrl+b / ctrl+f", "character back / forward"),
            entry("alt+b / alt+f", "word back / forward"),
            entry("ctrl+d", "delete forward"),
            entry("ctrl+k / ctrl+u", "kill to line end / start"),
            entry("ctrl+w", "kill word back"),
            entry("ctrl+y", "yank last kill"),
            entry("ctrl+_ / ctrl+- / ctrl+/", "undo"),
        ],
    ));

    if !keybinds.custom_commands.is_empty() {
        groups.push((
            "custom",
            keybinds
                .custom_commands
                .iter()
                .map(|binding| CommandEntry {
                    key: binding.label.clone(),
                    label: binding
                        .description
                        .clone()
                        .map(Cow::Owned)
                        .unwrap_or(Cow::Borrowed("custom command")),
                    action: None,
                })
                .collect(),
        ));
    }
    groups
}

/// The help panel's view of the command table.
pub(crate) fn keybind_help_groups(
    keybinds: &Keybinds,
    prefixes: &[crate::config::KeyCombo],
) -> Vec<KeybindHelpGroup> {
    command_table(keybinds, prefixes)
        .into_iter()
        .map(|(group, entries)| {
            (
                group,
                entries
                    .into_iter()
                    .map(|entry| (entry.key, entry.label))
                    .collect(),
            )
        })
        .collect()
}

pub(crate) fn filter_keybind_help_groups(
    groups: Vec<KeybindHelpGroup>,
    query: &str,
) -> Vec<KeybindHelpGroup> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(key: &'static str, label: &'static str) -> KeybindHelpEntry {
        (key.into(), Cow::Borrowed(label))
    }

    fn groups() -> Vec<KeybindHelpGroup> {
        vec![
            (
                "workspaces / tabs",
                vec![pair("w", "workspace navigation"), pair("c", "new tab")],
            ),
            (
                "panes",
                vec![pair("v", "split vertical"), pair("x", "close pane")],
            ),
        ]
    }

    #[test]
    fn text_char_prefers_generated_text_then_alternate_then_code() {
        let parse = |bytes: &str| {
            crate::input::parse_terminal_key_sequence(bytes)
                .unwrap_or_else(|| panic!("{bytes:?} parses"))
        };
        for (bytes, want) in [
            // Shift+/ typing `?` with no shifted alternate.
            ("\x1b[47;2;63u", Some('?')),
            ("\x1b[47u", Some('/')),
            ("/", Some('/')),
            // Portuguese Shift+7: alternate and text agree.
            ("\x1b[55:47;2;47u", Some('/')),
            ("\x1b[55:47;2u", Some('/')),
            ("\x1b[55;2u", Some('7')),
            ("\x1b[47;5u", None),
            ("\x1b[47;3u", None),
        ] {
            assert_eq!(keybind_help_text_char(&parse(bytes)), want, "{bytes:?}");
        }
    }

    #[test]
    fn filter_matches_labels_and_shortcuts_case_insensitively() {
        let filtered = filter_keybind_help_groups(groups(), "WoRk");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].1[0].1, "workspace navigation");

        let filtered = filter_keybind_help_groups(groups(), "x");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].1[0].1, "close pane");
        assert!(filter_keybind_help_groups(groups(), "panes").is_empty());
    }

    #[test]
    fn help_lists_every_configured_prefix() {
        let groups = keybind_help_groups(
            &Keybinds::default(),
            &[
                (KeyCode::Char(' '), KeyModifiers::CONTROL),
                (KeyCode::Char('s'), KeyModifiers::CONTROL),
            ],
        );
        let global = &groups[0].1;
        assert_eq!(global[0].0, "ctrl+space / ctrl+s");
        assert_eq!(global[0].1, "prefix mode");
    }

    /// A shortcut that works but is absent from `prefix+?` is incomplete work.
    #[test]
    fn respawn_pane_is_discoverable_in_the_help_panel() {
        let (live, _) = crate::config::Config::default()
            .live_keybinds_with_diagnostics()
            .expect("default keybinds");

        let entry = keybind_help_groups(&live.keybinds, &live.prefix)
            .into_iter()
            .find(|(group, _)| *group == "panes")
            .expect("the panes group should exist")
            .1
            .into_iter()
            .find(|(_, label)| label == "respawn pane")
            .expect("respawn pane should appear in the help panel");

        assert_eq!(entry.0, "prefix+ctrl+x");
    }

    #[test]
    fn pin_actions_are_discoverable_even_when_unbound() {
        let groups = keybind_help_groups(&Keybinds::default(), &[]);
        for name in ["pin / unpin space", "pin / unpin agent"] {
            let (keys, _) = groups
                .iter()
                .flat_map(|(_, entries)| entries)
                .find(|(_, label)| label.as_ref() == name)
                .expect("pin help entry");
            assert_eq!(keys, "unset");
        }
    }

    fn table() -> Vec<CommandGroup> {
        let (live, _) = crate::config::Config::default()
            .live_keybinds_with_diagnostics()
            .expect("default keybinds");
        command_table(&live.keybinds, &live.prefix)
    }

    fn row<'a>(table: &'a [CommandGroup], label: &str) -> &'a CommandEntry {
        table
            .iter()
            .flat_map(|(_, entries)| entries)
            .find(|entry| entry.label == label)
            .unwrap_or_else(|| panic!("no {label:?} row in the command table"))
    }

    #[test]
    fn the_help_groups_are_the_command_tables_groups_in_order() {
        let names: Vec<_> = table().iter().map(|(group, _)| *group).collect();
        assert_eq!(
            names,
            [
                "global",
                "navigation",
                "workspaces / tabs",
                "panes",
                "pane todos",
                "notification center",
                "move pane picker",
                "command palette",
                "todo board",
                "todo edit modal",
            ]
        );
        let (live, _) = crate::config::Config::default()
            .live_keybinds_with_diagnostics()
            .expect("default keybinds");
        let help = keybind_help_groups(&live.keybinds, &live.prefix);
        assert_eq!(
            help.iter().map(|(group, _)| *group).collect::<Vec<_>>(),
            names
        );
    }

    #[test]
    fn command_rows_name_the_action_their_key_runs() {
        let table = table();
        assert!(matches!(
            row(&table, "split vertical").action,
            Some(CommandAction::Run(KeybindAction::SplitVertical))
        ));
        assert!(matches!(
            row(&table, "command palette").action,
            Some(CommandAction::Run(KeybindAction::CommandPalette))
        ));
        assert_eq!(row(&table, "command palette").key, "prefix+:");
        assert!(matches!(
            row(&table, "switch workspace 1-9").action,
            Some(CommandAction::Indexed(make)) if matches!(make(2), KeybindAction::SwitchWorkspace(2))
        ));
    }

    #[test]
    fn fixed_chords_and_prefix_mode_are_not_commands() {
        let table = table();
        for label in ["prefix mode", "back", "edit selected todo", "save todo"] {
            assert!(row(&table, label).action.is_none(), "{label}");
        }
    }

    #[test]
    fn every_command_with_a_binding_field_reaches_the_table() {
        // A new `KeybindAction` that has a help row must be a palette row, so
        // count the rows that carry an action against the variants a user can
        // bind (indexed actions count once).
        let with_action = table()
            .iter()
            .flat_map(|(_, entries)| entries)
            .filter(|entry| entry.action.is_some())
            .count();
        assert!(with_action >= 75, "only {with_action} command rows");
    }
}
