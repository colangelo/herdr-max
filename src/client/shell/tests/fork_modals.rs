//! The classic modals, dialogs and menus as the fork draws them, re-expressed
//! from its oracles (src/ui/dialogs.rs, menus.rs, keybind_help.rs and
//! settings.rs at sync-snapshot/2026-10-08) over the client shell's own
//! overlays. The boxes are exact rows of the composed frame; the padding is
//! added by `framed`, so only the content of each row is written out.

use super::*;
use crate::api::schema::ResponseResult;
use crate::input::TerminalKey;
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

fn state_at(width: u16, height: u16) -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    state.compose(width, height).expect("shell");
    state
}

fn screen(state: &mut ClientShellState, width: u16, height: u16) -> Vec<String> {
    frame_rows(&state.compose(width, height).expect("frame"))
}

/// The columns and rows of `area` out of the screen's rows.
fn region(rows: &[String], area: Rect) -> Vec<String> {
    rows.iter()
        .skip(usize::from(area.y))
        .take(usize::from(area.height))
        .map(|row| {
            row.chars()
                .skip(usize::from(area.x))
                .take(usize::from(area.width))
                .collect()
        })
        .collect()
}

/// A box `inner` columns wide around `rows`, each row padded to the width.
fn framed(inner: usize, rows: &[&str]) -> Vec<String> {
    let mut out = vec![format!("┌{}┐", "─".repeat(inner))];
    out.extend(rows.iter().map(|row| format!("│{row:<inner$}│")));
    out.push(format!("└{}┘", "─".repeat(inner)));
    out
}

/// ` {label} ` for each, two spaces apart.
fn button_group(labels: &[&str]) -> String {
    labels
        .iter()
        .map(|label| format!(" {label} "))
        .collect::<Vec<_>>()
        .join("  ")
}

/// `text` indented to sit in the middle of `inner` columns.
fn centred(inner: usize, text: &str) -> String {
    let width = text.chars().count();
    format!("{}{text}", " ".repeat(inner.saturating_sub(width) / 2))
}

/// `left` and `right` at the two ends of `inner` columns.
fn spread(inner: usize, left: &str, right: &str) -> String {
    let used = left.chars().count() + right.chars().count();
    format!("{left}{}{right}", " ".repeat(inner.saturating_sub(used)))
}

fn key(state: &mut ClientShellState, code: KeyCode, modifiers: KeyModifiers) {
    state.handle_raw_events(vec![RawInputEvent::Key(TerminalKey::new(code, modifiers))]);
}

fn click_at(state: &mut ClientShellState, x: u16, y: u16) -> ClientShellInput {
    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::empty(),
    })])
}

fn click_rect(state: &mut ClientShellState, rect: Rect) -> ClientShellInput {
    click_at(state, rect.x + rect.width / 2, rect.y)
}

fn cell_at(frame: &FrameData, x: u16, y: u16) -> &crate::protocol::CellData {
    &frame.cells[usize::from(y) * usize::from(frame.width) + usize::from(x)]
}

fn has(frame: &FrameData, x: u16, y: u16, modifier: Modifier) -> bool {
    cell_at(frame, x, y).modifier & crate::protocol::modifier_to_u16(modifier) != 0
}

fn rename_overlay(title: &'static str, text: &str) -> ClientShellOverlay {
    ClientShellOverlay::Rename(ClientRenameOverlay {
        title,
        input: TextEditor::new(text, false),
        target: ClientRenameTarget::Workspace {
            workspace_id: "ws_1".into(),
        },
    })
}

// -- rename -------------------------------------------------------------------------

#[test]
fn snapshot_rename_workspace() {
    let mut state = state_at(80, 25);
    state.overlay = Some(rename_overlay("rename workspace", "api-gateway"));
    let rows = screen(&mut state, 80, 25);
    let buttons = centred(54, &button_group(&["↵ save", "^c clear", "esc cancel"]));
    assert_eq!(
        region(&rows, Rect::new(12, 9, 56, 7)),
        framed(
            54,
            &[
                " rename workspace",
                "",
                " api-gateway",
                buttons.as_str(),
                "",
            ]
        )
    );
    // Sized from their text: 8, 10 and 12 columns.
    assert_eq!(state.hits.overlay_primary.width, 8);
    assert_eq!(state.hits.overlay_clear.width, 10);
    assert_eq!(state.hits.overlay_cancel.width, 12);
}

#[test]
fn rename_title_lines_up_with_the_rows_under_it() {
    for title in [
        "new workspace",
        "rename workspace",
        "new tab",
        "rename tab",
        "rename pane",
    ] {
        let mut state = state_at(80, 25);
        state.overlay = Some(rename_overlay(title, "x"));
        let rows = screen(&mut state, 80, 25);
        let box_rows = region(&rows, Rect::new(12, 9, 56, 7));
        // The title and the input text start in the same column.
        assert_eq!(box_rows[1].find(title), box_rows[3].find('x'), "{title}");
        assert!(box_rows[1].starts_with(&format!("│ {title}")), "{title}");
    }
}

#[test]
fn rename_caret_follows_the_text_and_sits_on_a_blank_cell() {
    let mut state = state_at(80, 25);
    state.overlay = Some(rename_overlay("rename workspace", "api"));
    let frame = state.compose(80, 25).expect("frame");
    // Inner x 13, the text from 14, caret after three characters.
    let cursor = frame.cursor.as_ref().expect("a host cursor");
    assert_eq!((cursor.x, cursor.y), (17, 12));
    assert!(cursor.visible);
    assert_eq!(cell_at(&frame, 16, 12).symbol, "i");
    assert_eq!(cell_at(&frame, 17, 12).symbol, " ");
}

#[test]
fn rename_caret_is_the_same_in_every_rename_mode() {
    let mut cursors = Vec::new();
    for open in [
        ClientShellState::open_new_workspace_overlay as fn(&mut ClientShellState),
        ClientShellState::open_rename_workspace_overlay,
        ClientShellState::open_new_tab_overlay,
        ClientShellState::open_rename_tab_overlay,
        ClientShellState::open_rename_pane_overlay,
    ] {
        let mut state = state_at(80, 25);
        open(&mut state);
        key(&mut state, KeyCode::Char('c'), KeyModifiers::CONTROL);
        state.handle_input_bytes(b"abc");
        let frame = state.compose(80, 25).expect("frame");
        let cursor = frame.cursor.as_ref().expect("a host cursor");
        cursors.push((cursor.x, cursor.y));
    }
    assert!(
        cursors.windows(2).all(|pair| pair[0] == pair[1]),
        "{cursors:?}"
    );
}

// -- confirm close -------------------------------------------------------------------

fn confirm_overlay() -> ClientShellOverlay {
    ClientShellOverlay::ConfirmClose(ClientConfirmCloseOverlay {
        workspace_id: "ws_1".into(),
        close_group: false,
        tab_target: None,
        force: None,
        title: "Close workspace?".into(),
        detail: "herdr — 3 panes".into(),
    })
}

#[test]
fn snapshot_confirm_close_is_centred_on_the_pane_area() {
    let mut state = state_at(120, 30);
    let pane = state.layout(120, 30).pane_surface;
    assert!(pane.width >= 68 && pane.height >= 8, "{pane:?}");
    assert!(!pane.contains((0, 0).into()), "{pane:?}");
    state.overlay = Some(confirm_overlay());
    let rows = screen(&mut state, 120, 30);
    let popup = Rect::new(
        pane.x + (pane.width - 64) / 2,
        pane.y + (pane.height - 6) / 2,
        64,
        6,
    );
    let buttons = centred(62, &button_group(&["↵ confirm", "esc cancel"]));
    assert_eq!(
        region(&rows, popup),
        framed(
            62,
            &[
                " Close workspace?",
                " herdr — 3 panes",
                "",
                buttons.as_str(),
            ]
        )
    );
    assert_eq!(state.hits.overlay_primary.width, 11);
    assert_eq!(state.hits.overlay_cancel.width, 12);
}

#[test]
fn confirm_close_dims_only_the_pane_area() {
    let mut state = state_at(120, 30);
    let pane = state.layout(120, 30).pane_surface;
    state.overlay = Some(confirm_overlay());
    let frame = state.compose(120, 30).expect("frame");
    assert!(has(&frame, pane.x, pane.y, Modifier::DIM));
    assert!(!has(&frame, 0, 0, Modifier::DIM));
}

#[test]
fn clicking_anywhere_but_confirm_cancels_a_close_confirmation_like_escape() {
    for click in [false, true] {
        let mut state = state_at(120, 30);
        let pane = state.layout(120, 30).pane_surface;
        state.overlay = Some(confirm_overlay());
        state.compose(120, 30).expect("frame");
        let before = state.mode;
        let outcome = if click {
            click_at(&mut state, pane.x + 1, pane.y + 1)
        } else {
            state.handle_input_bytes(b"\x1b")
        };
        assert!(outcome.actions.is_empty());
        assert!(state.overlay.is_none());
        assert_eq!(state.mode, ClientShellMode::Navigate);
        assert_ne!(before, ClientShellMode::Navigate);
    }
}

#[test]
fn clicking_confirm_accepts_the_close() {
    let mut state = state_at(120, 30);
    state.overlay = Some(confirm_overlay());
    state.compose(120, 30).expect("frame");
    let primary = state.hits.overlay_primary;
    let outcome = click_rect(&mut state, primary);
    assert!(!outcome.actions.is_empty());
    assert!(state.overlay.is_none());
}

// -- new worktree --------------------------------------------------------------------

fn create_overlay(creating: bool, error: Option<&str>) -> ClientShellOverlay {
    ClientShellOverlay::WorktreeCreate(ClientWorktreeCreateOverlay {
        source_workspace_id: "ws_1".into(),
        repo_name: "herdr".into(),
        branch: TextEditor::new("feature/x", true),
        checkout_path: "/repo/.worktrees/herdr/feature-x".into(),
        error: error.map(str::to_owned),
        creating,
    })
}

#[test]
fn snapshot_new_worktree_creating() {
    let mut state = state_at(80, 25);
    state.overlay = Some(create_overlay(true, None));
    let rows = screen(&mut state, 80, 25);
    let buttons = centred(66, &button_group(&["↵ create and open", "esc cancel"]));
    assert_eq!(
        region(&rows, Rect::new(6, 6, 68, 13)),
        framed(
            66,
            &[
                " new worktree",
                "",
                " branch",
                " feature/x",
                " checkout",
                " /repo/.worktrees/herdr/feature-x",
                " creating…",
                "",
                "",
                "",
                buttons.as_str(),
            ]
        )
    );
    assert_eq!(state.hits.overlay_primary.width, 19);
    assert_eq!(state.hits.overlay_cancel.width, 12);
    // No caret while the checkout is being made.
    assert!(state.compose(80, 25).expect("frame").cursor.is_none());
}

#[test]
fn new_worktree_error_renders_every_line_of_git_stderr() {
    let mut state = state_at(80, 25);
    state.overlay = Some(create_overlay(
        false,
        Some("git failed\nfatal: 'feature/x' is already used\nhint: pick another name"),
    ));
    let rows = screen(&mut state, 80, 25);
    let box_rows = region(&rows, Rect::new(6, 6, 68, 13));
    // Three status rows, inner rows 6 to 8.
    assert_eq!(box_rows[7].trim_matches('│').trim_end(), " git failed");
    assert_eq!(
        box_rows[8].trim_matches('│').trim_end(),
        "fatal: 'feature/x' is already used"
    );
    assert_eq!(
        box_rows[9].trim_matches('│').trim_end(),
        "hint: pick another name"
    );
}

#[test]
fn new_worktree_caret_sits_on_the_input_row_after_the_text() {
    let mut state = state_at(80, 25);
    state.overlay = Some(create_overlay(false, None));
    let frame = state.compose(80, 25).expect("frame");
    let cursor = frame.cursor.as_ref().expect("a host cursor");
    // Popup y 6, inner row 3; inner x 7, the text from 8, nine characters.
    assert_eq!((cursor.x, cursor.y), (17, 10));
}

// -- delete worktree checkout --------------------------------------------------------

fn remove_overlay(force: bool) -> ClientShellOverlay {
    ClientShellOverlay::WorktreeRemove(ClientWorktreeRemoveOverlay {
        workspace_id: "ws_1".into(),
        path: "/repo/.worktrees/herdr/feature-x".into(),
        error: None,
        removing: false,
        force_confirmation: force,
    })
}

#[test]
fn snapshot_remove_worktree() {
    let mut state = state_at(80, 25);
    state.overlay = Some(remove_overlay(false));
    let rows = screen(&mut state, 80, 25);
    let buttons = centred(70, &button_group(&["↵ remove", "esc cancel"]));
    assert_eq!(
        region(&rows, Rect::new(4, 7, 72, 11)),
        framed(
            70,
            &[
                " delete worktree checkout?",
                " This removes the checkout folder:",
                " /repo/.worktrees/herdr/feature-x",
                " The branch is not deleted. The Herdr workspace will close.",
                "",
                "",
                "",
                "",
                buttons.as_str(),
            ]
        )
    );
    assert_eq!(state.hits.overlay_primary.width, 10);
}

#[test]
fn remove_worktree_force_adds_the_warning_and_widens_the_button() {
    let mut state = state_at(80, 25);
    state.overlay = Some(remove_overlay(true));
    let rows = screen(&mut state, 80, 25);
    let box_rows = region(&rows, Rect::new(4, 7, 72, 11));
    assert!(
        box_rows[5].starts_with("│ Dirty or untracked files will be permanently deleted."),
        "{}",
        box_rows[5]
    );
    assert!(box_rows[9].contains(" ↵ delete anyway "), "{}", box_rows[9]);
    assert_eq!(state.hits.overlay_primary.width, 17);
    // The fork's colours: the explanation dim, the path bright.
    let frame = state.compose(80, 25).expect("frame");
    let p = &state.config.palette;
    let color = crate::protocol::color_to_u32;
    assert_eq!(cell_at(&frame, 7, 9).fg, color(p.overlay0));
    assert_eq!(cell_at(&frame, 7, 10).fg, color(p.text));
    assert_eq!(cell_at(&frame, 7, 12).fg, color(p.red));
}

#[test]
fn clicking_remove_worktree_buttons_requests_remove_or_cancels() {
    let mut state = state_at(80, 25);
    state.overlay = Some(remove_overlay(false));
    state.compose(80, 25).expect("frame");
    let cancel = state.hits.overlay_cancel;
    click_rect(&mut state, cancel);
    assert!(state.overlay.is_none());

    state.overlay = Some(remove_overlay(false));
    state.compose(80, 25).expect("frame");
    let primary = state.hits.overlay_primary;
    let outcome = click_rect(&mut state, primary);
    assert!(outcome.actions.iter().any(|action| matches!(
        action,
        ClientShellAction::Endpoint { request, .. }
            if matches!(&request.method, crate::api::schema::Method::WorktreeRemove(_))
    )));
}

// -- open worktree -------------------------------------------------------------------

fn open_entry(path: &str, branch: Option<&str>, label: &str) -> ClientWorktreeOpenEntry {
    ClientWorktreeOpenEntry {
        path: path.into(),
        branch: branch.map(str::to_owned),
        is_linked_worktree: branch.is_some(),
        is_detached: false,
        open_workspace_id: None,
        label: label.into(),
    }
}

fn open_overlay(entries: Vec<ClientWorktreeOpenEntry>) -> ClientShellOverlay {
    ClientShellOverlay::WorktreeOpen(ClientWorktreeOpenOverlay {
        source_workspace_id: "ws_1".into(),
        entries,
        selected: 0,
        query: TextEditor::default(),
        search_focused: false,
        error: None,
        opening: false,
    })
}

fn two_checkouts() -> ClientShellOverlay {
    open_overlay(vec![
        open_entry(
            "/repo/.worktrees/herdr/feature-x",
            Some("feature-x"),
            "feature-x",
        ),
        open_entry("/repo/herdr", None, "herdr"),
    ])
}

#[test]
fn snapshot_open_worktree() {
    let mut state = state_at(120, 30);
    state.overlay = Some(two_checkouts());
    let rows = screen(&mut state, 120, 30);
    let search = spread(94, " / filter worktrees", "2 checkouts");
    let rule = "─".repeat(94);
    let root = spread(94, "  herdr", "root");
    let buttons = centred(94, &button_group(&["↵ open", "esc cancel"]));
    assert_eq!(
        region(&rows, Rect::new(12, 8, 96, 13)),
        framed(
            94,
            &[
                " open worktree",
                "",
                search.as_str(),
                rule.as_str(),
                "› feature-x",
                "  /repo/.worktrees/herdr/feature-x",
                root.as_str(),
                "  /repo/herdr",
                "",
                "",
                buttons.as_str(),
            ]
        )
    );
    assert_eq!(state.hits.overlay_primary.width, 8);
    assert_eq!(state.hits.overlay_cancel.width, 12);
}

#[test]
fn open_worktree_selected_row_is_a_surface_bar_across_both_lines() {
    let mut state = state_at(120, 30);
    state.overlay = Some(two_checkouts());
    let frame = state.compose(120, 30).expect("frame");
    let p = &state.config.palette;
    let color = crate::protocol::color_to_u32;
    // Popup x 12, inner x 13; entries from inner row 4, i.e. y 8 + 1 + 4.
    for (x, y) in [(14, 13), (100, 13), (14, 14), (105, 14)] {
        assert_eq!(cell_at(&frame, x, y).bg, color(p.surface0), "({x}, {y})");
    }
    assert!(has(&frame, 16, 13, Modifier::BOLD));
    assert_eq!(cell_at(&frame, 16, 13).fg, color(p.text));
    assert_eq!(cell_at(&frame, 16, 14).fg, color(p.subtext0));
    // The other entry is plain.
    assert_eq!(cell_at(&frame, 16, 15).fg, color(p.subtext0));
    assert_eq!(cell_at(&frame, 16, 16).fg, color(p.overlay0));
    assert_ne!(cell_at(&frame, 16, 15).bg, color(p.surface0));
}

#[test]
fn open_worktree_with_no_match_says_so_on_the_first_list_row() {
    let mut state = state_at(120, 30);
    state.overlay = Some(two_checkouts());
    state.handle_input_bytes(b"/zzz");
    let rows = screen(&mut state, 120, 30);
    let box_rows = region(&rows, Rect::new(12, 8, 96, 13));
    assert_eq!(
        box_rows[3].trim_matches('│'),
        spread(94, " / zzz", "0/2 checkouts")
    );
    // The rule stays whole; the message is on the row under it.
    assert_eq!(box_rows[4].trim_matches('│'), "─".repeat(94));
    assert_eq!(
        box_rows[5].trim_matches('│').trim_end(),
        " no matching worktrees"
    );
}

#[test]
fn open_worktree_shows_every_entry_and_hit_tests_the_rows_it_drew() {
    let entries = (0..5)
        .map(|n| {
            open_entry(
                &format!("/repo/wt{n}"),
                Some(&format!("branch-{n}")),
                &format!("branch-{n}"),
            )
        })
        .collect();
    let mut state = state_at(120, 40);
    state.overlay = Some(open_overlay(entries));
    let rows = screen(&mut state, 120, 40);
    // (2n + 8) rows for n entries: 18 here, centred in 40.
    let popup = Rect::new(12, 11, 96, 18);
    let box_rows = region(&rows, popup);
    assert_eq!(state.hits.worktree_rows.len(), 5);
    for (index, (rect, entry)) in state.hits.worktree_rows.iter().enumerate() {
        assert_eq!(*entry, index);
        assert_eq!(rect.y, popup.y + 1 + 4 + index as u16 * 2);
        assert_eq!(rect.height, 2);
        assert!(
            box_rows[5 + index * 2].contains(&format!("branch-{index}")),
            "{}",
            box_rows[5 + index * 2]
        );
    }
}

#[test]
fn clicking_open_worktree_buttons_requests_open_or_cancels() {
    let mut state = state_at(120, 30);
    state.overlay = Some(two_checkouts());
    state.compose(120, 30).expect("frame");
    let cancel = state.hits.overlay_cancel;
    click_rect(&mut state, cancel);
    assert!(state.overlay.is_none());

    state.overlay = Some(two_checkouts());
    state.compose(120, 30).expect("frame");
    let primary = state.hits.overlay_primary;
    let outcome = click_rect(&mut state, primary);
    assert!(outcome.actions.iter().any(|action| matches!(
        action,
        ClientShellAction::Endpoint { request, .. }
            if matches!(&request.method, crate::api::schema::Method::WorktreeOpen(_))
    )));
}

#[test]
fn open_worktree_entries_are_named_the_way_the_fork_names_them() {
    let mut state = state_at(106, 30);
    let ResponseResult::WorktreeList {
        source,
        mut worktrees,
    } = worktree_list_result(None)
    else {
        unreachable!("worktree list result");
    };
    for (path, branch) in [
        ("/work/detached-tree", None),
        ("/work/trailing/", None),
        ("/work/other", Some("topic/x".to_owned())),
    ] {
        worktrees.push(crate::api::schema::WorktreeInfo {
            path: path.into(),
            branch,
            is_bare: false,
            is_detached: true,
            is_prunable: false,
            is_linked_worktree: true,
            open_workspace_id: None,
            label: "repo".into(),
        });
    }
    state.handle_worktree_endpoint_result(
        PendingEndpointKind::PrepareWorktreeOpen {
            workspace_id: "ws_1".into(),
        },
        Ok(ResponseResult::WorktreeList { source, worktrees }),
        &mut ClientShellInput::default(),
    );
    let Some(ClientShellOverlay::WorktreeOpen(open)) = state.overlay.as_ref() else {
        panic!("open worktree overlay");
    };
    let labels: Vec<_> = open.entries.iter().map(|e| e.label.as_str()).collect();
    assert_eq!(labels, ["feature", "detached-tree", "trailing", "topic/x"]);
}

// -- keybinds ------------------------------------------------------------------------

fn help_overlay(search_focused: bool) -> ClientShellOverlay {
    ClientShellOverlay::Help(ClientHelpOverlay {
        query: TextEditor::default(),
        search_focused,
        scroll: 0,
    })
}

fn help_scroll(state: &ClientShellState) -> usize {
    match state.overlay.as_ref() {
        Some(ClientShellOverlay::Help(help)) => help.scroll,
        other => panic!("expected the help, got {other:?}"),
    }
}

#[test]
fn snapshot_keybind_help() {
    let mut state = state_at(80, 25);
    state.overlay = Some(help_overlay(false));
    let rows = screen(&mut state, 80, 25);
    assert_eq!(state.hits.help_popup, Rect::new(2, 1, 76, 22));
    let box_rows = region(&rows, state.hits.help_popup);
    assert_eq!(
        box_rows[1].trim_matches('│'),
        spread(74, " keybinds", " esc close ")
    );
    assert_eq!(
        box_rows[2].trim_matches('│').trim_end(),
        " press / to filter by command or shortcut"
    );
    assert_eq!(box_rows[3].trim_matches('│').trim_end(), "");
    assert_eq!(
        box_rows[20].trim_matches('│').trim_end(),
        " search / · scroll j/k/↑↓/pgup/pgdn · close esc/enter"
    );
    // The close button is 11 wide, flush right.
    assert_eq!(state.hits.overlay_cancel, Rect::new(66, 2, 11, 1));
    // The scrollbar's track is the thin bar, the thumb the thick one.
    let frame = state.compose(80, 25).expect("frame");
    assert_eq!(cell_at(&frame, 76, 5).symbol, "▐");
    assert_eq!(cell_at(&frame, 76, 19).symbol, "▕");
}

#[test]
fn keybind_help_focused_search_keeps_its_wording_with_the_fork_colours() {
    let mut state = state_at(80, 25);
    state.overlay = Some(help_overlay(true));
    let frame = state.compose(80, 25).expect("frame");
    let rows = frame_rows(&frame);
    let box_rows = region(&rows, Rect::new(2, 1, 76, 22));
    assert!(box_rows[1].contains(" esc back "), "{}", box_rows[1]);
    assert_eq!(box_rows[2].trim_matches('│').trim_end(), " /");
    assert_eq!(
        box_rows[20].trim_matches('│').trim_end(),
        " edit ←→/home/end · kill ^u/^k · yank ^y · scroll ↑↓ · back esc"
    );
    let p = &state.config.palette;
    let color = crate::protocol::color_to_u32;
    // Footer row y 21: " edit " dim, "←→/home/end" bright.
    assert_eq!(cell_at(&frame, 4, 21).fg, color(p.overlay0));
    assert_eq!(cell_at(&frame, 10, 21).fg, color(p.text));
    // The caret is in the search row, after " / ".
    let cursor = frame.cursor.as_ref().expect("a host cursor");
    assert_eq!((cursor.x, cursor.y), (6, 3));
}

#[test]
fn keybind_help_scrolls_on_every_shared_chord() {
    let mut state = state_at(80, 25);
    state.overlay = Some(help_overlay(false));
    state.compose(80, 25).expect("frame");
    let max = state.hits.help_max_scroll;
    assert!(max > 20, "{max}");
    let ctrl = KeyModifiers::CONTROL;
    let none = KeyModifiers::empty();
    for (code, modifiers, expected) in [
        (KeyCode::Char('n'), ctrl, 1),
        (KeyCode::Char('d'), ctrl, 9),
        (KeyCode::Char('u'), ctrl, 1),
        (KeyCode::Char('p'), ctrl, 0),
        (KeyCode::Char('j'), none, 1),
        (KeyCode::Char('j'), ctrl, 2),
        (KeyCode::Down, none, 3),
        (KeyCode::Char('k'), none, 2),
        (KeyCode::Char('k'), ctrl, 1),
        (KeyCode::PageDown, none, 9),
        (KeyCode::PageUp, none, 1),
        (KeyCode::End, none, max),
        (KeyCode::Home, none, 0),
    ] {
        key(&mut state, code, modifiers);
        assert_eq!(help_scroll(&state), expected, "{code:?} {modifiers:?}");
    }
}

#[test]
fn keybind_help_search_box_scrolls_on_modified_chords_and_types_plain_ones() {
    let mut state = state_at(80, 25);
    state.overlay = Some(help_overlay(true));
    state.compose(80, 25).expect("frame");
    key(&mut state, KeyCode::Char('j'), KeyModifiers::CONTROL);
    assert_eq!(help_scroll(&state), 1);
    key(&mut state, KeyCode::Char('n'), KeyModifiers::CONTROL);
    assert_eq!(help_scroll(&state), 2);
    key(&mut state, KeyCode::Char('p'), KeyModifiers::CONTROL);
    assert_eq!(help_scroll(&state), 1);
    // A plain j is text, and any edit starts the page over.
    key(&mut state, KeyCode::Char('j'), KeyModifiers::empty());
    let Some(ClientShellOverlay::Help(help)) = state.overlay.as_ref() else {
        panic!("help");
    };
    assert_eq!(&*help.query, "j");
    assert_eq!(help.scroll, 0);
}

// -- menus ---------------------------------------------------------------------------

fn context_menu() -> ClientShellOverlay {
    ClientShellOverlay::ContextMenu(ClientContextMenuOverlay {
        target: ClientContextMenuTarget::Workspace {
            pinned: Some(false),
            workspace_id: "ws_1".into(),
            is_git: false,
            is_linked_worktree: false,
            has_worktree_children: false,
            close_group: false,
            collapsed: false,
        },
        x: 10,
        y: 5,
        highlighted: 0,
    })
}

#[test]
fn snapshot_context_menu_has_a_gutter_and_a_full_row_highlight() {
    let mut state = state_at(80, 25);
    state.overlay = Some(context_menu());
    let frame = state.compose(80, 25).expect("frame");
    let rows = frame_rows(&frame);
    assert_eq!(
        region(&rows, Rect::new(10, 5, 14, 5)),
        framed(12, &[" Pin", " Rename", " Close"])
    );
    let p = &state.config.palette;
    let color = crate::protocol::color_to_u32;
    // Inner x 11 to 22: the whole first row, gutter included, is the bar.
    for x in [11, 12, 22] {
        assert_eq!(cell_at(&frame, x, 6).bg, color(p.accent), "x {x}");
    }
    assert_ne!(cell_at(&frame, 12, 7).bg, color(p.accent));
}

#[test]
fn context_menu_moves_on_every_shared_chord() {
    let mut state = state_at(80, 25);
    state.overlay = Some(context_menu());
    let highlighted = |state: &ClientShellState| match state.overlay.as_ref() {
        Some(ClientShellOverlay::ContextMenu(menu)) => menu.highlighted,
        other => panic!("expected the context menu, got {other:?}"),
    };
    let ctrl = KeyModifiers::CONTROL;
    let none = KeyModifiers::empty();
    for (code, modifiers, expected) in [
        (KeyCode::End, none, 2),
        (KeyCode::Home, none, 0),
        (KeyCode::Char('n'), ctrl, 1),
        (KeyCode::Char('j'), none, 2),
        (KeyCode::Down, none, 2),
        (KeyCode::Char('k'), none, 1),
        (KeyCode::Char('p'), ctrl, 0),
        (KeyCode::Char('d'), ctrl, 1),
        (KeyCode::Char('u'), ctrl, 0),
    ] {
        key(&mut state, code, modifiers);
        assert_eq!(highlighted(&state), expected, "{code:?} {modifiers:?}");
    }
}

#[test]
fn snapshot_global_menu_highlights_only_the_label() {
    let mut state = state_at(106, 30);
    let launcher = state.hits.global_launcher;
    state.overlay = Some(ClientShellOverlay::GlobalMenu(ClientGlobalMenuOverlay {
        highlighted: 0,
    }));
    let frame = state.compose(106, 30).expect("frame");
    let rows = frame_rows(&frame);
    let x = launcher.right().saturating_sub(17).min(106 - 17);
    let y = launcher.y.saturating_sub(6);
    assert_eq!(
        region(&rows, Rect::new(x, y, 17, 6)),
        framed(15, &[" settings", " keybinds", " reload config", " detach"])
    );
    let p = &state.config.palette;
    let color = crate::protocol::color_to_u32;
    // " settings " is the bar; the rest of the row is the panel.
    assert_eq!(cell_at(&frame, x + 1, y + 1).bg, color(p.accent));
    assert_eq!(cell_at(&frame, x + 10, y + 1).bg, color(p.accent));
    assert_ne!(cell_at(&frame, x + 11, y + 1).bg, color(p.accent));
    assert_ne!(cell_at(&frame, x + 1, y + 2).bg, color(p.accent));
}

#[test]
fn global_menu_moves_on_every_shared_chord() {
    let mut state = state_at(80, 25);
    state.overlay = Some(ClientShellOverlay::GlobalMenu(ClientGlobalMenuOverlay {
        highlighted: 0,
    }));
    let highlighted = |state: &ClientShellState| match state.overlay.as_ref() {
        Some(ClientShellOverlay::GlobalMenu(menu)) => menu.highlighted,
        other => panic!("expected the global menu, got {other:?}"),
    };
    let ctrl = KeyModifiers::CONTROL;
    let none = KeyModifiers::empty();
    for (code, modifiers, expected) in [
        (KeyCode::End, none, 3),
        (KeyCode::Char('u'), ctrl, 1),
        (KeyCode::Char('d'), ctrl, 3),
        (KeyCode::Char('p'), ctrl, 2),
        (KeyCode::Char('k'), none, 1),
        (KeyCode::Char('n'), ctrl, 2),
        (KeyCode::Home, none, 0),
    ] {
        key(&mut state, code, modifiers);
        assert_eq!(highlighted(&state), expected, "{code:?} {modifiers:?}");
    }
}

// -- settings ------------------------------------------------------------------------

fn settings_state() -> ClientShellState {
    let mut state = state_at(80, 25);
    state.open_settings_overlay();
    state
}

fn settings_mut(state: &mut ClientShellState) -> &mut ClientSettingsOverlay {
    match state.overlay.as_mut() {
        Some(ClientShellOverlay::Settings(settings)) => settings,
        other => panic!("expected settings, got {other:?}"),
    }
}

const SETTINGS_POPUP: Rect = Rect::new(2, 1, 76, 22);
const TABS: &str = " theme   indicators   sound   toasts   pane labels   integrations";

#[test]
fn snapshot_settings_pane_labels() {
    for enabled in [false, true] {
        let mut state = settings_state();
        state.config.show_agent_labels_on_pane_borders = enabled;
        state.select_settings_section(
            ClientSettingsSection::PaneLabels,
            &mut ClientShellInput::default(),
        );
        let rows = screen(&mut state, 80, 25);
        let rule = "─".repeat(74);
        let buttons = centred(74, &button_group(&["↵ apply", "esc close"]));
        let on = if enabled {
            " agent border labels: on ✓"
        } else {
            " agent border labels: on"
        };
        let off = if enabled {
            " agent border labels: off"
        } else {
            " agent border labels: off ✓"
        };
        let mut expected = vec![
            " settings",
            TABS,
            rule.as_str(),
            "",
            " show detected agent names in split pane borders",
            "",
            "",
            on,
            off,
        ];
        expected.extend([""; 9]);
        expected.extend([" ↑↓ select  tab section", buttons.as_str()]);
        assert_eq!(region(&rows, SETTINGS_POPUP), framed(74, &expected));
        assert_eq!(state.hits.settings_choices.len(), 2);
        let selected = state.hits.settings_choices[usize::from(!enabled)].0;
        let frame = state.compose(80, 25).expect("frame");
        assert_eq!(
            cell_at(&frame, selected.x + 1, selected.y).bg,
            crate::protocol::color_to_u32(state.config.palette.surface0)
        );
    }
}

#[test]
fn settings_pane_labels_tab_cycles_and_clicks_at_the_fork_position() {
    for enabled in [false, true] {
        let mut state = settings_state();
        state.config.show_agent_labels_on_pane_borders = enabled;
        for _ in 0..4 {
            key(&mut state, KeyCode::Tab, KeyModifiers::empty());
        }
        assert_eq!(
            settings_mut(&mut state).section,
            ClientSettingsSection::PaneLabels
        );
        assert_eq!(settings_mut(&mut state).selected, usize::from(!enabled));
        key(&mut state, KeyCode::Right, KeyModifiers::empty());
        assert_eq!(
            settings_mut(&mut state).section,
            ClientSettingsSection::Integrations
        );
        key(&mut state, KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_eq!(
            settings_mut(&mut state).section,
            ClientSettingsSection::PaneLabels
        );
        key(&mut state, KeyCode::Char('h'), KeyModifiers::empty());
        assert_eq!(
            settings_mut(&mut state).section,
            ClientSettingsSection::Toast
        );
        state.compose(80, 25).expect("tab hits");
        let tab = state
            .hits
            .settings_tabs
            .iter()
            .find(|(_, section)| *section == ClientSettingsSection::PaneLabels)
            .unwrap()
            .0;
        let clicked = click_rect(&mut state, tab);
        assert!(clicked.actions.is_empty());
        assert_eq!(
            settings_mut(&mut state).section,
            ClientSettingsSection::PaneLabels
        );
        assert_eq!(settings_mut(&mut state).selected, usize::from(!enabled));
        key(&mut state, KeyCode::End, KeyModifiers::empty());
        assert_eq!(settings_mut(&mut state).selected, 1);
        key(&mut state, KeyCode::Home, KeyModifiers::empty());
        assert_eq!(settings_mut(&mut state).selected, 0);
    }
}

#[test]
fn settings_pane_labels_save_and_reload_from_keyboard_and_mouse() {
    let _lock = crate::config::test_config_env_lock().lock().unwrap();
    struct ConfigPathGuard {
        original: Option<std::ffi::OsString>,
        path: std::path::PathBuf,
    }
    impl Drop for ConfigPathGuard {
        fn drop(&mut self) {
            if let Some(original) = &self.original {
                std::env::set_var(crate::config::CONFIG_PATH_ENV_VAR, original);
            } else {
                std::env::remove_var(crate::config::CONFIG_PATH_ENV_VAR);
            }
            let _ = std::fs::remove_file(&self.path);
        }
    }
    let guard = ConfigPathGuard {
        original: std::env::var_os(crate::config::CONFIG_PATH_ENV_VAR),
        path: std::env::temp_dir().join(format!(
            "herdr-settings-pane-labels-{}.toml",
            std::process::id()
        )),
    };
    std::env::set_var(crate::config::CONFIG_PATH_ENV_VAR, &guard.path);
    for apply in [KeyCode::Enter, KeyCode::Char(' '), KeyCode::Null] {
        for enabled in [false, true] {
            let content = format!(
                "# keep this comment\n[ui]\nshow_agent_labels_on_pane_borders = {}\nsidebar_width = 31\n[ui.sound]\nenabled = false\n", !enabled
            );
            std::fs::write(&guard.path, &content).unwrap();
            let config: Config = toml::from_str(&content).unwrap();
            let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
            state.set_snapshot(Box::new(snapshot()));
            state.set_pane_surface(surface());
            state.open_settings_overlay();
            state.select_settings_section(
                ClientSettingsSection::PaneLabels,
                &mut ClientShellInput::default(),
            );
            let outcome = if apply == KeyCode::Null {
                state.compose(80, 25).expect("choice hits");
                let choice = state.hits.settings_choices[usize::from(!enabled)].0;
                // Like the fork, clicking a toggle choice saves immediately.
                click_rect(&mut state, choice)
            } else {
                key(
                    &mut state,
                    if enabled { KeyCode::Up } else { KeyCode::Down },
                    KeyModifiers::empty(),
                );
                state.handle_raw_events(vec![RawInputEvent::Key(TerminalKey::new(
                    apply,
                    KeyModifiers::empty(),
                ))])
            };
            let [ClientShellAction::Endpoint { request, .. }] = &outcome.actions[..] else {
                panic!("saving must request endpoint reload: {:?}", outcome.actions);
            };
            assert!(matches!(
                request.method,
                crate::api::schema::Method::ServerReloadConfig(_)
            ));
            let saved = std::fs::read_to_string(&guard.path).unwrap();
            let saved_config: Config = toml::from_str(&saved).unwrap();
            assert_eq!(saved_config.ui.show_agent_labels_on_pane_borders, enabled);
            assert_eq!(state.config.show_agent_labels_on_pane_borders, enabled);
            assert_eq!(saved_config.ui.sidebar_width, 31);
            assert!(!saved_config.ui.sound.enabled);
            assert!(saved.contains("# keep this comment"));
            assert_eq!(
                settings_mut(&mut state).section,
                ClientSettingsSection::PaneLabels
            );
        }
    }
}

#[test]
fn snapshot_settings_theme() {
    let mut state = settings_state();
    assert_eq!(settings_mut(&mut state).selected, 0);
    let rows = screen(&mut state, 80, 25);
    assert_eq!(state.hits.settings_popup, SETTINGS_POPUP);
    let rule = "─".repeat(74);
    let buttons = centred(74, &button_group(&["↵ apply", "esc close"]));
    let themes: Vec<String> = crate::config::THEME_NAMES
        .iter()
        .take(13)
        .enumerate()
        .map(|(index, name)| {
            let gutter = if index == 0 { " ▸ " } else { "   " };
            let current = if index == 0 { " ✓" } else { "" };
            format!("{gutter}{name}{current}")
        })
        .collect();
    let mut expected: Vec<&str> = vec![" settings", TABS, rule.as_str(), ""];
    expected.extend(themes.iter().map(String::as_str));
    expected.extend(["", " ↑↓ select  tab section", buttons.as_str()]);
    assert_eq!(region(&rows, SETTINGS_POPUP), framed(74, &expected));
    assert_eq!(state.hits.overlay_primary.width, 9);
    assert_eq!(state.hits.overlay_cancel.width, 11);
}

#[test]
fn settings_theme_rows_are_the_fork_colours() {
    let mut state = settings_state();
    let frame = state.compose(80, 25).expect("frame");
    let color = crate::protocol::color_to_u32;
    {
        let p = &state.config.palette;
        // Content starts on inner row 4, y 6; the saved theme is the selected
        // one, so the highlight wins over the tick's green.
        assert_eq!(cell_at(&frame, 5, 6).bg, color(p.surface0));
        assert_eq!(cell_at(&frame, 5, 6).fg, color(p.text));
        assert!(has(&frame, 5, 6, Modifier::BOLD));
        assert_eq!(cell_at(&frame, 5, 7).fg, color(p.subtext0));
    }
    // Moving on leaves the tick green on the saved one.
    key(&mut state, KeyCode::Down, KeyModifiers::empty());
    let frame = state.compose(80, 25).expect("frame");
    let p = &state.config.palette;
    let rows = frame_rows(&frame);
    let tick = rows[6].chars().position(|c| c == '✓').expect("tick") as u16;
    assert_eq!(cell_at(&frame, tick, 6).fg, color(p.green));
    // The tabs: the current one is the accent bar over its label only.
    assert_eq!(cell_at(&frame, 4, 3).bg, color(p.accent));
    assert_ne!(cell_at(&frame, 3, 3).bg, color(p.accent));
}

#[test]
fn snapshot_settings_indicators() {
    let mut state = settings_state();
    {
        let settings = settings_mut(&mut state);
        settings.section = ClientSettingsSection::Indicators;
        settings.selected = 1;
    }
    let rows = screen(&mut state, 80, 25);
    let box_rows = region(&rows, SETTINGS_POPUP);
    let text = |index: usize| box_rows[index].trim_matches('│').trim_end().to_owned();
    assert_eq!(
        text(5),
        " choose color dots or distinct symbols for each state"
    );
    assert_eq!(text(6), "");
    assert_eq!(text(7), "");
    assert_eq!(text(8), " agent status indicators: color dots  ● ● ● ○ · ✓");
    assert_eq!(
        text(9),
        " agent status indicators: distinct symbols  × ◐ □ ✓ ·"
    );
    // The second option is the selected bar.
    let frame = state.compose(80, 25).expect("frame");
    let p = &state.config.palette;
    let color = crate::protocol::color_to_u32;
    assert_eq!(cell_at(&frame, 5, 10).bg, color(p.surface0));
    assert_ne!(cell_at(&frame, 5, 9).bg, color(p.surface0));
}

#[test]
fn settings_toast_options_take_two_rows_each_and_mark_the_saved_one() {
    let mut state = settings_state();
    state.config.toast_delivery = crate::config::ToastDelivery::Terminal;
    {
        let settings = settings_mut(&mut state);
        settings.section = ClientSettingsSection::Toast;
        settings.selected = 1;
    }
    let rows = screen(&mut state, 80, 25);
    let box_rows = region(&rows, SETTINGS_POPUP);
    let text = |index: usize| box_rows[index].trim_matches('│').trim_end().to_owned();
    assert_eq!(text(8), " notification popups: off");
    assert_eq!(text(10), " notification popups: inside herdr");
    assert_eq!(text(12), " notification popups: via terminal ✓");
    assert_eq!(text(14), " notification popups: via system");
    // Click the second row of the third option: it saves and stays open.
    let (rect, index) = state.hits.settings_choices[2];
    assert_eq!((index, rect.height), (2, 2));
}

#[test]
fn settings_integrations_lists_targets_and_hints_in_the_footer() {
    let mut state = settings_state();
    {
        let settings = settings_mut(&mut state);
        settings.section = ClientSettingsSection::Integrations;
        settings.integrations = vec![
            crate::api::schema::IntegrationInfo {
                target: crate::api::schema::IntegrationTarget::Codex,
                label: "codex".into(),
                command: "codex".into(),
                available: true,
                state: crate::api::schema::IntegrationState::Outdated,
            },
            crate::api::schema::IntegrationInfo {
                target: crate::api::schema::IntegrationTarget::Claude,
                label: "claude".into(),
                command: "claude".into(),
                available: false,
                state: crate::api::schema::IntegrationState::NotInstalled,
            },
        ];
    }
    let rows = screen(&mut state, 80, 25);
    let box_rows = region(&rows, SETTINGS_POPUP);
    let text = |index: usize| box_rows[index].trim_matches('│').trim_end().to_owned();
    assert_eq!(text(5), "agent integrations");
    assert_eq!(
        text(6),
        "enable session restore and, where supported, direct status updates"
    );
    // The list starts under the description's two rows and a blank one, and
    // the status column is the twelfth.
    assert_eq!(text(9), format!(" ↻ {:<9}update available", "codex"));
    assert_eq!(text(10), format!(" – {:<9}not found", "claude"));
    assert_eq!(
        text(17),
        " press install to add available or outdated integrations"
    );
    let buttons = centred(74, &button_group(&["↵ install", "esc close"]));
    assert_eq!(text(20), buttons.trim_end());
    assert_eq!(state.hits.overlay_primary.width, 11);
}

#[test]
fn settings_blank_interior_click_cancels_and_restores_the_theme() {
    let mut state = settings_state();
    let original = state.config.theme_name.clone();
    key(&mut state, KeyCode::Down, KeyModifiers::empty());
    assert_ne!(state.config.theme_name, original);
    state.compose(80, 25).expect("frame");
    let popup = state.hits.settings_popup;
    click_at(&mut state, popup.right() - 3, popup.y + 1);
    assert!(state.overlay.is_none());
    assert_eq!(state.config.theme_name, original);
}

#[test]
fn settings_moves_on_every_shared_chord() {
    let mut state = settings_state();
    state.compose(80, 25).expect("frame");
    let ctrl = KeyModifiers::CONTROL;
    let none = KeyModifiers::empty();
    let last = crate::config::THEME_NAMES.len() - 1;
    for (code, modifiers, expected) in [
        (KeyCode::Char('n'), ctrl, 1),
        (KeyCode::Char('j'), ctrl, 2),
        (KeyCode::Char('p'), ctrl, 1),
        (KeyCode::Char('k'), ctrl, 0),
        (KeyCode::Up, none, 0),
        (KeyCode::Char('d'), ctrl, 6),
        (KeyCode::Char('u'), ctrl, 0),
        (KeyCode::End, none, last),
        (KeyCode::Home, none, 0),
    ] {
        key(&mut state, code, modifiers);
        assert_eq!(
            settings_mut(&mut state).selected,
            expected,
            "{code:?} {modifiers:?}"
        );
        // The theme previews as the selection moves.
        assert_eq!(
            state.config.theme_name,
            crate::config::THEME_NAMES[expected]
        );
    }
}
