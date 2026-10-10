use crate::api::schema::{EmptyParams, Method, PluginPopupCloseParams, Request};

pub(super) fn run_popup_command(args: &[String]) -> std::io::Result<i32> {
    let Some(subcommand) = args.first().map(|arg| arg.as_str()) else {
        print_popup_help();
        return Ok(2);
    };

    match subcommand {
        "close" => popup_close(&args[1..]),
        "help" | "--help" | "-h" => {
            print_popup_help();
            Ok(0)
        }
        _ => {
            print_popup_help();
            Ok(2)
        }
    }
}

/// `Ok(None)` closes whatever popup is open; `Ok(Some(id))` closes it only if
/// plugin `id` opened it.
fn parse_popup_close_args(args: &[String]) -> Result<Option<String>, String> {
    let mut plugin = None;
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        let value = if arg == "--plugin" {
            let Some(value) = args.get(index + 1) else {
                return Err("missing value for --plugin".into());
            };
            index += 2;
            value.clone()
        } else if let Some(value) = arg.strip_prefix("--plugin=") {
            index += 1;
            value.to_owned()
        } else {
            return Err(format!("unknown option: {arg}"));
        };
        if plugin.replace(value).is_some() {
            return Err("--plugin may only be given once".into());
        }
    }
    Ok(plugin)
}

fn popup_close(args: &[String]) -> std::io::Result<i32> {
    let plugin_id = match parse_popup_close_args(args) {
        Ok(plugin_id) => plugin_id,
        Err(message) => {
            eprintln!("{message}");
            eprintln!("usage: herdr popup close [--plugin ID]");
            return Ok(2);
        }
    };
    // The targeted close is its own method, not a field on `popup.close`: an
    // older server would ignore the extra field and close any popup.
    let method = match plugin_id {
        Some(plugin_id) => Method::PluginPopupClose(PluginPopupCloseParams { plugin_id }),
        None => Method::PopupClose(EmptyParams::default()),
    };
    super::print_response(&super::send_request(&Request {
        id: "cli:popup:close".into(),
        method,
    })?)
}

fn print_popup_help() {
    eprintln!("herdr popup commands:");
    eprintln!("  herdr popup close [--plugin ID]");
}

#[cfg(test)]
mod tests {
    use super::parse_popup_close_args;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn close_without_options_targets_any_popup() {
        assert_eq!(parse_popup_close_args(&args(&[])), Ok(None));
    }

    #[test]
    fn close_plugin_accepts_both_spellings() {
        for values in [
            &["--plugin", "gestore.asks"][..],
            &["--plugin=gestore.asks"][..],
        ] {
            assert_eq!(
                parse_popup_close_args(&args(values)),
                Ok(Some("gestore.asks".to_owned()))
            );
        }
    }

    #[test]
    fn close_rejects_unknown_missing_and_repeated_options() {
        for values in [
            &["--plugin"][..],
            &["--force"][..],
            &["gestore.asks"][..],
            &["--plugin", "a", "--plugin", "b"][..],
        ] {
            assert!(
                parse_popup_close_args(&args(values)).is_err(),
                "{values:?} should be rejected"
            );
        }
    }
}
