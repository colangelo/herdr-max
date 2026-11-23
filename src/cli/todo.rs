use crate::api::schema::Method;
#[path = "todo_bindings.rs"]
pub(crate) mod bindings;

pub(super) fn run_todo_command(args: &[String]) -> std::io::Result<i32> {
    use super::projection::ParseError;
    let invocation = match bindings::invocation(args, super::target::caller_pane_id().as_deref()) {
        Ok(invocation) => invocation,
        Err(ParseError::Clap(error)) => return super::print_clap_error(&error),
        Err(ParseError::Usage(message)) => {
            eprintln!("{message}");
            return Ok(2);
        }
        Err(ParseError::Io(error)) => return Err(error),
    };
    let response = super::send_request(&invocation.request)?;
    if invocation.json || response.get("error").is_some() {
        return super::print_response(&response);
    }
    match &invocation.request.method {
        Method::TodoAdd(_) => print_todo_mutation(&response, false, "added"),
        Method::TodoUpdate(_) => print_todo_mutation(
            &response,
            false,
            match args.first().map(String::as_str) {
                Some("done") => "completed",
                Some("undone") => "reopened",
                _ => "updated",
            },
        ),
        Method::TodoRemove(params) => {
            println!("removed todo {} from {}", params.id, params.pane_id);
            Ok(0)
        }
        Method::TodoClear(params) => {
            let removed = response["result"]["removed"].as_u64().unwrap_or(0);
            println!(
                "cleared {removed} todo{} on {}",
                if removed == 1 { "" } else { "s" },
                params.pane_id
            );
            Ok(0)
        }
        Method::TodoList(params) => {
            let Some(todos) = response["result"]["todos"]
                .as_array()
                .filter(|todos| !todos.is_empty())
            else {
                println!("no todos");
                return Ok(0);
            };
            for todo in todos {
                println!("{}", format_todo_line(todo, params.pane_id.is_none()));
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
        _ => unreachable!("todo bindings only produce todo methods"),
    }
}

fn print_todo_mutation(
    response: &serde_json::Value,
    json: bool,
    verb: &str,
) -> std::io::Result<i32> {
    if json || response.get("error").is_some() {
        return super::print_response(response);
    }

    println!("{}", todo_mutation_line(response, verb));
    Ok(0)
}

fn todo_mutation_line(response: &serde_json::Value, verb: &str) -> String {
    let todo = &response["result"]["todo"];
    let id = todo["id"].as_u64().unwrap_or(0);
    let pane_id = super::target::shown(todo["pane_id"].as_str().unwrap_or(""));
    let text = super::target::shown(todo["text"].as_str().unwrap_or(""));
    format!("{verb} todo {id} on {pane_id}: {text}")
}

/// One rendered todo. `*` marks an outstanding todo, matching how
/// `herdr notification list` marks unread entries. A link is shown by the
/// label it captured, with `(gone)` when its target pane no longer exists.
fn format_todo_line(todo: &serde_json::Value, show_pane: bool) -> String {
    let done = todo["done"].as_bool().unwrap_or(false);
    let marker = if done { " " } else { "*" };
    let id = todo["id"].as_u64().unwrap_or(0);
    let priority = todo["priority"].as_str().unwrap_or("normal");
    let text = super::target::shown(todo["text"].as_str().unwrap_or(""));

    let mut line = String::from(marker);
    if show_pane {
        let pane_id = super::target::shown(todo["pane_id"].as_str().unwrap_or(""));
        line.push_str(&format!(" {pane_id:<12}"));
    }
    line.push_str(&format!(" {id:>4}  {priority:<6}  {text}"));
    if let Some(label) = todo["link_label"].as_str() {
        line.push_str(&format!(" → {}", super::target::shown(label)));
        if !todo["link_alive"].as_bool().unwrap_or(false) {
            line.push_str(" (gone)");
        }
    }
    line
}

#[cfg(test)]
mod far_output_tests {
    use super::*;

    use crate::cli::target::{as_remote, assert_escaped, FAR_CONTROLS};
    use serde_json::json;

    fn todo() -> serde_json::Value {
        json!({"id": 3, "pane_id": FAR_CONTROLS, "text": FAR_CONTROLS,
               "priority": "normal", "link_label": FAR_CONTROLS, "link_alive": true})
    }

    #[test]
    fn a_remote_todo_list_line_prints_escaped_and_a_local_one_raw() {
        assert_escaped(&as_remote(|| format_todo_line(&todo(), true)));
        assert!(format_todo_line(&todo(), true).contains(FAR_CONTROLS));
    }

    #[test]
    fn a_remote_todo_mutation_line_prints_escaped_and_a_local_one_raw() {
        let response = json!({"result": {"todo": todo()}});
        assert_escaped(&as_remote(|| todo_mutation_line(&response, "added")));
        assert!(todo_mutation_line(&response, "added").contains(FAR_CONTROLS));
    }
}
