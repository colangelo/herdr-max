use super::super::projection::{
    self, boolean, cli_projection, optional_text, Field, Input, Leaf, ParseError,
};
use crate::api::schema::{
    Method, Request, TodoAddParams, TodoClearParams, TodoListParams, TodoRemoveParams,
    TodoUpdateParams,
};
use crate::terminal::todo::TodoPriority;
use clap::{Arg, ArgAction, ArgMatches, Command};

fn pane() -> Field<String> {
    Field::new(String::new()).input(Input::value("pane", "ID", |slot, value| {
        *slot = value.to_owned();
        Ok(())
    }))
}
fn text() -> Input<String> {
    Input::positional("text", 0, |slot, value| {
        *slot = value.to_owned();
        Ok(())
    })
    .required()
}
fn id() -> Input<u64> {
    Input::positional("id", 0, |slot, value| {
        *slot = value
            .parse()
            .map_err(|_| ParseError::Usage(format!("invalid todo id: {value}")))?;
        Ok(())
    })
    .required()
}
fn priority() -> Input<Option<TodoPriority>> {
    Input::value("priority", "PRIORITY", |slot, value| {
        *slot = Some(match value {
            "high" => TodoPriority::High,
            "normal" => TodoPriority::Normal,
            "low" => TodoPriority::Low,
            _ => {
                return Err(ParseError::Usage(format!(
                    "invalid priority: {value} (expected high, normal, or low)"
                )))
            }
        });
        Ok(())
    })
}
fn options(mut args: Vec<Arg>) -> Vec<Arg> {
    args.push(
        Arg::new("current")
            .long("current")
            .action(ArgAction::SetTrue),
    );
    args.push(Arg::new("json").long("json").action(ArgAction::SetTrue));
    args
}

cli_projection! { pub(crate) TodoAddProjection for TodoAddParams {
    pane_id => pane(),
    text => Field::new(String::new()).input(text()),
    priority => Field::new(None).input(priority()),
    link_pane_id => Field::new(None).input(optional_text("link", "TARGET")),
} }

cli_projection! { pub(crate) TodoListProjection for TodoListParams {
    pane_id => Field::new(None).input(optional_text("pane", "ID")),
} }

cli_projection! { pub(crate) TodoEditProjection for TodoUpdateParams {
    pane_id => pane(),
    id => Field::new(0u64).input(id()),
    text => Field::new(None).input(optional_text("text", "TEXT")),
    priority => Field::new(None).input(priority()),
    link_pane_id => Field::new(None).input(optional_text("link", "TARGET")),
    clear_link => Field::new(false).input(boolean("unlink", true)),
    done => Field::new(None::<bool>),
} }

cli_projection! { pub(crate) TodoRemoveProjection for TodoRemoveParams {
    pane_id => pane(),
    id => Field::new(0u64).input(id()),
} }

cli_projection! { pub(crate) TodoClearProjection for TodoClearParams {
    pane_id => pane(),
    done_only => Field::new(false).input(boolean("done", true)),
} }

cli_projection! { pub(crate) TodoDoneProjection for TodoUpdateParams {
    pane_id => pane(),
    id => Field::new(0u64).input(id()),
    text => Field::new(None::<String>),
    priority => Field::new(None::<TodoPriority>),
    link_pane_id => Field::new(None::<String>),
    clear_link => Field::new(false),
    done => Field::new(Some(true)),
} }

cli_projection! { pub(crate) TodoUndoneProjection for TodoUpdateParams {
    pane_id => pane(),
    id => Field::new(0u64).input(id()),
    text => Field::new(None::<String>),
    priority => Field::new(None::<TodoPriority>),
    link_pane_id => Field::new(None::<String>),
    clear_link => Field::new(false),
    done => Field::new(Some(false)),
} }

const LEAVES: &[Leaf] = &[
    Leaf {
        name: "add",
        about: "Add pane todos",
        id: "cli:todo:add",
        arguments: || options(TodoAddProjection::arguments()),
        method: |matches| TodoAddProjection::from_matches(matches).map(Method::TodoAdd),
    },
    Leaf {
        name: "list",
        about: "List pane todos",
        id: "cli:todo:list",
        arguments: || {
            let mut args = options(TodoListProjection::arguments());
            args.push(Arg::new("all").long("all").action(ArgAction::SetTrue));
            args
        },
        method: |matches| TodoListProjection::from_matches(matches).map(Method::TodoList),
    },
    Leaf {
        name: "done",
        about: "Done pane todos",
        id: "cli:todo:done",
        arguments: || options(TodoDoneProjection::arguments()),
        method: |matches| TodoDoneProjection::from_matches(matches).map(Method::TodoUpdate),
    },
    Leaf {
        name: "undone",
        about: "Undone pane todos",
        id: "cli:todo:undone",
        arguments: || options(TodoUndoneProjection::arguments()),
        method: |matches| TodoUndoneProjection::from_matches(matches).map(Method::TodoUpdate),
    },
    Leaf {
        name: "edit",
        about: "Edit pane todos",
        id: "cli:todo:edit",
        arguments: || options(TodoEditProjection::arguments()),
        method: |matches| TodoEditProjection::from_matches(matches).map(Method::TodoUpdate),
    },
    Leaf {
        name: "rm",
        about: "Rm pane todos",
        id: "cli:todo:rm",
        arguments: || options(TodoRemoveProjection::arguments()),
        method: |matches| TodoRemoveProjection::from_matches(matches).map(Method::TodoRemove),
    },
    Leaf {
        name: "clear",
        about: "Clear pane todos",
        id: "cli:todo:clear",
        arguments: || options(TodoClearProjection::arguments()),
        method: |matches| TodoClearProjection::from_matches(matches).map(Method::TodoClear),
    },
];
pub(crate) fn command() -> Command {
    Command::new("todo")
        .about("Manage per-pane todos")
        .subcommand_required(true)
        .subcommands(LEAVES.iter().map(Leaf::command))
}

pub(crate) struct Invocation {
    pub request: Request,
    pub json: bool,
}

fn looks_like_pane_id(value: &str) -> bool {
    if let Some(rest) = value.strip_prefix("p_") {
        return !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit() || c == '_');
    }
    value.rsplit_once(":p").is_some_and(|(workspace, pane)| {
        !workspace.is_empty()
            && workspace
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            && !pane.is_empty()
            && pane.chars().all(|c| c.is_ascii_alphanumeric())
    })
}

fn last(matches: &ArgMatches, name: &str) -> Option<usize> {
    if matches.value_source(name) != Some(clap::parser::ValueSource::CommandLine) {
        return None;
    }
    matches
        .indices_of(name)
        .and_then(|mut indices| indices.next_back())
}

pub(crate) fn invocation(args: &[String], caller: Option<&str>) -> Result<Invocation, ParseError> {
    if let Some(flag @ ("--pane" | "--priority" | "--link" | "--text")) =
        args.last().map(String::as_str)
    {
        return Err(ParseError::Usage(format!("missing value for {flag}")));
    }
    if let [verb, pane, text, ..] = args {
        if verb == "add" && looks_like_pane_id(pane) && !text.starts_with('-') {
            return Err(ParseError::Usage(format!("{pane} looks like a pane id, but todo add takes the text first: herdr todo add \"{text}\" --pane {pane}")));
        }
    }
    projection::parse(command().bin_name("herdr todo"), args, |matches| {
        let (name, matches) = matches
            .subcommand()
            .ok_or_else(|| ParseError::Usage("missing todo command".into()))?;
        let leaf = LEAVES
            .iter()
            .find(|leaf| leaf.name == name)
            .ok_or_else(|| ParseError::Usage("unknown todo command".into()))?;
        let mut method = (leaf.method)(matches)?;
        let current = last(matches, "current");
        let explicit = last(matches, "pane");
        let pane = if current > explicit || explicit.is_none() {
            caller.map(str::to_owned)
        } else {
            matches.get_one::<String>("pane").cloned()
        };
        let all = if name == "list" {
            last(matches, "all")
        } else {
            None
        };
        let all_wins = all.is_some() && all > current.max(explicit);
        let require = || {
            pane.clone().ok_or_else(|| {
                ParseError::Usage(
                    "no pane target: run inside a herdr pane or pass --pane <pane_id>".into(),
                )
            })
        };
        match &mut method {
            Method::TodoAdd(p) => p.pane_id = require()?,
            Method::TodoUpdate(p) => p.pane_id = require()?,
            Method::TodoRemove(p) => p.pane_id = require()?,
            Method::TodoClear(p) => p.pane_id = require()?,
            Method::TodoList(p) => {
                p.pane_id = if all_wins {
                    None
                } else {
                    Some(pane.ok_or_else(|| ParseError::Usage("no pane target: run inside a herdr pane, pass --pane <pane_id>, or use --all".into()))?)
                }
            }
            _ => unreachable!("todo leaves"),
        }
        if let Method::TodoUpdate(p) = &method {
            if p.clear_link && p.link_pane_id.is_some() {
                return Err(ParseError::Usage(
                    "--link and --unlink are mutually exclusive".into(),
                ));
            }
        }
        Ok(Invocation {
            request: Request {
                id: leaf.id.into(),
                method,
            },
            json: matches.get_flag("json"),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test adapters invoke the new projection path; assertions below are the baseline tests.
    #[derive(Debug, PartialEq, Eq)]
    enum TodoArgError {
        Usage,
        Message(String),
    }
    fn mapped(error: ParseError) -> TodoArgError {
        match error {
            ParseError::Usage(message) => TodoArgError::Message(message),
            ParseError::Clap(_) => TodoArgError::Usage,
            ParseError::Io(error) => TodoArgError::Message(error.to_string()),
        }
    }
    fn project(
        verb: &str,
        args: &[String],
        caller: Option<&str>,
    ) -> Result<Invocation, TodoArgError> {
        invocation(&[&[verb.to_owned()][..], args].concat(), caller).map_err(mapped)
    }
    fn parse_todo_add_args(
        args: &[String],
        caller: Option<&str>,
    ) -> Result<TodoAddParams, TodoArgError> {
        let Method::TodoAdd(params) = project("add", args, caller)?.request.method else {
            panic!("add");
        };
        Ok(params)
    }
    #[derive(Debug)]
    struct TodoListArgs {
        pane_id: Option<String>,
        json: bool,
    }
    fn parse_todo_list_args(
        args: &[String],
        caller: Option<&str>,
    ) -> Result<TodoListArgs, TodoArgError> {
        let parsed = project("list", args, caller)?;
        let Method::TodoList(params) = parsed.request.method else {
            panic!("list");
        };
        Ok(TodoListArgs {
            pane_id: params.pane_id,
            json: parsed.json,
        })
    }
    fn parse_todo_edit_args(
        args: &[String],
        caller: Option<&str>,
    ) -> Result<TodoUpdateParams, TodoArgError> {
        let Method::TodoUpdate(params) = project("edit", args, caller)?.request.method else {
            panic!("edit");
        };
        Ok(params)
    }
    fn parse_todo_id_args(
        args: &[String],
        caller: Option<&str>,
    ) -> Result<TodoUpdateParams, TodoArgError> {
        let Method::TodoUpdate(params) = project("done", args, caller)?.request.method else {
            panic!("done");
        };
        Ok(params)
    }

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
    fn add_args_name_the_right_form_when_a_pane_id_comes_first() {
        for pane in ["w1:p3", "p_3", "p_1_3"] {
            let error = parse_todo_add_args(&args(&[pane, "fix it"]), Some("w1:p1")).unwrap_err();

            let TodoArgError::Message(message) = error else {
                panic!("{pane}: expected a message, got {error:?}");
            };
            assert!(
                message.contains(&format!("herdr todo add \"fix it\" --pane {pane}")),
                "{pane}: {message}"
            );
        }
    }

    #[test]
    fn add_args_keep_a_lone_pane_like_word_as_text() {
        let parsed = parse_todo_add_args(&args(&["w1:p3"]), Some("w1:p1")).unwrap();
        assert_eq!(parsed.text, "w1:p3");

        let parsed =
            parse_todo_add_args(&args(&["w1:p3", "--priority", "high"]), Some("w1:p1")).unwrap();
        assert_eq!(parsed.text, "w1:p3");
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
