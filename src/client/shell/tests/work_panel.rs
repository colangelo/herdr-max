//! The work panel (fork issue 241, task 2.14): four views over `issue.list`,
//! `task.list` and `ask.list`, drawn and driven through snapshots and emitted
//! endpoint requests.

use super::super::work_panel::{ClientWorkPanelOverlay, Feed, WorkPanelLayout, WorkRow, WorkView};
use super::*;
use crate::api::schema::{
    AskInfo, IssueInfo, IssueProjectInfo, IssueRefInfo, IssueRepoInfo, Method, MirrorInfo,
    ResponseResult, TaskInfo, TaskIssueRefInfo, TaskPaneInfo, TaskPriorityInfo, TaskShownInfo,
    TaskStatusInfo, TaskStoreInfo,
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
    let project_id = if project == "ac/worklink" { 84 } else { 85 };
    IssueInfo {
        key: format!("issue.gitea.gitea-lab.{project_id}.{number}"),
        issue_ref: IssueRefInfo {
            forge: "gitea".into(),
            forge_id: "gitea-lab".into(),
            host: "gitea.example".into(),
            project_id,
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
        mine: false,
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
        issues_open: 0,
        issues_total: 0,
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

/// `task` that refers to the issue `number` of the worklink project.
fn on_issue(mut task: TaskInfo, number: u64) -> TaskInfo {
    task.refs = vec![TaskIssueRefInfo {
        forge: "gitea".into(),
        forge_id: "gitea-lab".into(),
        host: "gitea.example".into(),
        project_id: 84,
        project_path: "ac/worklink".into(),
        number,
    }];
    task
}

fn ask(key: &str, title: &str, status: &str, kind: &str, wher: &str) -> AskInfo {
    AskInfo {
        key: key.into(),
        rev: 2,
        status: status.into(),
        kind: Some(kind.into()),
        title: Some(title.into()),
        how: None,
        where_: Some(wher.into()),
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

fn store(state: &str) -> TaskStoreInfo {
    TaskStoreInfo {
        state: state.into(),
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

struct Answers {
    issues: Vec<IssueInfo>,
    projects: Vec<IssueProjectInfo>,
    tasks: Vec<TaskInfo>,
    asks: Vec<AskInfo>,
    store: &'static str,
    /// What the server says it read for the pane: `found` for worklink.
    repo: IssueRepoInfo,
    /// Closed issues of the pane's repo; sent only when `closed_since` is set.
    closed: Vec<IssueInfo>,
}

fn repo_found(path: &str, mirrored: bool) -> IssueRepoInfo {
    IssueRepoInfo {
        status: "found".into(),
        host: Some("gitea.example".into()),
        path: Some(path.into()),
        mirrored,
    }
}

fn repo_status(status: &str) -> IssueRepoInfo {
    IssueRepoInfo {
        status: status.into(),
        host: None,
        path: None,
        mirrored: false,
    }
}

fn answers() -> Answers {
    Answers {
        issues: vec![
            issue(
                "ac/worklink",
                1,
                "sync fails on rename",
                &["bug", "horizon/later"],
                "2026-10-21T09:00:00Z",
            ),
            issue(
                "ac/worklink",
                2,
                "pick a forge for the pilot",
                &["needs/decision", "horizon/now"],
                "2026-10-21T11:00:00Z",
            ),
            issue(
                "ac/herdr-max",
                241,
                "the task store",
                &["status/in-progress"],
                "2026-10-21T12:00:00Z",
            ),
        ],
        projects: vec![
            project("ac/worklink", 84, true),
            project("ac/herdr-max", 85, false),
        ],
        tasks: vec![
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
        ],
        asks: vec![ask(
            "gestore-lab.a1",
            "do the thing",
            "open",
            "task",
            "herdr-max #241",
        )],
        store: "ready",
        repo: repo_found("ac/worklink", true),
        closed: Vec::new(),
    }
}

fn answer(sent: Vec<(String, Method)>, a: &Answers) -> Vec<(String, ResponseResult)> {
    sent.into_iter()
        .map(|(id, method)| {
            let result = match method {
                // Like the server: with a pane, only the pane's repo's issues,
                // and the closed ones only when asked for.
                Method::IssueList(params) if params.pane.is_some() => {
                    let mut issues: Vec<IssueInfo> = match a.repo.path.as_deref() {
                        Some(path) if a.repo.status == "found" => a
                            .issues
                            .iter()
                            .filter(|issue| issue.issue_ref.project_path == path)
                            .cloned()
                            .collect(),
                        _ => Vec::new(),
                    };
                    if params.closed_since.is_some() && a.repo.status == "found" {
                        issues.extend(a.closed.iter().cloned());
                    }
                    ResponseResult::IssueList {
                        total: issues.len() as u64,
                        issues,
                        projects: a.projects.clone(),
                        mirror: loaded(),
                        repo: Some(a.repo.clone()),
                    }
                }
                Method::IssueList(_) => ResponseResult::IssueList {
                    issues: a.issues.clone(),
                    total: a.issues.len() as u64,
                    projects: a.projects.clone(),
                    mirror: loaded(),
                    repo: None,
                },
                Method::TaskList(_) => ResponseResult::TaskList {
                    tasks: a.tasks.clone(),
                    store: store(a.store),
                },
                Method::AskList(_) => ResponseResult::AskList {
                    asks: a.asks.clone(),
                    total: a.asks.len() as u64,
                    mirror: loaded(),
                },
                other => panic!("unexpected request {other:?}"),
            };
            (id, result)
        })
        .collect()
}

fn state_with() -> ClientShellState {
    let snapshot = super::snapshot();
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface());
    state.compose(W, H).unwrap();
    state
}

/// Open the panel and answer its three lists.
fn open_with(state: &mut ClientShellState, a: &Answers) {
    let outcome = bind(state, KeybindAction::OpenWorkPanel);
    let sent = requests(&outcome);
    assert_eq!(sent.len(), 4, "{sent:?}");
    for (id, result) in answer(sent, a) {
        state.handle_endpoint_result("boot-1", &id, Ok(result));
    }
    state.compose(W, H).unwrap();
}

/// Ask again (what `r` and the poll do) and answer.
fn refresh_with(state: &mut ClientShellState, a: &Answers) {
    let mut outcome = ClientShellInput::default();
    if let Some(ClientShellOverlay::WorkPanel(panel)) = state.overlay.as_mut() {
        panel.in_flight = 0;
    }
    state.request_work_panel(&mut outcome);
    for (id, result) in answer(requests(&outcome), a) {
        state.handle_endpoint_result("boot-1", &id, Ok(result));
    }
    state.compose(W, H).unwrap();
}

fn sample() -> ClientShellState {
    let mut state = state_with();
    open_with(&mut state, &answers());
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

fn mouse_down(column: u16, row: u16) -> crossterm::event::MouseEvent {
    crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column,
        row,
        modifiers: crossterm::event::KeyModifiers::empty(),
    }
}

/// The rows of the current view as short strings.
fn titles(state: &ClientShellState) -> Vec<String> {
    let panel = panel(state);
    panel
        .rows
        .iter()
        .map(|row| match row {
            WorkRow::Heading { text, count } => format!("# {text} {count}"),
            WorkRow::Note(text) => format!("note: {text}"),
            WorkRow::Problem(text) => format!("problem: {text}"),
            WorkRow::Issue(i) => format!("issue {}", panel.issue_at(*i).unwrap().issue_ref.number),
            WorkRow::IssueWhy(i, why) => format!(
                "issue {} [{why}]",
                panel.issue_at(*i).unwrap().issue_ref.number
            ),
            WorkRow::Task(i) => format!("task {}", panel.tasks.data().unwrap().tasks[*i].title),
            WorkRow::Ask(i) => format!("ask {}", panel.asks.data().unwrap().asks[*i].key),
        })
        .collect()
}

fn selected_key(state: &ClientShellState) -> Option<String> {
    panel(state).selected_key.clone()
}

#[test]
fn opening_the_panel_asks_the_server_for_its_lists() {
    let mut state = state_with();
    let sent = requests(&bind(&mut state, KeybindAction::OpenWorkPanel));
    let names: Vec<_> = sent
        .iter()
        .map(|(_, method)| crate::api::api_method_name(method).to_owned())
        .collect();
    assert_eq!(names, ["issue.list", "issue.list", "task.list", "ask.list"]);
    // Open issues, a page at a time; every kind of ask, for the reasons.
    assert!(matches!(
        &sent[0].1,
        Method::IssueList(p) if p.state.as_deref() == Some("open") && p.limit == Some(500)
    ));
    // The pane's repo, read by the server from the pane's git remote.
    assert!(matches!(
        &sent[1].1,
        Method::IssueList(p) if p.pane.as_deref() == Some("pane_1")
            && p.state.as_deref() == Some("open")
            && p.closed_since.is_none()
    ));
    assert!(matches!(
        &sent[3].1,
        Method::AskList(p) if p.kind.as_deref() == Some("any")
    ));
}

#[test]
fn this_repo_orders_by_horizon_and_shows_the_sync_line_in_utc() {
    let mut state = sample();
    assert_eq!(
        titles(&state),
        ["# ac/worklink 2", "issue 2", "issue 1"],
        "horizon now before later, whatever the activity"
    );
    let text = boxed(&mut state).join("\n");
    assert!(text.contains("1 this repo 2"), "{text}");
    assert!(text.contains("2 needs me 1"), "{text}");
    assert!(
        text.contains("ac/worklink synced 2026-10-21 12:00 UTC (stale)"),
        "the sync time says UTC, and an old one is marked: {text}"
    );
    assert!(text.contains("#2 pick a forge for the pilot"), "{text}");
    assert!(text.contains("open"), "the state is drawn: {text}");
    assert!(!text.contains("the task store"), "{text}");
}

#[test]
fn this_repo_is_what_the_server_resolved_not_a_folder_guess() {
    // The pane's remote names the herdr-max repo, whatever the folder is called.
    let mut a = answers();
    a.repo = repo_found("ac/herdr-max", false);
    let mut state = state_with();
    open_with(&mut state, &a);
    let rows = titles(&state);
    assert!(
        rows[0].starts_with("note: ac/herdr-max is not mirrored"),
        "{rows:?}"
    );
    assert!(rows.contains(&"# ac/herdr-max 1".to_owned()), "{rows:?}");
    assert!(
        !rows.iter().any(|row| row.contains("ac/worklink")),
        "{rows:?}"
    );
}

#[test]
fn this_repo_says_why_when_the_pane_has_no_repo_or_it_is_still_being_read() {
    let mut a = answers();
    a.repo = repo_status("no_remote");
    let mut state = state_with();
    open_with(&mut state, &a);
    let rows = titles(&state);
    assert!(
        rows[0].starts_with("note: this pane is not in a git repo with a remote"),
        "{rows:?}"
    );
    assert_eq!(rows.len(), 1, "no other project is shown in its place");
    a.repo = repo_status("resolving");
    refresh_with(&mut state, &a);
    assert_eq!(titles(&state), ["note: reading this pane's git remote…"]);
    // The other views do not wait for it.
    key(&mut state, crossterm::event::KeyCode::Tab);
    assert_eq!(panel(&state).view, WorkView::NeedsMe);
    assert!(titles(&state).iter().any(|row| row.contains("issue 2")));
}

#[test]
fn w_adds_the_issues_closed_in_the_last_seven_days_and_sends_the_cutoff() {
    let mut a = answers();
    let mut closed = issue(
        "ac/worklink",
        7,
        "old rename bug",
        &[],
        "2026-10-20T09:00:00Z",
    );
    closed.state = "closed".into();
    a.closed = vec![closed];
    let mut state = state_with();
    open_with(&mut state, &a);
    assert!(!titles(&state)
        .iter()
        .any(|row| row.contains("old rename bug")));
    let mut outcome = ClientShellInput::default();
    state.route_work_panel_key(
        &crate::input::TerminalKey::new(
            crossterm::event::KeyCode::Char('w'),
            crossterm::event::KeyModifiers::empty(),
        ),
        &mut outcome,
    );
    let sent = requests(&outcome);
    let repo_ask = sent
        .iter()
        .find_map(|(_, m)| match m {
            Method::IssueList(p) if p.pane.is_some() => Some(p.clone()),
            _ => None,
        })
        .expect("the repo list is asked for again");
    assert_eq!(repo_ask.state.as_deref(), Some("all"));
    let cutoff = repo_ask.closed_since.expect("a cutoff");
    assert!(cutoff.ends_with('Z') && cutoff.len() >= 20, "{cutoff}");
    for (id, result) in answer(sent, &a) {
        state.handle_endpoint_result("boot-1", &id, Ok(result));
    }
    state.compose(W, H).unwrap();
    let rows = titles(&state);
    let text = boxed(&mut state).join("\n");
    assert!(rows.contains(&"issue 7".to_owned()), "{rows:?}");
    assert!(text.contains("closed"), "the closed state is drawn: {text}");
    assert!(
        text.contains("also the issues closed in the last 7 days"),
        "{text}"
    );
    // Closed after open within the project.
    let open = rows.iter().position(|row| row == "issue 1").unwrap();
    let shut = rows.iter().position(|row| row == "issue 7").unwrap();
    assert!(open < shut, "{rows:?}");
}

#[test]
fn needs_me_shows_the_reason_on_each_row_in_the_plans_order() {
    let mut a = answers();
    // #1 has a run that is unknown, #2 needs a decision, #3 has an open ask.
    a.tasks = vec![on_issue(
        task(
            "task.m4m.tk-9",
            "stuck run",
            TaskShownInfo::RunUnknown,
            "herdr-wrk",
            "wrk",
        ),
        1,
    )];
    a.issues.push(issue(
        "ac/worklink",
        3,
        "asked about",
        &[],
        "2026-10-20T08:00:00Z",
    ));
    a.issues.push(issue(
        "ac/worklink",
        4,
        "not mine to decide",
        &["needs/design", "not-mine"],
        "2026-10-20T07:00:00Z",
    ));
    a.issues.push(issue(
        "ac/worklink",
        5,
        "needs a repro",
        &["needs/repro"],
        "2026-10-20T06:00:00Z",
    ));
    a.asks = vec![ask(
        "gestore-lab.d1",
        "pick",
        "open",
        "decision",
        "https://gitea.example/ac/worklink/issues/3",
    )];
    let mut state = state_with();
    open_with(&mut state, &a);
    key(&mut state, crossterm::event::KeyCode::Char('2'));
    assert_eq!(
        titles(&state),
        [
            "# run unknown 0",
            "issue 1 [run unknown]",
            "# ask open 0",
            "issue 3 [ask open]",
            "# needs a decision 0",
            "issue 2 [needs a decision]",
        ],
        "by reason; not-mine, needs/repro and the rest do not count"
    );
    let text = boxed(&mut state).join("\n");
    assert!(text.contains("run unknown"), "{text}");
    assert_eq!(panel(&state).counts[1], 3, "the count is the badge");
}

#[test]
fn an_issue_marked_mine_needs_me_and_the_needs_me_chip_is_the_badge() {
    let mut a = answers();
    a.tasks.clear();
    a.asks.clear();
    // #1 has no needs label; the server marked it as ac's. #2 needs a decision.
    a.issues[0].mine = true;
    let mut state = state_with();
    open_with(&mut state, &a);
    // The badge is drawn while another view is open, in yellow and bold.
    assert_eq!(panel(&state).view, WorkView::ThisRepo);
    let (chip, _) = layout(&state)
        .tabs
        .iter()
        .find(|(_, view)| *view == WorkView::NeedsMe)
        .cloned()
        .unwrap();
    let p = state.config.palette.clone();
    let frame = state.compose(W, H).unwrap();
    let cell = &frame.cells[usize::from(chip.y) * usize::from(W) + usize::from(chip.x)];
    assert_eq!(cell.fg, crate::protocol::color_to_u32(p.yellow));
    assert!(boxed(&mut state).join("\n").contains("2 needs me 2"));
    key(&mut state, crossterm::event::KeyCode::Char('2'));
    assert_eq!(
        titles(&state),
        [
            "# assigned to you 0",
            "issue 1 [assigned to you]",
            "# needs a decision 0",
            "issue 2 [needs a decision]",
        ],
        "the assignment comes before a decision, as the plan orders them"
    );
    // With nothing waiting the chip goes back to dim.
    a.issues[0].mine = false;
    a.issues[1].labels.clear();
    refresh_with(&mut state, &a);
    let frame = state.compose(W, H).unwrap();
    let cell = &frame.cells[usize::from(chip.y) * usize::from(W) + usize::from(chip.x)];
    assert_ne!(cell.fg, crate::protocol::color_to_u32(p.yellow));
}

#[test]
fn in_flight_has_issues_with_a_run_or_the_label_then_tasks_without_an_issue() {
    let mut a = answers();
    a.tasks = vec![
        on_issue(
            task(
                "task.m4m.tk-1",
                "run on #1",
                TaskShownInfo::Running,
                "herdr-wrk",
                "wrk",
            ),
            1,
        ),
        task(
            "task.m4m.tk-2",
            "loose run",
            TaskShownInfo::Running,
            "herdr-wrk",
            "wrk",
        ),
        task(
            "task.m4m.tk-3",
            "idle",
            TaskShownInfo::Open,
            "herdr-wrk",
            "wrk",
        ),
    ];
    let mut state = state_with();
    open_with(&mut state, &a);
    key(&mut state, crossterm::event::KeyCode::Char('3'));
    assert_eq!(
        titles(&state),
        [
            "# issues 2",
            "issue 241 [in progress]",
            "issue 1 [run running]",
            "# tasks without an issue 1",
            "task loose run",
        ],
        "newest first"
    );
}

#[test]
fn my_tasks_lists_mine_first_then_the_agents_open_tasks_by_owner() {
    let mut a = answers();
    a.asks
        .push(ask("gestore-lab.d2", "not a task", "open", "decision", ""));
    let mut state = state_with();
    open_with(&mut state, &a);
    key(&mut state, crossterm::event::KeyCode::Char('4'));
    assert_eq!(
        titles(&state),
        [
            "# mine — ac's task rows 1",
            "ask gestore-lab.a1",
            "# herdr-helper · helper 1",
            "task review it",
            "# herdr-wrk · wrk 2",
            "task write the store",
            "task wait for ac",
        ]
    );
    let text = boxed(&mut state).join("\n");
    assert!(text.contains("● write the store"), "{text}");
    assert!(text.contains("◌ wait for ac"), "{text}");
    assert!(text.contains("☐ review it"), "{text}");
    assert!(
        text.contains("run unknown") || text.contains("running"),
        "{text}"
    );
}

#[test]
fn a_task_store_that_is_not_ready_is_said_not_shown_as_no_tasks() {
    for state_name in ["unavailable", "loading", "disabled"] {
        let mut a = answers();
        a.tasks.clear();
        a.asks.clear();
        a.store = state_name;
        let mut state = state_with();
        open_with(&mut state, &a);
        key(&mut state, crossterm::event::KeyCode::Char('4'));
        let rows = titles(&state);
        assert_eq!(
            rows,
            [format!("problem: task store {state_name}")],
            "{rows:?}"
        );
    }
    let mut a = answers();
    a.tasks.clear();
    a.asks.clear();
    let mut state = state_with();
    open_with(&mut state, &a);
    key(&mut state, crossterm::event::KeyCode::Char('4'));
    assert_eq!(titles(&state), ["note: no tasks"]);
}

#[test]
fn the_selection_stays_on_its_issue_when_the_list_changes_under_it() {
    let mut state = sample();
    key(&mut state, crossterm::event::KeyCode::Down);
    let before = selected_key(&state).expect("a selection");
    assert_eq!(before, "issue.gitea.gitea-lab.84.1");
    // Poll: a new issue sorts in before the selected one, and an old one goes.
    let mut a = answers();
    a.issues.push(issue(
        "ac/worklink",
        9,
        "brand new",
        &["horizon/now"],
        "2026-10-21T20:00:00Z",
    ));
    a.issues.remove(1);
    refresh_with(&mut state, &a);
    assert_eq!(selected_key(&state).as_deref(), Some(before.as_str()));
    let panel = panel(&state);
    let row = &panel.rows[panel.selected];
    assert_eq!(
        panel.row_key_for_test(row).as_deref(),
        Some(before.as_str())
    );
}

#[test]
fn a_failed_poll_keeps_the_last_answer_and_marks_it_stale() {
    let mut state = sample();
    let rows = titles(&state);
    let mut outcome = ClientShellInput::default();
    if let Some(ClientShellOverlay::WorkPanel(panel)) = state.overlay.as_mut() {
        panel.in_flight = 0;
    }
    state.request_work_panel(&mut outcome);
    for (id, method) in requests(&outcome) {
        let _ = method;
        state.handle_endpoint_result(
            "boot-1",
            &id,
            Err(ClientShellEndpointError {
                code: None,
                message: "the bus is away".into(),
            }),
        );
    }
    state.compose(W, H).unwrap();
    let now = titles(&state);
    assert!(now[0].starts_with("problem: stale"), "{now:?}");
    assert!(now[0].contains("the bus is away"), "{now:?}");
    assert_eq!(&now[1..], &rows[..], "the rows stay");
    assert!(matches!(panel(&state).issues, Feed::Stale(..)));
    // The next good answer clears it.
    refresh_with(&mut state, &answers());
    assert_eq!(titles(&state), rows);
}

#[test]
fn issues_are_fetched_a_page_at_a_time_until_the_total() {
    let mut state = state_with();
    let outcome = bind(&mut state, KeybindAction::OpenWorkPanel);
    let sent = requests(&outcome);
    let page = |from: u64, count: u64| -> Vec<IssueInfo> {
        (from..from + count)
            .map(|n| {
                issue(
                    "ac/worklink",
                    n,
                    &format!("issue {n}"),
                    &[],
                    "2026-10-21T09:00:00Z",
                )
            })
            .collect()
    };
    let (issue_id, _) = sent
        .iter()
        .find(|(_, m)| matches!(m, Method::IssueList(p) if p.pane.is_none()))
        .cloned()
        .unwrap();
    let (_, first) = state.handle_endpoint_result(
        "boot-1",
        &issue_id,
        Ok(ResponseResult::IssueList {
            issues: page(1, 500),
            total: 620,
            projects: Vec::new(),
            mirror: loaded(),
            repo: None,
        }),
    );
    let next = first
        .iter()
        .find_map(|action| match action {
            ClientShellAction::Endpoint { request, .. } => Some(request.clone()),
            _ => None,
        })
        .expect("the second page is asked for");
    assert!(
        matches!(&next.method, Method::IssueList(p) if p.offset == Some(500)),
        "{:?}",
        next.method
    );
    assert!(
        matches!(panel(&state).issues, Feed::Waiting),
        "nothing is shown from half the pages"
    );
    state.handle_endpoint_result(
        "boot-1",
        &next.id,
        Ok(ResponseResult::IssueList {
            issues: page(501, 120),
            total: 620,
            projects: Vec::new(),
            mirror: loaded(),
            repo: None,
        }),
    );
    assert_eq!(panel(&state).issues.data().unwrap().issues.len(), 620);
}

#[test]
fn tab_and_the_view_chips_switch_views_and_the_action_letters_are_left_alone() {
    let mut state = sample();
    key(&mut state, crossterm::event::KeyCode::Tab);
    assert_eq!(panel(&state).view, WorkView::NeedsMe);
    key(&mut state, crossterm::event::KeyCode::BackTab);
    key(&mut state, crossterm::event::KeyCode::BackTab);
    assert_eq!(panel(&state).view, WorkView::MyTasks, "wraps around");
    // h and l are the reserved action keys, not navigation.
    key(&mut state, crossterm::event::KeyCode::Char('h'));
    key(&mut state, crossterm::event::KeyCode::Char('l'));
    assert_eq!(panel(&state).view, WorkView::MyTasks);

    state.compose(W, H).unwrap();
    let (rect, view) = layout(&state).tabs[2];
    assert_eq!(view, WorkView::InFlight);
    let mut outcome = ClientShellInput::default();
    state.handle_mouse(mouse_down(rect.x + 1, rect.y), &mut outcome);
    assert_eq!(panel(&state).view, WorkView::InFlight);
}

#[test]
fn the_selection_skips_headings_and_notes() {
    let mut state = sample();
    key(&mut state, crossterm::event::KeyCode::Char('4'));
    let on_row = |state: &ClientShellState| panel(state).rows[panel(state).selected].clone();
    assert!(
        matches!(on_row(&state), WorkRow::Ask(0)),
        "mine comes first"
    );
    for _ in 0..10 {
        key(&mut state, crossterm::event::KeyCode::Down);
        assert!(!matches!(
            on_row(&state),
            WorkRow::Heading { .. } | WorkRow::Note(_) | WorkRow::Problem(_)
        ));
    }
    assert!(
        matches!(on_row(&state), WorkRow::Task(_)),
        "the end is a task"
    );
    for _ in 0..3 {
        key(&mut state, crossterm::event::KeyCode::Up);
    }
    assert!(!matches!(on_row(&state), WorkRow::Heading { .. }));
}

#[test]
fn a_server_without_the_work_methods_is_said_to_lack_them() {
    let mut state = state_with();
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
    let mut state = state_with();
    let outcome = bind(&mut state, KeybindAction::OpenWorkPanel);
    for (id, method) in requests(&outcome) {
        if let Method::IssueList(_) = method {
            state.handle_endpoint_result(
                "boot-1",
                &id,
                Ok(ResponseResult::IssueList {
                    issues: Vec::new(),
                    total: 0,
                    projects: Vec::new(),
                    mirror: MirrorInfo {
                        loaded: false,
                        detail: Some("not available with this backend".into()),
                    },
                    repo: Some(repo_found("ac/worklink", true)),
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
    assert_eq!(requests(&outcome).len(), 4);
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
    assert_eq!(requests(&due).len(), 4);
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
fn a_screen_that_is_too_small_gets_one_yellow_line_not_a_panic() {
    let mut state = sample();
    for (w, h) in [(10, 5), (30, 8), (59, 30)] {
        let frame = state.compose(w, h).expect("frame");
        let row: String = frame.cells[..usize::from(w)]
            .iter()
            .map(|cell| cell.symbol.as_str())
            .collect();
        let line = "widen the popup — work needs 60×12";
        let shown: String = line.chars().take(usize::from(w)).collect();
        assert!(row.starts_with(&shown), "{row:?}");
        key(&mut state, crossterm::event::KeyCode::Down);
        key(&mut state, crossterm::event::KeyCode::Char('4'));
    }
}

#[test]
fn the_footer_is_a_key_bar_without_buttons_and_failures_are_red() {
    let mut state = sample();
    let text = boxed(&mut state).join("\n");
    assert!(text.contains("1-4 view"), "{text}");
    assert!(text.contains("esc close"), "{text}");
    // A sync that fails is drawn in red, not dim.
    let mut a = answers();
    a.projects[0].status = Some("error".into());
    a.projects[0].error_class = Some("auth".into());
    let mut state = state_with();
    open_with(&mut state, &a);
    let p = state.config.palette.clone();
    let layout = layout(&state);
    let frame = state.compose(W, H).unwrap();
    let cell = &frame.cells
        [usize::from(layout.header.y + 2) * usize::from(W) + usize::from(layout.header.x + 1)];
    assert_eq!(cell.fg, crate::protocol::color_to_u32(p.red));
    assert!(boxed(&mut state).join("\n").contains("sync failing (auth)"));
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
    let chords = groups
        .iter()
        .find(|(name, _)| *name == "work panel")
        .expect("the panel's own chords have a group");
    assert!(chords
        .1
        .iter()
        .any(|(key, label)| key.contains("1-4") && label == "switch view"));
    assert!(chords
        .1
        .iter()
        .any(|(key, label)| key == "esc / q" && label == "close panel"));
}
