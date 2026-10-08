//! `herdr todo` — the CLI face of the per-pane todo store.
//!
//! Target resolution mirrors `herdr pane current`: an explicit `--pane`, an
//! explicit `--current`, or the `HERDR_PANE_ID` the server exports into every
//! pane. The environment default is the point of the feature — an agent about
//! to exit records its next steps with `herdr todo add "..."` and never has to
//! know its own pane id.

use crate::api::schema::{
    Method, Request, TodoAddParams, TodoClearParams, TodoListParams, TodoRemoveParams,
    TodoUpdateParams,
};
use crate::terminal::todo::TodoPriority;

const TODO_ADD_USAGE: &str = "usage: herdr todo add <text> [--pane ID|--current] [--priority high|normal|low] [--link <target>] [--json]";
const TODO_LIST_USAGE: &str = "usage: herdr todo list [--pane ID|--current|--all] [--json]";
const TODO_DONE_USAGE: &str = "usage: herdr todo done <id> [--pane ID|--current] [--json]";
const TODO_UNDONE_USAGE: &str = "usage: herdr todo undone <id> [--pane ID|--current] [--json]";
const TODO_EDIT_USAGE: &str = "usage: herdr todo edit <id> [--text TEXT] [--priority high|normal|low] [--link <target>|--unlink] [--pane ID|--current] [--json]";
const TODO_RM_USAGE: &str = "usage: herdr todo rm <id> [--pane ID|--current] [--json]";
const TODO_CLEAR_USAGE: &str = "usage: herdr todo clear [--done] [--pane ID|--current] [--json]";

const NO_TARGET_MESSAGE: &str = "no pane target: run inside a herdr pane or pass --pane <pane_id>";
const NO_LIST_TARGET_MESSAGE: &str =
    "no pane target: run inside a herdr pane, pass --pane <pane_id>, or use --all";

pub(super) fn run_todo_command(args: &[String]) -> std::io::Result<i32> {
    let Some(subcommand) = args.first().map(|arg| arg.as_str()) else {
        print_todo_help();
        return Ok(2);
    };

    match subcommand {
        "add" => todo_add(&args[1..]),
        "list" => todo_list(&args[1..]),
        "done" => todo_set_done(&args[1..], true, TODO_DONE_USAGE),
        "undone" => todo_set_done(&args[1..], false, TODO_UNDONE_USAGE),
        "edit" => todo_edit(&args[1..]),
        "rm" => todo_remove(&args[1..]),
        "clear" => todo_clear(&args[1..]),
        "help" | "--help" | "-h" => {
            print_todo_help();
            Ok(0)
        }
        _ => {
            print_todo_help();
            Ok(2)
        }
    }
}

/// The pane the calling process runs in, as exported by the server. Blank
/// values are treated as absent so a stale empty export cannot win over
/// `--pane`.
fn env_pane_id() -> Option<String> {
    std::env::var("HERDR_PANE_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn todo_add(args: &[String]) -> std::io::Result<i32> {
    let parsed = match parse_todo_add_args(args, env_pane_id().as_deref()) {
        Ok(parsed) => parsed,
        Err(error) => return Ok(report_arg_error(error, TODO_ADD_USAGE)),
    };

    let response = super::send_request(&Request {
        id: "cli:todo:add".into(),
        method: Method::TodoAdd(TodoAddParams {
            pane_id: parsed.pane_id,
            text: parsed.text,
            priority: parsed.priority,
            link_pane_id: parsed.link_pane_id,
        }),
    })?;
    print_todo_mutation(&response, parsed.json, "added")
}

fn todo_list(args: &[String]) -> std::io::Result<i32> {
    let parsed = match parse_todo_list_args(args, env_pane_id().as_deref()) {
        Ok(parsed) => parsed,
        Err(error) => return Ok(report_arg_error(error, TODO_LIST_USAGE)),
    };

    // A scoped listing already names its pane on the command line; only the
    // whole-session listing needs a pane column.
    let show_pane = parsed.pane_id.is_none();
    let response = super::send_request(&Request {
        id: "cli:todo:list".into(),
        method: Method::TodoList(TodoListParams {
            pane_id: parsed.pane_id,
        }),
    })?;
    if parsed.json || response.get("error").is_some() {
        return super::print_response(&response);
    }

    let todos = response["result"]["todos"].as_array();
    let Some(todos) = todos.filter(|list| !list.is_empty()) else {
        println!("no todos");
        return Ok(0);
    };

    for todo in todos {
        println!("{}", format_todo_line(todo, show_pane));
    }
    let outstanding = todos
        .iter()
        .filter(|todo| !todo["done"].as_bool().unwrap_or(false))
        .count();
    if outstanding > 0 {
        println!("{outstanding} outstanding");
    }
    Ok(0)
}

fn todo_set_done(args: &[String], done: bool, usage: &str) -> std::io::Result<i32> {
    let parsed = match parse_todo_id_args(args, env_pane_id().as_deref()) {
        Ok(parsed) => parsed,
        Err(error) => return Ok(report_arg_error(error, usage)),
    };

    let response = super::send_request(&Request {
        id: if done {
            "cli:todo:done".into()
        } else {
            "cli:todo:undone".into()
        },
        method: Method::TodoUpdate(TodoUpdateParams {
            pane_id: parsed.pane_id,
            id: parsed.id,
            done: Some(done),
            ..TodoUpdateParams::default()
        }),
    })?;
    print_todo_mutation(
        &response,
        parsed.json,
        if done { "completed" } else { "reopened" },
    )
}

fn todo_edit(args: &[String]) -> std::io::Result<i32> {
    let parsed = match parse_todo_edit_args(args, env_pane_id().as_deref()) {
        Ok(parsed) => parsed,
        Err(error) => return Ok(report_arg_error(error, TODO_EDIT_USAGE)),
    };

    let response = super::send_request(&Request {
        id: "cli:todo:edit".into(),
        method: Method::TodoUpdate(TodoUpdateParams {
            pane_id: parsed.pane_id,
            id: parsed.id,
            text: parsed.text,
            done: None,
            priority: parsed.priority,
            link_pane_id: parsed.link_pane_id,
            clear_link: parsed.clear_link,
        }),
    })?;
    print_todo_mutation(&response, parsed.json, "updated")
}

fn todo_remove(args: &[String]) -> std::io::Result<i32> {
    let parsed = match parse_todo_id_args(args, env_pane_id().as_deref()) {
        Ok(parsed) => parsed,
        Err(error) => return Ok(report_arg_error(error, TODO_RM_USAGE)),
    };

    let pane_id = parsed.pane_id.clone();
    let response = super::send_request(&Request {
        id: "cli:todo:rm".into(),
        method: Method::TodoRemove(TodoRemoveParams {
            pane_id: parsed.pane_id,
            id: parsed.id,
        }),
    })?;
    if parsed.json || response.get("error").is_some() {
        return super::print_response(&response);
    }

    println!("removed todo {} from {pane_id}", parsed.id);
    Ok(0)
}

fn todo_clear(args: &[String]) -> std::io::Result<i32> {
    let parsed = match parse_todo_clear_args(args, env_pane_id().as_deref()) {
        Ok(parsed) => parsed,
        Err(error) => return Ok(report_arg_error(error, TODO_CLEAR_USAGE)),
    };

    let pane_id = parsed.pane_id.clone();
    let response = super::send_request(&Request {
        id: "cli:todo:clear".into(),
        method: Method::TodoClear(TodoClearParams {
            pane_id: parsed.pane_id,
            done_only: parsed.done_only,
        }),
    })?;
    if parsed.json || response.get("error").is_some() {
        return super::print_response(&response);
    }

    let removed = response["result"]["removed"].as_u64().unwrap_or(0);
    let suffix = if removed == 1 { "" } else { "s" };
    println!("cleared {removed} todo{suffix} on {pane_id}");
    Ok(0)
}

/// Shared tail of every single-todo mutation: `--json` (and any error) defers
/// to the raw response, otherwise the updated todo is echoed back.
fn print_todo_mutation(
    response: &serde_json::Value,
    json: bool,
    verb: &str,
) -> std::io::Result<i32> {
    if json || response.get("error").is_some() {
        return super::print_response(response);
    }

    let todo = &response["result"]["todo"];
    let id = todo["id"].as_u64().unwrap_or(0);
    let pane_id = todo["pane_id"].as_str().unwrap_or("");
    let text = todo["text"].as_str().unwrap_or("");
    println!("{verb} todo {id} on {pane_id}: {text}");
    Ok(0)
}

/// One rendered todo. `*` marks an outstanding todo, matching how
/// `herdr notification list` marks unread entries. A link is shown by the
/// label it captured, with `(gone)` when its target pane no longer exists.
fn format_todo_line(todo: &serde_json::Value, show_pane: bool) -> String {
    let done = todo["done"].as_bool().unwrap_or(false);
    let marker = if done { " " } else { "*" };
    let id = todo["id"].as_u64().unwrap_or(0);
    let priority = todo["priority"].as_str().unwrap_or("normal");
    let text = todo["text"].as_str().unwrap_or("");

    let mut line = String::from(marker);
    if show_pane {
        let pane_id = todo["pane_id"].as_str().unwrap_or("");
        line.push_str(&format!(" {pane_id:<12}"));
    }
    line.push_str(&format!(" {id:>4}  {priority:<6}  {text}"));
    if let Some(label) = todo["link_label"].as_str() {
        line.push_str(&format!(" → {label}"));
        if !todo["link_alive"].as_bool().unwrap_or(false) {
            line.push_str(" (gone)");
        }
    }
    line
}

fn report_arg_error(error: TodoArgError, usage: &str) -> i32 {
    match error {
        TodoArgError::Usage => eprintln!("{usage}"),
        TodoArgError::Message(message) => eprintln!("{message}"),
    }
    2
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TodoArgError {
    /// The verb was called wrong; the caller prints its usage line.
    Usage,
    Message(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TodoAddArgs {
    pane_id: String,
    text: String,
    priority: Option<TodoPriority>,
    link_pane_id: Option<String>,
    json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TodoListArgs {
    /// `None` lists every pane, which is what `--all` selects.
    pane_id: Option<String>,
    json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TodoEditArgs {
    pane_id: String,
    id: u64,
    text: Option<String>,
    priority: Option<TodoPriority>,
    link_pane_id: Option<String>,
    clear_link: bool,
    json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TodoIdArgs {
    pane_id: String,
    id: u64,
    json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TodoClearArgs {
    pane_id: String,
    done_only: bool,
    json: bool,
}

/// Accumulates the pane target while a verb's flags are parsed. Starts at the
/// calling pane so a bare `herdr todo add "..."` lands where it was typed.
struct PaneTarget<'a> {
    env_pane_id: Option<&'a str>,
    pane_id: Option<String>,
}

impl<'a> PaneTarget<'a> {
    fn new(env_pane_id: Option<&'a str>) -> Self {
        Self {
            env_pane_id,
            pane_id: env_pane_id.map(super::normalize_pane_id),
        }
    }

    fn set_explicit(&mut self, value: &str) {
        self.pane_id = Some(super::normalize_pane_id(value));
    }

    fn use_current(&mut self) {
        self.pane_id = self.env_pane_id.map(super::normalize_pane_id);
    }

    fn require(self, message: &str) -> Result<String, TodoArgError> {
        self.pane_id
            .ok_or_else(|| TodoArgError::Message(message.to_string()))
    }
}

fn flag_value(args: &[String], index: usize, flag: &str) -> Result<String, TodoArgError> {
    args.get(index + 1)
        .cloned()
        .ok_or_else(|| TodoArgError::Message(format!("missing value for {flag}")))
}

fn parse_priority(value: &str) -> Result<TodoPriority, TodoArgError> {
    match value {
        "high" => Ok(TodoPriority::High),
        "normal" => Ok(TodoPriority::Normal),
        "low" => Ok(TodoPriority::Low),
        _ => Err(TodoArgError::Message(format!(
            "invalid priority: {value} (expected high, normal, or low)"
        ))),
    }
}

fn unknown_option(other: &str) -> TodoArgError {
    TodoArgError::Message(format!("unknown option: {other}"))
}

fn is_help_arg(value: &str) -> bool {
    matches!(value, "help" | "--help" | "-h")
}

/// The leading positional of the id-taking verbs. A flag in that slot is a
/// usage error rather than a bogus "invalid todo id: --pane".
fn parse_leading_id(args: &[String]) -> Result<u64, TodoArgError> {
    let Some(raw) = args.first() else {
        return Err(TodoArgError::Usage);
    };
    if is_help_arg(raw) || raw.starts_with('-') {
        return Err(TodoArgError::Usage);
    }
    raw.parse::<u64>()
        .map_err(|_| TodoArgError::Message(format!("invalid todo id: {raw}")))
}

fn parse_todo_add_args(
    args: &[String],
    env_pane_id: Option<&str>,
) -> Result<TodoAddArgs, TodoArgError> {
    let Some(text) = args.first() else {
        return Err(TodoArgError::Usage);
    };
    // The text is positional, so a leading flag means the text was forgotten.
    if is_help_arg(text) || text.starts_with('-') {
        return Err(TodoArgError::Usage);
    }

    let mut target = PaneTarget::new(env_pane_id);
    let mut priority = None;
    let mut link_pane_id = None;
    let mut json = false;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--pane" => {
                target.set_explicit(&flag_value(args, index, "--pane")?);
                index += 2;
            }
            "--current" => {
                target.use_current();
                index += 1;
            }
            "--priority" => {
                priority = Some(parse_priority(&flag_value(args, index, "--priority")?)?);
                index += 2;
            }
            "--link" => {
                link_pane_id = Some(super::normalize_pane_id(&flag_value(
                    args, index, "--link",
                )?));
                index += 2;
            }
            "--json" => {
                json = true;
                index += 1;
            }
            other => return Err(unknown_option(other)),
        }
    }

    Ok(TodoAddArgs {
        pane_id: target.require(NO_TARGET_MESSAGE)?,
        text: text.clone(),
        priority,
        link_pane_id,
        json,
    })
}

fn parse_todo_list_args(
    args: &[String],
    env_pane_id: Option<&str>,
) -> Result<TodoListArgs, TodoArgError> {
    let mut target = PaneTarget::new(env_pane_id);
    let mut all = false;
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--pane" => {
                target.set_explicit(&flag_value(args, index, "--pane")?);
                all = false;
                index += 2;
            }
            "--current" => {
                target.use_current();
                all = false;
                index += 1;
            }
            "--all" => {
                all = true;
                index += 1;
            }
            "--json" => {
                json = true;
                index += 1;
            }
            other if is_help_arg(other) => return Err(TodoArgError::Usage),
            other => return Err(unknown_option(other)),
        }
    }

    if all {
        return Ok(TodoListArgs {
            pane_id: None,
            json,
        });
    }
    Ok(TodoListArgs {
        pane_id: Some(target.require(NO_LIST_TARGET_MESSAGE)?),
        json,
    })
}

fn parse_todo_edit_args(
    args: &[String],
    env_pane_id: Option<&str>,
) -> Result<TodoEditArgs, TodoArgError> {
    let id = parse_leading_id(args)?;

    let mut target = PaneTarget::new(env_pane_id);
    let mut text = None;
    let mut priority = None;
    let mut link_pane_id = None;
    let mut clear_link = false;
    let mut json = false;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--pane" => {
                target.set_explicit(&flag_value(args, index, "--pane")?);
                index += 2;
            }
            "--current" => {
                target.use_current();
                index += 1;
            }
            "--text" => {
                text = Some(flag_value(args, index, "--text")?);
                index += 2;
            }
            "--priority" => {
                priority = Some(parse_priority(&flag_value(args, index, "--priority")?)?);
                index += 2;
            }
            "--link" => {
                link_pane_id = Some(super::normalize_pane_id(&flag_value(
                    args, index, "--link",
                )?));
                index += 2;
            }
            "--unlink" => {
                clear_link = true;
                index += 1;
            }
            "--json" => {
                json = true;
                index += 1;
            }
            other => return Err(unknown_option(other)),
        }
    }

    if clear_link && link_pane_id.is_some() {
        return Err(TodoArgError::Message(
            "--link and --unlink are mutually exclusive".into(),
        ));
    }

    Ok(TodoEditArgs {
        pane_id: target.require(NO_TARGET_MESSAGE)?,
        id,
        text,
        priority,
        link_pane_id,
        clear_link,
        json,
    })
}

fn parse_todo_id_args(
    args: &[String],
    env_pane_id: Option<&str>,
) -> Result<TodoIdArgs, TodoArgError> {
    let id = parse_leading_id(args)?;

    let mut target = PaneTarget::new(env_pane_id);
    let mut json = false;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--pane" => {
                target.set_explicit(&flag_value(args, index, "--pane")?);
                index += 2;
            }
            "--current" => {
                target.use_current();
                index += 1;
            }
            "--json" => {
                json = true;
                index += 1;
            }
            other => return Err(unknown_option(other)),
        }
    }

    Ok(TodoIdArgs {
        pane_id: target.require(NO_TARGET_MESSAGE)?,
        id,
        json,
    })
}

fn parse_todo_clear_args(
    args: &[String],
    env_pane_id: Option<&str>,
) -> Result<TodoClearArgs, TodoArgError> {
    let mut target = PaneTarget::new(env_pane_id);
    let mut done_only = false;
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--pane" => {
                target.set_explicit(&flag_value(args, index, "--pane")?);
                index += 2;
            }
            "--current" => {
                target.use_current();
                index += 1;
            }
            "--done" => {
                done_only = true;
                index += 1;
            }
            "--json" => {
                json = true;
                index += 1;
            }
            other if is_help_arg(other) => return Err(TodoArgError::Usage),
            other => return Err(unknown_option(other)),
        }
    }

    Ok(TodoClearArgs {
        pane_id: target.require(NO_TARGET_MESSAGE)?,
        done_only,
        json,
    })
}

fn print_todo_help() {
    eprintln!("herdr todo commands:");
    eprintln!("  {TODO_ADD_USAGE}");
    eprintln!("  {TODO_LIST_USAGE}");
    eprintln!("  {TODO_DONE_USAGE}");
    eprintln!("  {TODO_UNDONE_USAGE}");
    eprintln!("  {TODO_EDIT_USAGE}");
    eprintln!("  {TODO_RM_USAGE}");
    eprintln!("  {TODO_CLEAR_USAGE}");
    eprintln!("  without --pane or --current, commands act on the calling pane");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn add_args_default_to_the_calling_pane() {
        let parsed = parse_todo_add_args(&args(&["fix the flaky test"]), Some("w1:p2")).unwrap();

        assert_eq!(parsed.pane_id, "w1:p2");
        assert_eq!(parsed.text, "fix the flaky test");
        assert_eq!(parsed.priority, None);
        assert_eq!(parsed.link_pane_id, None);
    }

    #[test]
    fn add_args_accept_an_explicit_pane_priority_and_link() {
        let parsed = parse_todo_add_args(
            &args(&[
                "rerun deploy",
                "--pane",
                "w1:p3",
                "--priority",
                "high",
                "--link",
                "infra",
            ]),
            Some("w1:p2"),
        )
        .unwrap();

        assert_eq!(
            parsed.pane_id, "w1:p3",
            "explicit --pane beats the env default"
        );
        assert_eq!(
            parsed.priority,
            Some(crate::terminal::todo::TodoPriority::High)
        );
        assert_eq!(parsed.link_pane_id.as_deref(), Some("infra"));
    }

    #[test]
    fn add_args_require_a_target_when_not_in_a_pane() {
        let error = parse_todo_add_args(&args(&["orphan todo"]), None).unwrap_err();

        assert_eq!(
            error,
            TodoArgError::Message(
                "no pane target: run inside a herdr pane or pass --pane <pane_id>".into()
            )
        );
    }

    #[test]
    fn add_args_reject_an_invalid_priority() {
        let error =
            parse_todo_add_args(&args(&["x", "--priority", "urgent"]), Some("w1:p1")).unwrap_err();

        assert_eq!(
            error,
            TodoArgError::Message(
                "invalid priority: urgent (expected high, normal, or low)".into()
            )
        );
    }

    #[test]
    fn add_args_require_text() {
        assert_eq!(
            parse_todo_add_args(&args(&[]), Some("w1:p1")).unwrap_err(),
            TodoArgError::Usage
        );
        assert_eq!(
            parse_todo_add_args(&args(&["--pane", "w1:p1"]), Some("w1:p1")).unwrap_err(),
            TodoArgError::Usage,
            "a flag must not be swallowed as the todo text"
        );
    }

    #[test]
    fn add_args_reject_a_missing_flag_value() {
        let error = parse_todo_add_args(&args(&["x", "--priority"]), Some("w1:p1")).unwrap_err();

        assert_eq!(
            error,
            TodoArgError::Message("missing value for --priority".into())
        );
    }

    #[test]
    fn list_args_support_all_and_json() {
        let parsed = parse_todo_list_args(&args(&["--all", "--json"]), Some("w1:p1")).unwrap();

        assert_eq!(parsed.pane_id, None, "--all lists every pane");
        assert!(parsed.json);

        let scoped = parse_todo_list_args(&args(&[]), Some("w1:p1")).unwrap();
        assert_eq!(scoped.pane_id.as_deref(), Some("w1:p1"));
        assert!(!scoped.json);
    }

    #[test]
    fn edit_args_parse_text_priority_and_unlink() {
        let parsed = parse_todo_edit_args(
            &args(&["3", "--text", "new text", "--unlink"]),
            Some("w1:p1"),
        )
        .unwrap();

        assert_eq!(parsed.id, 3);
        assert_eq!(parsed.text.as_deref(), Some("new text"));
        assert!(parsed.clear_link);
        assert_eq!(parsed.link_pane_id, None);
    }

    #[test]
    fn edit_args_reject_link_and_unlink_together() {
        let error =
            parse_todo_edit_args(&args(&["3", "--link", "infra", "--unlink"]), Some("w1:p1"))
                .unwrap_err();

        assert_eq!(
            error,
            TodoArgError::Message("--link and --unlink are mutually exclusive".into())
        );
    }

    #[test]
    fn id_args_reject_a_non_numeric_id() {
        let error = parse_todo_id_args(&args(&["abc"]), Some("w1:p1")).unwrap_err();

        assert_eq!(error, TodoArgError::Message("invalid todo id: abc".into()));
    }
}
