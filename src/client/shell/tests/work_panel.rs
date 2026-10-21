//! The work panel (fork issue 241, task 2.14): four views over `issue.list`,
//! `task.list` and `ask.list`, drawn and driven through snapshots and emitted
//! endpoint requests.

use super::super::work_panel::{ClientWorkPanelOverlay, Feed, WorkPanelLayout, WorkRow, WorkView};
use super::*;
use crate::api::schema::{
    AskInfo, IssueInfo, IssueProjectInfo, IssueRefInfo, Method, MirrorInfo, ResponseResult,
    TaskInfo, TaskPaneInfo, TaskPriorityInfo, TaskShownInfo, TaskStatusInfo, TaskStoreInfo,
};
use crate::input::KeybindAction;

const W: u16 = 120;
const H: u16 = 34;

fn bind(state: &mut ClientShellState, action: KeybindAction) -> ClientShellInput {
    let mut outcome = ClientShellInput::default();
    state.record_binding(crate::input::KeybindMatch::Action(action), &mut outcome);
    outcome
}

fn requests(outcome: &ClientShellInput) -> Vec<(String, Method)> {
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

fn issue(project: &str, number: u64, title: &str, labels: &[&str], updated: &str) -> IssueInfo {
    IssueInfo {
        key: format!("issue.gitea.gitea-lab.84.{number}"),
        issue_ref: IssueRefInfo {
            forge: "gitea".into(),
            forge_id: "gitea-lab".into(),
            host: "gitea.example".into(),
            project_id: if project == "ac/worklink" { 84 } else { 85 },
            project_path: project.into(),
            number,
        },
        mirror_rev: 10 + number,
        title: title.into(),
        state: "open".into(),
        labels: labels.iter().map(|l| (*l).to_owned()).collect(),
        assignees: Vec::new(),
        author: Some("ac".into()),
        created_at: None,
        updated_at: Some(updated.into()),
        web_url: Some(format!("https://gitea.example/{project}/issues/{number}")),
        source_rev: None,
        last_sync: None,
        truncated: false,
    }
}

fn project(path: &str, id: u64, synced: bool) -> IssueProjectInfo {
    IssueProjectInfo {
        forge: "gitea".into(),
        forge_id: "gitea-lab".into(),
        project_id: id,
        project_path: Some(path.into()),
        synced,
        status: synced.then(|| "ok".to_owned()),
        error_class: None,
        last_attempt: None,
        last_ok: synced.then(|| "2026-10-21T12:00:00Z".to_owned()),
    }
}

fn task(key: &str, title: &str, shown: TaskShownInfo, owner: &str, pane: &str) -> TaskInfo {
    TaskInfo {
        key: key.into(),
        id: key.rsplit('.').next().unwrap_or(key).into(),
        owner_host: "m4m".into(),
        rev: 1,
        title: title.into(),
        body: None,
        status: TaskStatusInfo::Open,
        shown,
        priority: TaskPriorityInfo::Normal,
        horizon: None,
        owner: Some(owner.into()),
        waits_on: None,
        waits_on_missing: false,
        refs: Vec::new(),
        runs: Vec::new(),
        pane: Some(TaskPaneInfo {
            host: "m4m".into(),
            session: "s".into(),
            pane_key: None,
            label: pane.into(),
            pane_id: None,
        }),
        related_pane: None,
        client: None,
        created_by: "x".into(),
        created_via: "cli".into(),
        review: None,
        created_at: "2026-10-21T10:00:00Z".into(),
        updated_at: "2026-10-21T10:00:00Z".into(),
        closed: None,
        moved_to: None,
        moved_from: None,
        pending: false,
        conflict: false,
    }
}

fn ask(key: &str, title: &str, status: &str) -> AskInfo {
    AskInfo {
        key: key.into(),
        rev: 2,
        status: status.into(),
        kind: Some("task".into()),
        title: Some(title.into()),
        how: None,
        where_: Some("herdr-max #241".into()),
        horizon: None,
        filed_by: None,
        answer: None,
        answered_at: None,
        superseded_by: None,
        source: None,
        created_at: None,
        updated_at: None,
    }
}

fn store() -> TaskStoreInfo {
    TaskStoreInfo {
        state: "ready".into(),
        detail: None,
        pending: 0,
        conflicts: 0,
    }
}

fn loaded() -> MirrorInfo {
    MirrorInfo {
        loaded: true,
        detail: None,
    }
}

fn sample_issues() -> Vec<IssueInfo> {
    vec![
        issue(
            "ac/worklink",
            1,
            "sync fails on rename",
            &["bug"],
            "2026-10-21T09:00:00Z",
        ),
        issue(
            "ac/worklink",
            2,
            "pick a forge for the pilot",
            &["needs/decision"],
            "2026-10-21T11:00:00Z",
        ),
        issue(
            "ac/herdr-max",
            241,
            "the task store",
            &["status/in-progress"],
            "2026-10-21T12:00:00Z",
        ),
    ]
}

fn sample_tasks() -> Vec<TaskInfo> {
    vec![
        task(
            "task.m4m.tk-1",
            "write the store",
            TaskShownInfo::Running,
            "herdr-wrk",
            "wrk",
        ),
        task(
            "task.m4m.tk-2",
            "review it",
            TaskShownInfo::Open,
            "herdr-helper",
            "helper",
        ),
        task(
            "task.m4m.tk-3",
            "wait for ac",
            TaskShownInfo::Waiting,
            "herdr-wrk",
            "wrk",
        ),
    ]
}

fn state_with(folder: Option<&str>) -> ClientShellState {
    let mut snapshot = super::snapshot();
    if let Some(folder) = folder {
        for pane in &mut snapshot.panes {
            pane.foreground_cwd = Some(folder.to_owned());
        }
    }
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface());
    state.compose(W, H).unwrap();
    state
}

/// Open the panel and answer its three lists.
fn open_with(
    state: &mut ClientShellState,
    issues: Vec<IssueInfo>,
    projects: Vec<IssueProjectInfo>,
    tasks: Vec<TaskInfo>,
    asks: Vec<AskInfo>,
) {
    let outcome = bind(state, KeybindAction::OpenWorkPanel);
    let sent = requests(&outcome);
    assert_eq!(sent.len(), 3, "{sent:?}");
    for (id, method) in sent {
        let result = match method {
            Method::IssueList(_) => ResponseResult::IssueList {
                issues: issues.clone(),
                projects: projects.clone(),
                mirror: loaded(),
            },
            Method::TaskList(_) => ResponseResult::TaskList {
                tasks: tasks.clone(),
                store: store(),
            },
            Method::AskList(_) => ResponseResult::AskList {
                asks: asks.clone(),
                mirror: loaded(),
            },
            other => panic!("unexpected request {other:?}"),
        };
        state.handle_endpoint_result("boot-1", &id, Ok(result));
    }
    state.compose(W, H).unwrap();
}

fn sample() -> ClientShellState {
    let mut state = state_with(Some("/Users/ac/dev/worktrees/worklink"));
    open_with(
        &mut state,
        sample_issues(),
        vec![
            project("ac/worklink", 84, true),
            project("ac/herdr-max", 85, false),
        ],
        sample_tasks(),
        vec![ask("gestore-lab.a1", "do the thing", "open")],
    );
    state
}

fn panel(state: &ClientShellState) -> &ClientWorkPanelOverlay {
    match state.overlay.as_ref() {
        Some(ClientShellOverlay::WorkPanel(panel)) => panel,
        other => panic!("expected the work panel, got {other:?}"),
    }
}

fn layout(state: &ClientShellState) -> WorkPanelLayout {
    state.hits.work_panel.clone().expect("panel layout")
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

/// The panel box as text, one string per row.
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

fn key(state: &mut ClientShellState, code: crossterm::event::KeyCode) {
    let mut outcome = ClientShellInput::default();
    state.route_overlay_key(
        &crate::input::TerminalKey::new(code, crossterm::event::KeyModifiers::empty()),
        &mut outcome,
    );
}

fn titles(state: &ClientShellState) -> Vec<String> {
    panel(state)
        .rows
        .iter()
        .filter_map(|row| match row {
            WorkRow::Heading { text, count } => Some(format!("# {text} {count}")),
            WorkRow::Note(text) => Some(format!("note: {text}")),
            WorkRow::Issue(i) => match &panel(state).issues {
                Feed::Ready(data) => Some(format!("issue {}", data.issues[*i].issue_ref.number)),
                _ => None,
            },
            WorkRow::Task(i) => match &panel(state).tasks {
                Feed::Ready(tasks) => Some(format!("task {}", tasks[*i].title)),
                _ => None,
            },
            WorkRow::Ask(i) => match &panel(state).asks {
                Feed::Ready(data) => Some(format!("ask {}", data.asks[*i].key)),
                _ => None,
            },
        })
        .collect()
}

#[test]
fn opening_the_panel_asks_the_server_for_the_three_lists() {
    let mut state = state_with(None);
    let sent = requests(&bind(&mut state, KeybindAction::OpenWorkPanel));
    let names: Vec<_> = sent
        .iter()
        .map(|(_, method)| crate::api::api_method_name(method).to_owned())
        .collect();
    assert_eq!(names, ["issue.list", "task.list", "ask.list"]);
}

#[test]
fn this_repo_shows_the_project_the_focused_folder_names_and_its_last_sync() {
    let mut state = sample();
    assert_eq!(
        titles(&state),
        ["# ac/worklink 2", "issue 2", "issue 1"],
        "the project named like a folder on the pane's path, newest first"
    );
    let text = boxed(&mut state).join("\n");
    assert!(text.contains("work"), "{text}");
    assert!(text.contains("1 this repo 2"), "{text}");
    assert!(text.contains("2 needs me 1"), "{text}");
    assert!(
        text.contains("ac/worklink synced 2026-10-21 12:00"),
        "{text}"
    );
    assert!(text.contains("#2 pick a forge for the pilot"), "{text}");
    assert!(!text.contains("the task store"), "{text}");
}

#[test]
fn this_repo_with_no_match_says_so_and_shows_every_project() {
    let mut state = state_with(Some("/tmp/unrelated"));
    open_with(
        &mut state,
        sample_issues(),
        vec![
            project("ac/worklink", 84, true),
            project("ac/herdr-max", 85, false),
        ],
        Vec::new(),
        Vec::new(),
    );
    let rows = titles(&state);
    assert_eq!(
        rows[0],
        "note: no project matches this pane's folder; every project is shown"
    );
    assert!(rows.contains(&"# ac/herdr-max 1".to_owned()), "{rows:?}");
    let text = boxed(&mut state).join("\n");
    assert!(
        text.contains("ac/herdr-max not synced"),
        "a project with no sync row: {text}"
    );
}

#[test]
fn the_other_views_pick_their_rows_by_label_and_status() {
    let mut state = sample();
    key(&mut state, crossterm::event::KeyCode::Char('2'));
    assert_eq!(panel(&state).view, WorkView::NeedsMe);
    assert_eq!(titles(&state), ["# ac/worklink 1", "issue 2"]);

    key(&mut state, crossterm::event::KeyCode::Char('3'));
    assert_eq!(
        titles(&state),
        [
            "# ac/herdr-max 1",
            "issue 241",
            "# tasks running 1",
            "task write the store"
        ]
    );

    key(&mut state, crossterm::event::KeyCode::Char('4'));
    assert_eq!(
        titles(&state),
        [
            "# herdr-helper · helper 1",
            "task review it",
            "# herdr-wrk · wrk 2",
            "task write the store",
            "task wait for ac",
            "# ac's task rows 1",
            "ask gestore-lab.a1",
        ]
    );
    let text = boxed(&mut state).join("\n");
    assert!(text.contains("do the thing"), "{text}");
    assert!(text.contains("herdr-max #241"), "{text}");
}

#[test]
fn tab_and_the_view_chips_switch_views() {
    let mut state = sample();
    key(&mut state, crossterm::event::KeyCode::Tab);
    assert_eq!(panel(&state).view, WorkView::NeedsMe);
    key(&mut state, crossterm::event::KeyCode::BackTab);
    key(&mut state, crossterm::event::KeyCode::BackTab);
    assert_eq!(panel(&state).view, WorkView::MyTasks, "wraps around");

    state.compose(W, H).unwrap();
    let (rect, view) = layout(&state).tabs[2];
    assert_eq!(view, WorkView::InFlight);
    let mut outcome = ClientShellInput::default();
    state.handle_mouse(mouse_down(rect.x + 1, rect.y), &mut outcome);
    assert_eq!(panel(&state).view, WorkView::InFlight);
}

fn mouse_down(column: u16, row: u16) -> crossterm::event::MouseEvent {
    crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column,
        row,
        modifiers: crossterm::event::KeyModifiers::empty(),
    }
}

#[test]
fn the_selection_skips_headings_and_notes() {
    let mut state = sample();
    key(&mut state, crossterm::event::KeyCode::Char('4'));
    let on_row = |state: &ClientShellState| panel(state).rows[panel(state).selected].clone();
    // The first selectable row is the first task, below its heading.
    assert!(matches!(on_row(&state), WorkRow::Task(_)));
    for _ in 0..10 {
        key(&mut state, crossterm::event::KeyCode::Down);
        assert!(!matches!(
            on_row(&state),
            WorkRow::Heading { .. } | WorkRow::Note(_)
        ));
    }
    assert_eq!(
        on_row(&state),
        WorkRow::Ask(0),
        "the end of the list is the ask row"
    );
    key(&mut state, crossterm::event::KeyCode::Up);
    key(&mut state, crossterm::event::KeyCode::Up);
    assert!(
        matches!(on_row(&state), WorkRow::Task(_)),
        "going up steps over the heading in between"
    );
}

#[test]
fn a_server_without_the_work_methods_is_said_to_lack_them() {
    let mut state = state_with(None);
    // The server advertises task.* but not the work methods.
    state.set_endpoint_methods(Some(vec!["task.list".to_owned(), "task.show".to_owned()]));
    let sent = requests(&bind(&mut state, KeybindAction::OpenWorkPanel));
    let names: Vec<_> = sent
        .iter()
        .map(|(_, method)| crate::api::api_method_name(method).to_owned())
        .collect();
    assert_eq!(names, ["task.list"], "only what the server has is asked");
    state.compose(W, H).unwrap();
    let text = boxed(&mut state).join("\n");
    assert!(text.contains("does not have the work methods"), "{text}");
    assert!(
        matches!(panel(&state).issues, Feed::Failed(_)),
        "the issue source is marked, the rest of the shell stays up"
    );
    assert!(state.overlay.is_some());
}

#[test]
fn a_mirror_that_is_not_read_yet_says_why_instead_of_showing_an_empty_list() {
    let mut state = state_with(None);
    let outcome = bind(&mut state, KeybindAction::OpenWorkPanel);
    for (id, method) in requests(&outcome) {
        if let Method::IssueList(_) = method {
            state.handle_endpoint_result(
                "boot-1",
                &id,
                Ok(ResponseResult::IssueList {
                    issues: Vec::new(),
                    projects: Vec::new(),
                    mirror: MirrorInfo {
                        loaded: false,
                        detail: Some("not available with this backend".into()),
                    },
                }),
            );
        }
    }
    state.compose(W, H).unwrap();
    assert!(
        titles(&state)[0].contains("issues not read yet: not available with this backend"),
        "{:?}",
        titles(&state)
    );
}

#[test]
fn refresh_asks_again_and_close_leaves_the_terminal() {
    let mut state = sample();
    let mut outcome = ClientShellInput::default();
    state.route_overlay_key(
        &crate::input::TerminalKey::new(
            crossterm::event::KeyCode::Char('r'),
            crossterm::event::KeyModifiers::empty(),
        ),
        &mut outcome,
    );
    assert_eq!(requests(&outcome).len(), 3);
    key(&mut state, crossterm::event::KeyCode::Esc);
    assert!(state.overlay.is_none());
    assert_eq!(state.mode, ClientShellMode::Terminal);
}

#[test]
fn the_panel_polls_but_not_while_a_request_is_on_its_way() {
    let mut state = sample();
    let mut quiet = ClientShellInput::default();
    state.tick_work_panel(&mut quiet);
    assert!(requests(&quiet).is_empty(), "asked a moment ago");
    if let Some(ClientShellOverlay::WorkPanel(panel)) = state.overlay.as_mut() {
        panel.last_fetch = std::time::Instant::now() - std::time::Duration::from_secs(60);
    }
    let mut due = ClientShellInput::default();
    state.tick_work_panel(&mut due);
    assert_eq!(requests(&due).len(), 3);
    let mut busy = ClientShellInput::default();
    if let Some(ClientShellOverlay::WorkPanel(panel)) = state.overlay.as_mut() {
        panel.last_fetch = std::time::Instant::now() - std::time::Duration::from_secs(60);
    }
    state.tick_work_panel(&mut busy);
    assert!(
        requests(&busy).is_empty(),
        "the last ask is not answered yet"
    );
}

#[test]
fn keybind_help_lists_open_work_panel() {
    let (live, _) = Config::default().live_keybinds_with_diagnostics().unwrap();
    let groups = crate::input::keybind_help_groups(&live.keybinds, &live.prefix);
    let found = groups
        .iter()
        .flat_map(|(_, entries)| entries.iter())
        .find(|(_, label)| label == "work panel")
        .map(|(key, _)| key.clone());
    assert_eq!(
        found.as_deref(),
        Some("unset"),
        "unbound until the user binds it"
    );
}

#[test]
fn a_click_selects_a_row_and_a_click_outside_closes() {
    let mut state = sample();
    state.compose(W, H).unwrap();
    let layout = layout(&state);
    // Row 2 of the this-repo view is the older issue (row 0 is the heading).
    let row = layout.list.y + 2;
    let mut outcome = ClientShellInput::default();
    state.handle_mouse(mouse_down(layout.list.x + 3, row), &mut outcome);
    assert_eq!(panel(&state).selected, 2);
    // A click on the heading changes nothing.
    state.handle_mouse(mouse_down(layout.list.x + 3, layout.list.y), &mut outcome);
    assert_eq!(panel(&state).selected, 2);
    state.handle_mouse(mouse_down(0, 0), &mut outcome);
    assert!(state.overlay.is_none(), "outside the box closes the panel");
}

#[test]
fn a_tiny_screen_draws_nothing_instead_of_panicking() {
    for (w, h) in [(10, 5), (30, 8), (61, 12)] {
        let mut state = sample();
        let _ = state.compose(w, h);
        key(&mut state, crossterm::event::KeyCode::Down);
        key(&mut state, crossterm::event::KeyCode::Char('4'));
        let _ = state.compose(w, h);
    }
}

#[test]
fn a_row_shows_its_marks_and_the_dim_words_on_the_right() {
    let mut state = sample();
    key(&mut state, crossterm::event::KeyCode::Char('4'));
    let text = boxed(&mut state).join("\n");
    assert!(text.contains("● write the store"), "running mark: {text}");
    assert!(text.contains("◌ wait for ac"), "waiting mark: {text}");
    assert!(text.contains("☐ review it"), "open mark: {text}");
}
