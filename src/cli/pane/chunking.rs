use crate::api::schema::{Method, PaneSendTextParams};

const PANE_SEND_TEXT_USAGE: &str =
    "usage: herdr pane send-text <pane_id> <text> [--chunk BYTES [--chunk-delay MS]]";

/// Pause between `--chunk` pieces when `--chunk-delay` is not given. Without a
/// pause, back-to-back writes can reach the program in the pane as one read,
/// which a TUI still treats as one paste-sized burst.
const DEFAULT_CHUNK_DELAY_MS: u64 = 20;

#[derive(Debug, PartialEq, Eq)]
struct PaneSendTextArgs {
    pane_id: String,
    text: String,
    chunk: Option<ChunkPacing>,
}

#[derive(Debug, PartialEq, Eq)]
struct ChunkPacing {
    max_bytes: usize,
    delay: std::time::Duration,
}

pub(super) fn pane_send_text(args: &[String]) -> std::io::Result<i32> {
    let parsed = match parse_pane_send_text_args(args) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("{message}");
            return Ok(2);
        }
    };

    let Some(pacing) = parsed.chunk else {
        return crate::cli::send_ok_request(Method::PaneSendText(PaneSendTextParams {
            pane_id: parsed.pane_id,
            text: parsed.text,
        }));
    };

    // One request per piece, paced client-side: each piece reaches the pane as
    // its own write, and the command returns only after the last one is sent.
    let pieces = crate::ui::text::split_utf8_chunks(&parsed.text, pacing.max_bytes);
    let total = pieces.len();
    for (index, piece) in pieces.into_iter().enumerate() {
        if index > 0 {
            std::thread::sleep(pacing.delay);
        }
        let code = crate::cli::send_ok_request(Method::PaneSendText(PaneSendTextParams {
            pane_id: parsed.pane_id.clone(),
            text: piece.to_string(),
        }))?;
        if code != 0 {
            eprintln!("sent {index} of {total} pieces before the failure");
            return Ok(code);
        }
    }
    Ok(0)
}

/// Parse `pane send-text`. `--chunk` and `--chunk-delay` are recognised
/// anywhere, before or after the pane id; `--` ends option parsing so text
/// that starts with those words can still be sent. The first other word is the
/// pane id and the rest is text, joined with single spaces exactly as before.
fn parse_pane_send_text_args(args: &[String]) -> Result<PaneSendTextArgs, String> {
    let mut words: Vec<&str> = Vec::new();
    let mut chunk = None;
    let mut delay_ms = None;
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
            "--chunk" | "--chunk-delay" => {
                let value = match attached {
                    Some(value) => value,
                    None => {
                        index += 1;
                        args.get(index)
                            .map(String::as_str)
                            .ok_or_else(|| format!("missing value for {flag}"))?
                    }
                };
                let parsed: u64 = value
                    .parse()
                    .map_err(|_| format!("invalid value for {flag}: {value}"))?;
                if flag == "--chunk" {
                    if parsed == 0 {
                        return Err("--chunk must be at least 1".into());
                    }
                    chunk = Some(
                        usize::try_from(parsed)
                            .map_err(|_| format!("invalid value for --chunk: {value}"))?,
                    );
                } else {
                    delay_ms = Some(parsed);
                }
            }
            _ => words.push(arg),
        }
        index += 1;
    }

    let Some((raw_pane_id, text_words)) = words.split_first() else {
        return Err(PANE_SEND_TEXT_USAGE.into());
    };
    if text_words.is_empty() {
        return Err(PANE_SEND_TEXT_USAGE.into());
    }
    if chunk.is_none() && delay_ms.is_some() {
        return Err("--chunk-delay requires --chunk".into());
    }

    Ok(PaneSendTextArgs {
        pane_id: crate::cli::normalize_pane_id(raw_pane_id),
        text: text_words.join(" "),
        chunk: chunk.map(|max_bytes| ChunkPacing {
            max_bytes,
            delay: std::time::Duration::from_millis(delay_ms.unwrap_or(DEFAULT_CHUNK_DELAY_MS)),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }
    #[test]
    fn parse_pane_send_text_args_without_chunk_joins_words_as_before() {
        let parsed = parse_pane_send_text_args(&args(&["p1", "hello", "world"])).unwrap();
        assert_eq!(parsed.pane_id, crate::cli::normalize_pane_id("p1"));
        assert_eq!(parsed.text, "hello world");
        assert_eq!(parsed.chunk, None);
    }

    #[test]
    fn parse_pane_send_text_args_accepts_chunk_options_anywhere_after_pane() {
        for form in [
            args(&["p1", "--chunk", "300", "--chunk-delay", "5", "hi", "there"]),
            args(&["p1", "hi", "there", "--chunk=300", "--chunk-delay=5"]),
            args(&["p1", "hi", "--chunk", "300", "there", "--chunk-delay", "5"]),
        ] {
            let parsed = parse_pane_send_text_args(&form).unwrap();
            assert_eq!(parsed.text, "hi there", "{form:?}");
            assert_eq!(
                parsed.chunk,
                Some(ChunkPacing {
                    max_bytes: 300,
                    delay: std::time::Duration::from_millis(5),
                }),
                "{form:?}"
            );
        }
    }

    #[test]
    fn parse_pane_send_text_args_accepts_chunk_options_before_the_pane() {
        for form in [
            args(&["--chunk", "300", "p1", "hi", "there"]),
            args(&["--chunk=300", "p1", "hi", "there"]),
            args(&["--chunk-delay", "5", "--chunk", "300", "p1", "hi", "there"]),
        ] {
            let parsed = parse_pane_send_text_args(&form).unwrap();
            assert_eq!(parsed.pane_id, "p1", "{form:?}");
            assert_eq!(parsed.text, "hi there", "{form:?}");
            assert_eq!(
                parsed.chunk.map(|pacing| pacing.max_bytes),
                Some(300),
                "{form:?}"
            );
        }
    }

    #[test]
    fn parse_pane_send_text_args_double_dash_before_the_pane_ends_options() {
        let parsed =
            parse_pane_send_text_args(&args(&["--chunk", "4", "--", "p1", "--chunk"])).unwrap();
        assert_eq!(parsed.pane_id, "p1");
        assert_eq!(parsed.text, "--chunk");
    }

    #[test]
    fn parse_pane_send_text_args_defaults_chunk_delay() {
        let parsed = parse_pane_send_text_args(&args(&["p1", "x", "--chunk", "8"])).unwrap();
        assert_eq!(
            parsed.chunk.map(|pacing| pacing.delay),
            Some(std::time::Duration::from_millis(DEFAULT_CHUNK_DELAY_MS))
        );
    }

    #[test]
    fn parse_pane_send_text_args_double_dash_keeps_option_words_as_text() {
        let parsed =
            parse_pane_send_text_args(&args(&["p1", "--chunk", "4", "--", "--chunk", "9"]))
                .unwrap();
        assert_eq!(parsed.text, "--chunk 9");
        assert_eq!(parsed.chunk.map(|pacing| pacing.max_bytes), Some(4));
    }

    #[test]
    fn parse_pane_send_text_args_rejects_bad_chunk_options() {
        for (form, needle) in [
            (args(&["p1"]), "usage"),
            (args(&["p1", "--chunk", "300"]), "usage"),
            (args(&["--chunk", "300", "p1"]), "usage"),
            (args(&["p1", "x", "--chunk"]), "missing value for --chunk"),
            (args(&["p1", "x", "--chunk", "0"]), "at least 1"),
            (
                args(&["p1", "x", "--chunk", "-3"]),
                "invalid value for --chunk",
            ),
            (args(&["p1", "x", "--chunk-delay", "5"]), "requires --chunk"),
            (
                args(&["p1", "x", "--chunk", "4", "--chunk-delay", "soon"]),
                "invalid value for --chunk-delay",
            ),
        ] {
            let error = parse_pane_send_text_args(&form).unwrap_err();
            assert!(error.contains(needle), "{form:?}: {error}");
        }
    }
}
