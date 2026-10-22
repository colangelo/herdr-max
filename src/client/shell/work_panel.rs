//! The work panel (fork issue 241, task 2.14): a read-only popup over what the
//! server knows about work. It has four views, each a list built from three
//! server answers and nothing else; the client never holds bus credentials.
//!
//! What picks a view's rows is the plan's (v5 § 4), one function per rule
//! (`is_this_repo`, `needs_me_reason`, `in_flight_reason`, `in_flight_task`,
//! `my_task`), so a change to a rule is a change to one function:
//!
//! - *This repo*: open issues of the project the focused pane's repo is, by
//!   horizon (now, next, later, icebox, none) then latest activity. The plan
//!   says the pane's git remote; the client's snapshot has none yet, so the
//!   folder name stands in, and with no match the view says so.
//! - *Needs me*: open issues with the reason on the row: a run on one of their
//!   tasks is unknown, an open ask points at them, or they need a decision or
//!   a design and are not marked not-mine; by reason, then longest waiting.
//! - *In flight*: open issues with an unfinished run on one of their tasks or
//!   labelled `status/in-progress`, newest first, then tasks with such a run
//!   and no issue.
//! - *My tasks*: ac's own task rows from `ask.list`, then every open task by
//!   owner and pane.
//!
//! Left for the data that does not reach the client yet: the pane's git
//! remote, ac's forge account (`[tasks.forge_accounts]`), the toggle for issues
//! closed in the last seven days, and ac's intents (Phase 3).
//!
//! The panel keeps the last answer it got and marks it stale when a later ask
//! fails, rather than blanking the view. Everything it shows is computed when
//! an answer arrives (`rebuild`), never in the render path.

use std::time::{Duration, Instant, SystemTime};

use super::move_picker::{list_chord, render_rule, render_scrollbar, reveal_scroll, Chord};
use super::render::put_text;
use super::todo_panel::{contrast, render_panel_shell};
use super::*;
use crate::api::schema::{
    AskInfo, AskListParams, IssueInfo, IssueListParams, IssueProjectInfo, Method, MirrorInfo,
    ResponseResult, TaskInfo, TaskListParams, TaskShownInfo, TaskStoreInfo,
};
use crate::ui::text::{display_width as width_of, truncate_end};
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::style::Color;

const MIN_WIDTH: u16 = 60;
const MAX_WIDTH: u16 = 120;
const MAX_HEIGHT: u16 = 30;
/// The least the panel can be drawn in.
const MIN_SCREEN: (u16, u16) = (60, 12);
/// Title, the view chips, the sync line, a rule.
const HEADER_ROWS: u16 = 4;
const FOOTER_ROWS: u16 = 2;
const DETAIL_ROWS: u16 = 2;
/// How long an open panel waits before it asks the server again.
const POLL: Duration = Duration::from_secs(5);
/// Issues are fetched this many at a time, up to `MAX_PAGES` pages.
const PAGE: u32 = 500;
const MAX_PAGES: usize = 8;
/// Which issues the panel fetches. Closed ones are not shown in any view; this
/// is the one place to change that.
const ISSUE_STATE: &str = "open";
/// A project whose last good sync is older than this is marked stale.
const STALE_SYNC_SECS: i64 = 3600;
const LACKS_METHODS: &str = "this server does not have the work methods; update and restart it";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WorkView {
    ThisRepo,
    NeedsMe,
    InFlight,
    MyTasks,
}

impl WorkView {
    pub(super) const ALL: [WorkView; 4] = [
        WorkView::ThisRepo,
        WorkView::NeedsMe,
        WorkView::InFlight,
        WorkView::MyTasks,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::ThisRepo => "this repo",
            Self::NeedsMe => "needs me",
            Self::InFlight => "in flight",
            Self::MyTasks => "my tasks",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|view| *view == self).unwrap_or(0)
    }

    fn step(self, by: isize) -> Self {
        let len = Self::ALL.len() as isize;
        Self::ALL[(self.index() as isize + by).rem_euclid(len) as usize]
    }
}

// -- the rules that pick rows: one function each --------------------------------------
//
// From the plan (v5 § 4). What needs data the client does not have yet is
// left as a stated hook: the pane's git remote, ac's forge account, the
// closed-in-7-days toggle, ac's intents (Phase 3).

/// Whether `issue` belongs to the repo the focused pane is in. The plan says
/// the pane's git remote; the snapshot has none, so the folder name stands in.
fn is_this_repo(folders: &[String], issue: &IssueInfo) -> bool {
    let name = issue
        .issue_ref
        .project_path
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_lowercase();
    !name.is_empty() && folders.contains(&name)
}

/// `now`, `next`, `later`, `icebox`, then none.
fn horizon_rank(issue: &IssueInfo) -> u8 {
    for (rank, name) in ["now", "next", "later", "icebox"].into_iter().enumerate() {
        let label = format!("horizon/{name}");
        if issue.labels.contains(&label) {
            return rank as u8;
        }
    }
    4
}

fn not_mine(issue: &IssueInfo) -> bool {
    issue.labels.iter().any(|label| label == "not-mine")
}

fn in_progress(issue: &IssueInfo) -> bool {
    issue
        .labels
        .iter()
        .any(|label| label == "status/in-progress")
}

/// A run that has not finished.
fn run_unfinished(task: &TaskInfo) -> bool {
    matches!(
        task.shown,
        TaskShownInfo::Running | TaskShownInfo::RunUnknown
    )
}

fn issue_key_of(issue: &IssueInfo) -> (&str, u64, u64) {
    (
        issue.issue_ref.forge_id.as_str(),
        issue.issue_ref.project_id,
        issue.issue_ref.number,
    )
}

/// Why `issue` needs ac, best reason first (`None`: it does not). Reasons
/// (2) a run on one of its tasks is unknown, (3) an open ask points at it,
/// (5) it needs a decision or a design and is not marked not mine. Reason (1),
/// a stale intent, is for Phase 3, and (4), assigned to ac's forge account,
/// waits for the `[tasks.forge_accounts]` setting to reach the client.
fn needs_me_reason(
    issue: &IssueInfo,
    tasks_on: &[&TaskInfo],
    asked: &std::collections::HashSet<&str>,
) -> Option<(u8, &'static str)> {
    if tasks_on
        .iter()
        .any(|task| task.shown == TaskShownInfo::RunUnknown)
    {
        return Some((0, "run unknown"));
    }
    if issue
        .web_url
        .as_deref()
        .is_some_and(|url| asked.contains(url))
    {
        return Some((1, "ask open"));
    }
    if !not_mine(issue)
        && issue
            .labels
            .iter()
            .any(|label| label == "needs/decision" || label == "needs/design")
    {
        return Some((2, "needs a decision"));
    }
    None
}

/// Why `issue` is in flight (`None`: it is not).
fn in_flight_reason(issue: &IssueInfo, tasks_on: &[&TaskInfo]) -> Option<&'static str> {
    if tasks_on.iter().any(|task| run_unfinished(task)) {
        return Some(
            if tasks_on
                .iter()
                .any(|t| t.shown == TaskShownInfo::RunUnknown)
            {
                "run unknown"
            } else {
                "run running"
            },
        );
    }
    in_progress(issue).then_some("in progress")
}

/// Whether a task with no issue is in flight.
fn in_flight_task(task: &TaskInfo) -> bool {
    task.refs.is_empty() && run_unfinished(task)
}

fn my_task(_task: &TaskInfo) -> bool {
    true
}

// -- the answers ----------------------------------------------------------------------

/// One source of rows. A later ask that fails keeps the last answer, marked
/// stale with the reason.
#[derive(Debug, Clone, Default)]
pub(super) enum Feed<T> {
    #[default]
    Waiting,
    Ready(T),
    Stale(T, String),
    Failed(String),
}

impl<T> Feed<T> {
    pub(super) fn data(&self) -> Option<&T> {
        match self {
            Self::Ready(data) | Self::Stale(data, _) => Some(data),
            _ => None,
        }
    }

    /// Why the newest ask did not work, when it did not.
    pub(super) fn problem(&self) -> Option<&str> {
        match self {
            Self::Stale(_, why) | Self::Failed(why) => Some(why),
            _ => None,
        }
    }

    fn fail(&mut self, why: String) {
        *self = match std::mem::take(self) {
            Self::Ready(data) | Self::Stale(data, _) => Self::Stale(data, why),
            _ => Self::Failed(why),
        };
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct IssuesData {
    pub(super) issues: Vec<IssueInfo>,
    /// How many the server has; more than `issues` when the pages ran out.
    pub(super) total: u64,
    pub(super) projects: Vec<IssueProjectInfo>,
    pub(super) mirror: Option<MirrorInfo>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct AsksData {
    pub(super) asks: Vec<AskInfo>,
    pub(super) total: u64,
    pub(super) mirror: Option<MirrorInfo>,
}

#[derive(Debug, Clone)]
pub(super) struct TasksData {
    pub(super) tasks: Vec<TaskInfo>,
    pub(super) store: TaskStoreInfo,
}

/// A line of the list. Headings, notes and problems cannot be selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WorkRow {
    Heading {
        text: String,
        count: usize,
    },
    Note(String),
    /// Something that did not work, drawn in red.
    Problem(String),
    Issue(usize),
    /// An issue with the reason it is in the view (needs me, in flight).
    IssueWhy(usize, &'static str),
    Task(usize),
    Ask(usize),
}

impl WorkRow {
    fn selectable(&self) -> bool {
        matches!(
            self,
            Self::Issue(_) | Self::IssueWhy(..) | Self::Task(_) | Self::Ask(_)
        )
    }
}

#[derive(Debug)]
pub(super) struct ClientWorkPanelOverlay {
    pub(super) boot_id: String,
    pub(super) view: WorkView,
    /// The two folder names closest to the focused pane's directory,
    /// lowercase: its own and its parent's.
    pub(super) folders: Vec<String>,
    pub(super) issues: Feed<IssuesData>,
    pub(super) tasks: Feed<TasksData>,
    pub(super) asks: Feed<AsksData>,
    /// Issue pages collected so far, until the last one arrives.
    pub(super) issue_stage: Option<IssuesData>,
    pub(super) rows: Vec<WorkRow>,
    /// How many selectable rows each view has, in `WorkView::ALL` order.
    pub(super) counts: [usize; 4],
    /// When each project shown last synced, and whether that is a problem.
    pub(super) sync: String,
    pub(super) sync_alarm: bool,
    pub(super) selected: usize,
    /// What the selection is on, which survives the rows being built again.
    pub(super) selected_key: Option<String>,
    pub(super) scroll: usize,
    pub(super) in_flight: u8,
    pub(super) last_fetch: Instant,
}

impl ClientWorkPanelOverlay {
    pub(super) fn new(boot_id: String, folders: Vec<String>) -> Self {
        Self {
            boot_id,
            view: WorkView::ThisRepo,
            folders,
            issues: Feed::Waiting,
            tasks: Feed::Waiting,
            asks: Feed::Waiting,
            issue_stage: None,
            rows: Vec::new(),
            counts: [0; 4],
            sync: String::new(),
            sync_alarm: false,
            selected: 0,
            selected_key: None,
            scroll: 0,
            in_flight: 0,
            last_fetch: Instant::now(),
        }
    }

    /// A row's identity across answers: the key of what it shows.
    #[cfg(test)]
    pub(super) fn row_key_for_test(&self, row: &WorkRow) -> Option<String> {
        self.row_key(row)
    }

    fn row_key(&self, row: &WorkRow) -> Option<String> {
        match row {
            WorkRow::Issue(i) | WorkRow::IssueWhy(i, _) => {
                Some(self.issues.data()?.issues.get(*i)?.key.clone())
            }
            WorkRow::Task(i) => Some(self.tasks.data()?.tasks.get(*i)?.key.clone()),
            WorkRow::Ask(i) => Some(self.asks.data()?.asks.get(*i)?.key.clone()),
            _ => None,
        }
    }

    fn remember_selection(&mut self) {
        self.selected_key = self
            .rows
            .get(self.selected)
            .and_then(|row| self.row_key(row));
    }

    /// Build every view's rows again from the last answers, keep the
    /// selection on the same issue or task (by key, not by place in a list
    /// that has just been fetched again), and compute the sync line.
    pub(super) fn rebuild(&mut self) {
        for view in WorkView::ALL {
            self.counts[view.index()] = self
                .rows_for(view)
                .iter()
                .filter(|row| row.selectable())
                .count();
        }
        self.rows = self.rows_for(self.view);
        let found = self.selected_key.as_ref().and_then(|key| {
            self.rows
                .iter()
                .position(|row| self.row_key(row).as_ref() == Some(key))
        });
        self.selected = found
            .unwrap_or(self.selected)
            .min(self.rows.len().saturating_sub(1));
        self.snap_selection();
        if found.is_none() {
            self.remember_selection();
        }
        let (line, alarm) = self.sync_summary(SystemTime::now());
        self.sync = line;
        self.sync_alarm = alarm;
    }

    /// Land on a row that can be selected: the next one, else the one before.
    fn snap_selection(&mut self) {
        let from = self.selected.min(self.rows.len().saturating_sub(1));
        let found = (from..self.rows.len())
            .chain((0..from).rev())
            .find(|index| self.rows[*index].selectable());
        self.selected = found.unwrap_or(0);
    }

    fn move_to(&mut self, chord: Chord, visible: usize) {
        let target = chord.target(self.selected, visible, self.rows.len());
        let forward = target >= self.selected;
        let found = if forward {
            (target..self.rows.len())
                .chain((0..target).rev())
                .find(|index| self.rows[*index].selectable())
        } else {
            (0..=target)
                .rev()
                .chain(target + 1..self.rows.len())
                .find(|index| self.rows[*index].selectable())
        };
        if let Some(index) = found {
            self.selected = index;
        }
        self.remember_selection();
        self.scroll = reveal_scroll(self.scroll, self.selected, visible, self.rows.len());
    }

    pub(super) fn select(&mut self, index: usize) {
        self.selected = index;
        self.remember_selection();
    }

    pub(super) fn start(&self, visible: usize) -> usize {
        reveal_scroll(self.scroll, self.selected, visible, self.rows.len())
    }

    /// The open issues the panel has, by index into the answer.
    fn issues_shown(&self) -> Vec<(usize, &IssueInfo)> {
        self.issues
            .data()
            .map(|data| {
                data.issues
                    .iter()
                    .enumerate()
                    .filter(|(_, issue)| issue.state == "open")
                    .collect()
            })
            .unwrap_or_default()
    }

    /// What to say before the rows of a view that reads issues. Returns true
    /// when there is nothing to show under it.
    fn issue_notes(&self, rows: &mut Vec<WorkRow>) -> bool {
        match &self.issues {
            Feed::Waiting => {
                rows.push(WorkRow::Note("loading…".to_owned()));
                return true;
            }
            Feed::Failed(why) => {
                rows.push(WorkRow::Problem(why.clone()));
                return true;
            }
            Feed::Stale(_, why) => rows.push(WorkRow::Problem(format!(
                "stale — the last ask failed ({why}); showing the last answer"
            ))),
            Feed::Ready(_) => {}
        }
        if let Some(data) = self.issues.data() {
            if let Some(MirrorInfo {
                loaded: false,
                detail,
            }) = &data.mirror
            {
                rows.push(WorkRow::Note(format!(
                    "issues not read yet: {}",
                    detail.as_deref().unwrap_or("waiting for the bus")
                )));
            }
            if data.total > data.issues.len() as u64 {
                rows.push(WorkRow::Note(format!(
                    "showing {} of {} open issues",
                    data.issues.len(),
                    data.total
                )));
            }
        }
        false
    }

    /// What to say before the rows of a view that reads tasks.
    fn task_notes(&self, rows: &mut Vec<WorkRow>) -> bool {
        match &self.tasks {
            Feed::Waiting => {
                rows.push(WorkRow::Note("loading…".to_owned()));
                return true;
            }
            Feed::Failed(why) => {
                rows.push(WorkRow::Problem(why.clone()));
                return true;
            }
            Feed::Stale(_, why) => rows.push(WorkRow::Problem(format!(
                "stale — the last ask failed ({why}); showing the last answer"
            ))),
            Feed::Ready(_) => {}
        }
        if let Some(data) = self.tasks.data() {
            let store = &data.store;
            if store.state != "ready" {
                rows.push(WorkRow::Problem(format!(
                    "task store {}{}",
                    store.state,
                    store
                        .detail
                        .as_deref()
                        .map_or(String::new(), |detail| format!(": {detail}"))
                )));
            }
            if store.pending > 0 || store.conflicts > 0 {
                rows.push(WorkRow::Note(format!(
                    "{} writes waiting for the store, {} held for a decision",
                    store.pending, store.conflicts
                )));
            }
        }
        false
    }

    pub(super) fn rows_for(&self, view: WorkView) -> Vec<WorkRow> {
        let mut rows = Vec::new();
        match view {
            WorkView::ThisRepo => {
                if self.issue_notes(&mut rows) {
                    return rows;
                }
                let mut issues = self.issues_shown();
                let here: Vec<_> = issues
                    .iter()
                    .copied()
                    .filter(|(_, issue)| is_this_repo(&self.folders, issue))
                    .collect();
                if here.is_empty() {
                    if !issues.is_empty() {
                        rows.push(WorkRow::Note(
                            "no project matches this pane's folder (this pane's repo is not \
                             mirrored, or its git remote is not known here); every project is shown"
                                .to_owned(),
                        ));
                    }
                } else {
                    issues = here;
                }
                push_issue_groups(&mut rows, issues);
                if !rows.iter().any(WorkRow::selectable) {
                    rows.push(WorkRow::Note("no open issues".to_owned()));
                }
            }
            WorkView::NeedsMe => {
                let mut tasks_missing = false;
                if self.issue_notes(&mut rows) {
                    return rows;
                }
                // The tasks and asks only add reasons; without them the
                // others still show.
                if !matches!(self.tasks, Feed::Ready(_) | Feed::Stale(..)) {
                    tasks_missing = true;
                }
                let by_issue = self.tasks_by_issue();
                let asked = self.asked_urls();
                let mut found: Vec<(u8, &str, usize, &'static str)> = self
                    .issues_shown()
                    .into_iter()
                    .filter_map(|(index, issue)| {
                        let on = by_issue.get(&issue_key_of(issue)).map_or(&[][..], |v| v);
                        let (rank, why) = needs_me_reason(issue, on, &asked)?;
                        Some((rank, issue.updated_at.as_deref().unwrap_or(""), index, why))
                    })
                    .collect();
                // By reason, then the longest waiting (the oldest touch) first.
                found.sort();
                let mut current = None;
                for (rank, _, index, why) in found {
                    if current != Some(rank) {
                        current = Some(rank);
                        rows.push(WorkRow::Heading {
                            text: why.to_owned(),
                            count: 0,
                        });
                    }
                    rows.push(WorkRow::IssueWhy(index, why));
                }
                if tasks_missing {
                    rows.push(WorkRow::Note(
                        "run reasons wait for the task list".to_owned(),
                    ));
                }
                if !rows.iter().any(WorkRow::selectable) {
                    rows.push(WorkRow::Note("nothing is waiting for ac".to_owned()));
                }
            }
            WorkView::InFlight => {
                // Either half can show without the other.
                let issues_missing = self.issue_notes(&mut rows);
                let tasks_missing = self.task_notes(&mut rows);
                if !issues_missing {
                    let by_issue = self.tasks_by_issue();
                    let mut found: Vec<(std::cmp::Reverse<&str>, usize, &'static str)> = self
                        .issues_shown()
                        .into_iter()
                        .filter_map(|(index, issue)| {
                            let on = by_issue.get(&issue_key_of(issue)).map_or(&[][..], |v| v);
                            let why = in_flight_reason(issue, on)?;
                            Some((
                                std::cmp::Reverse(issue.updated_at.as_deref().unwrap_or("")),
                                index,
                                why,
                            ))
                        })
                        .collect();
                    found.sort();
                    if !found.is_empty() {
                        rows.push(WorkRow::Heading {
                            text: "issues".to_owned(),
                            count: found.len(),
                        });
                    }
                    rows.extend(
                        found
                            .into_iter()
                            .map(|(_, index, why)| WorkRow::IssueWhy(index, why)),
                    );
                }
                if !tasks_missing {
                    if let Some(data) = self.tasks.data() {
                        let mut running: Vec<_> = data
                            .tasks
                            .iter()
                            .enumerate()
                            .filter(|(_, task)| in_flight_task(task))
                            .collect();
                        running.sort_by(|a, b| b.1.updated_at.cmp(&a.1.updated_at));
                        if !running.is_empty() {
                            rows.push(WorkRow::Heading {
                                text: "tasks without an issue".to_owned(),
                                count: running.len(),
                            });
                            rows.extend(running.into_iter().map(|(i, _)| WorkRow::Task(i)));
                        }
                    }
                }
                if !rows.iter().any(WorkRow::selectable) && !issues_missing && !tasks_missing {
                    rows.push(WorkRow::Note("nothing is in flight".to_owned()));
                }
            }
            WorkView::MyTasks => {
                // Mine first: ac's own task rows, then every open task of the
                // agents by owner.
                match &self.asks {
                    Feed::Failed(why) => rows.push(WorkRow::Problem(format!("asks: {why}"))),
                    Feed::Stale(_, why) => {
                        rows.push(WorkRow::Problem(format!("asks stale: {why}")));
                        self.push_asks(&mut rows);
                    }
                    Feed::Ready(_) => self.push_asks(&mut rows),
                    Feed::Waiting => {}
                }
                let tasks_missing = self.task_notes(&mut rows);
                if !tasks_missing {
                    if let Some(data) = self.tasks.data() {
                        let mut owners: Vec<(String, Vec<usize>)> = Vec::new();
                        for (index, task) in data.tasks.iter().enumerate() {
                            if !my_task(task) {
                                continue;
                            }
                            let name = task_group(task);
                            match owners.iter_mut().find(|(owner, _)| *owner == name) {
                                Some((_, list)) => list.push(index),
                                None => owners.push((name, vec![index])),
                            }
                        }
                        owners.sort_by(|a, b| a.0.cmp(&b.0));
                        for (name, list) in owners {
                            rows.push(WorkRow::Heading {
                                text: name,
                                count: list.len(),
                            });
                            rows.extend(list.into_iter().map(WorkRow::Task));
                        }
                    }
                }
                // "no tasks" only when the store really is empty and ready.
                let store_ready = self
                    .tasks
                    .data()
                    .is_some_and(|data| data.store.state == "ready");
                if !rows.iter().any(WorkRow::selectable) && store_ready {
                    rows.push(WorkRow::Note("no tasks".to_owned()));
                }
            }
        }
        rows
    }

    /// The open tasks by the issue they refer to.
    fn tasks_by_issue(&self) -> std::collections::HashMap<(&str, u64, u64), Vec<&TaskInfo>> {
        let mut map: std::collections::HashMap<(&str, u64, u64), Vec<&TaskInfo>> =
            std::collections::HashMap::new();
        if let Some(data) = self.tasks.data() {
            for task in &data.tasks {
                for r in &task.refs {
                    map.entry((r.forge_id.as_str(), r.project_id, r.number))
                        .or_default()
                        .push(task);
                }
            }
        }
        map
    }

    /// Where open (or answered) ask rows point: their `where`.
    fn asked_urls(&self) -> std::collections::HashSet<&str> {
        self.asks
            .data()
            .map(|data| {
                data.asks
                    .iter()
                    .filter(|ask| matches!(ask.status.as_str(), "open" | "answered"))
                    .filter_map(|ask| ask.where_.as_deref())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn push_asks(&self, rows: &mut Vec<WorkRow>) {
        let Some(data) = self.asks.data() else {
            return;
        };
        if data.mirror.as_ref().is_some_and(|m| !m.loaded) {
            rows.push(WorkRow::Note(format!(
                "asks not read yet: {}",
                data.mirror
                    .as_ref()
                    .and_then(|m| m.detail.as_deref())
                    .unwrap_or("waiting for the bus")
            )));
        }
        // Only ac's own task rows; the other kinds only add reasons elsewhere.
        let mine: Vec<usize> = data
            .asks
            .iter()
            .enumerate()
            .filter(|(_, ask)| ask.kind.as_deref() == Some("task"))
            .filter(|(_, ask)| matches!(ask.status.as_str(), "open" | "answered"))
            .map(|(i, _)| i)
            .collect();
        if mine.is_empty() {
            return;
        }
        rows.push(WorkRow::Heading {
            text: "mine — ac's task rows".to_owned(),
            count: mine.len(),
        });
        rows.extend(mine.into_iter().map(WorkRow::Ask));
        if data.total > data.asks.len() as u64 {
            rows.push(WorkRow::Note(format!(
                "showing {} of {} task rows",
                data.asks.len(),
                data.total
            )));
        }
    }

    /// When each project of the rows shown last synced, and whether that is a
    /// problem (a sync that fails, or one that is old). With no issue row to
    /// go by, every project the server knows.
    fn sync_summary(&self, now: SystemTime) -> (String, bool) {
        let Some(data) = self.issues.data() else {
            return (String::new(), self.issues.problem().is_some());
        };
        let mut seen: Vec<(&str, u64)> = Vec::new();
        for row in &self.rows {
            if let WorkRow::Issue(index) | WorkRow::IssueWhy(index, _) = row {
                if let Some(issue) = data.issues.get(*index) {
                    let id = (
                        issue.issue_ref.forge_id.as_str(),
                        issue.issue_ref.project_id,
                    );
                    if !seen.contains(&id) {
                        seen.push(id);
                    }
                }
            }
        }
        if seen.is_empty() {
            seen = data
                .projects
                .iter()
                .map(|p| (p.forge_id.as_str(), p.project_id))
                .collect();
        }
        if seen.is_empty() {
            return ("no project synced yet".to_owned(), false);
        }
        let mut alarm = self.issues.problem().is_some();
        let parts: Vec<String> = seen
            .into_iter()
            .map(|(forge_id, project_id)| {
                let project = data
                    .projects
                    .iter()
                    .find(|p| p.forge_id == forge_id && p.project_id == project_id);
                let name = project
                    .and_then(|p| p.project_path.clone())
                    .unwrap_or_else(|| project_id.to_string());
                match project {
                    Some(p) if p.synced && p.status.as_deref() == Some("error") => {
                        alarm = true;
                        let when = p.last_ok.as_deref().map_or("never".to_owned(), sync_time);
                        format!(
                            "{name} sync failing ({}), last ok {when}",
                            p.error_class.as_deref().unwrap_or("error")
                        )
                    }
                    Some(p) if p.synced => {
                        let stale = p
                            .last_ok
                            .as_deref()
                            .is_some_and(|stamp| older_than(stamp, now, STALE_SYNC_SECS));
                        alarm |= stale;
                        format!(
                            "{name} synced {}{}",
                            p.last_ok.as_deref().map_or("never".to_owned(), sync_time),
                            if stale { " (stale)" } else { "" }
                        )
                    }
                    _ => format!("{name} not synced"),
                }
            })
            .collect();
        (parts.join("  ·  "), alarm)
    }
}

fn parse_stamp(stamp: &str) -> Option<time::OffsetDateTime> {
    time::OffsetDateTime::parse(stamp, &time::format_description::well_known::Rfc3339).ok()
}

/// `2026-10-21T12:00:00Z` as `2026-10-21 12:00 UTC`.
fn sync_time(stamp: &str) -> String {
    match parse_stamp(stamp) {
        Some(at) => {
            let at = at.to_offset(time::UtcOffset::UTC);
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02} UTC",
                at.year(),
                u8::from(at.month()),
                at.day(),
                at.hour(),
                at.minute()
            )
        }
        None => stamp.to_owned(),
    }
}

fn older_than(stamp: &str, now: SystemTime, secs: i64) -> bool {
    let Some(at) = parse_stamp(stamp) else {
        return false;
    };
    let now = time::OffsetDateTime::from(now);
    (now - at).whole_seconds() > secs
}

/// The owner and pane a task is grouped under.
fn task_group(task: &TaskInfo) -> String {
    let owner = task.owner.as_deref().unwrap_or("nobody");
    match task.pane.as_ref() {
        Some(pane) if !pane.label.is_empty() => format!("{owner} · {}", pane.label),
        _ => owner.to_owned(),
    }
}

fn shown_label(shown: TaskShownInfo) -> &'static str {
    match shown {
        TaskShownInfo::Open => "open",
        TaskShownInfo::Running => "running",
        TaskShownInfo::Waiting => "waiting",
        TaskShownInfo::RunUnknown => "run unknown",
        TaskShownInfo::Done => "done",
        TaskShownInfo::Dropped => "dropped",
        TaskShownInfo::Unknown => "unknown",
    }
}

/// Issues grouped by project, the most recently touched first.
fn push_issue_groups(rows: &mut Vec<WorkRow>, mut issues: Vec<(usize, &IssueInfo)>) {
    // Horizon first (now, next, later, icebox, none), then the latest activity.
    issues.sort_by(|a, b| {
        (
            &a.1.issue_ref.project_path,
            horizon_rank(a.1),
            std::cmp::Reverse(&a.1.updated_at),
        )
            .cmp(&(
                &b.1.issue_ref.project_path,
                horizon_rank(b.1),
                std::cmp::Reverse(&b.1.updated_at),
            ))
    });
    let mut current: Option<&str> = None;
    for (index, issue) in issues.iter() {
        let path = issue.issue_ref.project_path.as_str();
        if current != Some(path) {
            current = Some(path);
            let count = issues
                .iter()
                .filter(|(_, other)| other.issue_ref.project_path == path)
                .count();
            rows.push(WorkRow::Heading {
                text: path.to_owned(),
                count,
            });
        }
        rows.push(WorkRow::Issue(*index));
    }
}

// -- layout -----------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(super) struct WorkPanelLayout {
    pub(super) outer: Rect,
    pub(super) header: Rect,
    /// The view chips, in `WorkView::ALL` order.
    pub(super) tabs: Vec<(Rect, WorkView)>,
    pub(super) list: Rect,
    pub(super) detail: Option<Rect>,
    pub(super) footer_row: Option<Rect>,
    /// The first row shown.
    pub(super) start: usize,
}

impl WorkPanelLayout {
    /// The row under a cell, if the cell is on a row of the list.
    pub(super) fn row_at(&self, column: u16, row: u16, len: usize) -> Option<usize> {
        if column < self.list.x || column >= self.list.right() {
            return None;
        }
        if row < self.list.y || row >= self.list.bottom() {
            return None;
        }
        let index = self.start + usize::from(row - self.list.y);
        (index < len).then_some(index)
    }
}

fn chip_text(view: WorkView, count: usize) -> String {
    format!(" {} {} {} ", view.index() + 1, view.label(), count)
}

pub(super) fn work_panel_layout(
    panel: &ClientWorkPanelOverlay,
    screen: Rect,
) -> Option<WorkPanelLayout> {
    if screen.width < MIN_SCREEN.0 || screen.height < MIN_SCREEN.1 {
        return None;
    }
    let width = MAX_WIDTH.min(screen.width).max(MIN_WIDTH);
    let height = MAX_HEIGHT.min(screen.height).max(MIN_SCREEN.1);
    let outer = Rect::new(
        screen.x + (screen.width - width) / 2,
        screen.y + (screen.height - height) / 2,
        width,
        height,
    );
    let inner = Rect::new(outer.x + 1, outer.y + 1, outer.width - 2, outer.height - 2);
    let footer_row = (inner.height > FOOTER_ROWS + HEADER_ROWS)
        .then(|| Rect::new(inner.x, inner.bottom() - 1, inner.width, 1));
    let body_bottom = if footer_row.is_some() {
        inner.bottom() - FOOTER_ROWS
    } else {
        inner.bottom()
    };
    let header = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        HEADER_ROWS.min(body_bottom - inner.y),
    );
    let mut list = Rect::new(
        inner.x,
        header.bottom(),
        inner.width,
        body_bottom.saturating_sub(header.bottom()),
    );
    let detail = (list.height >= DETAIL_ROWS + 3).then(|| {
        list.height -= DETAIL_ROWS;
        Rect::new(list.x, list.bottom(), list.width, DETAIL_ROWS)
    });
    let mut x = header.x + 1;
    let tabs = WorkView::ALL
        .iter()
        .map(|view| {
            let text = chip_text(*view, panel.counts[view.index()]);
            let w = (width_of(&text) as u16).min(header.right().saturating_sub(x));
            let rect = Rect::new(x, header.y + 1, w, 1);
            x = x.saturating_add(w + 1).min(header.right());
            (rect, *view)
        })
        .collect();
    Some(WorkPanelLayout {
        outer,
        header,
        tabs,
        list,
        detail,
        footer_row,
        start: panel.start(usize::from(list.height)),
    })
}

// -- render -----------------------------------------------------------------------

fn task_glyph(shown: TaskShownInfo, p: &Palette) -> (&'static str, Color) {
    match shown {
        TaskShownInfo::Running => ("●", p.yellow),
        TaskShownInfo::Waiting => ("◌", p.yellow),
        TaskShownInfo::RunUnknown => ("?", p.red),
        TaskShownInfo::Done => ("✔", p.green),
        TaskShownInfo::Dropped => ("·", p.overlay0),
        _ => ("☐", p.text),
    }
}

fn ask_glyph(status: &str, p: &Palette) -> (&'static str, Color) {
    match status {
        "answered" => ("✔", p.green),
        "closed" | "dropped" => ("·", p.overlay0),
        _ => ("☐", p.text),
    }
}

/// What a row says: its mark, its text, and the dim words at its right.
fn row_parts(
    panel: &ClientWorkPanelOverlay,
    row: &WorkRow,
    p: &Palette,
) -> Option<((&'static str, Color), String, String)> {
    match row {
        WorkRow::Issue(index) | WorkRow::IssueWhy(index, _) => {
            let issue = panel.issues.data()?.issues.get(*index)?;
            let mark = if issue.state == "open" {
                ("☐", p.text)
            } else {
                ("·", p.overlay0)
            };
            let mut right = Vec::new();
            if let WorkRow::IssueWhy(_, why) = row {
                right.push((*why).to_owned());
            }
            right.push(issue.state.clone());
            if !issue.labels.is_empty() {
                right.push(issue.labels.join(","));
            }
            if !issue.assignees.is_empty() {
                right.push(format!("@{}", issue.assignees.join(",@")));
            }
            if let Some(updated) = issue.updated_at.as_deref() {
                right.push(updated.get(..10).unwrap_or(updated).to_owned());
            }
            Some((
                mark,
                format!("#{} {}", issue.issue_ref.number, issue.title),
                right.join("  "),
            ))
        }
        WorkRow::Task(index) => {
            let task = panel.tasks.data()?.tasks.get(*index)?;
            let mut right = Vec::new();
            if task.conflict {
                right.push("held".to_owned());
            }
            if task.pending {
                right.push("pending".to_owned());
            }
            if task.waits_on_missing {
                right.push("ask gone".to_owned());
            }
            right.push(shown_label(task.shown).to_owned());
            Some((
                task_glyph(task.shown, p),
                task.title.clone(),
                right.join("  "),
            ))
        }
        WorkRow::Ask(index) => {
            let ask = panel.asks.data()?.asks.get(*index)?;
            let title = ask.title.clone().unwrap_or_else(|| ask.key.clone());
            let right = [ask.where_.clone(), Some(ask.status.clone())]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("  ");
            Some((ask_glyph(&ask.status, p), title, right))
        }
        _ => None,
    }
}

fn detail_text(panel: &ClientWorkPanelOverlay) -> String {
    match panel.rows.get(panel.selected) {
        Some(WorkRow::Issue(index) | WorkRow::IssueWhy(index, _)) => panel
            .issues
            .data()
            .and_then(|data| data.issues.get(*index))
            .map_or(String::new(), |issue| {
                let r = &issue.issue_ref;
                format!(
                    "{} {}#{}  {}",
                    r.forge,
                    r.project_path,
                    r.number,
                    issue.web_url.as_deref().unwrap_or("")
                )
            }),
        Some(WorkRow::Task(index)) => panel
            .tasks
            .data()
            .and_then(|data| data.tasks.get(*index))
            .map_or(String::new(), |task| {
                format!("{}  rev {}", task.key, task.rev)
            }),
        Some(WorkRow::Ask(index)) => panel
            .asks
            .data()
            .and_then(|data| data.asks.get(*index))
            .map_or(String::new(), |ask| match ask.answer.as_deref() {
                Some(answer) => format!("{}  answer: {answer}", ask.key),
                None => ask.key.clone(),
            }),
        _ => String::new(),
    }
}

fn render_row(
    b: &mut Buffer,
    row: Rect,
    panel: &ClientWorkPanelOverlay,
    item: &WorkRow,
    selected: bool,
    p: &Palette,
) {
    let bar = Style::default()
        .fg(contrast(p))
        .bg(p.accent)
        .add_modifier(Modifier::BOLD);
    let plain = |fg: Color| Style::default().fg(fg).bg(p.panel_bg);
    b.set_style(row, if selected { bar } else { plain(p.text) });
    let width = usize::from(row.width);
    let line = |b: &mut Buffer, text: &str, style: Style| {
        put_text(
            b,
            row.x + 1,
            row.y,
            row.width.saturating_sub(1),
            &truncate_end(text, width.saturating_sub(1)),
            style,
        );
    };
    match item {
        WorkRow::Heading { text, count } => line(
            b,
            &format!("{text}  {count}"),
            plain(p.accent).add_modifier(Modifier::BOLD),
        ),
        WorkRow::Note(text) => line(b, text, plain(p.overlay0)),
        WorkRow::Problem(text) => line(b, text, plain(p.red)),
        _ => {
            let Some((mark, text, right)) = row_parts(panel, item, p) else {
                return;
            };
            let right_w = width_of(&right).min(width / 2);
            let right = truncate_end(&right, right_w);
            let left_budget = width.saturating_sub(right_w + 5);
            let mark_style = if selected { bar } else { plain(mark.1) };
            let text_style = if selected { bar } else { plain(p.text) };
            let dim = if selected { bar } else { plain(p.overlay0) };
            put_text(b, row.x + 2, row.y, 1, mark.0, mark_style);
            put_text(
                b,
                row.x + 4,
                row.y,
                left_budget as u16,
                &truncate_end(&text, left_budget),
                text_style,
            );
            if right_w > 0 {
                put_text(
                    b,
                    row.right().saturating_sub(right_w as u16 + 1),
                    row.y,
                    right_w as u16,
                    &right,
                    dim,
                );
            }
        }
    }
}

/// The key bar: `key label` pairs, the key in bold accent, the label dim, two
/// spaces apart; pairs that do not fit are dropped from the right.
fn render_key_bar(b: &mut Buffer, row: Rect, p: &Palette) {
    const KEYS: [(&str, &str); 5] = [
        ("1-4", "view"),
        ("tab", "next"),
        ("j/k", "move"),
        ("r", "refresh"),
        ("esc", "close"),
    ];
    let mut x = row.x + 1;
    for (key, label) in KEYS {
        let need = (width_of(key) + 1 + width_of(label)) as u16;
        if x + need > row.right() {
            break;
        }
        put_text(
            b,
            x,
            row.y,
            key.len() as u16,
            key,
            Style::default()
                .fg(p.accent)
                .bg(p.panel_bg)
                .add_modifier(Modifier::BOLD),
        );
        put_text(
            b,
            x + key.len() as u16 + 1,
            row.y,
            width_of(label) as u16,
            label,
            Style::default().fg(p.overlay0).bg(p.panel_bg),
        );
        x += need + 2;
    }
}

pub(super) fn render_work_panel(
    b: &mut Buffer,
    panel: &ClientWorkPanelOverlay,
    palette: &Palette,
) -> Option<WorkPanelLayout> {
    let p = palette;
    for y in b.area.y..b.area.bottom() {
        for x in b.area.x..b.area.right() {
            let cell = &mut b[(x, y)];
            cell.set_style(cell.style().add_modifier(Modifier::DIM));
        }
    }
    let Some(layout) = work_panel_layout(panel, b.area) else {
        // Too small: one yellow line, as every popup says it.
        let text = format!(
            "widen the popup — work needs {}×{}, this is {}×{}",
            MIN_SCREEN.0, MIN_SCREEN.1, b.area.width, b.area.height
        );
        put_text(
            b,
            b.area.x,
            b.area.y,
            b.area.width,
            &text,
            Style::default().fg(p.yellow).bg(p.panel_bg),
        );
        return None;
    };
    render_panel_shell(b, layout.outer, p.accent, p.panel_bg)?;
    let header = layout.header;
    let panel_style = |fg: Color| Style::default().fg(fg).bg(p.panel_bg);
    if header.height >= 1 {
        put_text(
            b,
            header.x + 1,
            header.y,
            header.width.saturating_sub(2),
            "work",
            panel_style(p.text).add_modifier(Modifier::BOLD),
        );
        let total: usize = panel.counts[panel.view.index()];
        let count = format!("{total} {}", if total == 1 { "row" } else { "rows" });
        let w = width_of(&count) as u16;
        put_text(
            b,
            header.right().saturating_sub(w + 1),
            header.y,
            w,
            &count,
            panel_style(p.overlay0),
        );
    }
    for (rect, view) in &layout.tabs {
        let text = chip_text(*view, panel.counts[view.index()]);
        let style = if *view == panel.view {
            Style::default()
                .fg(contrast(p))
                .bg(p.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            panel_style(p.overlay0)
        };
        b.set_style(*rect, style);
        put_text(b, rect.x, rect.y, rect.width, &text, style);
    }
    if header.height >= 3 {
        let line = if panel.sync.is_empty() {
            "sync: waiting for the first answer"
        } else {
            panel.sync.as_str()
        };
        put_text(
            b,
            header.x + 1,
            header.y + 2,
            header.width.saturating_sub(2),
            &truncate_end(line, usize::from(header.width.saturating_sub(2))),
            panel_style(if panel.sync_alarm { p.red } else { p.overlay0 }),
        );
    }
    if header.height >= 4 {
        render_rule(b, header.x, header.y + 3, header.width, p);
    }

    let list = layout.list;
    for (offset, item) in panel
        .rows
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
        render_row(b, row, panel, item, offset == panel.selected, p);
    }
    render_scrollbar(b, list, layout.start, panel.rows.len(), p);

    if let Some(detail) = layout.detail {
        render_rule(b, detail.x, detail.y, detail.width, p);
        put_text(
            b,
            detail.x + 1,
            detail.y + 1,
            detail.width.saturating_sub(2),
            &crate::ui::text::middle_elide(
                &detail_text(panel),
                usize::from(detail.width.saturating_sub(2)),
            ),
            panel_style(p.overlay0),
        );
    }
    if let Some(footer) = layout.footer_row {
        render_key_bar(b, footer, p);
    }
    Some(layout)
}

// -- input and requests -------------------------------------------------------------

/// The two folder names nearest the end of `path`, lowercase, the deepest first.
fn folders_of(path: &str) -> Vec<String> {
    path.split('/')
        .rev()
        .filter(|part| !part.is_empty())
        .take(2)
        .map(str::to_lowercase)
        .collect()
}

impl ClientShellState {
    fn work_panel_mut(&mut self) -> Option<&mut ClientWorkPanelOverlay> {
        match self.overlay.as_mut() {
            Some(ClientShellOverlay::WorkPanel(panel)) => Some(panel),
            _ => None,
        }
    }

    pub(super) fn open_work_panel(&mut self, outcome: &mut ClientShellInput) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        let folders = snapshot
            .focused_pane_id
            .as_deref()
            .and_then(|id| snapshot.panes.iter().find(|pane| pane.pane_id == id))
            .and_then(|pane| pane.foreground_cwd.as_deref().or(pane.cwd.as_deref()))
            .map(folders_of)
            .unwrap_or_default();
        self.overlay = Some(ClientShellOverlay::WorkPanel(ClientWorkPanelOverlay::new(
            snapshot.boot_id.clone(),
            folders,
        )));
        self.request_work_panel(outcome);
        outcome.repaint = true;
    }

    /// Open the panel, or close it when it is already open: the tab-bar count's
    /// toggle.
    pub(super) fn toggle_work_panel(&mut self, outcome: &mut ClientShellInput) {
        if matches!(self.overlay, Some(ClientShellOverlay::WorkPanel(_))) {
            self.close_work_panel(outcome);
            return;
        }
        self.open_work_panel(outcome);
    }

    fn close_work_panel(&mut self, outcome: &mut ClientShellInput) {
        self.overlay = None;
        self.mode = ClientShellMode::Terminal;
        outcome.repaint = true;
    }

    fn issue_page_request(offset: u32) -> Method {
        Method::IssueList(IssueListParams {
            project: None,
            state: Some(ISSUE_STATE.to_owned()),
            limit: Some(PAGE),
            offset: (offset > 0).then_some(offset),
        })
    }

    /// Ask one source; a source the server lacks is marked so and not asked.
    fn ask_work_source(
        &mut self,
        method: Method,
        kind: PendingEndpointKind,
        outcome: &mut ClientShellInput,
    ) {
        if !self.supports_endpoint_method(&method) {
            let name = crate::api::api_method_name(&method);
            self.work_feed_failed(&kind, format!("{LACKS_METHODS} ({name})"));
            return;
        }
        if self.push_endpoint_method_with_kind(method, kind, outcome) {
            if let Some(panel) = self.work_panel_mut() {
                panel.in_flight += 1;
            }
        }
    }

    /// Ask the server for the three lists (issues in pages). One already on
    /// its way is not asked twice.
    pub(super) fn request_work_panel(&mut self, outcome: &mut ClientShellInput) {
        if !matches!(self.overlay, Some(ClientShellOverlay::WorkPanel(_))) {
            return;
        }
        if self
            .work_panel_mut()
            .is_some_and(|panel| panel.in_flight > 0)
        {
            return;
        }
        if let Some(panel) = self.work_panel_mut() {
            panel.last_fetch = Instant::now();
            panel.issue_stage = None;
        }
        self.ask_work_source(
            Self::issue_page_request(0),
            PendingEndpointKind::WorkIssues { offset: 0 },
            outcome,
        );
        self.ask_work_source(
            Method::TaskList(TaskListParams::default()),
            PendingEndpointKind::WorkTasks,
            outcome,
        );
        self.ask_work_source(
            Method::AskList(AskListParams {
                kind: Some("any".to_owned()),
                ..AskListParams::default()
            }),
            PendingEndpointKind::WorkAsks,
            outcome,
        );
        if let Some(panel) = self.work_panel_mut() {
            panel.rebuild();
        }
        outcome.repaint = true;
    }

    fn work_feed_failed(&mut self, kind: &PendingEndpointKind, why: String) {
        let Some(panel) = self.work_panel_mut() else {
            return;
        };
        match kind {
            PendingEndpointKind::WorkIssues { .. } => {
                panel.issue_stage = None;
                panel.issues.fail(why);
            }
            PendingEndpointKind::WorkTasks => panel.tasks.fail(why),
            PendingEndpointKind::WorkAsks => panel.asks.fail(why),
            _ => {}
        }
    }

    /// An answer to one of the three lists.
    pub(super) fn handle_work_panel_result(
        &mut self,
        kind: PendingEndpointKind,
        result: Result<ResponseResult, ClientShellEndpointError>,
        outcome: &mut ClientShellInput,
    ) -> bool {
        let Some(panel) = self.work_panel_mut() else {
            return false;
        };
        panel.in_flight = panel.in_flight.saturating_sub(1);
        let mut next_page = None;
        match (&kind, result) {
            (
                PendingEndpointKind::WorkIssues { offset },
                Ok(ResponseResult::IssueList {
                    issues,
                    total,
                    projects,
                    mirror,
                }),
            ) => {
                // The first page starts the collection over.
                if *offset == 0 {
                    panel.issue_stage = None;
                }
                let stage = panel.issue_stage.get_or_insert_with(IssuesData::default);
                let empty_page = issues.is_empty();
                stage.issues.extend(issues);
                stage.total = total;
                stage.projects = projects;
                stage.mirror = Some(mirror);
                let have = stage.issues.len();
                let pages = have.div_ceil(PAGE as usize);
                if !empty_page && (have as u64) < total && pages < MAX_PAGES {
                    next_page = Some(have as u32);
                } else if let Some(done) = panel.issue_stage.take() {
                    panel.issues = Feed::Ready(done);
                }
            }
            (PendingEndpointKind::WorkTasks, Ok(ResponseResult::TaskList { tasks, store })) => {
                panel.tasks = Feed::Ready(TasksData { tasks, store })
            }
            (
                PendingEndpointKind::WorkAsks,
                Ok(ResponseResult::AskList {
                    asks,
                    total,
                    mirror,
                }),
            ) => {
                panel.asks = Feed::Ready(AsksData {
                    asks,
                    total,
                    mirror: Some(mirror),
                })
            }
            (_, Err(error)) => {
                let why = error.message;
                self.work_feed_failed(&kind, why);
            }
            (_, Ok(_)) => self.work_feed_failed(&kind, "unexpected answer".to_owned()),
        }
        if let Some(offset) = next_page {
            self.ask_work_source(
                Self::issue_page_request(offset),
                PendingEndpointKind::WorkIssues { offset },
                outcome,
            );
        }
        if let Some(panel) = self.work_panel_mut() {
            panel.rebuild();
        }
        true
    }

    /// Keep an open panel in step: close it when its server boot is gone,
    /// ask again every few seconds.
    pub(crate) fn tick_work_panel(&mut self, outcome: &mut ClientShellInput) {
        let Some(ClientShellOverlay::WorkPanel(panel)) = self.overlay.as_ref() else {
            return;
        };
        if self
            .snapshot
            .as_deref()
            .is_some_and(|snapshot| snapshot.boot_id != panel.boot_id)
        {
            self.close_work_panel(outcome);
            return;
        }
        if panel.in_flight == 0 && panel.last_fetch.elapsed() >= POLL {
            self.request_work_panel(outcome);
        }
    }

    fn work_panel_visible_rows(&self) -> usize {
        self.hits
            .work_panel
            .as_ref()
            .map_or(0, |layout| usize::from(layout.list.height))
    }

    fn set_work_view(&mut self, view: WorkView) {
        if let Some(panel) = self.work_panel_mut() {
            panel.view = view;
            panel.selected = 0;
            panel.selected_key = None;
            panel.scroll = 0;
            panel.rebuild();
        }
    }

    pub(super) fn route_work_panel_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) {
        if key.kind == crossterm::event::KeyEventKind::Release {
            return;
        }
        let (code, modifiers) = (key.code, key.modifiers);
        outcome.repaint = true;
        let visible = self.work_panel_visible_rows();
        let Some(panel) = self.work_panel_mut() else {
            return;
        };
        let current = panel.view;
        match code {
            KeyCode::Esc | KeyCode::Char('q') if modifiers.is_empty() => {
                self.close_work_panel(outcome)
            }
            KeyCode::Char('r') if modifiers.is_empty() => {
                panel.in_flight = 0;
                self.request_work_panel(outcome);
            }
            // h, l and the other letters are the actions' keys, kept free.
            KeyCode::Tab | KeyCode::Right if modifiers.is_empty() => {
                self.set_work_view(current.step(1))
            }
            KeyCode::BackTab | KeyCode::Left
                if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT =>
            {
                self.set_work_view(current.step(-1))
            }
            KeyCode::Char(digit @ '1'..='4') if modifiers.is_empty() => {
                let index = usize::from(digit as u8 - b'1');
                self.set_work_view(WorkView::ALL[index])
            }
            KeyCode::PageDown => panel.move_to(Chord::HalfPageDown, visible),
            KeyCode::PageUp => panel.move_to(Chord::HalfPageUp, visible),
            _ => match list_chord(code, modifiers, false) {
                Some(chord) => panel.move_to(chord, visible),
                None => outcome.repaint = false,
            },
        }
    }

    pub(super) fn route_work_panel_mouse(
        &mut self,
        mouse: MouseEvent,
        outcome: &mut ClientShellInput,
    ) {
        let point = (mouse.column, mouse.row);
        let Some(layout) = self.hits.work_panel.clone() else {
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                self.close_work_panel(outcome);
            }
            return;
        };
        let visible = usize::from(layout.list.height);
        match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                if let Some(panel) = self.work_panel_mut() {
                    let chord = if mouse.kind == MouseEventKind::ScrollUp {
                        Chord::Prev
                    } else {
                        Chord::Next
                    };
                    panel.move_to(chord, visible);
                    outcome.repaint = true;
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some((_, view)) = layout
                    .tabs
                    .iter()
                    .find(|(rect, _)| super::contains(*rect, point))
                {
                    self.set_work_view(*view);
                    outcome.repaint = true;
                    return;
                }
                if let Some(panel) = self.work_panel_mut() {
                    if let Some(index) = layout.row_at(mouse.column, mouse.row, panel.rows.len()) {
                        if panel.rows[index].selectable() {
                            panel.select(index);
                            outcome.repaint = true;
                        }
                        return;
                    }
                }
                if !super::contains(layout.outer, point) {
                    self.close_work_panel(outcome);
                }
            }
            _ => {}
        }
    }
}
