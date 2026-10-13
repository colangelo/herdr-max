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
    let result = &response["result"];
    let notifications = result["notifications"].as_array();
    let Some(notifications) = notifications.filter(|list| !list.is_empty()) else {
        println!("no notifications");
        return Ok(0);
    };

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
        let kind = notification["kind"].as_str().unwrap_or("unknown");
        let title = notification["title"].as_str().unwrap_or("");
        let context = notification["context"].as_str().unwrap_or("");
        let mut line = format!("{marker} {id:>4}  {age:>4}  {kind:<16}  {title}");
        if !context.is_empty() {
            line.push_str(&format!(" — {context}"));
        }
        println!("{line}");
    }
    let unread = result["unread_count"].as_u64().unwrap_or(0);
    if unread > 0 {
        println!("{unread} unread");
    }
    Ok(0)
}
