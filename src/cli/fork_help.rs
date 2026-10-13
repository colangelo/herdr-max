//! Fork-owned help/completion metadata, kept separate from upstream declarations.
use clap::builder::{PossibleValue, StringValueParser, TypedValueParser};
use clap::{Arg, Command};
use std::ffi::OsStr;

pub(crate) const STRIP_DIM: &str = "Leave out faint (dim) text, such as a grey suggestion";
pub(crate) const PRIORITIES: &[&str] = &["high", "normal", "low"];
pub(crate) const HINT_KINDS: &[&str] = &["question", "permission"];
pub(crate) const CHUNK_HELP: &str = "The text is sent as one raw write. A TUI such as Claude Code may treat a large single write (about 1000 bytes or more) as a paste and collapse it into a placeholder like [Pasted text #1]. Use --chunk 300 to send it as separate paced writes that arrive as typed text; the command returns after the last piece is sent. Use -- before text that starts with --chunk.";

/// Choices are presentation metadata; the fork reducers retain their existing errors.
#[derive(Clone)]
pub(crate) struct Values(pub(crate) &'static [&'static str]);
impl TypedValueParser for Values {
    type Value = String;
    fn parse_ref(
        &self,
        command: &Command,
        argument: Option<&Arg>,
        value: &OsStr,
    ) -> Result<String, clap::Error> {
        StringValueParser::new().parse_ref(command, argument, value)
    }
    fn possible_values(&self) -> Option<Box<dyn Iterator<Item = PossibleValue> + '_>> {
        Some(Box::new(self.0.iter().copied().map(PossibleValue::new)))
    }
}

pub(crate) fn pane_leaf(command: Command) -> Command {
    match command.get_name() {
        "layout" => command
            .about("Show pane geometry or apply a layout preset")
            .mut_arg("preset", |arg| arg.display_order(0))
            .mut_arg("pane", |arg| arg.display_order(1))
            .mut_arg("current", |arg| arg.display_order(2)),
        "send-text" => {
            let after = command.get_after_help().map(ToString::to_string);
            let help = after.map_or_else(
                || CHUNK_HELP.to_owned(),
                |after| format!("{CHUNK_HELP}\n\n{after}"),
            );
            command
                .arg(Arg::new("chunk").long("chunk").value_name("BYTES").help(
                    "Send the text in pieces of at most BYTES bytes, never splitting a character",
                ))
                .arg(
                    Arg::new("chunk-delay")
                        .long("chunk-delay")
                        .value_name("MS")
                        .help("Pause between --chunk pieces in milliseconds [default: 20]"),
                )
                .after_help(help)
        }
        _ => command,
    }
}

/// Place fork commands beside their pre-port neighbors without reordering upstream commands.
pub(crate) fn ordered(
    commands: impl IntoIterator<Item = Command>,
    anchors: &[(&str, &str, bool)],
) -> Vec<Command> {
    let mut commands: Vec<_> = commands.into_iter().collect();
    for (name, anchor, after) in anchors {
        let Some(index) = commands
            .iter()
            .position(|command| command.get_name() == *name)
        else {
            continue;
        };
        let command = commands.remove(index);
        let index = commands
            .iter()
            .position(|command| command.get_name() == *anchor)
            .map_or(commands.len(), |index| index + usize::from(*after));
        commands.insert(index, command);
    }
    commands
}
