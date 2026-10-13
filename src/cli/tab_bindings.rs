use clap::Command;

use crate::api::schema::tabs::{
    tab_id, TabCreateProjection, TabListProjection, TabRenameProjection,
};
use crate::api::schema::{Method, Request, TabCloseParams, TabSyncParams, TabTarget};

use crate::cli::projection::{self, cli_projection, Field, Leaf, ParseError};

cli_projection! {
    pub(crate) TabTargetProjection for TabTarget {
        tab_id => Field::new(String::new()).input(tab_id()),
    }
}

cli_projection! { pub(crate) TabCloseProjection for TabCloseParams {
    tab_id => Field::new(String::new()).input(tab_id()),
    force => Field::new(false).input(crate::cli::projection::boolean("force", true)),
} }

const LEAVES: [Leaf; 7] = [
    Leaf {
        name: "sync",
        about: "Type into every pane of a tab at once, or stop",
        id: "cli:tab:sync",
        arguments: TabSyncProjection::arguments,
        method: |matches| TabSyncProjection::from_matches(matches).map(Method::TabSync),
    },
    Leaf {
        name: "list",
        about: "List tabs",
        id: "cli:tab:list",
        arguments: TabListProjection::arguments,
        method: |matches| TabListProjection::from_matches(matches).map(Method::TabList),
    },
    Leaf {
        name: "create",
        about: "Create a tab",
        id: "cli:tab:create",
        arguments: TabCreateProjection::arguments,
        method: |matches| TabCreateProjection::from_matches(matches).map(Method::TabCreate),
    },
    Leaf {
        name: "get",
        about: "Show a tab",
        id: "cli:tab:get",
        arguments: TabTargetProjection::arguments,
        method: |matches| TabTargetProjection::from_matches(matches).map(Method::TabGet),
    },
    Leaf {
        name: "focus",
        about: "Focus a tab",
        id: "cli:tab:focus",
        arguments: TabTargetProjection::arguments,
        method: |matches| TabTargetProjection::from_matches(matches).map(Method::TabFocus),
    },
    Leaf {
        name: "rename",
        about: "Rename a tab",
        id: "cli:tab:rename",
        arguments: TabRenameProjection::arguments,
        method: |matches| TabRenameProjection::from_matches(matches).map(Method::TabRename),
    },
    Leaf {
        name: "close",
        about: "Close a tab",
        id: "cli:tab:close",
        arguments: TabCloseProjection::arguments,
        method: |matches| TabCloseProjection::from_matches(matches).map(Method::TabClose),
    },
];

pub(crate) fn command() -> Command {
    Command::new("tab")
        .about("Manage tabs over the socket API")
        .subcommand_required(true)
        .subcommands(crate::cli::fork_help::ordered(
            LEAVES.iter().map(Leaf::command),
            &[("sync", "close", false)],
        ))
}

pub(crate) fn request(args: &[String]) -> Result<Request, ParseError> {
    projection::group_request(command(), &LEAVES, args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Request, ParseError> {
        request(&args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn tab_projection_preserves_requests_defaults_and_maps() {
        for (args, expected) in [
            (
                vec!["list"],
                serde_json::json!({"id":"cli:tab:list","method":"tab.list","params":{}}),
            ),
            (
                vec!["create"],
                serde_json::json!({"id":"cli:tab:create","method":"tab.create","params":{"focus":false}}),
            ),
            (
                vec![
                    "create",
                    "--workspace",
                    "2",
                    "--cwd",
                    "relative",
                    "--label",
                    "first",
                    "--label",
                    "last",
                    "--focus",
                    "--no-focus",
                    "--env",
                    "K=first",
                    "--env",
                    "K=",
                    "--env",
                    "V=a=b",
                ],
                serde_json::json!({"id":"cli:tab:create","method":"tab.create","params":{"workspace_id":"2","cwd":"relative","label":"last","focus":false,"env":{"K":"","V":"a=b"}}}),
            ),
            (
                vec!["rename", "2", "two", "--words", "here"],
                serde_json::json!({"id":"cli:tab:rename","method":"tab.rename","params":{"tab_id":"2","label":"two --words here"}}),
            ),
            (
                vec!["get", "2"],
                serde_json::json!({"id":"cli:tab:get","method":"tab.get","params":{"tab_id":"2"}}),
            ),
            (
                vec!["focus", "2"],
                serde_json::json!({"id":"cli:tab:focus","method":"tab.focus","params":{"tab_id":"2"}}),
            ),
            (
                vec!["close", "tab-2"],
                serde_json::json!({"id":"cli:tab:close","method":"tab.close","params":{"tab_id":"tab-2"}}),
            ),
        ] {
            assert_eq!(
                serde_json::to_value(parse(&args).unwrap()).unwrap(),
                expected,
                "{args:?}"
            );
        }
        assert!(
            matches!(parse(&["create", "--env", "=value"]), Err(ParseError::Usage(message)) if message == "env key must not be empty")
        );
    }

    #[test]
    fn tab_projection_clap_grammar_and_ordered_focus() {
        let value = serde_json::to_value(
            parse(&[
                "create",
                "--focus",
                "--no-focus",
                "--focus",
                "--cwd=relative",
            ])
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            value["params"],
            serde_json::json!({"cwd":"relative", "focus":true})
        );
        assert!(
            matches!(parse(&["get", "2", "--help"]), Err(ParseError::Clap(error)) if error.exit_code() == 0)
        );
        for args in [
            vec!["get"],
            vec!["get", "2", "extra"],
            vec!["create", "--cwd"],
        ] {
            assert!(matches!(parse(&args), Err(ParseError::Clap(error)) if error.exit_code() == 2));
        }
        command().debug_assert();
    }
}

cli_projection! { pub(crate) TabSyncProjection for TabSyncParams {
    tab_id => Field::new(None::<String>)
        .input(crate::cli::projection::optional_text("tab", "ID"))
        .input(crate::cli::projection::Input::positional("tab_id", 0, |slot, value| { if value.starts_with("--") { return Err(ParseError::Usage(format!("unknown option: {value}"))); } *slot = Some(value.to_owned()); Ok(()) }).value_name("TAB_ID").configure(|arg| arg.required(false)))
        .input(crate::cli::projection::Input::flag("current", |slot, _| { *slot = None; Ok(()) })),
    mode => crate::cli::pane::bindings::sync_mode(),
} }
