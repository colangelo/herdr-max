//! `issue.list` and `ask.list`: read-only views of the buckets worklink and
//! the gestore fill (fork issue 241). They answer from the caches the store
//! thread keeps up to date and never wait on the bus.

use std::collections::BTreeMap;

use super::responses::{encode_error, encode_success};
use crate::api::schema::{
    AskInfo, AskListParams, IssueInfo, IssueListParams, IssueProjectInfo, IssueRefInfo, MirrorInfo,
    ResponseResult,
};
use crate::app::App;
use crate::tasks::{
    AskCache, AskRecord, IssueCache, IssueRecord, IssueRow, IssueState, Mirror, MirrorStatus, Rev,
};

fn mirror_info(status: MirrorStatus) -> MirrorInfo {
    MirrorInfo {
        loaded: status.loaded,
        detail: status.detail,
    }
}

pub(crate) fn issue_info(key: &str, row: &IssueRow) -> IssueInfo {
    let IssueRecord {
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
    }
}

/// Every project that has issues or a sync row, with how its last sync went.
/// A project with issues and no sync row is `synced: false`.
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
    };
    for (_, row) in cache.issues() {
        let r = &row.record.issue_ref;
        let entry = projects
            .entry((r.forge.clone(), r.forge_id.clone(), r.project_id))
            .or_insert_with(|| blank(&r.forge, &r.forge_id, r.project_id));
        entry.project_path = Some(r.project_path.clone());
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

pub(crate) fn list_issues(
    cache: &IssueCache,
    params: &IssueListParams,
) -> Result<Vec<IssueInfo>, String> {
    let wanted = match params.state.as_deref() {
        None | Some("open") => Some(IssueState::Open),
        Some("closed") => Some(IssueState::Closed),
        Some("all") => None,
        Some(other) => return Err(format!("state must be open, closed or all, not {other}")),
    };
    let mut issues: Vec<_> = cache
        .issues()
        .filter(|(_, row)| wanted.is_none_or(|state| row.record.state == state))
        .filter(|(_, row)| {
            params.project.as_deref().is_none_or(|project| {
                let r = &row.record.issue_ref;
                r.project_path == project || r.project_id.to_string() == project
            })
        })
        .map(|(key, row)| issue_info(key, row))
        .collect();
    issues.sort_by(|a, b| {
        let key = |i: &IssueInfo| {
            (
                i.issue_ref.forge.clone(),
                i.issue_ref.forge_id.clone(),
                i.issue_ref.project_id,
                i.issue_ref.number,
            )
        };
        key(a).cmp(&key(b))
    });
    Ok(issues)
}

pub(crate) fn list_asks(cache: &AskCache, params: &AskListParams) -> Vec<AskInfo> {
    let kind = params.kind.as_deref().unwrap_or("task");
    cache
        .rows()
        .filter(|(_, entry)| kind == "any" || entry.record.kind.as_deref() == Some(kind))
        .filter(|(_, entry)| {
            params.include_closed || !matches!(entry.record.status.as_str(), "closed" | "dropped")
        })
        .map(|(key, entry)| ask_info(key, entry.rev, &entry.record))
        .collect()
}

impl App {
    pub(super) fn handle_issue_list(&mut self, id: String, params: IssueListParams) -> String {
        let issues = match list_issues(self.tasks.issues(), &params) {
            Ok(issues) => issues,
            Err(message) => return encode_error(id, "invalid_issue_state", message),
        };
        encode_success(
            id,
            ResponseResult::IssueList {
                issues,
                projects: project_infos(self.tasks.issues()),
                mirror: mirror_info(self.tasks.mirror_status(Mirror::Issues)),
            },
        )
    }

    pub(super) fn handle_ask_list(&mut self, id: String, params: AskListParams) -> String {
        encode_success(
            id,
            ResponseResult::AskList {
                asks: list_asks(self.tasks.asks(), &params),
                mirror: mirror_info(self.tasks.mirror_status(Mirror::Asks)),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use crate::api::schema::{AskListParams, IssueListParams, Method, Request};
    use crate::app::App;
    use crate::config::Config;
    use crate::tasks::{
        derive, AskRow, HostId, Mirror, Origin, RunCache, Shown, StoreEvent, StoreKey, Task,
        TaskId, Timestamp, Via, WatchEvent,
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
            rev: crate::tasks::Rev(rev),
            value: Some(serde_json::to_vec(&doc).unwrap()),
            at: None,
        }
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
        app.tasks.apply(StoreEvent::MirrorLoaded {
            which: Mirror::Issues,
            events: rows
                .iter()
                .map(|(key, rev, doc)| event(key, *rev, doc.clone()))
                .collect(),
        });
        (app, rows)
    }

    #[test]
    fn issue_rows_equal_mirror_records() {
        let (mut app, rows) = mirrored_issues();
        let answer = call(
            &mut app,
            Method::IssueList(IssueListParams {
                state: Some("all".into()),
                ..IssueListParams::default()
            }),
        );
        let listed = answer["result"]["issues"].as_array().unwrap();
        assert_eq!(listed.len(), rows.len());
        for (listed, (key, rev, doc)) in listed.iter().zip(&rows) {
            assert_eq!(listed["key"], key.as_str());
            assert_eq!(listed["mirror_rev"], *rev, "the revision the watch saw");
            for field in ["state", "web_url", "author", "created_at", "updated_at"] {
                assert_eq!(listed[field], doc[field], "{key}.{field}");
            }
            assert_eq!(listed["source_rev"], doc["source_rev"]);
            assert_eq!(listed["last_sync"], doc["last_sync"]);
            // The whole IssueRef rides along, not only the number.
            assert_eq!(listed["ref"], doc["ref"], "{key}.ref");
            assert_eq!(listed["labels"], doc["labels"], "{key}.labels");
            assert_eq!(listed["assignees"], doc["assignees"], "{key}.assignees");
            assert!(listed["labels"].is_array() && listed["assignees"].is_array());
            // Non-ASCII text survives; a cut title is only a prefix of the forge's.
            let title = listed["title"].as_str().unwrap();
            assert!(
                doc["title"].as_str().unwrap().starts_with(title),
                "{key}.title"
            );
            assert_eq!(
                listed["truncated"].as_bool(),
                doc["truncated"].as_bool(),
                "{key}"
            );
        }
        assert!(listed[1]["title"].as_str().unwrap().contains("\u{65e5}"));
        assert!(listed[0]["truncated"].is_null(), "left out unless true");
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
        assert_eq!(bad["error"]["code"], "invalid_issue_state");

        // No sync row yet: the project is listed, not synced.
        let project = &open["result"]["projects"][0];
        assert_eq!(project["project_id"], 84);
        assert_eq!(project["synced"], false);
        assert!(project["last_ok"].is_null());
        app.tasks.apply(StoreEvent::MirrorChanged {
            which: Mirror::Issues,
            event: event(
                "sync.gitea.gitea-lab.84",
                5,
                serde_json::json!({
                    "project": {"ref": {"forge": "gitea", "forge_id": "gitea-lab",
                                        "project_id": 84, "project_path": "ac/worklink"}},
                    "status": "ok", "last_ok": "2026-10-21T12:00:00Z",
                    "last_attempt": "2026-10-21T12:00:01Z", "issues": 3
                }),
            ),
        });
        let synced = call(&mut app, Method::IssueList(IssueListParams::default()));
        let project = &synced["result"]["projects"][0];
        assert_eq!(project["synced"], true);
        assert_eq!(project["status"], "ok");
        assert_eq!(project["last_ok"], "2026-10-21T12:00:00Z");
        assert_eq!(synced["result"]["projects"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn issue_list_before_the_mirror_is_read_says_why() {
        let mut app = app();
        app.tasks.apply(StoreEvent::MirrorState {
            which: Mirror::Issues,
            detail: Some("the bucket is away".into()),
        });
        let answer = call(&mut app, Method::IssueList(IssueListParams::default()));
        assert_eq!(answer["result"]["mirror"]["loaded"], false);
        assert_eq!(answer["result"]["mirror"]["detail"], "the bucket is away");
        assert!(answer["result"]["issues"].as_array().unwrap().is_empty());
    }

    fn asks_app() -> App {
        let mut app = app();
        let row = |kind: &str, status: &str, title: &str| {
            serde_json::json!({"status": status, "kind": kind, "title": title,
                               "where": "herdr-max #241", "filed_by": "gestore-lab"})
        };
        app.tasks.apply(StoreEvent::MirrorLoaded {
            which: Mirror::Asks,
            events: vec![
                event("gestore-lab.a1", 2, row("task", "open", "do the thing")),
                event("gestore-lab.a2", 3, row("decision", "open", "pick one")),
                event("gestore-lab.a3", 4, row("task", "closed", "done thing")),
            ],
        });
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
    }

    #[test]
    fn a_task_waits_on_what_the_real_ask_rows_say() {
        let mut app = asks_app();
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
        let runs = RunCache::new();
        let now = SystemTime::now();
        let mut shown = |app: &App, row: &str| {
            task.waits_on = Some(row.to_owned());
            derive(&task, &runs, |row| app.tasks.ask_state(row), now)
        };
        // Open: waiting.
        assert_eq!(shown(&app, "gestore-lab.a1").status, Shown::Waiting);
        // Closed: the stored status comes back.
        let closed = shown(&app, "gestore-lab.a3");
        assert_eq!(closed.status, Shown::Open);
        assert!(!closed.waits_on_missing);
        // Answered: still waiting (the task has not picked the answer up).
        app.tasks.apply(StoreEvent::MirrorChanged {
            which: Mirror::Asks,
            event: event(
                "gestore-lab.a1",
                9,
                serde_json::json!({"status": "answered", "kind": "task"}),
            ),
        });
        assert_eq!(shown(&app, "gestore-lab.a1").status, Shown::Waiting);
        // A row that is not there: flagged, not waiting.
        let missing = shown(&app, "gestore-lab.gone");
        assert_eq!(missing.status, Shown::Open);
        assert!(missing.waits_on_missing);
        // Until the asks are read nothing is called missing.
        let fresh = self::app();
        assert_eq!(fresh.tasks.ask_state("gestore-lab.gone"), AskRow::Unknown);
    }
}
