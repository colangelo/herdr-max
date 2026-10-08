use std::collections::HashMap;

use crate::api::schema::{TabCreateParams, TabListParams, TabRenameParams};

pub(super) fn run_tab_command(args: &[String]) -> std::io::Result<i32> {
    let Some(subcommand) = args.first().map(|arg| arg.as_str()) else {
        print_tab_help();
        return Ok(2);
    };

    match subcommand {
        "list" => tab_list(&args[1..]),
        "create" => tab_create(&args[1..]),
        "get" => tab_get(&args[1..]),
        "focus" => tab_focus(&args[1..]),
        "rename" => tab_rename(&args[1..]),
        "sync" => tab_sync(&args[1..]),
        "close" => tab_close(&args[1..]),
        "help" | "--help" | "-h" => {
            print_tab_help();
            Ok(0)
        }
        _ => {
            print_tab_help();
            Ok(2)
        }
    }
}

fn tab_list(args: &[String]) -> std::io::Result<i32> {
    let mut workspace_id = None;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--workspace" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --workspace");
                    return Ok(2);
                };
                workspace_id = Some(super::normalize_workspace_id(value));
                index += 2;
            }
            other => {
                eprintln!("unknown option: {other}");
                return Ok(2);
            }
        }
    }

    super::runtime::tab_list(TabListParams { workspace_id })
}

fn tab_create(args: &[String]) -> std::io::Result<i32> {
    let mut workspace_id = None;
    let mut cwd = None;
    let mut focus = false;
    let mut label = None;
    let mut env = HashMap::new();

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--workspace" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --workspace");
                    return Ok(2);
                };
                workspace_id = Some(super::normalize_workspace_id(value));
                index += 2;
            }
            "--cwd" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --cwd");
                    return Ok(2);
                };
                cwd = Some(value.clone());
                index += 2;
            }
            "--label" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --label");
                    return Ok(2);
                };
                label = Some(value.clone());
                index += 2;
            }
            "--focus" => {
                focus = true;
                index += 1;
            }
            "--no-focus" => {
                focus = false;
                index += 1;
            }
            "--env" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --env");
                    return Ok(2);
                };
                let (key, value) = match super::parse_env_assignment(value) {
                    Ok(pair) => pair,
                    Err(err) => {
                        eprintln!("{err}");
                        return Ok(2);
                    }
                };
                env.insert(key, value);
                index += 2;
            }
            other => {
                eprintln!("unknown option: {other}");
                return Ok(2);
            }
        }
    }

    super::runtime::tab_create(TabCreateParams {
        workspace_id,
        cwd,
        focus,
        label,
        env,
    })
}

fn tab_get(args: &[String]) -> std::io::Result<i32> {
    let Some(raw_tab_id) = args.first() else {
        eprintln!("usage: herdr tab get <tab_id>");
        return Ok(2);
    };
    if args.len() != 1 {
        eprintln!("usage: herdr tab get <tab_id>");
        return Ok(2);
    }

    super::runtime::tab_get(super::normalize_tab_id(raw_tab_id))
}

fn tab_focus(args: &[String]) -> std::io::Result<i32> {
    let Some(raw_tab_id) = args.first() else {
        eprintln!("usage: herdr tab focus <tab_id>");
        return Ok(2);
    };
    if args.len() != 1 {
        eprintln!("usage: herdr tab focus <tab_id>");
        return Ok(2);
    }

    super::runtime::tab_focus(super::normalize_tab_id(raw_tab_id))
}

fn tab_rename(args: &[String]) -> std::io::Result<i32> {
    if args.len() < 2 {
        eprintln!("usage: herdr tab rename <tab_id> <label>");
        return Ok(2);
    }

    super::runtime::tab_rename(TabRenameParams {
        tab_id: super::normalize_tab_id(&args[0]),
        label: args[1..].join(" "),
    })
}

fn tab_sync(args: &[String]) -> std::io::Result<i32> {
    match parse_tab_sync_args(args) {
        Ok(params) => super::runtime::tab_sync(params),
        Err(message) => {
            eprintln!("{message}");
            Ok(2)
        }
    }
}

/// `herdr tab sync [<tab_id>|--tab ID|--current] [--toggle|--on|--off]`.
fn parse_tab_sync_args(args: &[String]) -> Result<crate::api::schema::TabSyncParams, String> {
    let mut tab_id = None;
    let mut mode = None;
    let mut index = 0;
    if args
        .first()
        .is_some_and(|arg| !arg.as_str().starts_with("--"))
    {
        tab_id = args.first().map(|arg| super::normalize_tab_id(arg));
        index = 1;
    }
    while index < args.len() {
        match args[index].as_str() {
            "--tab" => {
                let Some(value) = args.get(index + 1) else {
                    return Err("missing value for --tab".into());
                };
                tab_id = Some(super::normalize_tab_id(value));
                index += 2;
            }
            "--current" => {
                tab_id = None;
                index += 1;
            }
            flag @ ("--toggle" | "--on" | "--off") => {
                if mode.is_some() {
                    return Err("provide only one of --toggle, --on, or --off".into());
                }
                mode = Some(super::sync_mode(&flag[2..]));
                index += 1;
            }
            other => return Err(format!("unknown option: {other}")),
        }
    }
    Ok(crate::api::schema::TabSyncParams {
        tab_id,
        mode: mode.unwrap_or_default(),
    })
}

fn tab_close(args: &[String]) -> std::io::Result<i32> {
    let (args, force) = super::take_force_flag(args);
    let [raw_tab_id] = args.as_slice() else {
        eprintln!("usage: herdr tab close <tab_id> [--force]");
        return Ok(2);
    };

    super::runtime::tab_close(super::normalize_tab_id(raw_tab_id), force)
}

fn print_tab_help() {
    eprintln!("herdr tab commands:");
    eprintln!("  herdr tab list [--workspace <workspace_id>]");
    eprintln!(
        "  herdr tab create [--workspace <workspace_id>] [--cwd PATH] [--label TEXT] [--env KEY=VALUE] [--focus] [--no-focus]"
    );
    eprintln!("  herdr tab get <tab_id>");
    eprintln!("  herdr tab focus <tab_id>");
    eprintln!("  herdr tab rename <tab_id> <label>");
    eprintln!("  herdr tab sync [<tab_id>|--tab ID|--current] [--toggle|--on|--off]");
    eprintln!("  herdr tab close <tab_id>");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schema::SyncMode;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn tab_sync_args_default_to_the_current_tab_toggle() {
        let params = parse_tab_sync_args(&args(&[])).unwrap();
        assert_eq!(params.tab_id, None);
        assert_eq!(params.mode, SyncMode::Toggle);
    }

    #[test]
    fn tab_sync_args_take_a_tab_and_a_mode() {
        let params = parse_tab_sync_args(&args(&["w_1:2", "--on"])).unwrap();
        assert_eq!(params.tab_id.as_deref(), Some("w_1:2"));
        assert_eq!(params.mode, SyncMode::On);
        let params = parse_tab_sync_args(&args(&["--tab", "w_1:2", "--off"])).unwrap();
        assert_eq!(params.mode, SyncMode::Off);
        assert!(parse_tab_sync_args(&args(&["--on", "--off"])).is_err());
        assert!(parse_tab_sync_args(&args(&["--bogus"])).is_err());
    }
}
