//! Pin fork-owned help fragments, without freezing upstream help screens.

fn command(path: &[&str]) -> clap::Command {
    let mut command = super::spec::command();
    for name in path {
        command = command.find_subcommand(name).expect("fork command").clone();
    }
    command.build();
    command
}

fn help(path: &[&str]) -> String {
    command(path).render_long_help().to_string()
}

#[test]
fn fork_help_preserves_option_descriptions() {
    for (path, phrases) in [
        (
            &["agent", "message"][..],
            &[
                "Send a peer message through the note command instead of typing the text",
                "Interrupt a working agent first",
                "Press this option's key for an agent blocked on a prompt, instead of typing text",
                "A note is delivered by palette.send.note_command (agent-bell by default) and approves nothing.",
            ][..],
        ),
        (&["agent", "read"][..], &[super::fork_help::STRIP_DIM][..]),
        (&["pane", "read"][..], &[super::fork_help::STRIP_DIM][..]),
        (&["todo", "add"][..], &["Link the todo to another pane"][..]),
        (
            &["todo", "edit"][..],
            &["Link the todo to another pane", "Drop the todo's pane link"][..],
        ),
        (&["todo", "list"][..], &["List todos from every pane"][..]),
        (&["todo", "clear"][..], &["Clear only the done todos"][..]),
        (
            &["pane", "send-text"][..],
            &[
                "Send the text in pieces of at most BYTES bytes, never splitting a character",
                "Pause between --chunk pieces in milliseconds [default: 20]",
                "[Pasted text #1]",
                "Use --chunk 300",
                "the command returns after the last piece is sent",
                "Use -- before text that starts with --chunk.",
            ][..],
        ),
        (
            &["popup", "close"][..],
            &["Close the popup only if this plugin opened it"][..],
        ),
    ] {
        let rendered = help(path);
        for phrase in phrases {
            assert!(rendered.contains(phrase), "{path:?}: missing {phrase:?}\n{rendered}");
        }
    }
}

#[test]
fn fork_help_preserves_command_summaries() {
    for (path, summary) in [
        (&["todo", "add"][..], "Add a todo to a pane"),
        (&["todo", "done"][..], "Mark a todo done"),
        (&["todo", "undone"][..], "Reopen a done todo"),
        (&["todo", "edit"][..], "Edit a todo"),
        (&["todo", "rm"][..], "Remove a todo"),
        (&["todo", "clear"][..], "Clear a pane's todos"),
        (
            &["tab", "sync"][..],
            "Type into every pane of a tab at once, or stop",
        ),
        (
            &["pane", "layout"][..],
            "Show pane geometry or apply a layout preset",
        ),
        (
            &["agent", "message"][..],
            "Send one line of text to an agent, typed as you or as a note",
        ),
        (
            &["agent", "pin"][..],
            "Pin an agent to the top of the agent panel",
        ),
        (
            &["workspace", "pin"][..],
            "Pin a workspace to the top of the list",
        ),
        (
            &["client", "list"][..],
            "List attached clients: foreground, focused tab, input idle time, window focus",
        ),
        (&["notification", "list"][..], "List the notification log"),
        (&["notification", "clear"][..], "Clear the notification log"),
    ] {
        assert!(help(path).starts_with(summary), "{path:?}: {summary}");
    }
}

#[test]
fn fork_help_preserves_values_and_visible_layout_alias() {
    for path in [&["todo", "add"][..], &["todo", "edit"][..]] {
        assert!(help(path).contains("[possible values: high, normal, low]"));
    }
    let layout = help(&["pane", "layout"]);
    assert!(layout.contains("[alias: --set]"), "{layout}");
    let hint = help(&["pane", "report-hint"]);
    assert!(hint.contains("--kind <question|permission>"), "{hint}");
    assert!(!help(&["workspace", "close"]).contains("--group"));
}

#[test]
fn fork_completion_advertises_priority_hint_kinds_and_layout_alias() {
    for (path, option, values) in [
        (
            &["todo", "add"][..],
            "priority",
            &["high", "normal", "low"][..],
        ),
        (
            &["todo", "edit"][..],
            "priority",
            &["high", "normal", "low"][..],
        ),
        (
            &["pane", "report-hint"][..],
            "kind",
            &["question", "permission"][..],
        ),
    ] {
        let mut projected = super::completion::fork_command_for_test(&command(path));
        let mut script = Vec::new();
        clap_complete::generate(
            clap_complete::Shell::Zsh,
            &mut projected,
            "herdr",
            &mut script,
        );
        let script = String::from_utf8(script).expect("UTF-8 completion");
        let line = script
            .lines()
            .find(|line| line.contains(&format!("--{option}")))
            .expect("completion option");
        for value in values {
            assert!(line.contains(value), "{path:?}: missing {value} in {line}");
        }
    }
    let mut projected = super::completion::fork_command_for_test(&command(&["pane", "layout"]));
    let mut script = Vec::new();
    clap_complete::generate(
        clap_complete::Shell::Zsh,
        &mut projected,
        "herdr",
        &mut script,
    );
    assert!(String::from_utf8(script)
        .expect("UTF-8 completion")
        .contains("--set"));
}
