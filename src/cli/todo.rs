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
