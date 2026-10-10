//! `herdr agent message`: send one line of text to an agent without switching to
//! it (fork issue 182). The same `agent.message` method the command bar uses.

use crate::api::schema::{AgentMessageBusy, AgentMessageMode, AgentMessageParams, Method, Request};

const USAGE: &str = "usage: herdr agent message <target> <text> [--note] [--interrupt] | herdr agent message <target> --answer KEY";

/// Parse `agent message`. The options are recognised anywhere; `--` ends option
/// parsing so text that starts with `--` can still be sent. The first other
/// word is the target and the rest is the text, joined with single spaces.
fn parse_agent_message_args(args: &[String]) -> Result<AgentMessageParams, String> {
    let mut words: Vec<&str> = Vec::new();
    let mut mode = AgentMessageMode::Typed;
    let mut busy = AgentMessageBusy::Queue;
    let mut answer = None;
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        let (flag, attached) = match arg.split_once('=') {
            Some((flag, value)) => (flag, Some(value)),
            None => (arg, None),
        };
        match flag {
            "--" if attached.is_none() => {
                words.extend(args[index + 1..].iter().map(String::as_str));
                break;
            }
            "--note" if attached.is_none() => mode = AgentMessageMode::Note,
            "--interrupt" if attached.is_none() => busy = AgentMessageBusy::Interrupt,
            "--answer" => {
                let value = match attached {
                    Some(value) => value,
                    None => {
                        index += 1;
                        args.get(index)
                            .map(String::as_str)
                            .ok_or("missing value for --answer")?
                    }
                };
                answer = Some(value.to_owned());
            }
            _ => words.push(arg),
        }
        index += 1;
    }

    let Some((target, text_words)) = words.split_first() else {
        return Err(USAGE.into());
    };
    let text = text_words.join(" ");
    match (&answer, text.is_empty()) {
        (Some(_), false) => return Err("--answer takes a key, not text".into()),
        (None, true) => return Err(USAGE.into()),
        _ => {}
    }
    if answer.is_some() && mode == AgentMessageMode::Note {
        return Err("--answer presses a key in the pane; it cannot be a note".into());
    }
    Ok(AgentMessageParams {
        target: (*target).to_owned(),
        text,
        mode,
        busy,
        answer,
    })
}

pub(super) fn agent_message(args: &[String]) -> std::io::Result<i32> {
    let params = match parse_agent_message_args(args) {
        Ok(params) => params,
        Err(message) => {
            eprintln!("{message}");
            return Ok(2);
        }
    };
    super::print_response(&super::send_request(&Request {
        id: "cli:agent:message".into(),
        method: Method::AgentMessage(params),
    })?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn text_after_the_target_is_joined_and_typed_by_default() {
        let params = parse_agent_message_args(&args(&["reviewer", "look", "at", "this"])).unwrap();
        assert_eq!(params.target, "reviewer");
        assert_eq!(params.text, "look at this");
        assert_eq!(params.mode, AgentMessageMode::Typed);
        assert_eq!(params.busy, AgentMessageBusy::Queue);
        assert_eq!(params.answer, None);
    }

    #[test]
    fn options_work_anywhere_and_select_the_mode_and_busy_policy() {
        for form in [
            args(&["--note", "--interrupt", "reviewer", "hi"]),
            args(&["reviewer", "hi", "--note", "--interrupt"]),
            args(&["reviewer", "--interrupt", "hi", "--note"]),
        ] {
            let params = parse_agent_message_args(&form).unwrap();
            assert_eq!(params.text, "hi", "{form:?}");
            assert_eq!(params.mode, AgentMessageMode::Note, "{form:?}");
            assert_eq!(params.busy, AgentMessageBusy::Interrupt, "{form:?}");
        }
    }

    #[test]
    fn double_dash_keeps_option_words_as_text() {
        let params =
            parse_agent_message_args(&args(&["reviewer", "--", "--note", "is", "a", "flag"]))
                .unwrap();
        assert_eq!(params.text, "--note is a flag");
        assert_eq!(params.mode, AgentMessageMode::Typed);
    }

    #[test]
    fn an_answer_is_a_key_for_a_blocked_agent() {
        let params = parse_agent_message_args(&args(&["reviewer", "--answer", "1"])).unwrap();
        assert_eq!(params.answer.as_deref(), Some("1"));
        assert_eq!(params.text, "");
        let params = parse_agent_message_args(&args(&["reviewer", "--answer=2"])).unwrap();
        assert_eq!(params.answer.as_deref(), Some("2"));
    }

    #[test]
    fn bad_forms_say_what_is_wrong() {
        for (form, needle) in [
            (args(&[]), "usage"),
            (args(&["reviewer"]), "usage"),
            (args(&["--note", "reviewer"]), "usage"),
            (
                args(&["reviewer", "--answer"]),
                "missing value for --answer",
            ),
            (args(&["reviewer", "hi", "--answer", "1"]), "takes a key"),
            (
                args(&["reviewer", "--note", "--answer", "1"]),
                "cannot be a note",
            ),
        ] {
            let error = parse_agent_message_args(&form).unwrap_err();
            assert!(error.contains(needle), "{form:?}: {error}");
        }
    }
}
