//! The pane-move picker and the pane-move keys, re-expressed from the fork's
//! oracles (src/app/input/navigate.rs, src/app/input/mouse.rs and
//! src/ui/dialogs.rs at sync-snapshot/2026-10-08) over snapshots and emitted
//! endpoint requests.

use super::super::move_picker::{
    fit_pane_names, move_picker_for_snapshot, ClientMovePickerOverlay, MoveItem, MovePickerLayout,
    MoveTarget,
};
use super::*;
use crate::api::schema::{
    AgentStatus, Method, PaneMoveDestination, PaneMoveParams, ResponseResult, SplitDirection,
};
use crate::input::KeybindAction;
use crate::protocol::{ClientShellPane, ClientShellTab};

const W: u16 = 80;
const H: u16 = 25;

type Space<'a> = (&'a str, &'a [(&'a str, &'a [&'a str])]);

/// A session of spaces, each a list of tabs (`""` for an unnamed tab), each
/// a list of pane names. `focus` is (space, tab, pane) by position.
fn session(spaces: &[Space], focus: (usize, usize, usize)) -> ClientShellSnapshot {
    let mut snapshot = super::snapshot();
    snapshot.workspaces.clear();
    snapshot.tabs.clear();
    snapshot.panes.clear();
    let mut pane_number = 0;
    for (w, (label, tabs)) in spaces.iter().enumerate() {
        let workspace_id = format!("w{w}");
        let mut workspace = super::snapshot().workspaces.remove(0);
        workspace.workspace_id = workspace_id.clone();
        workspace.number = w + 1;
        workspace.label = (*label).to_owned();
        workspace.focused = w == focus.0;
        workspace.active_tab_id = format!("{workspace_id}:t1");
        snapshot.workspaces.push(workspace);
        for (t, (tab_label, panes)) in tabs.iter().enumerate() {
            let tab_id = format!("{workspace_id}:t{}", t + 1);
            snapshot.tabs.push(ClientShellTab {
                tab_id: tab_id.clone(),
                workspace_id: workspace_id.clone(),
                number: t + 1,
                label: if tab_label.is_empty() {
                    (t + 1).to_string()
                } else {
                    (*tab_label).to_owned()
                },
                custom_label: !tab_label.is_empty(),
                zoomed: false,
                focused: (w, t) == (focus.0, focus.1),
                agent_status: AgentStatus::Idle,
            });
            for (p, name) in panes.iter().enumerate() {
                pane_number += 1;
                let pane_id = format!("{workspace_id}:p{pane_number}");
                let focused = (w, t, p) == focus;
                if focused {
                    snapshot.focused_workspace_id = Some(workspace_id.clone());
                    snapshot.focused_tab_id = Some(tab_id.clone());
                    snapshot.focused_pane_id = Some(pane_id.clone());
                }
                snapshot.panes.push(ClientShellPane {
                    pane_id,
                    workspace_id: workspace_id.clone(),
                    tab_id: tab_id.clone(),
                    label: (!name.is_empty()).then(|| (*name).to_owned()),
                    cwd: None,
                    foreground_cwd: None,
                    focused,
                    right_click_passthrough: false,
                });
            }
        }
    }
    snapshot
}

/// The fork's `move_picker_session`: the moving pane is `herdr-fixes`.
fn sample() -> ClientShellSnapshot {
    session(
        &[
            ("herdr", &[("cc", &["herdr-relay", "herdr-fixes"])]),
            (
                "macOS",
                &[
                    (
                        "",
                        &["macos-relay-2", "macos-relay-3", "vpn-morning-issues-2"],
                    ),
                    ("", &["keyboard-shortcuts"]),
                ],
            ),
            ("CONTEXT", &[("", &["context-relay"])]),
        ],
        (0, 0, 1),
    )
}

/// Three spaces; the source space has an extra pane and space two a tab
/// named `logs`.
fn three() -> ClientShellSnapshot {
    session(
        &[
            ("one", &[("", &["alpha", "beta"])]),
            ("two", &[("", &["gamma"]), ("logs", &["tail"])]),
            ("three", &[("", &["keyboard-shortcuts"])]),
        ],
        (0, 0, 0),
    )
}

fn state_with(snapshot: ClientShellSnapshot) -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface());
    state.compose(W, H).unwrap();
    state
}

fn methods(outcome: &ClientShellInput) -> Vec<(String, Method)> {
    outcome
        .actions
        .iter()
        .filter_map(|action| match action {
            ClientShellAction::Endpoint { request, .. } => {
                Some((request.id.clone(), request.method.clone()))
            }
            _ => None,
        })
        .collect()
}

fn only(outcome: &ClientShellInput) -> (String, Method) {
    let methods = methods(outcome);
    let [method] = methods.as_slice() else {
        panic!("expected one endpoint request, got {methods:?}");
    };
    method.clone()
}

fn bind(state: &mut ClientShellState, action: KeybindAction) -> ClientShellInput {
    let mut outcome = ClientShellInput::default();
    state.record_binding(crate::input::KeybindMatch::Action(action), &mut outcome);
    outcome
}

fn open(state: &mut ClientShellState) {
    bind(state, KeybindAction::MovePaneToTab);
    state.compose(W, H).unwrap();
}

fn picker(state: &ClientShellState) -> &ClientMovePickerOverlay {
    match state.overlay.as_ref() {
        Some(ClientShellOverlay::MovePicker(picker)) => picker,
        other => panic!("expected the move picker, got {other:?}"),
    }
}

fn layout(state: &ClientShellState) -> MovePickerLayout {
    state.hits.move_picker.clone().expect("picker layout")
}

fn screen(state: &mut ClientShellState) -> Vec<String> {
    let frame = state.compose(W, H).expect("frame");
    frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect()
}

/// The picker box: columns `x..x + width` of rows `y..y + height`.
fn boxed(state: &mut ClientShellState) -> Vec<String> {
    let outer = layout(state).outer;
    screen(state)
        .iter()
        .skip(usize::from(outer.y))
        .take(usize::from(outer.height))
        .map(|row| {
            row.chars()
                .skip(usize::from(outer.x))
                .take(usize::from(outer.width))
                .collect()
        })
        .collect()
}

/// One letter per row kind: `#` heading, `@` here, `-` destination, blank gap.
fn kinds(items: &[MoveItem]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            MoveItem::SpaceHeading { label, .. } => format!("# {label}"),
            MoveItem::Here(_) => "here".to_owned(),
            MoveItem::Destination(entry) => match &entry.target {
                MoveTarget::Tab { .. } if entry.label.is_empty() => format!("tab {}", entry.number),
                MoveTarget::Tab { .. } => format!("tab {} · {}", entry.number, entry.label),
                MoveTarget::NewTab { .. } => "new tab".to_owned(),
                MoveTarget::NewSpace => "new space".to_owned(),
            },
            MoveItem::Gap => String::new(),
        })
        .collect()
}

fn targets(picker: &ClientMovePickerOverlay) -> Vec<MoveTarget> {
    picker
        .all_items
        .iter()
        .filter_map(|item| match item {
            MoveItem::Destination(entry) => Some(entry.target.clone()),
            _ => None,
        })
        .collect()
}

fn click(state: &mut ClientShellState, x: u16, y: u16) -> ClientShellInput {
    state.handle_raw_events(vec![crate::raw_input::RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::empty(),
    })])
}

fn hover(state: &mut ClientShellState, x: u16, y: u16) {
    state.handle_raw_events(vec![crate::raw_input::RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Moved,
        column: x,
        row: y,
        modifiers: KeyModifiers::empty(),
    })]);
}

fn notice(state: &ClientShellState) -> (String, String) {
    let notice = state.visible_endpoint_notice.as_ref().expect("a notice");
    (notice.title.clone(), notice.body.clone())
}

fn move_result(changed: bool, reason: Option<&str>) -> ResponseResult {
    let layout = serde_json::json!({
        "workspace_id": "w0",
        "tab_id": "w0:t1",
        "zoomed": false,
        "area": {"x": 0, "y": 0, "width": 80, "height": 24},
        "focused_pane_id": "w0:p2",
        "panes": [],
        "splits": [],
    });
    let mut value = serde_json::json!({
        "changed": changed,
        "previous_pane_id": "w0:p2",
        "previous_workspace_id": "w0",
        "previous_tab_id": "w0:t1",
        "pane": {
            "pane_id": "w0:p2",
            "terminal_id": "term-2",
            "workspace_id": "w0",
            "tab_id": "w0:t1",
            "focused": true,
            "agent_status": "idle",
            "revision": 0,
        },
        "target_layout": layout,
        "focused_pane_id": "w0:p2",
    });
    if let Some(reason) = reason {
        value["reason"] = serde_json::json!(reason);
    }
    ResponseResult::PaneMove {
        move_result: serde_json::from_value(value).expect("pane move result"),
    }
}

// -- the picker as drawn --------------------------------------------------------

#[test]
fn snapshot_move_picker() {
    let mut state = state_with(sample());
    open(&mut state);
    assert_eq!(layout(&state).outer, Rect::new(10, 1, 60, 23));
    let expected = [
        "┌──────────────────────────────────────────────────────────┐",
        "│ move pane  herdr-fixes  from herdr › tab 1 · cc          │",
        "│ / search destinations                     7 destinations │",
        "│──────────────────────────────────────────────────────────│",
        "│   ○ herdr (2)                                            │",
        "│ ◆ ├── ○ tab 1 · cc  you are here                         │",
        "│   └── + new tab                                          │",
        "│                                                          │",
        "│   ○ macOS (4)                                            │",
        "│   ├── ○ tab 1       macos-relay-2, macos-relay-3, +1     │",
        "│   ├── ○ tab 2       keyboard-shortcuts                   │",
        "│   └── + new tab                                          │",
        "│                                                          │",
        "│   ○ CONTEXT (1)                                          │",
        "│   ├── ○ tab 1       context-relay                        │",
        "│   └── + new tab                                          │",
        "│                                                          │",
        "│   + new space                                            │",
        "│──────────────────────────────────────────────────────────│",
        "│ a new tab in herdr                                       │",
        "│                                                          │",
        "│                   ↵ move    esc cancel                   │",
        "└──────────────────────────────────────────────────────────┘",
    ];
    assert_eq!(boxed(&mut state), expected);
}

#[test]
fn snapshot_move_picker_searching() {
    let mut state = state_with(sample());
    open(&mut state);
    state.handle_input_bytes(b"/mac");
    let rows = boxed(&mut state);
    assert_eq!(layout(&state).outer, Rect::new(10, 1, 60, 23), "same size");
    assert_eq!(
        rows[2],
        "│ / mac                                     7 destinations │"
    );
    assert_eq!(
        rows[4],
        "│   ○ macOS (4)                                            │"
    );
    assert_eq!(
        rows[5],
        "│   ├── ○ tab 1       macos-relay-2, macos-relay-3, +1     │"
    );
    assert_eq!(
        rows[6],
        "│   ├── ○ tab 2       keyboard-shortcuts                   │"
    );
    assert_eq!(
        rows[7],
        "│   └── + new tab                                          │"
    );
    assert_eq!(
        rows[8],
        "│                                                          │"
    );
    assert_eq!(
        rows[19],
        "│ macOS › tab 1: macos-relay-…elay-3, vpn-morning-issues-2 │"
    );
    assert_eq!(picker(&state).selected, 1);
}

#[test]
fn snapshot_move_picker_small_scrolls_with_a_scrollbar() {
    let mut snapshot = sample();
    for n in 0..4 {
        let mut extra = session(&[("x", &[("", &["pane 1"])])], (0, 0, 0));
        let workspace_id = format!("s{n}");
        for workspace in &mut extra.workspaces {
            workspace.workspace_id = workspace_id.clone();
            workspace.label = format!("space-{n}");
            workspace.focused = false;
        }
        for tab in &mut extra.tabs {
            tab.workspace_id = workspace_id.clone();
            tab.tab_id = format!("{workspace_id}:t1");
            tab.focused = false;
        }
        for pane in &mut extra.panes {
            pane.workspace_id = workspace_id.clone();
            pane.tab_id = format!("{workspace_id}:t1");
            pane.pane_id = format!("{workspace_id}:p1");
            pane.focused = false;
        }
        snapshot.workspaces.extend(extra.workspaces);
        snapshot.tabs.extend(extra.tabs);
        snapshot.panes.extend(extra.panes);
    }
    let mut state = state_with(snapshot);
    open(&mut state);
    assert_eq!(layout(&state).outer, Rect::new(10, 0, 60, 25));
    let rows = boxed(&mut state);
    assert!(rows[2].contains("15 destinations"), "{}", rows[2]);
    let list = layout(&state).list;
    let scrollbar: String = rows
        .iter()
        .skip(usize::from(list.y))
        .take(usize::from(list.height))
        .map(|row| row.chars().nth(usize::from(list.right() - 1 - 10)).unwrap())
        .collect();
    assert!(scrollbar.chars().all(|c| c == '▕'), "{scrollbar}");
    // End jumps to the last destination: the list scrolls to its bottom.
    state.handle_input_bytes(b"\x1b[F");
    let rows = boxed(&mut state);
    let bottom = usize::from(list.y + list.height - 1);
    assert!(rows[bottom].contains("+ new space"), "{}", rows[bottom]);
}

#[test]
fn move_picker_rows_are_styled_like_the_navigator() {
    let mut state = state_with(sample());
    open(&mut state);
    let frame = state.compose(W, H).unwrap();
    let p = &state.config.palette;
    let cell = |x: u16, y: u16| &frame.cells[usize::from(y) * usize::from(W) + usize::from(x)];
    let color = crate::protocol::color_to_u32;
    let has = |x: u16, y: u16, modifier: Modifier| {
        cell(x, y).modifier & crate::protocol::modifier_to_u16(modifier) != 0
    };
    // Heading: accent and bold.
    assert_eq!(cell(16, 5).fg, color(p.accent));
    assert!(has(16, 5, Modifier::BOLD));
    // The here row: ◆ in the gutter, a dim label, an italic status.
    assert_eq!(cell(12, 6).symbol, "◆");
    assert_eq!(cell(20, 6).fg, color(p.overlay0));
    assert_eq!(cell(32, 6).fg, color(p.overlay0));
    assert!(has(32, 6, Modifier::ITALIC));
    // The first destination is the selected bar.
    assert_eq!(cell(20, 7).bg, color(p.accent));
    // A tab row: "tab" dim, the number bold text, pane names subtext0.
    assert_eq!(cell(14, 10).symbol, "├");
    assert_eq!(cell(14, 10).fg, color(p.surface1));
    assert_eq!(cell(20, 10).fg, color(p.overlay0));
    assert_eq!(cell(24, 10).fg, color(p.text));
    assert!(has(24, 10, Modifier::BOLD));
    assert_eq!(cell(32, 10).fg, color(p.subtext0));
    // An action row: "+" accent, "new tab" overlay1.
    assert_eq!(cell(18, 12).symbol, "+");
    assert_eq!(cell(18, 12).fg, color(p.accent));
    assert_eq!(cell(20, 12).fg, color(p.overlay1));
}

#[test]
fn move_picker_last_visible_child_gets_the_corner_branch() {
    let mut state = state_with(sample());
    open(&mut state);
    state.handle_input_bytes(b"/keyboard");
    let rows = boxed(&mut state);
    assert!(rows[5].starts_with("│   └── ○ tab 2"), "{}", rows[5]);
}

#[test]
fn move_picker_width_is_measured_from_tree_rows() {
    let mut state = state_with(session(
        &[
            ("a-space-with-a-long-name", &[("", &["x", "y"])]),
            ("b", &[("", &["z"])]),
        ],
        (0, 0, 0),
    ));
    open(&mut state);
    // Widest left: "tab 1" under a branch is 3 + 4 + 2 + 5 + 1 = 15; the
    // heading "a-space-with-a-long-name (2)" is 3 + 2 + 28 + 1 = 34. The
    // widest status is "you are here" (12) + 2.
    assert_eq!(layout(&state).outer.width, 34 + 14 + 2);
}

#[test]
fn fit_pane_names_ends_with_the_count_of_the_rest() {
    let names = ["macos-relay-2", "macos-relay-3", "vpn-morning-issues-2"].map(str::to_owned);
    assert_eq!(
        fit_pane_names(&names, 80),
        "macos-relay-2, macos-relay-3, vpn-morning-issues-2"
    );
    assert_eq!(
        fit_pane_names(&names, 34),
        "macos-relay-2, macos-relay-3, +1"
    );
    assert_eq!(fit_pane_names(&names, 20), "macos-relay-2, +2");
}

// -- rows -----------------------------------------------------------------------

#[test]
fn move_picker_shows_the_current_tab_as_context_and_gaps_between_spaces() {
    let picker = move_picker_for_snapshot(&three()).unwrap();
    assert_eq!(
        kinds(&picker.items),
        [
            "# one",
            "here",
            "new tab",
            "",
            "# two",
            "tab 1",
            "tab 2 · logs",
            "new tab",
            "",
            "# three",
            "tab 1",
            "new tab",
            "",
            "new space",
        ]
    );
}

#[test]
fn move_picker_never_selects_or_counts_the_current_tab() {
    let mut state = state_with(three());
    open(&mut state);
    assert_eq!(picker(&state).selected, 2);
    state.handle_input_bytes(b"k");
    assert_eq!(picker(&state).selected, 2, "nothing above to step to");
    // Hovering the here row does not select it.
    let list = layout(&state).list;
    hover(&mut state, list.x + 8, list.y + 1);
    assert_eq!(picker(&state).selected, 2);
    assert_eq!(targets(picker(&state)).len(), 7);
    assert!(boxed(&mut state)[2].contains("7 destinations"));
}

#[test]
fn move_picker_rows_carry_pane_names_state_and_counts() {
    let mut snapshot = three();
    snapshot.workspaces[0].agent_status = AgentStatus::Working;
    let picker = move_picker_for_snapshot(&snapshot).unwrap();
    let MoveItem::SpaceHeading {
        pane_count, status, ..
    } = &picker.items[0]
    else {
        panic!("heading first");
    };
    assert_eq!((*pane_count, *status), (2, AgentStatus::Working));
    let MoveItem::Here(here) = &picker.items[1] else {
        panic!("here second");
    };
    assert_eq!(here.pane_names, ["alpha", "beta"]);
    assert_eq!(picker.source_place, "one › tab 1");
    assert_eq!(picker.source_label, "alpha");
}

#[test]
fn move_picker_headings_carry_the_spaces_activity() {
    let mut snapshot = three();
    let mut agent = |pane: &str, status| {
        let pane = snapshot
            .panes
            .iter()
            .find(|p| p.pane_id == pane)
            .unwrap()
            .clone();
        snapshot.agents.push(crate::protocol::ClientShellAgent {
            pane_id: pane.pane_id,
            workspace_id: pane.workspace_id,
            tab_id: pane.tab_id,
            name: None,
            display_agent: None,
            agent: Some("claude".into()),
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            agent_status: status,
            state_change_seq: 0,
            state_labels: Vec::new(),
            tokens: Vec::new(),
            focused: false,
        });
    };
    agent("w1:p3", AgentStatus::Done);
    agent("w1:p4", AgentStatus::Blocked);
    let picker = move_picker_for_snapshot(&snapshot).unwrap();
    let MoveItem::SpaceHeading { activity, .. } = &picker.items[4] else {
        panic!("space two's heading");
    };
    assert_eq!(activity, "1 blocked · 1 done");
}

#[test]
fn move_picker_search_matches_pane_names_and_keeps_the_here_row_with_its_space() {
    let mut state = state_with(three());
    open(&mut state);
    state.handle_input_bytes(b"/keyboard");
    assert_eq!(kinds(&picker(&state).items), ["# three", "tab 1"]);
    // ctrl+u clears the field; `one` keeps the source space with its here row.
    state.handle_input_bytes(b"\x15one");
    assert_eq!(kinds(&picker(&state).items), ["# one", "here", "new tab"]);
}

#[test]
fn move_picker_slash_filters_to_a_space_and_enter_moves_there() {
    let mut state = state_with(three());
    open(&mut state);
    state.handle_input_bytes(b"/thr");
    let items = kinds(&picker(&state).items);
    assert_eq!(items[0], "# three");
    assert_eq!(picker(&state).selected, 1, "first match selected");
    let outcome = state.handle_input_bytes(b"\r");
    let (_, method) = only(&outcome);
    assert_eq!(
        method,
        Method::PaneMove(PaneMoveParams {
            pane_id: "w0:p1".into(),
            destination: PaneMoveDestination::Tab {
                tab_id: "w2:t1".into(),
                target_pane_id: None,
                split: SplitDirection::Right,
                ratio: None,
            },
            focus: true,
        })
    );
    assert!(state.overlay.is_none());
    assert_eq!(state.mode, ClientShellMode::Terminal);
}

#[test]
fn move_picker_heading_stays_while_a_destination_matches() {
    let mut state = state_with(three());
    open(&mut state);
    state.handle_input_bytes(b"/logs");
    assert_eq!(kinds(&picker(&state).items), ["# two", "tab 2 · logs"]);
    state.handle_input_bytes(b"\x15two");
    assert_eq!(
        kinds(&picker(&state).items),
        ["# two", "tab 1", "tab 2 · logs", "new tab"]
    );
}

#[test]
fn move_picker_no_match_keeps_the_query() {
    let mut state = state_with(three());
    open(&mut state);
    state.handle_input_bytes(b"/zzz");
    assert!(picker(&state).items.is_empty());
    assert!(boxed(&mut state)[4].starts_with("│ no match"));
    let outcome = state.handle_input_bytes(b"\r");
    assert!(methods(&outcome).is_empty(), "enter is inert");
    assert_eq!(picker(&state).search.text(), "zzz");
    // Esc: leave the search, clear it, close.
    state.handle_input_bytes(b"\x1b");
    assert!(!picker(&state).search_focused);
    assert_eq!(picker(&state).search.text(), "zzz");
    state.handle_input_bytes(b"\x1b");
    assert_eq!(picker(&state).search.text(), "");
    assert_eq!(picker(&state).items.len(), picker(&state).all_items.len());
    state.handle_input_bytes(b"\x1b");
    assert!(state.overlay.is_none());
    assert_eq!(state.mode, ClientShellMode::Terminal);
}

#[test]
fn focused_search_keeps_ctrl_j_k_as_moves_and_plain_letters_as_text() {
    let mut state = state_with(three());
    open(&mut state);
    state.handle_input_bytes(b"/");
    state.handle_input_bytes(b"\x0e");
    assert_eq!(picker(&state).selected, 5, "ctrl+n moved");
    state.handle_input_bytes(b"\x0b");
    assert_eq!(picker(&state).selected, 2, "ctrl+k moved");
    state.handle_input_bytes(b"jk");
    assert_eq!(picker(&state).search.text(), "jk");
}

// -- what opens -----------------------------------------------------------------

#[test]
fn move_target_picker_does_not_open_when_the_pane_is_the_whole_session() {
    let mut state = state_with(session(&[("only", &[("", &["solo"])])], (0, 0, 0)));
    open(&mut state);
    assert!(state.overlay.is_none());
    let (title, body) = notice(&state);
    assert_eq!(title, "pane move unavailable");
    assert!(body.contains("nowhere to move"), "{body}");
    assert_eq!(state.mode, ClientShellMode::Terminal);
}

#[test]
fn move_target_picker_opens_for_a_lone_tab_holding_more_than_one_pane() {
    let picker =
        move_picker_for_snapshot(&session(&[("only", &[("", &["a", "b"])])], (0, 0, 0))).unwrap();
    assert_eq!(
        targets(&picker),
        [
            MoveTarget::NewTab {
                workspace_id: "w0".into()
            },
            MoveTarget::NewSpace
        ]
    );
}

#[test]
fn move_target_picker_groups_every_space_with_the_source_space_first() {
    let picker = move_picker_for_snapshot(&session(
        &[
            ("one", &[("", &["a"])]),
            ("two", &[("", &["b", "c"])]),
            ("three", &[("", &["d"])]),
        ],
        (1, 0, 0),
    ))
    .unwrap();
    let headings: Vec<_> = kinds(&picker.items)
        .into_iter()
        .filter(|row| row.starts_with('#'))
        .collect();
    assert_eq!(headings, ["# two", "# one", "# three"]);
    assert_eq!(
        targets(&picker),
        [
            MoveTarget::NewTab {
                workspace_id: "w1".into()
            },
            MoveTarget::Tab {
                tab_id: "w0:t1".into()
            },
            MoveTarget::NewTab {
                workspace_id: "w0".into()
            },
            MoveTarget::Tab {
                tab_id: "w2:t1".into()
            },
            MoveTarget::NewTab {
                workspace_id: "w2".into()
            },
            MoveTarget::NewSpace,
        ]
    );
}

#[test]
fn move_target_picker_selection_skips_space_headings_and_clamps() {
    // A lone pane: its own space has nothing to offer and is skipped.
    let mut state = state_with(session(
        &[("one", &[("", &["a"])]), ("two", &[("", &["b"])])],
        (0, 0, 0),
    ));
    open(&mut state);
    assert_eq!(kinds(&picker(&state).items)[0], "# two");
    assert_eq!(picker(&state).selected, 1);
    for _ in 0..5 {
        state.handle_input_bytes(b"j");
        assert!(matches!(
            picker(&state).items[picker(&state).selected],
            MoveItem::Destination(_)
        ));
    }
    assert_eq!(
        kinds(&picker(&state).items)[picker(&state).selected],
        "new space"
    );
    for _ in 0..5 {
        state.handle_input_bytes(b"k");
    }
    assert_eq!(picker(&state).selected, 1);
}

#[test]
fn move_target_picker_is_rejected_while_the_source_tab_is_zoomed() {
    let mut snapshot = three();
    snapshot.tabs[0].zoomed = true;
    let mut state = state_with(snapshot);
    open(&mut state);
    assert!(state.overlay.is_none());
    assert_eq!(
        notice(&state),
        (
            "pane move unavailable".to_owned(),
            "pane moves are unavailable while the source tab is zoomed".to_owned()
        )
    );
}

#[test]
fn move_target_picker_excludes_current_and_zoomed_tabs_and_cancel_is_safe() {
    let mut snapshot = session(
        &[("one", &[("", &["a"]), ("", &["b"]), ("", &["c"])])],
        (0, 0, 0),
    );
    snapshot.tabs[2].zoomed = true;
    let mut state = state_with(snapshot);
    open(&mut state);
    assert_eq!(
        targets(picker(&state)),
        [
            MoveTarget::Tab {
                tab_id: "w0:t2".into()
            },
            MoveTarget::NewSpace
        ]
    );
    let outcome = state.handle_input_bytes(b"\x1b");
    assert!(methods(&outcome).is_empty());
    assert!(state.overlay.is_none());
    assert_eq!(state.mode, ClientShellMode::Terminal);
}

#[test]
fn submit_sends_the_move_for_each_kind_of_row() {
    // The first destination is space one's new tab.
    let mut state = state_with(three());
    open(&mut state);
    let (_, method) = only(&state.handle_input_bytes(b"\r"));
    assert_eq!(
        method,
        Method::PaneMove(PaneMoveParams {
            pane_id: "w0:p1".into(),
            destination: PaneMoveDestination::NewTab {
                workspace_id: Some("w0".into()),
                label: None,
            },
            focus: true,
        })
    );
    // End selects the new space.
    open(&mut state);
    state.handle_input_bytes(b"\x1b[F");
    let (_, method) = only(&state.handle_input_bytes(b"\r"));
    assert_eq!(
        method,
        Method::PaneMove(PaneMoveParams {
            pane_id: "w0:p1".into(),
            destination: PaneMoveDestination::NewWorkspace {
                label: None,
                tab_label: None,
            },
            focus: true,
        })
    );
}

#[test]
fn a_declined_move_says_why_and_an_error_says_it_failed() {
    let mut state = state_with(three());
    open(&mut state);
    let (id, _) = only(&state.handle_input_bytes(b"\r"));
    state.handle_endpoint_result("boot-1", &id, Ok(move_result(false, Some("zoomed_tab"))));
    assert_eq!(
        notice(&state),
        (
            "pane move unavailable".to_owned(),
            "pane moves are unavailable while a source or target tab is zoomed".to_owned()
        )
    );

    open(&mut state);
    let (id, _) = only(&state.handle_input_bytes(b"\r"));
    state.handle_endpoint_result(
        "boot-1",
        &id,
        Err(ClientShellEndpointError {
            code: Some("tab_not_found".into()),
            message: "tab w2:t1 not found".into(),
        }),
    );
    assert_eq!(
        notice(&state),
        (
            "pane move failed".to_owned(),
            "tab w2:t1 not found".to_owned()
        )
    );
}

#[test]
fn a_move_that_landed_shows_nothing() {
    let mut state = state_with(three());
    open(&mut state);
    let (id, _) = only(&state.handle_input_bytes(b"\r"));
    state.handle_endpoint_result("boot-1", &id, Ok(move_result(true, None)));
    assert!(state.visible_endpoint_notice.is_none());
}

// -- mouse ----------------------------------------------------------------------

#[test]
fn clicking_a_destination_row_moves_at_once() {
    let mut state = state_with(three());
    open(&mut state);
    let list = layout(&state).list;
    // Row 5 is space two's first tab.
    let (_, method) = only(&click(&mut state, list.x + 10, list.y + 5));
    assert!(matches!(
        method,
        Method::PaneMove(PaneMoveParams {
            destination: PaneMoveDestination::Tab { ref tab_id, .. },
            ..
        }) if tab_id == "w1:t1"
    ));
    assert!(state.overlay.is_none());
    assert_eq!(state.mode, ClientShellMode::Terminal);
}

#[test]
fn move_picker_button_row_near_miss_does_not_close() {
    let mut state = state_with(three());
    open(&mut state);
    let (first, _) = layout(&state).buttons[0];
    let outcome = click(&mut state, first.x - 1, first.y);
    assert!(methods(&outcome).is_empty());
    assert!(state.overlay.is_some());
}

#[test]
fn move_picker_buttons_move_and_cancel() {
    let mut state = state_with(three());
    open(&mut state);
    let buttons = layout(&state).buttons;
    let (cancel, _) = buttons[1];
    assert!(methods(&click(&mut state, cancel.x, cancel.y)).is_empty());
    assert!(state.overlay.is_none());
    open(&mut state);
    let (go, _) = buttons[0];
    let (_, method) = only(&click(&mut state, go.x, go.y));
    assert!(matches!(method, Method::PaneMove(_)));
}

#[test]
fn move_picker_here_row_is_inert_under_the_mouse() {
    let mut state = state_with(three());
    open(&mut state);
    let list = layout(&state).list;
    hover(&mut state, list.x + 10, list.y + 1);
    let outcome = click(&mut state, list.x + 10, list.y + 1);
    assert!(methods(&outcome).is_empty());
    assert_eq!(picker(&state).selected, 2);
}

#[test]
fn inside_click_off_rows_does_not_close_the_move_picker() {
    let mut state = state_with(three());
    open(&mut state);
    let layout = layout(&state);
    // The rule under the header.
    click(&mut state, layout.list.x + 5, layout.list.y - 1);
    assert!(state.overlay.is_some());
    // A gap row.
    click(&mut state, layout.list.x + 5, layout.list.y + 3);
    assert!(state.overlay.is_some());
    click(&mut state, layout.outer.x - 1, layout.list.y);
    assert!(state.overlay.is_none());
}

#[test]
fn clicking_the_search_row_focuses_it() {
    let mut state = state_with(three());
    open(&mut state);
    let row = layout(&state).search_row();
    click(&mut state, row.x + 5, row.y);
    assert!(picker(&state).search_focused);
}

#[test]
fn the_wheel_moves_the_selection() {
    let mut state = state_with(three());
    open(&mut state);
    let list = layout(&state).list;
    state.handle_raw_events(vec![crate::raw_input::RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: list.x,
        row: list.y,
        modifiers: KeyModifiers::empty(),
    })]);
    assert_eq!(picker(&state).selected, 5);
}

// -- the other pane-move keys ---------------------------------------------------

#[test]
fn break_pane_moves_the_pane_to_a_new_tab_of_its_space() {
    let mut state = state_with(three());
    let (_, method) = only(&bind(&mut state, KeybindAction::BreakPane));
    assert_eq!(
        method,
        Method::PaneMove(PaneMoveParams {
            pane_id: "w0:p1".into(),
            destination: PaneMoveDestination::NewTab {
                workspace_id: Some("w0".into()),
                label: None,
            },
            focus: true,
        })
    );
}

#[test]
fn break_pane_refuses_a_pane_already_alone() {
    let mut state = state_with(session(
        &[("one", &[("", &["a"])]), ("two", &[("", &["b"])])],
        (0, 0, 0),
    ));
    let outcome = bind(&mut state, KeybindAction::BreakPane);
    assert!(methods(&outcome).is_empty());
    assert_eq!(
        notice(&state),
        (
            "nothing to break out".to_owned(),
            "the focused pane is already alone in its tab".to_owned()
        )
    );
}

#[test]
fn adjacent_pane_moves_do_not_wrap() {
    let mut state = state_with(session(
        &[("one", &[("", &["a"]), ("", &["b"])])],
        (0, 0, 0),
    ));
    let outcome = bind(&mut state, KeybindAction::MovePanePrevTab);
    assert!(methods(&outcome).is_empty());
    assert_eq!(
        notice(&state),
        (
            "no adjacent tab".to_owned(),
            "there is no previous tab to move into".to_owned()
        )
    );
    let (_, method) = only(&bind(&mut state, KeybindAction::MovePaneNextTab));
    assert_eq!(
        method,
        Method::PaneMove(PaneMoveParams {
            pane_id: "w0:p1".into(),
            destination: PaneMoveDestination::Tab {
                tab_id: "w0:t2".into(),
                target_pane_id: None,
                split: SplitDirection::Right,
                ratio: None,
            },
            focus: true,
        })
    );
}

#[test]
fn default_pane_move_bindings_and_help_entries() {
    let (live, _) = Config::default().live_keybinds_with_diagnostics().unwrap();
    let groups = crate::input::keybind_help_groups(&live.keybinds, &live.prefix);
    let panes = &groups.iter().find(|(name, _)| *name == "panes").unwrap().1;
    for (key, label) in [
        ("prefix+!", "break pane to new tab"),
        ("prefix+m", "move pane to tab or space"),
        ("prefix+>", "move pane to next tab"),
        ("prefix+<", "move pane to previous tab"),
    ] {
        assert!(
            panes.iter().any(|(k, l)| k == key && l == label),
            "{key} {label}: {panes:?}"
        );
    }
    assert!(groups.iter().any(|(name, _)| *name == "move pane picker"));
}
