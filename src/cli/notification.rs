#[path = "notification_bindings.rs"]
mod bindings;
pub(crate) use bindings::command;

pub(super) fn run_notification_command(args: &[String]) -> std::io::Result<i32> {
    let request = match bindings::request(args) {
        Ok(request) => request,
        Err(super::projection::ParseError::Clap(error)) => return super::print_clap_error(&error),
        Err(super::projection::ParseError::Usage(message)) => {
            eprintln!("{message}");
            return Ok(2);
        }
        Err(super::projection::ParseError::Io(error)) => return Err(error),
    };
    let response = super::send_request(&request)?;
    if args.iter().any(|arg| arg == "--json") || response.get("error").is_some() {
        return super::print_response(&response);
    }
    match request.method {
        crate::api::schema::Method::NotificationList(_) => print_list(&response),
        crate::api::schema::Method::NotificationClear(_) => {
            let cleared = response["result"]["cleared"].as_u64().unwrap_or(0);
            println!(
                "cleared {cleared} notification{}",
                if cleared == 1 { "" } else { "s" }
            );
            Ok(0)
        }
        _ => super::print_response(&response),
    }
}

fn print_list(response: &serde_json::Value) -> std::io::Result<i32> {
    let Some(text) = list_text(response) else {
        println!("no notifications");
        return Ok(0);
    };
    print!("{text}");
    Ok(0)
}

/// The list as printed, or `None` when there is nothing to list.
fn list_text(response: &serde_json::Value) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    let result = &response["result"];
    let notifications = result["notifications"].as_array();
    let notifications = notifications.filter(|list| !list.is_empty())?;

    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    for notification in notifications {
        let id = notification["id"].as_u64().unwrap_or(0);
        let read = notification["read"].as_bool().unwrap_or(false);
        let marker = if read { " " } else { "*" };
        let age = crate::ui::text::relative_time_label(
            now_unix,
            notification["posted_at_unix"].as_u64().unwrap_or(now_unix),
        );
        let kind = super::target::shown(notification["kind"].as_str().unwrap_or("unknown"));
        let title = super::target::shown(notification["title"].as_str().unwrap_or(""));
        let context = super::target::shown(notification["context"].as_str().unwrap_or(""));
        let mut line = format!("{marker} {id:>4}  {age:>4}  {kind:<16}  {title}");
        if !context.is_empty() {
            line.push_str(&format!(" — {context}"));
        }
        lines.push(line);
    }
    let unread = result["unread_count"].as_u64().unwrap_or(0);
    if unread > 0 {
        lines.push(format!("{unread} unread"));
    }
    Some(lines.iter().map(|line| format!("{line}\n")).collect())
}

#[cfg(test)]
pub(in crate::cli) mod far_output_tests {
    use super::*;

    use crate::cli::target::{as_remote, assert_escaped, FAR_CONTROLS};
    use serde_json::json;

    fn response() -> serde_json::Value {
        json!({"result": {"unread_count": 1, "notifications": [{
            "id": 1, "read": false, "posted_at_unix": 0,
            "kind": FAR_CONTROLS, "title": FAR_CONTROLS, "context": FAR_CONTROLS
        }]}})
    }

    /// `notification list` as printed, for the all-commands walk.
    pub(in crate::cli) fn walk_text() -> String {
        list_text(&response()).unwrap()
    }

    #[test]
    fn a_remote_notification_list_prints_escaped_and_a_local_one_raw() {
        assert_escaped(&as_remote(|| list_text(&response()).unwrap()));
        assert!(list_text(&response()).unwrap().contains(FAR_CONTROLS));
    }
}
