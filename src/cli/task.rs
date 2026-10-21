//! `herdr task`: the shared task store from the command line (fork issue 241).
//!
//! A write answers "pending" at once; this command then asks `task.write_status`
//! until the store has taken it, up to a timeout, and prints "pending sync" if
//! it has not. A server that has no `task.*` methods (an older one) is spoken to
//! in `todo.*` instead, for the verbs a todo can do.

use std::time::{Duration, Instant};

use clap::{Arg, ArgAction, ArgMatches, Command};
use serde_json::Value;

use super::projection::ParseError;
use crate::api::schema::{
    Method, Request, TaskAddParams, TaskBindParams, TaskFields, TaskListParams,
    TaskMoveStatusParams, TaskPriorityInfo, TaskTarget, TaskUpdateParams, TaskWriteStatusParams,
    TodoAddParams, TodoListParams, TodoRemoveParams, TodoUpdateParams,
};
use crate::terminal::todo::TodoPriority;

/// How long a write is waited for before it is reported "pending sync".
const DEFAULT_WAIT: Duration = Duration::from_millis(3000);
const POLL: Duration = Duration::from_millis(50);

fn key_arg() -> Arg {
    Arg::new("key").value_name("KEY").required(true)
}

fn common(command: Command) -> Command {
    command
        .arg(Arg::new("json").long("json").action(ArgAction::SetTrue))
        .arg(
            Arg::new("no-wait")
                .long("no-wait")
                .action(ArgAction::SetTrue)
                .help("Do not wait for the store to take the write"),
        )
        .arg(
            Arg::new("wait-ms")
                .long("wait-ms")
                .value_name("MS")
                .help("How long to wait for the store (default 3000)"),
        )
        .arg(
            Arg::new("by")
                .long("by")
                .value_name("NAME")
                .help("Who is asking, recorded on the task (default: $HERDR_AGENT_NAME)"),
        )
}

fn fields(command: Command) -> Command {
    command
        .arg(Arg::new("body").long("body").value_name("TEXT"))
        .arg(Arg::new("priority").long("priority").value_name("PRIORITY"))
        .arg(Arg::new("horizon").long("horizon").value_name("HORIZON"))
        .arg(
            Arg::new("owner")
                .long("owner")
                .value_name("SESSION")
                .help("The agent session that owns it; an empty value clears"),
        )
        .arg(Arg::new("waits-on").long("waits-on").value_name("ROW"))
}

fn pane_args(command: Command) -> Command {
    command
        .arg(Arg::new("pane").long("pane").value_name("ID"))
        .arg(
            Arg::new("current")
                .long("current")
                .action(ArgAction::SetTrue),
        )
}

pub(crate) fn command() -> Command {
    Command::new("task")
        .override_usage("herdr task [COMMAND]")
        .about("Manage tasks kept in the shared task store")
        .subcommand_required(true)
        .subcommand(common(pane_args(fields(
            Command::new("add")
                .about("Add a task")
                .override_usage("herdr task add <TITLE> [OPTIONS]")
                .arg(Arg::new("title").value_name("TITLE").required(true)),
        ))))
        .subcommand(common(pane_args(
            Command::new("list")
                .about("List tasks")
                .arg(Arg::new("owner").long("owner").value_name("SESSION"))
                .arg(
                    Arg::new("closed")
                        .long("closed")
                        .action(ArgAction::SetTrue)
                        .help("Include done and dropped tasks"),
                ),
        )))
        .subcommand(common(
            Command::new("show").about("Show a task").arg(key_arg()),
        ))
        .subcommand(common(fields(
            Command::new("update")
                .about("Edit a task")
                .arg(key_arg())
                .arg(Arg::new("title").long("title").value_name("TEXT"))
                .arg(Arg::new("review").long("review").value_name("TEXT"))
                .arg(
                    Arg::new("rev")
                        .long("rev")
                        .value_name("N")
                        .help("The revision you read (default: the current one)"),
                ),
        )))
        .subcommand(status_command("done", "Mark a task done"))
        .subcommand(status_command("drop", "Drop a task"))
        .subcommand(status_command("reopen", "Reopen a done or dropped task"))
        .subcommand(common(pane_args(
            Command::new("bind")
                .about("Put a task on a pane, or take it off")
                .arg(key_arg())
                .arg(
                    Arg::new("clear")
                        .long("clear")
                        .action(ArgAction::SetTrue)
                        .help("Remove the binding"),
                )
                .arg(
                    Arg::new("related")
                        .long("related")
                        .action(ArgAction::SetTrue)
                        .help("Bind the related pane, shown by label only"),
                )
                .arg(Arg::new("rev").long("rev").value_name("N")),
        )))
}

fn status_command(name: &'static str, about: &'static str) -> Command {
    common(
        Command::new(name)
            .about(about)
            .arg(key_arg())
            .arg(Arg::new("reason").long("reason").value_name("TEXT"))
            .arg(Arg::new("rev").long("rev").value_name("N")),
    )
}

/// What a command asks of the server, once parsed.
struct Plan {
    verb: &'static str,
    request: Request,
    /// For an edit: read the task first, for the revision to write against.
    needs_rev_of: Option<String>,
    json: bool,
    wait: Option<Duration>,
    /// What a todo-only server is asked instead, when there is a way.
    pane: Option<String>,
}

/// A value the verb may not define at all (`--pane` on `show`): then it is absent.
fn text(matches: &ArgMatches, name: &str) -> Option<String> {
    matches.try_get_one::<String>(name).ok().flatten().cloned()
}

fn flag(matches: &ArgMatches, name: &str) -> bool {
    matches
        .try_get_one::<bool>(name)
        .ok()
        .flatten()
        .copied()
        .unwrap_or(false)
}

fn number(matches: &ArgMatches, name: &str) -> Result<Option<u64>, ParseError> {
    match matches.try_get_one::<String>(name).ok().flatten() {
        None => Ok(None),
        Some(value) => value
            .parse()
            .map(Some)
            .map_err(|_| ParseError::Usage(format!("--{name} needs a number, not {value}"))),
    }
}

fn priority(matches: &ArgMatches) -> Result<Option<TaskPriorityInfo>, ParseError> {
    Ok(
        match matches.get_one::<String>("priority").map(String::as_str) {
            None => None,
            Some("low") => Some(TaskPriorityInfo::Low),
            Some("normal") => Some(TaskPriorityInfo::Normal),
            Some("high") => Some(TaskPriorityInfo::High),
            Some(other) => {
                return Err(ParseError::Usage(format!(
                    "invalid priority: {other} (expected high, normal, or low)"
                )))
            }
        },
    )
}

fn task_fields(matches: &ArgMatches) -> Result<TaskFields, ParseError> {
    Ok(TaskFields {
        title: text(matches, "title"),
        body: text(matches, "body"),
        priority: priority(matches)?,
        horizon: text(matches, "horizon"),
        owner: text(matches, "owner"),
        waits_on: text(matches, "waits-on"),
        review: text(matches, "review"),
    })
}

/// The pane a command names: `--pane`, or the caller's own with `--current`,
/// or (when `default_current`) the caller's own anyway.
fn pane_of(
    matches: &ArgMatches,
    caller: Option<&str>,
    default_current: bool,
) -> Result<Option<String>, ParseError> {
    if let Some(pane) = text(matches, "pane") {
        return Ok(Some(pane));
    }
    if flag(matches, "current") || default_current {
        return match caller {
            Some(pane) => Ok(Some(pane.to_owned())),
            None if flag(matches, "current") => Err(ParseError::Usage(
                "no pane target: run inside a herdr pane or pass --pane <pane_id>".into(),
            )),
            None => Ok(None),
        };
    }
    Ok(None)
}

fn plan(args: &[String], caller: Option<&str>) -> Result<Plan, ParseError> {
    let matches = super::projection::parse(command().bin_name("herdr task"), args, |matches| {
        Ok(matches.clone())
    })?;
    let (verb, matches) = matches
        .subcommand()
        .ok_or_else(|| ParseError::Usage("missing task command".into()))?;
    let by = text(matches, "by").or_else(|| {
        std::env::var("HERDR_AGENT_NAME")
            .ok()
            .filter(|name| !name.trim().is_empty())
    });
    let wait = if flag(matches, "no-wait") {
        None
    } else {
        Some(
            number(matches, "wait-ms")?
                .map(Duration::from_millis)
                .unwrap_or(DEFAULT_WAIT),
        )
    };
    let json = flag(matches, "json");
    let request = |method| Request {
        id: format!("cli:task:{verb}"),
        method,
    };
    let key = text(matches, "key");
    let (verb, method, needs_rev_of, pane) = match verb {
        "add" => {
            let pane = pane_of(matches, caller, true)?;
            let fields = task_fields(matches)?;
            (
                "add",
                Method::TaskAdd(TaskAddParams {
                    title: text(matches, "title").unwrap_or_default(),
                    body: fields.body,
                    priority: fields.priority,
                    horizon: fields.horizon,
                    owner: fields.owner,
                    pane_id: pane.clone(),
                    waits_on: fields.waits_on,
                    refs: Vec::new(),
                    by,
                    via: Some("cli".into()),
                }),
                None,
                pane,
            )
        }
        "list" => {
            let pane = pane_of(matches, caller, false)?;
            (
                "list",
                Method::TaskList(TaskListParams {
                    pane_id: pane.clone(),
                    owner: text(matches, "owner"),
                    include_closed: flag(matches, "closed"),
                }),
                None,
                pane,
            )
        }
        "show" => (
            "show",
            Method::TaskShow(TaskTarget {
                key: key.clone().unwrap_or_default(),
            }),
            None,
            None,
        ),
        "update" => {
            let rev = number(matches, "rev")?;
            let fields = task_fields(matches)?;
            let key = key.clone().unwrap_or_default();
            (
                "update",
                Method::TaskUpdate(TaskUpdateParams {
                    key: key.clone(),
                    expected_rev: rev.unwrap_or(0),
                    fields,
                }),
                rev.is_none().then_some(key),
                None,
            )
        }
        verb @ ("done" | "drop" | "reopen") => {
            let params = TaskMoveStatusParams {
                key: key.clone().unwrap_or_default(),
                expected_rev: number(matches, "rev")?,
                by,
                reason: text(matches, "reason"),
            };
            let (name, method) = match verb {
                "done" => ("done", Method::TaskDone(params)),
                "drop" => ("drop", Method::TaskDrop(params)),
                _ => ("reopen", Method::TaskReopen(params)),
            };
            (name, method, None, pane_of(matches, caller, false)?)
        }
        "bind" => {
            let rev = number(matches, "rev")?;
            let key = key.clone().unwrap_or_default();
            let clear = flag(matches, "clear");
            let pane = pane_of(matches, caller, !clear)?;
            if !clear && pane.is_none() {
                return Err(ParseError::Usage(
                    "no pane target: run inside a herdr pane, pass --pane <pane_id>, or use --clear"
                        .into(),
                ));
            }
            (
                "bind",
                Method::TaskBind(TaskBindParams {
                    key: key.clone(),
                    expected_rev: rev.unwrap_or(0),
                    pane_id: pane,
                    clear,
                    related: flag(matches, "related"),
                }),
                rev.is_none().then_some(key),
                None,
            )
        }
        _ => return Err(ParseError::Usage("unknown task command".into())),
    };
    let is_write = !matches!(verb, "list" | "show");
    Ok(Plan {
        verb,
        request: request(method),
        needs_rev_of,
        json,
        wait: if is_write { wait } else { None },
        pane,
    })
}

pub(super) fn run_task_command(args: &[String]) -> std::io::Result<i32> {
    let plan = match plan(args, super::target::caller_pane_id().as_deref()) {
        Ok(plan) => plan,
        Err(ParseError::Clap(error)) => return super::print_clap_error(&error),
        Err(ParseError::Usage(message)) => {
            eprintln!("{message}");
            return Ok(2);
        }
        Err(ParseError::Io(error)) => return Err(error),
    };
    run_plan(plan)
}

fn error_code(response: &Value) -> Option<&str> {
    response["error"]["code"].as_str()
}

fn run_plan(mut plan: Plan) -> std::io::Result<i32> {
    // An edit is written against the revision the caller read.
    if let Some(key) = plan.needs_rev_of.take() {
        let shown = super::send_request(&Request {
            id: "cli:task:rev".into(),
            method: Method::TaskShow(TaskTarget { key }),
        })?;
        if error_code(&shown) == Some("unknown_method") {
            return todo_fallback(&plan);
        }
        if shown.get("error").is_some() {
            return super::print_response(&shown);
        }
        let rev = shown["result"]["task"]["rev"].as_u64().unwrap_or(0);
        match &mut plan.request.method {
            Method::TaskUpdate(params) => params.expected_rev = rev,
            Method::TaskBind(params) => params.expected_rev = rev,
            _ => {}
        }
    }
    let mut response = super::send_request(&plan.request)?;
    if error_code(&response) == Some("unknown_method") {
        return todo_fallback(&plan);
    }
    if response.get("error").is_some() {
        return super::print_response(&response);
    }
    let mut code = 0;
    let mut outcome = None;
    if let (Some(wait), Some(write_id)) = (plan.wait, response["result"]["write_id"].as_str()) {
        outcome = wait_for_outcome(write_id, wait)?;
        match outcome.as_ref().map(|o| o["status"].as_str()) {
            Some(Some("applied")) => {}
            Some(_) => code = 1,
            None => {}
        }
        response["result"]["outcome"] = outcome.clone().unwrap_or(Value::Null);
        response["result"]["synced"] = Value::Bool(outcome.is_some());
    }
    if plan.json {
        println!("{}", serde_json::to_string(&response).unwrap());
        return Ok(code);
    }
    print_human(&plan, &response, outcome.as_ref(), plan.wait.is_some());
    Ok(code)
}

/// Ask `task.write_status` until the write has an outcome or `wait` is up.
fn wait_for_outcome(write_id: &str, wait: Duration) -> std::io::Result<Option<Value>> {
    let deadline = Instant::now() + wait;
    loop {
        let status = super::send_request(&Request {
            id: "cli:task:write_status".into(),
            method: Method::TaskWriteStatus(TaskWriteStatusParams {
                write_id: write_id.to_owned(),
            }),
        })?;
        if status.get("error").is_some() {
            return Ok(None);
        }
        let result = &status["result"];
        if result["pending"].as_bool() == Some(false) {
            return Ok(result.get("outcome").cloned());
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        // Never longer than the time that is left.
        std::thread::sleep(POLL.min(deadline.saturating_duration_since(Instant::now())));
    }
}

fn task_line(task: &Value) -> String {
    let mut line = format!(
        "{}  {:<11}  {:<6}  {:<14}  {}",
        task["key"].as_str().unwrap_or(""),
        task["shown"].as_str().unwrap_or("?"),
        task["priority"].as_str().unwrap_or("normal"),
        task["owner"].as_str().unwrap_or("-"),
        task["title"].as_str().unwrap_or("")
    );
    if task["pending"].as_bool() == Some(true) {
        line.push_str("  [pending sync]");
    }
    if task["conflict"].as_bool() == Some(true) {
        line.push_str("  [conflict]");
    }
    line
}

fn print_human(plan: &Plan, response: &Value, outcome: Option<&Value>, waited: bool) {
    let result = &response["result"];
    match plan.verb {
        "list" => {
            let Some(tasks) = result["tasks"].as_array().filter(|tasks| !tasks.is_empty()) else {
                println!("no tasks");
                return;
            };
            for task in tasks {
                println!("{}", task_line(task));
            }
            let store = &result["store"];
            if store["state"]
                .as_str()
                .is_some_and(|state| state != "ready")
            {
                println!(
                    "store: {}{}",
                    store["state"].as_str().unwrap_or("?"),
                    store["detail"]
                        .as_str()
                        .map(|detail| format!(" ({detail})"))
                        .unwrap_or_default()
                );
            }
        }
        "show" => {
            let task = &result["task"];
            println!("{}", task_line(task));
            for (label, field) in [
                ("status", "status"),
                ("rev", "rev"),
                ("horizon", "horizon"),
                ("waits on", "waits_on"),
                ("created", "created_at"),
                ("updated", "updated_at"),
            ] {
                match &task[field] {
                    Value::Null => {}
                    Value::String(value) => println!("  {label}: {value}"),
                    other => println!("  {label}: {other}"),
                }
            }
            if let Some(body) = task["body"].as_str() {
                println!("\n{body}");
            }
        }
        verb => {
            let task = &result["task"];
            let past = match verb {
                "add" => "added",
                "done" => "completed",
                "drop" => "dropped",
                "reopen" => "reopened",
                "bind" => "bound",
                _ => "updated",
            };
            println!(
                "{past} task {}: {}",
                task["key"].as_str().unwrap_or(""),
                task["title"].as_str().unwrap_or("")
            );
            if !waited {
                println!("pending sync");
                return;
            }
            match outcome {
                Some(outcome) => match outcome["status"].as_str() {
                    Some("applied") => {
                        println!("saved (revision {})", outcome["rev"].as_u64().unwrap_or(0))
                    }
                    Some("conflict" | "exists") => eprintln!(
                        "conflict: the task changed under you; `herdr task show` it and try again"
                    ),
                    Some(other) => eprintln!(
                        "{other}: {}",
                        outcome["message"]
                            .as_str()
                            .unwrap_or("the write did not land")
                    ),
                    None => {}
                },
                None => println!("pending sync"),
            }
        }
    }
}

fn todo_priority(priority: Option<TaskPriorityInfo>) -> Option<TodoPriority> {
    match priority? {
        TaskPriorityInfo::High => Some(TodoPriority::High),
        TaskPriorityInfo::Low => Some(TodoPriority::Low),
        _ => Some(TodoPriority::Normal),
    }
}

/// The server has no `task.*`: do what a todo can, and say so.
fn todo_fallback(plan: &Plan) -> std::io::Result<i32> {
    let note = "note: this server has no task store; using pane todos";
    let need_pane = || {
        plan.pane.clone().ok_or_else(|| {
            eprintln!("{note}\nno pane target: run inside a herdr pane or pass --pane <pane_id>");
            2
        })
    };
    let (method, past) = match &plan.request.method {
        Method::TaskAdd(params) => {
            let Ok(pane_id) = need_pane() else {
                return Ok(2);
            };
            (
                Method::TodoAdd(TodoAddParams {
                    pane_id,
                    text: params.title.clone(),
                    priority: todo_priority(params.priority),
                    link_pane_id: None,
                }),
                "added",
            )
        }
        Method::TaskList(_) => (
            Method::TodoList(TodoListParams {
                pane_id: plan.pane.clone(),
            }),
            "listed",
        ),
        Method::TaskDone(params) | Method::TaskReopen(params) | Method::TaskDrop(params) => {
            let (Ok(id), Ok(pane_id)) = (params.key.parse::<u64>(), need_pane()) else {
                eprintln!(
                    "{note}\nname the todo by its number and pass --pane <pane_id> (or run in its pane)"
                );
                return Ok(2);
            };
            match &plan.request.method {
                Method::TaskDone(_) => (
                    Method::TodoUpdate(TodoUpdateParams {
                        pane_id,
                        id,
                        done: Some(true),
                        ..TodoUpdateParams::default()
                    }),
                    "completed",
                ),
                Method::TaskReopen(_) => (
                    Method::TodoUpdate(TodoUpdateParams {
                        pane_id,
                        id,
                        done: Some(false),
                        ..TodoUpdateParams::default()
                    }),
                    "reopened",
                ),
                _ => (
                    Method::TodoRemove(TodoRemoveParams { pane_id, id }),
                    "removed",
                ),
            }
        }
        _ => {
            eprintln!("{note}\nthis server cannot do `herdr task {}`", plan.verb);
            return Ok(1);
        }
    };
    eprintln!("{note}");
    let response = super::send_request(&Request {
        id: format!("cli:todo:{}", plan.verb),
        method,
    })?;
    if plan.json || response.get("error").is_some() {
        return super::print_response(&response);
    }
    if let Some(todos) = response["result"]["todos"].as_array() {
        if todos.is_empty() {
            println!("no todos");
        }
        for todo in todos {
            println!(
                "{} {:>4}  {}",
                todo["pane_id"].as_str().unwrap_or(""),
                todo["id"].as_u64().unwrap_or(0),
                todo["text"].as_str().unwrap_or("")
            );
        }
    } else {
        println!("{past} todo");
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::cli::fork_inventory_tests::Capture;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    fn method_json(plan: &Plan) -> Value {
        serde_json::to_value(&plan.request.method).unwrap()
    }

    #[test]
    fn add_defaults_to_the_current_pane_and_carries_who_and_how() {
        let plan = plan(
            &args(&[
                "add",
                "write the store",
                "--priority",
                "high",
                "--by",
                "herdr-wrk",
            ]),
            Some("w1:p2"),
        )
        .unwrap();
        assert_eq!(
            method_json(&plan),
            json!({"method": "task.add", "params": {
                "title": "write the store", "priority": "high",
                "pane_id": "w1:p2", "by": "herdr-wrk", "via": "cli"
            }})
        );
        let outside = self::plan(&args(&["add", "no pane"]), None).unwrap();
        assert!(method_json(&outside)["params"].get("pane_id").is_none());
    }

    #[test]
    fn list_is_every_pane_unless_one_is_named() {
        let all = plan(&args(&["list"]), Some("w1:p2")).unwrap();
        assert_eq!(
            method_json(&all),
            json!({"method": "task.list", "params": {}})
        );
        let here = plan(&args(&["list", "--current", "--closed"]), Some("w1:p2")).unwrap();
        assert_eq!(
            method_json(&here),
            json!({"method": "task.list", "params": {"pane_id": "w1:p2", "include_closed": true}})
        );
        assert!(matches!(
            plan(&args(&["list", "--current"]), None),
            Err(ParseError::Usage(_))
        ));
    }

    #[test]
    fn an_edit_reads_the_revision_first_unless_it_is_given() {
        let read = plan(&args(&["update", "task.m4m.tk-1", "--title", "new"]), None).unwrap();
        assert_eq!(read.needs_rev_of.as_deref(), Some("task.m4m.tk-1"));
        let given = plan(
            &args(&["update", "task.m4m.tk-1", "--title", "new", "--rev", "4"]),
            None,
        )
        .unwrap();
        assert!(given.needs_rev_of.is_none());
        assert_eq!(
            method_json(&given),
            json!({"method": "task.update", "params": {
                "key": "task.m4m.tk-1", "expected_rev": 4, "title": "new"
            }})
        );
    }

    #[test]
    fn a_bad_priority_or_revision_is_a_usage_error() {
        assert!(matches!(
            plan(&args(&["add", "x", "--priority", "urgent"]), None),
            Err(ParseError::Usage(_))
        ));
        assert!(matches!(
            plan(&args(&["done", "k", "--rev", "soon"]), None),
            Err(ParseError::Usage(_))
        ));
    }

    #[test]
    fn done_drop_and_reopen_map_to_their_methods() {
        for (verb, method) in [
            ("done", "task.done"),
            ("drop", "task.drop"),
            ("reopen", "task.reopen"),
        ] {
            let plan = plan(&args(&[verb, "task.m4m.tk-1", "--reason", "why"]), None).unwrap();
            let json = method_json(&plan);
            assert_eq!(json["method"], method);
            assert_eq!(json["params"]["reason"], "why");
        }
    }

    #[test]
    fn cli_falls_back_when_task_methods_missing() {
        // An older server answers `unknown_method` to task.add; the command then
        // asks for a pane todo instead, and says so.
        let capture = Capture::begin();
        capture.queue_response(json!({
            "id": "cli:task:add",
            "error": {"code": "unknown_method", "message": "unknown method: task.add"}
        }));
        let plan = plan(
            &args(&["add", "look at this", "--priority", "low"]),
            Some("w1:p2"),
        )
        .unwrap();
        assert_eq!(run_plan(plan).unwrap(), 0);
        let sent = capture.take();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0]["method"], "task.add");
        assert_eq!(
            sent[1],
            json!({"method": "todo.add", "params": {
                "pane_id": "w1:p2", "text": "look at this", "priority": "low"
            }})
        );
    }

    #[test]
    fn a_verb_a_todo_cannot_do_is_refused_on_an_older_server() {
        let capture = Capture::begin();
        capture.queue_response(json!({
            "id": "cli:task:show",
            "error": {"code": "unknown_method", "message": "unknown method: task.show"}
        }));
        let plan = plan(&args(&["show", "task.m4m.tk-1"]), None).unwrap();
        assert_eq!(run_plan(plan).unwrap(), 1);
        assert_eq!(capture.take().len(), 1, "nothing else was sent");
    }

    #[test]
    fn a_write_waits_for_its_outcome_and_a_slow_one_is_pending_sync() {
        let capture = Capture::begin();
        capture.queue_response(json!({"id": "x", "result": {
            "type": "task", "write_id": "w1", "pending": true,
            "task": {"key": "task.m4m.tk-1", "title": "t"}
        }}));
        capture.queue_response(json!({"id": "x", "result": {
            "type": "task_write", "write_id": "w1", "pending": false, "saved": false,
            "outcome": {"status": "applied", "rev": 3}
        }}));
        let done = plan(&args(&["done", "task.m4m.tk-1"]), None).unwrap();
        assert_eq!(run_plan(done).unwrap(), 0);
        let sent = capture.take();
        assert_eq!(sent[1]["method"], "task.write_status");
        drop(capture);

        // The store never answers: the command gives up politely, exit 0.
        let capture = Capture::begin();
        capture.queue_response(json!({"id": "x", "result": {
            "type": "task", "write_id": "w2", "pending": true,
            "task": {"key": "task.m4m.tk-1", "title": "t"}
        }}));
        capture.set_default(json!({"type": "task_write", "write_id": "w2",
                                   "pending": true, "saved": true}));
        let slow = plan(&args(&["done", "task.m4m.tk-1", "--wait-ms", "120"]), None).unwrap();
        assert_eq!(run_plan(slow).unwrap(), 0);
        drop(capture);
    }

    #[test]
    fn a_conflict_outcome_exits_nonzero() {
        let capture = Capture::begin();
        capture.queue_response(json!({"id": "x", "result": {
            "type": "task", "write_id": "w3", "pending": true,
            "task": {"key": "task.m4m.tk-1", "title": "t"}
        }}));
        capture.queue_response(json!({"id": "x", "result": {
            "type": "task_write", "write_id": "w3", "pending": false, "saved": false,
            "outcome": {"status": "conflict"}
        }}));
        let plan = plan(&args(&["done", "task.m4m.tk-1"]), None).unwrap();
        assert_eq!(run_plan(plan).unwrap(), 1);
    }
}
