//! `issue.list` and `ask.list`: read-only views of the buckets worklink and
//! the gestore fill (fork issue 241). They answer from the caches the store
//! thread keeps up to date and never wait on the bus.

use std::collections::BTreeMap;

use super::responses::{encode_error, encode_success};
use crate::api::schema::{
    AskInfo, AskListParams, IssueInfo, IssueListParams, IssueProjectInfo, IssueRefInfo,
    IssueRepoInfo, MirrorInfo, ResponseResult,
};
use crate::app::App;
use crate::tasks::{
    AskCache, AskRecord, IssueCache, IssueRecord, IssueRow, IssueState, Mirror, MirrorStatus,
    RepoRemote, RepoState, Rev,
};

fn mirror_info(status: MirrorStatus) -> MirrorInfo {
    MirrorInfo {
        loaded: status.loaded,
        detail: status.detail,
    }
}

pub(crate) fn issue_info(key: &str, row: &IssueRow) -> IssueInfo {
    let IssueRecord {
        v: _,
        issue_ref,
        title,
        state,
        labels,
        assignees,
        author,
        created_at,
        updated_at,
        web_url,
        source_rev,
        last_sync,
        truncated,
    } = row.record.clone();
    IssueInfo {
        key: key.to_owned(),
        issue_ref: IssueRefInfo {
            forge: issue_ref.forge,
            forge_id: issue_ref.forge_id,
            host: issue_ref.host,
            project_id: issue_ref.project_id,
            project_path: issue_ref.project_path,
            number: issue_ref.number,
        },
        mirror_rev: row.mirror_rev.0,
        title,
        state: match state {
            IssueState::Open => "open",
            IssueState::Closed => "closed",
            IssueState::Unknown => "unknown",
        }
        .to_owned(),
        labels,
        assignees,
        author,
        created_at,
        updated_at,
        web_url,
        source_rev,
        last_sync,
        truncated,
        mine: false,
    }
}

/// Every project that has issues or a sync row, with how its last sync went.
/// A project with issues and no sync row is `synced: false`. The counts are
/// kept by the cache, so this walks projects, not issues.
pub(crate) fn project_infos(cache: &IssueCache) -> Vec<IssueProjectInfo> {
    let mut projects: BTreeMap<(String, String, u64), IssueProjectInfo> = BTreeMap::new();
    let blank = |forge: &str, forge_id: &str, project_id: u64| IssueProjectInfo {
        forge: forge.to_owned(),
        forge_id: forge_id.to_owned(),
        project_id,
        project_path: None,
        synced: false,
        status: None,
        error_class: None,
        last_attempt: None,
        last_ok: None,
        issues_open: 0,
        issues_total: 0,
    };
    for ((forge, forge_id, project_id), count) in cache.projects() {
        let mut info = blank(forge, forge_id, *project_id);
        info.project_path = Some(count.project_path.clone());
        info.issues_open = count.open as u64;
        info.issues_total = count.total as u64;
        projects.insert((forge.clone(), forge_id.clone(), *project_id), info);
    }
    for (_, row) in cache.syncs() {
        let Some(project) = row.record.project.as_ref().map(|p| &p.project_ref) else {
            continue;
        };
        let entry = projects
            .entry((
                project.forge.clone(),
                project.forge_id.clone(),
                project.project_id,
            ))
            .or_insert_with(|| blank(&project.forge, &project.forge_id, project.project_id));
        if entry.project_path.is_none() {
            entry.project_path = project.project_path.clone();
        }
        entry.synced = true;
        entry.status = Some(row.record.status.clone());
        entry.error_class = row.record.error_class.clone();
        entry.last_attempt = row.record.last_attempt.clone();
        entry.last_ok = row.record.last_ok.clone();
    }
    projects.into_values().collect()
}

pub(crate) fn ask_info(key: &str, rev: Rev, record: &AskRecord) -> AskInfo {
    AskInfo {
        key: key.to_owned(),
        rev: rev.0,
        status: record.status.clone(),
        kind: record.kind.clone(),
        title: record.title.clone(),
        how: record.how.clone(),
        where_: record.where_.clone(),
        horizon: record.horizon.clone(),
        filed_by: record.filed_by.clone(),
        answer: record.answer.clone(),
        answered_at: record.answered_at.clone(),
        superseded_by: record.superseded_by.clone(),
        source: record.source.clone(),
        created_at: record.created_at.clone(),
        updated_at: record.updated_at.clone(),
    }
}

const DEFAULT_LIMIT: usize = 500;
const MAX_LIMIT: usize = 2000;

fn window(limit: Option<u32>, offset: Option<u32>) -> (usize, usize) {
    let limit = limit
        .map_or(DEFAULT_LIMIT, |limit| limit as usize)
        .min(MAX_LIMIT);
    (limit, offset.map_or(0, |offset| offset as usize))
}

/// Whether one of `assignees` is ac's account on this issue's forge.
fn is_mine(issue: &IssueRecord, accounts: &std::collections::BTreeMap<String, String>) -> bool {
    accounts
        .get(&issue.issue_ref.forge_id)
        .is_some_and(|account| {
            issue
                .assignees
                .iter()
                .any(|assignee| assignee.eq_ignore_ascii_case(account))
        })
}

fn parse_time(stamp: &str) -> Option<time::OffsetDateTime> {
    time::OffsetDateTime::parse(stamp, &time::format_description::well_known::Rfc3339).ok()
}

/// What `issue.list` was asked to narrow to beyond the plain params.
#[derive(Default)]
pub(crate) struct IssueScope<'a> {
    /// The repo of the pane named in the request: `Found` keeps only that
    /// repo's issues; any other state keeps none.
    pub(crate) repo: Option<&'a RepoState>,
    /// `forge_id` to ac's account there.
    pub(crate) accounts: Option<&'a std::collections::BTreeMap<String, String>>,
}

/// The page asked for and how many issues match in all. Rows are already in
/// key order, so nothing is sorted, and only the page is copied.
pub(crate) fn list_issues(
    cache: &IssueCache,
    params: &IssueListParams,
    scope: &IssueScope<'_>,
) -> Result<(Vec<IssueInfo>, usize), String> {
    let wanted = match params.state.as_deref() {
        None | Some("open") => Some(IssueState::Open),
        Some("closed") => Some(IssueState::Closed),
        Some("all") => None,
        Some(other) => return Err(format!("state must be open, closed or all, not {other}")),
    };
    let closed_since = match params.closed_since.as_deref() {
        None => None,
        Some(stamp) => Some(parse_time(stamp).ok_or_else(|| {
            format!(
                "closed_since must be an RFC 3339 time such as 2026-10-15T00:00:00Z, not {stamp}"
            )
        })?),
    };
    let repo = match scope.repo {
        None => None,
        Some(RepoState::Found(repo)) => Some(repo),
        // Not known (yet): there is no repo to show issues of.
        Some(RepoState::Resolving | RepoState::NoRemote) => return Ok((Vec::new(), 0)),
    };
    let (limit, offset) = window(params.limit, params.offset);
    let mut total = 0;
    let mut page = Vec::new();
    for row in cache.issues() {
        if wanted.is_some_and(|state| row.record.state != state) {
            continue;
        }
        let r = &row.record.issue_ref;
        if let Some(project) = params.project.as_deref() {
            if r.project_path != project && r.project_id.to_string() != project {
                continue;
            }
        }
        if repo.is_some_and(|repo| !repo.is(&r.host, &r.project_path)) {
            continue;
        }
        if let Some(since) = closed_since {
            let touched = row.record.updated_at.as_deref().and_then(parse_time);
            if row.record.state == IssueState::Closed && touched.is_none_or(|t| t < since) {
                continue;
            }
        }
        if total >= offset && page.len() < limit {
            let mut info = issue_info(&row.name, row);
            info.mine = scope.accounts.is_some_and(|a| is_mine(&row.record, a));
            page.push(info);
        }
        total += 1;
    }
    Ok((page, total))
}

/// How reading the pane's repo went, and whether that repo is mirrored.
pub(crate) fn repo_info(cache: &IssueCache, state: &RepoState) -> IssueRepoInfo {
    match state {
        RepoState::Resolving => IssueRepoInfo {
            status: "resolving".to_owned(),
            host: None,
            path: None,
            mirrored: false,
        },
        RepoState::NoRemote => IssueRepoInfo {
            status: "no_remote".to_owned(),
            host: None,
            path: None,
            mirrored: false,
        },
        RepoState::Found(RepoRemote { host, path }) => IssueRepoInfo {
            status: "found".to_owned(),
            host: Some(host.clone()),
            path: Some(path.clone()),
            mirrored: cache
                .projects()
                .any(|(_, count)| count.project_path.eq_ignore_ascii_case(path)),
        },
    }
}

pub(crate) fn list_asks(cache: &AskCache, params: &AskListParams) -> (Vec<AskInfo>, usize) {
    let kind = params.kind.as_deref().unwrap_or("task");
    let (limit, offset) = window(params.limit, params.offset);
    let mut total = 0;
    let mut page = Vec::new();
    for (key, entry) in cache.rows() {
        if kind != "any" && entry.record.kind.as_deref() != Some(kind) {
            continue;
        }
        if !params.include_closed && matches!(entry.record.status.as_str(), "closed" | "dropped") {
            continue;
        }
        if total >= offset && page.len() < limit {
            page.push(ask_info(key, entry.rev, &entry.record));
        }
        total += 1;
    }
    (page, total)
}

impl App {
    pub(super) fn handle_issue_list(&mut self, id: String, params: IssueListParams) -> String {
        let repo = match params.pane.as_deref() {
            None => None,
            Some(pane) => match self.pane_repo(pane) {
                Some(state) => Some(state),
                None => {
                    return encode_error(
                        id,
                        "pane_not_found",
                        format!("pane {pane} does not exist"),
                    )
                }
            },
        };
        let scope = IssueScope {
            repo: repo.as_ref(),
            accounts: Some(&self.task_config.forge_accounts),
        };
        let (issues, total) = match list_issues(self.tasks.issues(), &params, &scope) {
            Ok(page) => page,
            Err(message) => return encode_error(id, "invalid_issue_list", message),
        };
        encode_success(
            id,
            ResponseResult::IssueList {
                issues,
                total: total as u64,
                projects: project_infos(self.tasks.issues()),
                mirror: mirror_info(self.tasks.mirror_status(Mirror::Issues)),
                repo: repo
                    .as_ref()
                    .map(|state| repo_info(self.tasks.issues(), state)),
            },
        )
    }

    /// The repo of `pane`'s folder, read off this thread (`None`: no such pane).
    fn pane_repo(&self, pane: &str) -> Option<RepoState> {
        let (ws_idx, pane_id) = self.parse_pane_id(pane)?;
        Some(
            match self.launch_cwd_for_pane_in_workspace(ws_idx, pane_id) {
                Some(cwd) => self.repo_resolver.lookup(&cwd),
                None => RepoState::NoRemote,
            },
        )
    }

    pub(super) fn handle_ask_list(&mut self, id: String, params: AskListParams) -> String {
        let (asks, total) = list_asks(self.tasks.asks(), &params);
        encode_success(
            id,
            ResponseResult::AskList {
                asks,
                total: total as u64,
                mirror: mirror_info(self.tasks.mirror_status(Mirror::Asks)),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::api::schema::{AskListParams, IssueListParams, Method, Request, TaskTarget};
    use crate::app::App;
    use crate::config::Config;
    use crate::tasks::{
        parse_ask_event, parse_issue_event, AskRow, HostId, Mirror, MirrorChanges, Origin, Rev,
        StoreEvent, StoreKey, Task, TaskId, Timestamp, Via, WatchEvent,
    };

    fn app() -> App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        App::new(
            &Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        )
    }

    fn event(key: &str, rev: u64, doc: serde_json::Value) -> WatchEvent {
        WatchEvent {
            key: StoreKey(key.into()),
            rev: Rev(rev),
            value: Some(serde_json::to_vec(&doc).unwrap()),
            at: None,
        }
    }

    fn gone(key: &str, rev: u64) -> WatchEvent {
        WatchEvent {
            key: StoreKey(key.into()),
            rev: Rev(rev),
            value: None,
            at: None,
        }
    }

    /// What the store thread hands the app: parsed rows, then `Current`.
    fn changes(which: Mirror, events: &[WatchEvent]) -> StoreEvent {
        StoreEvent::MirrorChanges {
            which,
            changes: match which {
                Mirror::Issues => {
                    MirrorChanges::Issues(events.iter().filter_map(parse_issue_event).collect())
                }
                Mirror::Asks => {
                    MirrorChanges::Asks(events.iter().filter_map(parse_ask_event).collect())
                }
            },
        }
    }

    fn load(app: &mut App, which: Mirror, events: &[WatchEvent]) {
        app.tasks.apply(StoreEvent::MirrorReset { which });
        app.tasks.apply(changes(which, events));
        app.tasks.apply(StoreEvent::MirrorCurrent { which });
    }

    fn issue(number: u64, title: &str, state: &str, extra: serde_json::Value) -> serde_json::Value {
        let mut doc = serde_json::json!({
            "v": 1,
            "ref": {"forge": "gitea", "forge_id": "gitea-lab", "host": "gitea.example",
                    "project_id": 84, "project_path": "ac/worklink", "number": number},
            "title": title, "state": state, "labels": [], "assignees": [],
            "author": "ac", "created_at": "2026-10-21T10:00:00Z",
            "updated_at": "2026-10-21T11:00:00Z",
            "web_url": format!("https://gitea.example/ac/worklink/issues/{number}"),
            "source_rev": "abc", "last_sync": "2026-10-21T12:00:00Z"
        });
        if let (Some(doc), Some(extra)) = (doc.as_object_mut(), extra.as_object()) {
            doc.extend(extra.clone());
        }
        doc
    }

    fn call(app: &mut App, method: Method) -> serde_json::Value {
        let raw = app.handle_api_request(Request {
            id: "test".into(),
            method,
        });
        serde_json::from_str(&raw).unwrap()
    }

    fn all() -> IssueListParams {
        IssueListParams {
            state: Some("all".into()),
            ..IssueListParams::default()
        }
    }

    fn mirrored_issues() -> (App, Vec<(String, u64, serde_json::Value)>) {
        let rows = vec![
            (
                "issue.gitea.gitea-lab.84.1".to_owned(),
                11,
                issue(1, "plain", "open", serde_json::json!({})),
            ),
            (
                // Empty lists must stay lists, never null.
                "issue.gitea.gitea-lab.84.2".to_owned(),
                12,
                issue(
                    2,
                    "caff\u{e8} \u{65e5}\u{672c}\u{8a9e} \u{1f600}",
                    "open",
                    serde_json::json!({"labels": ["bug", "p1"], "assignees": ["ac"]}),
                ),
            ),
            (
                // A cut title says so; the others do not carry the flag at all.
                "issue.gitea.gitea-lab.84.3".to_owned(),
                13,
                issue(
                    3,
                    "a title that was cut",
                    "closed",
                    serde_json::json!({"truncated": true}),
                ),
            ),
        ];
        let mut app = app();
        let events: Vec<_> = rows
            .iter()
            .map(|(key, rev, doc)| event(key, *rev, doc.clone()))
            .collect();
        load(&mut app, Mirror::Issues, &events);
        (app, rows)
    }

    #[test]
    fn issue_rows_equal_mirror_records() {
        let (mut app, rows) = mirrored_issues();
        let answer = call(&mut app, Method::IssueList(all()));
        let listed = answer["result"]["issues"].as_array().unwrap();
        assert_eq!(listed.len(), rows.len());
        assert_eq!(answer["result"]["total"], rows.len());
        for (listed, (key, rev, doc)) in listed.iter().zip(&rows) {
            assert_eq!(listed["key"], key.as_str());
            assert_eq!(listed["mirror_rev"], *rev, "the revision the watch saw");
            for field in [
                "title",
                "state",
                "web_url",
                "author",
                "created_at",
                "updated_at",
                "source_rev",
                "last_sync",
                "labels",
                "assignees",
                "ref",
            ] {
                assert_eq!(listed[field], doc[field], "{key}.{field}");
            }
            assert!(listed["labels"].is_array() && listed["assignees"].is_array());
            assert_eq!(
                listed["truncated"].as_bool(),
                doc["truncated"].as_bool(),
                "{key}"
            );
        }
        assert!(listed[0]["truncated"].is_null(), "left out unless true");
    }

    #[test]
    fn text_from_the_forge_is_cleaned_and_cut_not_refused() {
        let mut app = app();
        let long = "x".repeat(5000);
        load(
            &mut app,
            Mirror::Issues,
            &[event(
                "issue.gitea.gitea-lab.84.9",
                5,
                issue(
                    9,
                    &format!("evil\u{1b}[2J\u{202e}title{long}"),
                    "open",
                    serde_json::json!({
                        "labels": ["ok\u{7}", "bell"],
                        "author": "a\nb",
                        "source_rev": {"huge": "y".repeat(5000)},
                    }),
                ),
            )],
        );
        let answer = call(&mut app, Method::IssueList(all()));
        let row = &answer["result"]["issues"][0];
        let title = row["title"].as_str().unwrap();
        assert!(title.starts_with("evil[2Jtitle"), "{title}");
        assert!(!title.chars().any(char::is_control));
        assert!(title.chars().count() <= 2000);
        assert_eq!(row["truncated"], true, "a cut title says so");
        assert_eq!(row["labels"][0], "ok");
        assert_eq!(row["author"], "a b");
        assert!(
            row["source_rev"].is_null(),
            "an oversized marker is dropped"
        );
    }

    #[test]
    fn a_refused_record_takes_the_older_row_of_its_key_with_it() {
        let mut app = app();
        let key = "issue.gitea.gitea-lab.84.1";
        load(
            &mut app,
            Mirror::Issues,
            &[event(
                key,
                3,
                issue(1, "first", "open", serde_json::json!({})),
            )],
        );
        // The same key now holds a record that names another issue.
        app.tasks.apply(changes(
            Mirror::Issues,
            &[event(
                key,
                4,
                issue(2, "liar", "open", serde_json::json!({})),
            )],
        ));
        let answer = call(&mut app, Method::IssueList(all()));
        assert_eq!(answer["result"]["total"], 0, "no stale row stays");
        assert_eq!(app.tasks.issues().rejected().len(), 1);
        // A newer record version is skipped with a reason, too.
        let mut newer = issue(1, "future", "open", serde_json::json!({}));
        newer["v"] = serde_json::json!(2);
        app.tasks
            .apply(changes(Mirror::Issues, &[event(key, 5, newer)]));
        assert_eq!(app.tasks.issues().len(), 0);
        assert!(app.tasks.issues().rejected()[1].1.contains("newer"));
    }

    #[test]
    fn issue_list_pages_and_counts_without_copying_the_rest() {
        let mut app = app();
        let events: Vec<_> = (1..=30u64)
            .map(|n| {
                event(
                    &format!("issue.gitea.gitea-lab.84.{n}"),
                    n,
                    issue(n, &format!("issue {n}"), "open", serde_json::json!({})),
                )
            })
            .collect();
        load(&mut app, Mirror::Issues, &events);
        let page = call(
            &mut app,
            Method::IssueList(IssueListParams {
                limit: Some(10),
                offset: Some(5),
                ..IssueListParams::default()
            }),
        );
        let numbers: Vec<_> = page["result"]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["ref"]["number"].as_u64().unwrap())
            .collect();
        assert_eq!(
            numbers,
            (6..=15).collect::<Vec<_>>(),
            "key order, not text order"
        );
        assert_eq!(page["result"]["total"], 30);
        // The default and the most are bounded.
        let huge = call(
            &mut app,
            Method::IssueList(IssueListParams {
                limit: Some(u32::MAX),
                ..IssueListParams::default()
            }),
        );
        assert_eq!(huge["result"]["issues"].as_array().unwrap().len(), 30);
        let project = &huge["result"]["projects"][0];
        assert_eq!(project["issues_total"], 30);
        assert_eq!(project["issues_open"], 30);
    }

    #[test]
    fn project_counts_follow_rows_that_change_state_or_go_away() {
        let (mut app, _) = mirrored_issues();
        let counts = |app: &mut App| {
            let answer = call(app, Method::IssueList(all()));
            let p = &answer["result"]["projects"][0];
            (
                p["issues_open"].as_u64().unwrap(),
                p["issues_total"].as_u64().unwrap(),
            )
        };
        assert_eq!(counts(&mut app), (2, 3));
        // Closing one, then deleting another (a watch delete event).
        app.tasks.apply(changes(
            Mirror::Issues,
            &[event(
                "issue.gitea.gitea-lab.84.1",
                20,
                issue(1, "plain", "closed", serde_json::json!({})),
            )],
        ));
        assert_eq!(counts(&mut app), (1, 3));
        app.tasks.apply(changes(
            Mirror::Issues,
            &[gone("issue.gitea.gitea-lab.84.2", 21)],
        ));
        assert_eq!(counts(&mut app), (0, 2));
        // An old revision arriving late changes nothing.
        app.tasks.apply(changes(
            Mirror::Issues,
            &[event(
                "issue.gitea.gitea-lab.84.1",
                10,
                issue(1, "plain", "open", serde_json::json!({})),
            )],
        ));
        assert_eq!(counts(&mut app), (0, 2));
    }

    #[test]
    fn issue_list_filters_by_state_and_project_and_reports_syncs() {
        let (mut app, _) = mirrored_issues();
        let open = call(&mut app, Method::IssueList(IssueListParams::default()));
        assert_eq!(
            open["result"]["issues"].as_array().unwrap().len(),
            2,
            "open by default"
        );
        let closed = call(
            &mut app,
            Method::IssueList(IssueListParams {
                state: Some("closed".into()),
                ..IssueListParams::default()
            }),
        );
        assert_eq!(closed["result"]["issues"].as_array().unwrap().len(), 1);
        for project in ["84", "ac/worklink"] {
            let one = call(
                &mut app,
                Method::IssueList(IssueListParams {
                    project: Some(project.into()),
                    ..IssueListParams::default()
                }),
            );
            assert_eq!(
                one["result"]["issues"].as_array().unwrap().len(),
                2,
                "{project}"
            );
        }
        let none = call(
            &mut app,
            Method::IssueList(IssueListParams {
                project: Some("other/repo".into()),
                ..IssueListParams::default()
            }),
        );
        assert!(none["result"]["issues"].as_array().unwrap().is_empty());
        let bad = call(
            &mut app,
            Method::IssueList(IssueListParams {
                state: Some("merged".into()),
                ..IssueListParams::default()
            }),
        );
        assert_eq!(bad["error"]["code"], "invalid_issue_list");

        // No sync row yet: the project is listed, not synced.
        let project = &open["result"]["projects"][0];
        assert_eq!(project["project_id"], 84);
        assert_eq!(project["synced"], false);
        assert!(project["last_ok"].is_null());
        app.tasks.apply(changes(
            Mirror::Issues,
            &[event(
                "sync.gitea.gitea-lab.84",
                5,
                serde_json::json!({
                    "project": {"ref": {"forge": "gitea", "forge_id": "gitea-lab",
                                        "project_id": 84, "project_path": "ac/worklink"}},
                    "status": "ok", "last_ok": "2026-10-21T12:00:00Z",
                    "last_attempt": "2026-10-21T12:00:01Z", "issues": 3
                }),
            )],
        ));
        let synced = call(&mut app, Method::IssueList(IssueListParams::default()));
        let project = &synced["result"]["projects"][0];
        assert_eq!(project["synced"], true);
        assert_eq!(project["status"], "ok");
        assert_eq!(project["last_ok"], "2026-10-21T12:00:00Z");
        assert_eq!(synced["result"]["projects"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn mirror_state_is_shown_and_clears_when_the_bucket_is_read() {
        let mut app = app();
        app.tasks.apply(StoreEvent::MirrorState {
            which: Mirror::Issues,
            detail: Some("the bucket is away".into()),
        });
        let answer = call(&mut app, Method::IssueList(IssueListParams::default()));
        assert_eq!(answer["result"]["mirror"]["loaded"], false);
        assert_eq!(answer["result"]["mirror"]["detail"], "the bucket is away");
        assert!(answer["result"]["issues"].as_array().unwrap().is_empty());
        // Once the start is delivered the reason is gone.
        app.tasks.apply(StoreEvent::MirrorCurrent {
            which: Mirror::Issues,
        });
        let answer = call(&mut app, Method::IssueList(IssueListParams::default()));
        assert_eq!(answer["result"]["mirror"]["loaded"], true);
        assert!(answer["result"]["mirror"]["detail"].is_null());
        // Reading it again from its start empties what was kept.
        let (mut app, _) = mirrored_issues();
        app.tasks.apply(StoreEvent::MirrorReset {
            which: Mirror::Issues,
        });
        let answer = call(&mut app, Method::IssueList(all()));
        assert_eq!(answer["result"]["total"], 0);
        assert_eq!(answer["result"]["mirror"]["loaded"], false);
    }

    fn asks_app() -> App {
        let mut app = app();
        let row = |kind: &str, status: &str, title: &str| {
            serde_json::json!({"status": status, "kind": kind, "title": title,
                               "where": "herdr-max #241", "filed_by": "gestore-lab"})
        };
        load(
            &mut app,
            Mirror::Asks,
            &[
                event("gestore-lab.a1", 2, row("task", "open", "do the thing")),
                event("gestore-lab.a2", 3, row("decision", "open", "pick one")),
                event("gestore-lab.a3", 4, row("task", "closed", "done thing")),
            ],
        );
        app
    }

    #[test]
    fn ask_list_returns_task_rows_only_by_default() {
        let mut app = asks_app();
        let keys = |answer: &serde_json::Value| -> Vec<String> {
            answer["result"]["asks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|ask| ask["key"].as_str().unwrap().to_owned())
                .collect()
        };
        let default = call(&mut app, Method::AskList(AskListParams::default()));
        assert_eq!(keys(&default), ["gestore-lab.a1"]);
        assert_eq!(default["result"]["total"], 1);
        assert_eq!(default["result"]["asks"][0]["where"], "herdr-max #241");
        assert_eq!(default["result"]["asks"][0]["rev"], 2);
        assert_eq!(default["result"]["mirror"]["loaded"], true);
        let with_closed = call(
            &mut app,
            Method::AskList(AskListParams {
                include_closed: true,
                ..AskListParams::default()
            }),
        );
        assert_eq!(keys(&with_closed), ["gestore-lab.a1", "gestore-lab.a3"]);
        let any = call(
            &mut app,
            Method::AskList(AskListParams {
                kind: Some("any".into()),
                ..AskListParams::default()
            }),
        );
        assert_eq!(keys(&any), ["gestore-lab.a1", "gestore-lab.a2"]);
        let paged = call(
            &mut app,
            Method::AskList(AskListParams {
                kind: Some("any".into()),
                include_closed: true,
                limit: Some(1),
                offset: Some(1),
            }),
        );
        assert_eq!(keys(&paged), ["gestore-lab.a2"]);
        assert_eq!(paged["result"]["total"], 3);
    }

    /// Put a task that waits on `row` into the cache and read it back through
    /// `task.show`, the way a client does.
    fn shown_waiting_on(app: &mut App, row: &str) -> serde_json::Value {
        let mut task = Task::new(
            HostId::new("m4m").unwrap(),
            TaskId::new("tk-1").unwrap(),
            "waits",
            Origin {
                host: "m4m".into(),
                by: "x".into(),
                via: Via::Cli,
            },
            Timestamp("2026-10-21T20:00:00Z".into()),
        )
        .unwrap();
        task.waits_on = Some(row.to_owned());
        let rev = app.tasks.cache().revision() + 1;
        app.tasks.apply(StoreEvent::Changed(WatchEvent {
            key: StoreKey("task.m4m.tk-1".into()),
            rev: Rev(rev),
            value: Some(serde_json::to_vec(&task).unwrap()),
            at: None,
        }));
        let shown = call(
            app,
            Method::TaskShow(TaskTarget {
                key: "task.m4m.tk-1".into(),
            }),
        );
        shown["result"]["task"].clone()
    }

    #[test]
    fn a_task_waits_on_what_the_real_ask_rows_say() {
        let mut app = asks_app();
        // Open: waiting.
        assert_eq!(
            shown_waiting_on(&mut app, "gestore-lab.a1")["shown"],
            "waiting"
        );
        // Closed: the stored status comes back.
        let closed = shown_waiting_on(&mut app, "gestore-lab.a3");
        assert_eq!(closed["shown"], "open");
        assert!(closed["waits_on_missing"].is_null());
        // Answered: still waiting (the task has not picked the answer up).
        app.tasks.apply(changes(
            Mirror::Asks,
            &[event(
                "gestore-lab.a1",
                9,
                serde_json::json!({"status": "answered", "kind": "task"}),
            )],
        ));
        assert_eq!(
            shown_waiting_on(&mut app, "gestore-lab.a1")["shown"],
            "waiting"
        );
        // Dropped counts as closed.
        app.tasks.apply(changes(
            Mirror::Asks,
            &[event(
                "gestore-lab.a1",
                10,
                serde_json::json!({"status": "dropped", "kind": "task"}),
            )],
        ));
        assert_eq!(
            shown_waiting_on(&mut app, "gestore-lab.a1")["shown"],
            "open"
        );
        // A row the desk re-filed: the task follows `superseded_by`.
        app.tasks.apply(changes(
            Mirror::Asks,
            &[
                event(
                    "gestore-lab.old",
                    1,
                    serde_json::json!({"status": "closed", "superseded_by": "gestore-lab.new"}),
                ),
                event("gestore-lab.new", 1, serde_json::json!({"status": "open"})),
            ],
        ));
        assert_eq!(
            shown_waiting_on(&mut app, "gestore-lab.old")["shown"],
            "waiting"
        );
        // A row that is not there: flagged, not waiting.
        let missing = shown_waiting_on(&mut app, "gestore-lab.gone");
        assert_eq!(missing["shown"], "open");
        assert_eq!(missing["waits_on_missing"], true);
        // A row deleted in the bucket (a watch delete event) is missing too.
        app.tasks
            .apply(changes(Mirror::Asks, &[gone("gestore-lab.new", 12)]));
        assert_eq!(
            shown_waiting_on(&mut app, "gestore-lab.new")["waits_on_missing"],
            true
        );
        // Until the asks are read nothing is called missing.
        let mut fresh = self::app();
        let unread = shown_waiting_on(&mut fresh, "gestore-lab.gone");
        assert!(unread["waits_on_missing"].is_null());
        assert_eq!(fresh.tasks.ask_state("gestore-lab.gone"), AskRow::Unknown);
    }

    #[test]
    fn only_an_ask_a_task_waits_on_counts_as_a_change_for_readers() {
        let mut app = asks_app();
        shown_waiting_on(&mut app, "gestore-lab.a1");
        let moved = |app: &mut App, key: &str, rev: u64, status: &str| {
            app.tasks.apply(changes(
                Mirror::Asks,
                &[event(
                    key,
                    rev,
                    serde_json::json!({"status": status, "kind": "task"}),
                )],
            ))
        };
        assert!(
            !moved(&mut app, "gestore-lab.other", 5, "open"),
            "nobody waits on it"
        );
        assert!(
            !moved(&mut app, "gestore-lab.a1", 10, "answered"),
            "waiting either way"
        );
        assert!(
            moved(&mut app, "gestore-lab.a1", 11, "closed"),
            "waiting ends"
        );
    }

    fn with_pane(app: &mut App) -> String {
        app.state.workspaces = vec![crate::workspace::Workspace::test_new("work")];
        app.state.ensure_test_terminals();
        let pane = app.state.workspaces[0].tabs[0].root_pane;
        app.public_pane_id(0, pane).unwrap()
    }

    fn for_pane(pane: &str, state: &str) -> Method {
        Method::IssueList(IssueListParams {
            pane: Some(pane.into()),
            state: Some(state.into()),
            ..IssueListParams::default()
        })
    }

    fn resolved(app: &mut App, pane: &str, state: &str) -> serde_json::Value {
        for _ in 0..200 {
            let answer = call(app, for_pane(pane, state));
            if answer["result"]["repo"]["status"] != "resolving" {
                return answer;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the pane's remote was never read");
    }

    fn remote(host: &str, path: &str) -> crate::tasks::RepoRemote {
        crate::tasks::RepoRemote {
            host: host.into(),
            path: path.into(),
        }
    }

    #[test]
    fn issue_list_for_a_pane_shows_that_repos_issues_once_its_remote_is_read() {
        let (mut app, _) = mirrored_issues();
        let pane = with_pane(&mut app);
        app.repo_resolver = crate::tasks::RepoResolver::with_resolver(|_| {
            std::thread::sleep(std::time::Duration::from_millis(30));
            Some(remote("gitea.example", "AC/Worklink"))
        });
        // The first answer does not wait for the read.
        let first = call(&mut app, for_pane(&pane, "all"));
        assert_eq!(first["result"]["repo"]["status"], "resolving");
        assert_eq!(first["result"]["total"], 0);
        assert!(first["result"]["issues"].as_array().unwrap().is_empty());
        let done = resolved(&mut app, &pane, "all");
        let repo = &done["result"]["repo"];
        assert_eq!(repo["status"], "found");
        assert_eq!(repo["host"], "gitea.example");
        assert_eq!(repo["path"], "AC/Worklink");
        assert_eq!(repo["mirrored"], true, "case does not matter");
        assert_eq!(done["result"]["issues"].as_array().unwrap().len(), 3);
        assert_eq!(done["result"]["total"], 3);
    }

    #[test]
    fn issue_list_for_a_pane_in_another_repo_or_none_shows_nothing_and_says_why() {
        let (mut app, _) = mirrored_issues();
        let pane = with_pane(&mut app);
        // Another forge host with the same path is another repo.
        app.repo_resolver = crate::tasks::RepoResolver::with_resolver(|_| {
            Some(remote("github.com", "ac/worklink"))
        });
        let other_host = resolved(&mut app, &pane, "all");
        assert_eq!(other_host["result"]["repo"]["status"], "found");
        assert!(other_host["result"]["issues"]
            .as_array()
            .unwrap()
            .is_empty());
        app.repo_resolver = crate::tasks::RepoResolver::with_resolver(|_| {
            Some(remote("gitea.example", "ac/other"))
        });
        let other = resolved(&mut app, &pane, "all");
        assert!(
            other["result"]["repo"].get("mirrored").is_none(),
            "false is left out"
        );
        assert!(other["result"]["issues"].as_array().unwrap().is_empty());
        app.repo_resolver = crate::tasks::RepoResolver::with_resolver(|_| None);
        let none = resolved(&mut app, &pane, "all");
        assert_eq!(none["result"]["repo"]["status"], "no_remote");
        assert!(none["result"]["issues"].as_array().unwrap().is_empty());
        // Without a pane the answer has no repo and nothing is narrowed.
        let plain = call(&mut app, Method::IssueList(all()));
        assert!(plain["result"].get("repo").is_none());
        assert_eq!(plain["result"]["issues"].as_array().unwrap().len(), 3);
        let missing = call(&mut app, for_pane("w_nope:p_nope", "all"));
        assert_eq!(missing["error"]["code"], "pane_not_found");
    }

    #[test]
    fn closed_since_keeps_old_closed_issues_out_and_leaves_open_ones_alone() {
        let (mut app, _) = mirrored_issues();
        let mut since = |stamp: &str| {
            call(
                &mut app,
                Method::IssueList(IssueListParams {
                    state: Some("all".into()),
                    closed_since: Some(stamp.into()),
                    ..IssueListParams::default()
                }),
            )
        };
        // The closed issue was last touched at 2026-10-21T11:00:00Z.
        assert_eq!(since("2026-10-21T00:00:00Z")["result"]["total"], 3);
        let later = since("2026-10-22T00:00:00+02:00");
        assert_eq!(later["result"]["total"], 2, "only the closed one drops");
        assert!(later["result"]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .all(|issue| issue["state"] == "open"));
        let bad = since("last week");
        assert_eq!(bad["error"]["code"], "invalid_issue_list");
    }

    #[test]
    fn an_issue_assigned_to_acs_forge_account_is_marked_mine() {
        let (mut app, _) = mirrored_issues();
        let mine = |app: &mut App| -> Vec<bool> {
            let answer = call(app, Method::IssueList(all()));
            answer["result"]["issues"]
                .as_array()
                .unwrap()
                .iter()
                .map(|issue| issue.get("mine").is_some_and(|m| m == true))
                .collect()
        };
        assert_eq!(mine(&mut app), [false, false, false], "no mapping, no mark");
        app.task_config
            .forge_accounts
            .insert("gitea-lab".into(), "AC".into());
        assert_eq!(mine(&mut app), [false, true, false], "case does not matter");
        // The mapping is per forge: another forge's account is not ac's here.
        app.task_config.forge_accounts.clear();
        app.task_config
            .forge_accounts
            .insert("github-main".into(), "ac".into());
        assert_eq!(mine(&mut app), [false, false, false]);
    }
}
