//! Leaves faint (SGR 2) cells out of a terminal read (fork issue 146).
//!
//! Claude Code draws its grey reply suggestion and the `/compact` argument hint
//! faint, so in a plain-text read they look like typed text. `--strip-dim`
//! reads the ANSI snapshot and drops the faint runs here.

/// `text` without its faint runs. With `keep_sequences` the other escape
/// sequences stay (an ANSI read); without it every escape sequence goes (a
/// text read). Line breaks always stay, so a faint run that spans lines keeps
/// the shape of the screen.
///
/// A run starts at SGR 2 and ends at 22 or 0 (an empty `ESC[m` is 0). The
/// extended colours 38, 48 and 58 carry arguments that are not attributes
/// (`5;n` or `2;r;g;b`), so a `2` inside them does not start a run.
pub(crate) fn drop_dim_runs(text: &str, keep_sequences: bool) -> String {
    drop_runs(text, keep_sequences, None)
}

/// [`drop_dim_runs`] that also drops text drawn in the true-colour foreground
/// `hint_fg`: a caller that knows a program draws a hint in a fixed grey
/// instead of SGR 2 (fork issue 146).
pub(crate) fn drop_runs(text: &str, keep_sequences: bool, hint_fg: Option<[u8; 3]>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut hidden = Hidden::default();
    let mut rest = text;
    while let Some(start) = rest.find('\x1b') {
        emit(&mut out, &rest[..start], hidden.on());
        rest = &rest[start..];
        let Some((sequence, after)) = split_csi(rest) else {
            // A lone escape: no sequence to read, keep it as plain text.
            emit(&mut out, "\x1b", hidden.on());
            rest = &rest['\x1b'.len_utf8()..];
            continue;
        };
        if sequence.ends_with('m') && sequence.starts_with("\x1b[") {
            apply_sgr(&sequence[2..sequence.len() - 1], &mut hidden, hint_fg);
        }
        if keep_sequences {
            out.push_str(sequence);
        }
        rest = after;
    }
    emit(&mut out, rest, hidden.on());
    out
}

/// Text outside a faint run, and the line breaks inside one.
fn emit(out: &mut String, text: &str, dim: bool) {
    if !dim {
        out.push_str(text);
        return;
    }
    for ch in text.chars().filter(|ch| matches!(ch, '\n' | '\r')) {
        out.push(ch);
    }
}

/// The CSI sequence at the start of `text` and what follows it.
fn split_csi(text: &str) -> Option<(&str, &str)> {
    let body = text.strip_prefix("\x1b[")?;
    let end = body.find(|ch: char| ('\x40'..='\x7e').contains(&ch))?;
    let len = 2 + end + body[end..].chars().next()?.len_utf8();
    Some(text.split_at(len))
}

/// Whether text is currently left out: faint, or in the hint colour.
#[derive(Default)]
struct Hidden {
    dim: bool,
    hint: bool,
}

impl Hidden {
    fn on(&self) -> bool {
        self.dim || self.hint
    }
}

fn apply_sgr(params: &str, hidden: &mut Hidden, hint_fg: Option<[u8; 3]>) {
    let codes: Vec<&str> = if params.is_empty() {
        vec!["0"]
    } else {
        params.split(';').collect()
    };
    let mut index = 0;
    while index < codes.len() {
        match codes[index] {
            "38" | "48" | "58" => {
                let true_colour = codes.get(index + 1) == Some(&"2");
                let width = if codes.get(index + 1) == Some(&"5") {
                    3
                } else {
                    5
                };
                if codes[index] == "38" {
                    // A new foreground ends a hint run; the hint colour starts one.
                    hidden.hint = true_colour
                        && hint_fg.is_some_and(|want| {
                            let got: Vec<u8> = codes
                                .get(index + 2..index + 5)
                                .unwrap_or_default()
                                .iter()
                                .filter_map(|channel| channel.parse().ok())
                                .collect();
                            got == want
                        });
                }
                index += width;
                continue;
            }
            "2" => hidden.dim = true,
            "0" | "" => *hidden = Hidden::default(),
            "22" => hidden.dim = false,
            "39" => hidden.hint = false,
            _ => {}
        }
        index += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::{drop_dim_runs, drop_runs};

    #[test]
    fn a_hint_colour_run_is_left_out_when_asked() {
        let hint = [153, 153, 153];
        let line =
            "\x1b[38;2;177;185;249m/compact\x1b[0m  \x1b[38;2;153;153;153m<optional hint>\x1b[0m";
        assert_eq!(drop_runs(line, false, Some(hint)), "/compact  ");
        assert_eq!(
            drop_dim_runs(line, false),
            "/compact  <optional hint>",
            "off by default"
        );
        // Another colour, a later foreground and 39 end it.
        assert_eq!(
            drop_runs(
                "\x1b[38;2;153;153;153mx\x1b[38;2;1;2;3my",
                false,
                Some(hint)
            ),
            "y"
        );
        assert_eq!(
            drop_runs("\x1b[38;2;153;153;153mx\x1b[39my", false, Some(hint)),
            "y"
        );
        assert_eq!(
            drop_runs("\x1b[38;2;10;20;30mkept", false, Some(hint)),
            "kept"
        );
    }

    #[test]
    fn a_faint_suggestion_is_left_out_and_typed_text_stays() {
        let line = "\x1b[1m❯\x1b[0m \x1b[2mA/B as is\x1b[0m";
        assert_eq!(drop_dim_runs(line, false), "❯ ");
        assert_eq!(drop_dim_runs(line, true), "\x1b[1m❯\x1b[0m \x1b[2m\x1b[0m");
        assert_eq!(drop_dim_runs("❯ typed text", false), "❯ typed text");
    }

    #[test]
    fn reset_22_and_reset_0_both_end_a_run() {
        assert_eq!(drop_dim_runs("a\x1b[2mb\x1b[22mc", false), "ac");
        assert_eq!(drop_dim_runs("a\x1b[2mb\x1b[0mc", false), "ac");
        assert_eq!(
            drop_dim_runs("a\x1b[2mb\x1b[mc", false),
            "ac",
            "an empty SGR is 0"
        );
        assert_eq!(
            drop_dim_runs("a\x1b[2;22mb", false),
            "ab",
            "22 after 2 in one sequence"
        );
    }

    #[test]
    fn extended_colour_arguments_do_not_start_a_run() {
        // 38;5;2 is colour 2, 38;2;r;g;b carries a 2 and a 2-valued channel.
        assert_eq!(drop_dim_runs("\x1b[38;5;2mgreen\x1b[0m", false), "green");
        assert_eq!(drop_dim_runs("\x1b[38;2;2;2;2mrgb\x1b[0m", false), "rgb");
        assert_eq!(drop_dim_runs("\x1b[48;5;2mbg\x1b[0m", false), "bg");
        assert_eq!(
            drop_dim_runs("\x1b[58;2;1;2;3munderline\x1b[0m", false),
            "underline"
        );
        // A real 2 after an extended colour in the same sequence still starts one.
        assert_eq!(drop_dim_runs("a\x1b[38;5;1;2mb\x1b[0mc", false), "ac");
    }

    #[test]
    fn nested_and_repeated_runs_and_other_attributes() {
        assert_eq!(drop_dim_runs("a\x1b[2mb\x1b[2mc\x1b[22md", false), "ad");
        assert_eq!(
            drop_dim_runs("a\x1b[2m\x1b[1mb\x1b[0mc", false),
            "ac",
            "bold does not end it"
        );
        assert_eq!(drop_dim_runs("\x1b[1;4mx\x1b[0m", false), "x");
    }

    #[test]
    fn a_run_across_lines_keeps_the_line_breaks() {
        assert_eq!(drop_dim_runs("a\x1b[2mb\nc\x1b[0md\n", false), "a\nd\n");
    }

    #[test]
    fn other_sequences_and_a_lone_escape_are_handled() {
        assert_eq!(drop_dim_runs("a\x1b[2Kb\x1b[?25lc", false), "abc");
        assert_eq!(drop_dim_runs("a\x1b[2Kb", true), "a\x1b[2Kb");
        assert_eq!(drop_dim_runs("a\x1bb", false), "a\x1bb");
        assert_eq!(drop_dim_runs("trailing\x1b[", false), "trailing\x1b[");
    }
}
