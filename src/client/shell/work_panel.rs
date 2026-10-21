//! The work panel (fork issue 241, task 2.14): a read-only popup over what the
//! server knows about work. It has four views, each a list built from three
//! server answers and nothing else; the client never holds bus credentials.
//!
//! - *This repo*: open forge issues of the project that matches the focused
//!   pane's folder (any folder name on its path that is the project's name).
//!   When nothing matches the view says so and shows every project.
//! - *Needs me*: open issues that carry a `needs/…` label.
//! - *In flight*: open issues labelled `status/in-progress`, and the tasks that
//!   are running or whose run is unknown.
//! - *My tasks*: the agents' tasks from `task.list`, by owner and pane, and
//!   ac's own task rows from `ask.list`.
//!
//! The header shows when each project last synced (`not synced` for a project
//! with no sync record). A server without the work methods is said to lack
//! them, view by view; nothing else disconnects. The panel polls, because the
//! issue mirror has no event of its own.

use std::time::{Duration, Instant};

use super::move_picker::{list_chord, render_rule, render_scrollbar, reveal_scroll, Chord};
use super::render::put_text;
use super::todo_panel::{contrast, render_panel_shell};
use super::*;
use crate::api::schema::{
    AskInfo, AskListParams, IssueInfo, IssueListParams, IssueProjectInfo, Method, MirrorInfo,
    ResponseResult, TaskInfo, TaskListParams, TaskShownInfo,
};
use crate::ui::text::{display_width as width_of, truncate_end};
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::style::Color;

const MIN_WIDTH: u16 = 60;
const MAX_WIDTH: u16 = 120;
const MAX_HEIGHT: u16 = 30;
/// Title, the view chips, the sync line, a rule.
const HEADER_ROWS: u16 = 4;
const FOOTER_ROWS: u16 = 2;
const DETAIL_ROWS: u16 = 2;
const BUTTON_GAP: u16 = 2;
/// How long an open panel waits before it asks the server again.
const POLL: Duration = Duration::from_secs(5);
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

/// One source of rows: asked and not answered yet, answered, or why not.
#[derive(Debug, Clone, Default)]
pub(super) enum Feed<T> {
    #[default]
    Waiting,
    Ready(T),
    Failed(String),
}

#[derive(Debug, Clone, Default)]
pub(super) struct IssuesData {
    pub(super) issues: Vec<IssueInfo>,
    /// How many the server has; more than `issues` when it cut the list.
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

/// A line of the list. Headings and notes cannot be selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WorkRow {
    Heading { text: String, count: usize },
    Note(String),
    Issue(usize),
    Task(usize),
    Ask(usize),
}

impl WorkRow {
    fn selectable(&self) -> bool {
        matches!(self, Self::Issue(_) | Self::Task(_) | Self::Ask(_))
    }
}

#[derive(Debug)]
pub(super) struct ClientWorkPanelOverlay {
    pub(super) boot_id: String,
    pub(super) view: WorkView,
    /// Folder names on the focused pane's path, lowercase.
    pub(super) folders: Vec<String>,
    pub(super) issues: Feed<IssuesData>,
    pub(super) tasks: Feed<Vec<TaskInfo>>,
    pub(super) asks: Feed<AsksData>,
    pub(super) rows: Vec<WorkRow>,
    /// How many selectable rows each view has, in `WorkView::ALL` order.
    pub(super) counts: [usize; 4],
    pub(super) selected: usize,
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
            rows: Vec::new(),
            counts: [0; 4],
            selected: 0,
            scroll: 0,
            in_flight: 0,
            last_fetch: Instant::now(),
        }
    }

    /// Build the rows again from what the server last said, keeping the
    /// selection on the same row when it is still there.
    pub(super) fn rebuild(&mut self) {
        let keep = self.rows.get(self.selected).cloned();
        for view in WorkView::ALL {
            self.counts[view.index()] = self
                .rows_for(view)
                .iter()
                .filter(|row| row.selectable())
                .count();
        }
        self.rows = self.rows_for(self.view);
        self.selected = keep
            .and_then(|keep| self.rows.iter().position(|row| *row == keep))
            .unwrap_or(self.selected)
            .min(self.rows.len().saturating_sub(1));
        self.snap_selection();
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
        self.scroll = reveal_scroll(self.scroll, self.selected, visible, self.rows.len());
    }

    pub(super) fn start(&self, visible: usize) -> usize {
        reveal_scroll(self.scroll, self.selected, visible, self.rows.len())
    }

    fn feed_note(&self, view: WorkView) -> Option<String> {
        fn failed<T>(feed: &Feed<T>) -> Option<String> {
            match feed {
                Feed::Failed(why) => Some(why.clone()),
                _ => None,
            }
        }
        fn waiting<T>(feed: &Feed<T>) -> bool {
            matches!(feed, Feed::Waiting)
        }
        let issue_view = matches!(
            view,
            WorkView::ThisRepo | WorkView::NeedsMe | WorkView::InFlight
        );
        if issue_view {
            if let Some(why) = failed(&self.issues) {
                return Some(why);
            }
            if waiting(&self.issues) {
                return Some("loading…".to_owned());
            }
            if let Feed::Ready(data) = &self.issues {
                if let Some(MirrorInfo {
                    loaded: false,
                    detail,
                }) = &data.mirror
                {
                    return Some(format!(
                        "issues not read yet: {}",
                        detail.as_deref().unwrap_or("waiting for the bus")
                    ));
                }
            }
        }
        if view == WorkView::MyTasks {
            if let Some(why) = failed(&self.tasks) {
                return Some(why);
            }
            if waiting(&self.tasks) {
                return Some("loading…".to_owned());
            }
        }
        None
    }

    fn open_issues(&self) -> Vec<(usize, &IssueInfo)> {
        match &self.issues {
            Feed::Ready(data) => data
                .issues
                .iter()
                .enumerate()
                .filter(|(_, issue)| issue.state == "open")
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Whether `issue` is in a project that the focused pane's folder names.
    fn in_this_repo(&self, issue: &IssueInfo) -> bool {
        let name = issue
            .issue_ref
            .project_path
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_lowercase();
        !name.is_empty() && self.folders.contains(&name)
    }

    pub(super) fn rows_for(&self, view: WorkView) -> Vec<WorkRow> {
        let mut rows = Vec::new();
        if let Some(note) = self.feed_note(view) {
            rows.push(WorkRow::Note(note));
            // A failed or loading source still lets the other half show.
            if view != WorkView::InFlight {
                return rows;
            }
        }
        if matches!(
            view,
            WorkView::ThisRepo | WorkView::NeedsMe | WorkView::InFlight
        ) {
            if let Feed::Ready(data) = &self.issues {
                if data.total > data.issues.len() as u64 {
                    rows.push(WorkRow::Note(format!(
                        "the server sent the first {} of {} open issues",
                        data.issues.len(),
                        data.total
                    )));
                }
            }
        }
        match view {
            WorkView::ThisRepo => {
                let mut issues = self.open_issues();
                let here: Vec<_> = issues
                    .iter()
                    .copied()
                    .filter(|(_, issue)| self.in_this_repo(issue))
                    .collect();
                if here.is_empty() {
                    if !issues.is_empty() {
                        rows.push(WorkRow::Note(
                            "no project matches this pane's folder; every project is shown"
                                .to_owned(),
                        ));
                    }
                } else {
                    issues = here;
                }
                push_issue_groups(&mut rows, issues);
                if rows.is_empty() {
                    rows.push(WorkRow::Note("no open issues".to_owned()));
                }
            }
            WorkView::NeedsMe => {
                let issues = self
                    .open_issues()
                    .into_iter()
                    .filter(|(_, issue)| issue.labels.iter().any(|l| l.starts_with("needs/")))
                    .collect();
                push_issue_groups(&mut rows, issues);
                if rows.is_empty() {
                    rows.push(WorkRow::Note(
                        "nothing is waiting for a decision".to_owned(),
                    ));
                }
            }
            WorkView::InFlight => {
                let issues: Vec<_> = self
                    .open_issues()
                    .into_iter()
                    .filter(|(_, issue)| issue.labels.iter().any(|l| l == "status/in-progress"))
                    .collect();
                let mut any = !issues.is_empty();
                push_issue_groups(&mut rows, issues);
                if let Feed::Ready(tasks) = &self.tasks {
                    let running: Vec<_> = tasks
                        .iter()
                        .enumerate()
                        .filter(|(_, task)| {
                            matches!(
                                task.shown,
                                TaskShownInfo::Running | TaskShownInfo::RunUnknown
                            )
                        })
                        .collect();
                    if !running.is_empty() {
                        any = true;
                        rows.push(WorkRow::Heading {
                            text: "tasks running".to_owned(),
                            count: running.len(),
                        });
                        rows.extend(running.into_iter().map(|(index, _)| WorkRow::Task(index)));
                    }
                }
                if !any && rows.is_empty() {
                    rows.push(WorkRow::Note("nothing is in flight".to_owned()));
                }
            }
            WorkView::MyTasks => {
                if let Feed::Ready(tasks) = &self.tasks {
                    let mut owners: Vec<(String, Vec<usize>)> = Vec::new();
                    for (index, task) in tasks.iter().enumerate() {
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
                match &self.asks {
                    Feed::Ready(data) if data.mirror.as_ref().is_some_and(|m| !m.loaded) => {
                        rows.push(WorkRow::Note(format!(
                            "asks not read yet: {}",
                            data.mirror
                                .as_ref()
                                .and_then(|m| m.detail.as_deref())
                                .unwrap_or("waiting for the bus")
                        )));
                    }
                    Feed::Ready(data) if !data.asks.is_empty() => {
                        rows.push(WorkRow::Heading {
                            text: "ac's task rows".to_owned(),
                            count: data.asks.len(),
                        });
                        rows.extend((0..data.asks.len()).map(WorkRow::Ask));
                        if data.total > data.asks.len() as u64 {
                            rows.push(WorkRow::Note(format!(
                                "the server sent the first {} of {} task rows",
                                data.asks.len(),
                                data.total
                            )));
                        }
                    }
                    Feed::Failed(why) => rows.push(WorkRow::Note(format!("asks: {why}"))),
                    _ => {}
                }
                if rows.is_empty() {
                    rows.push(WorkRow::Note("no tasks".to_owned()));
                }
            }
        }
        rows
    }

    /// The line under the view chips: when each project shown last synced.
    pub(super) fn sync_line(&self) -> String {
        let Feed::Ready(data) = &self.issues else {
            return String::new();
        };
        let shown: Vec<&IssueInfo> = self
            .rows
            .iter()
            .filter_map(|row| match row {
                WorkRow::Issue(index) => data.issues.get(*index),
                _ => None,
            })
            .collect();
        let mut seen: Vec<(&str, u64)> = Vec::new();
        for issue in &shown {
            let id = (
                issue.issue_ref.forge_id.as_str(),
                issue.issue_ref.project_id,
            );
            if !seen.contains(&id) {
                seen.push(id);
            }
        }
        // With no issue to show, say how the projects stand anyway.
        if seen.is_empty() && matches!(self.view, WorkView::ThisRepo) {
            seen = data
                .projects
                .iter()
                .map(|p| (p.forge_id.as_str(), p.project_id))
                .collect();
        }
        let parts: Vec<String> = seen
            .into_iter()
            .map(|(forge_id, project_id)| {
                let project = data
                    .projects
                    .iter()
                    .find(|p| p.forge_id == forge_id && p.project_id == project_id);
                let name = project
                    .and_then(|p| p.project_path.clone())
                    .or_else(|| {
                        shown
                            .iter()
                            .find(|i| {
                                i.issue_ref.forge_id == forge_id
                                    && i.issue_ref.project_id == project_id
                            })
                            .map(|i| i.issue_ref.project_path.clone())
                    })
                    .unwrap_or_else(|| project_id.to_string());
                match project {
                    Some(p) if p.synced && p.status.as_deref() == Some("error") => {
                        let when = p.last_ok.as_deref().map_or("never".to_owned(), sync_time);
                        format!(
                            "{name} sync failing ({}), last ok {when}",
                            p.error_class.as_deref().unwrap_or("error")
                        )
                    }
                    Some(p) if p.synced => format!(
                        "{name} synced {}",
                        p.last_ok.as_deref().map_or("never".to_owned(), sync_time)
                    ),
                    _ => format!("{name} not synced"),
                }
            })
            .collect();
        parts.join("  ·  ")
    }
}

/// `2026-10-21T12:00:00Z` as `2026-10-21 12:00`.
fn sync_time(stamp: &str) -> String {
    let stamp = stamp.trim_end_matches('Z');
    match stamp.split_once('T') {
        Some((day, time)) => format!("{day} {}", time.get(..5).unwrap_or(time)),
        None => stamp.to_owned(),
    }
}

/// The owner and pane a task is grouped under.
fn task_group(task: &TaskInfo) -> String {
    let owner = task.owner.as_deref().unwrap_or("nobody");
    match task.pane.as_ref() {
        Some(pane) if !pane.label.is_empty() => format!("{owner} · {}", pane.label),
        _ => owner.to_owned(),
    }
}

/// Issues grouped by project, the most recently touched first.
fn push_issue_groups(rows: &mut Vec<WorkRow>, mut issues: Vec<(usize, &IssueInfo)>) {
    issues.sort_by(|a, b| {
        (
            &a.1.issue_ref.project_path,
            std::cmp::Reverse(&a.1.updated_at),
        )
            .cmp(&(
                &b.1.issue_ref.project_path,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WorkButton {
    Refresh,
    Close,
}

fn button_text(button: WorkButton) -> &'static str {
    match button {
        WorkButton::Refresh => " refresh ",
        WorkButton::Close => " close ",
    }
}

const BUTTONS: [WorkButton; 2] = [WorkButton::Refresh, WorkButton::Close];

#[derive(Debug, Clone)]
pub(super) struct WorkPanelLayout {
    pub(super) outer: Rect,
    pub(super) header: Rect,
    /// The view chips, in `WorkView::ALL` order.
    pub(super) tabs: Vec<(Rect, WorkView)>,
    pub(super) list: Rect,
    pub(super) detail: Option<Rect>,
    pub(super) footer_row: Option<Rect>,
    pub(super) buttons: Vec<(Rect, WorkButton)>,
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
    if screen.width < 20 || screen.height < 8 {
        return None;
    }
    let width = MAX_WIDTH.min(screen.width).max(MIN_WIDTH.min(screen.width));
    let width = width.min(screen.width.saturating_sub(2).max(20));
    let height = MAX_HEIGHT.min(screen.height.saturating_sub(2).max(8));
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
    let buttons_width: u16 = BUTTONS
        .iter()
        .map(|b| width_of(button_text(*b)) as u16)
        .sum::<u16>()
        + BUTTON_GAP * (BUTTONS.len() as u16 - 1);
    let buttons = footer_row
        .map(|row| {
            let mut x = row.right().saturating_sub(buttons_width + 1);
            BUTTONS
                .iter()
                .map(|button| {
                    let w =
                        (width_of(button_text(*button)) as u16).min(row.right().saturating_sub(x));
                    let rect = Rect::new(x, row.y, w, 1);
                    x = x.saturating_add(w + BUTTON_GAP).min(row.right());
                    (rect, *button)
                })
                .collect()
        })
        .unwrap_or_default();
    Some(WorkPanelLayout {
        outer,
        header,
        tabs,
        list,
        detail,
        footer_row,
        buttons,
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
        WorkRow::Issue(index) => {
            let Feed::Ready(data) = &panel.issues else {
                return None;
            };
            let issue = data.issues.get(*index)?;
            let mark = if issue.state == "open" {
                ("☐", p.text)
            } else {
                ("·", p.overlay0)
            };
            let mut right = Vec::new();
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
            let Feed::Ready(tasks) = &panel.tasks else {
                return None;
            };
            let task = tasks.get(*index)?;
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
            right.push(format!("{:?}", task.shown).to_lowercase());
            Some((
                task_glyph(task.shown, p),
                task.title.clone(),
                right.join("  "),
            ))
        }
        WorkRow::Ask(index) => {
            let Feed::Ready(data) = &panel.asks else {
                return None;
            };
            let ask = data.asks.get(*index)?;
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
        Some(WorkRow::Issue(index)) => match &panel.issues {
            Feed::Ready(data) => data.issues.get(*index).map_or(String::new(), |issue| {
                let r = &issue.issue_ref;
                format!(
                    "{} {}#{}  {}",
                    r.forge,
                    r.project_path,
                    r.number,
                    issue.web_url.as_deref().unwrap_or("")
                )
            }),
            _ => String::new(),
        },
        Some(WorkRow::Task(index)) => match &panel.tasks {
            Feed::Ready(tasks) => tasks.get(*index).map_or(String::new(), |task| {
                format!("{}  rev {}", task.key, task.rev)
            }),
            _ => String::new(),
        },
        Some(WorkRow::Ask(index)) => match &panel.asks {
            Feed::Ready(data) => {
                data.asks
                    .get(*index)
                    .map_or(String::new(), |ask| match ask.answer.as_deref() {
                        Some(answer) => format!("{}  answer: {answer}", ask.key),
                        None => ask.key.clone(),
                    })
            }
            _ => String::new(),
        },
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
    match item {
        WorkRow::Heading { text, count } => {
            let label = format!("{text}  {count}");
            put_text(
                b,
                row.x + 1,
                row.y,
                row.width.saturating_sub(1),
                &truncate_end(&label, width.saturating_sub(1)),
                plain(p.accent).add_modifier(Modifier::BOLD),
            );
        }
        WorkRow::Note(text) => {
            put_text(
                b,
                row.x + 1,
                row.y,
                row.width.saturating_sub(1),
                &truncate_end(text, width.saturating_sub(1)),
                plain(p.overlay0),
            );
        }
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
    let layout = work_panel_layout(panel, b.area)?;
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
        let line = panel.sync_line();
        let line = if line.is_empty() {
            "sync: unknown".to_owned()
        } else {
            line
        };
        put_text(
            b,
            header.x + 1,
            header.y + 2,
            header.width.saturating_sub(2),
            &truncate_end(&line, usize::from(header.width.saturating_sub(2))),
            panel_style(p.overlay0),
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
        let hint = "1-4 view  tab next  j/k move  r refresh  esc close";
        let room = footer.width.saturating_sub(2);
        put_text(
            b,
            footer.x + 1,
            footer.y,
            room,
            &truncate_end(hint, usize::from(room)),
            panel_style(p.overlay0),
        );
    }
    for (rect, button) in &layout.buttons {
        let style = match button {
            WorkButton::Refresh => Style::default()
                .fg(contrast(p))
                .bg(p.accent)
                .add_modifier(Modifier::BOLD),
            WorkButton::Close => Style::default()
                .fg(p.text)
                .bg(p.surface0)
                .add_modifier(Modifier::BOLD),
        };
        b.set_style(*rect, style);
        put_text(b, rect.x, rect.y, rect.width, button_text(*button), style);
    }
    Some(layout)
}

// -- input and requests -------------------------------------------------------------

/// Folder names on `path`, lowercase, the deepest first.
fn folders_of(path: &str) -> Vec<String> {
    path.split('/')
        .rev()
        .filter(|part| !part.is_empty())
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

    /// Ask the server for the three lists. A source the server lacks is marked
    /// so and not asked; one already on its way is not asked twice.
    pub(super) fn request_work_panel(&mut self, outcome: &mut ClientShellInput) {
        if !matches!(self.overlay, Some(ClientShellOverlay::WorkPanel(_))) {
            return;
        }
        let asks: [(Method, PendingEndpointKind); 3] = [
            (
                Method::IssueList(IssueListParams::default()),
                PendingEndpointKind::WorkIssues,
            ),
            (
                Method::TaskList(TaskListParams::default()),
                PendingEndpointKind::WorkTasks,
            ),
            (
                Method::AskList(AskListParams::default()),
                PendingEndpointKind::WorkAsks,
            ),
        ];
        if self
            .work_panel_mut()
            .is_some_and(|panel| panel.in_flight > 0)
        {
            return;
        }
        if let Some(panel) = self.work_panel_mut() {
            panel.last_fetch = Instant::now();
        }
        for (method, kind) in asks {
            let supported = self.supports_endpoint_method(&method);
            let name = crate::api::api_method_name(&method);
            if !supported {
                self.work_feed_failed(&kind, format!("{LACKS_METHODS} ({name})"));
                continue;
            }
            if self.push_endpoint_method_with_kind(method, kind, outcome) {
                if let Some(panel) = self.work_panel_mut() {
                    panel.in_flight += 1;
                }
            }
        }
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
            PendingEndpointKind::WorkIssues => panel.issues = Feed::Failed(why),
            PendingEndpointKind::WorkTasks => panel.tasks = Feed::Failed(why),
            PendingEndpointKind::WorkAsks => panel.asks = Feed::Failed(why),
            _ => {}
        }
    }

    /// An answer to one of the three lists.
    pub(super) fn handle_work_panel_result(
        &mut self,
        kind: PendingEndpointKind,
        result: Result<ResponseResult, ClientShellEndpointError>,
    ) -> bool {
        let Some(panel) = self.work_panel_mut() else {
            return false;
        };
        panel.in_flight = panel.in_flight.saturating_sub(1);
        match (&kind, result) {
            (
                PendingEndpointKind::WorkIssues,
                Ok(ResponseResult::IssueList {
                    issues,
                    total,
                    projects,
                    mirror,
                }),
            ) => {
                panel.issues = Feed::Ready(IssuesData {
                    issues,
                    total,
                    projects,
                    mirror: Some(mirror),
                })
            }
            (PendingEndpointKind::WorkTasks, Ok(ResponseResult::TaskList { tasks, .. })) => {
                panel.tasks = Feed::Ready(tasks)
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
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') if modifiers.is_empty() => {
                self.set_work_view(current.step(1))
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h')
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
                if let Some((_, button)) = layout
                    .buttons
                    .iter()
                    .find(|(rect, _)| super::contains(*rect, point))
                {
                    match button {
                        WorkButton::Refresh => {
                            if let Some(panel) = self.work_panel_mut() {
                                panel.in_flight = 0;
                            }
                            self.request_work_panel(outcome);
                        }
                        WorkButton::Close => self.close_work_panel(outcome),
                    }
                    return;
                }
                if let Some(panel) = self.work_panel_mut() {
                    if let Some(index) = layout.row_at(mouse.column, mouse.row, panel.rows.len()) {
                        if panel.rows[index].selectable() {
                            panel.selected = index;
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
