//! The OSC 7501 root record herdr mirrors into herdr-core for one pane
//! (openspec/changes/core-status-mirror, D2). Pure data: building and encoding
//! it needs no terminal.

/// The status words herdr mirrors; core keeps them as the program's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CoreStatusWord {
    Idle,
    Working,
    Blocked,
    Done,
}

/// An agent label that is safe inside the sequence: 1 to 64 bytes of
/// `[A-Za-z0-9._+-]`, so it can carry no field separator, terminator or
/// control byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CoreStatusApp(String);

impl CoreStatusApp {
    const MAX_LEN: usize = 64;

    /// `None` for any other label: it is left out, never rewritten.
    pub(crate) fn parse(label: &str) -> Option<Self> {
        let safe = (1..=Self::MAX_LEN).contains(&label.len())
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte));
        safe.then(|| Self(label.to_string()))
    }
}

/// One root record (no `id`): a report, or `clear` to remove it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CoreStatusRecord {
    Clear,
    Report {
        word: CoreStatusWord,
        app: Option<CoreStatusApp>,
    },
}

// Only the Unix core backend writes records; other platforms build and drop them.
#[cfg_attr(not(unix), allow(dead_code))]
impl CoreStatusRecord {
    /// The record as core's parser reads it (herdr-core `status.rs` `encode`):
    /// `ESC ]7501;state=<word>[:kind=permission][:app=<app>] ESC \`.
    pub(crate) fn encode(&self) -> Vec<u8> {
        let (state, kind, app) = match self {
            Self::Clear => ("clear", None, None),
            Self::Report { word, app } => (
                match word {
                    CoreStatusWord::Idle => "idle",
                    CoreStatusWord::Working => "working",
                    CoreStatusWord::Blocked => "blocked",
                    CoreStatusWord::Done => "done",
                },
                (*word == CoreStatusWord::Blocked).then_some("permission"),
                app.as_ref(),
            ),
        };
        let mut out = Vec::with_capacity(48);
        out.extend_from_slice(b"\x1b]7501;state=");
        out.extend_from_slice(state.as_bytes());
        if let Some(kind) = kind {
            out.extend_from_slice(b":kind=");
            out.extend_from_slice(kind.as_bytes());
        }
        if let Some(CoreStatusApp(app)) = app {
            out.extend_from_slice(b":app=");
            out.extend_from_slice(app.as_bytes());
        }
        out.extend_from_slice(b"\x1b\\");
        out
    }

    pub(crate) fn is_clear(&self) -> bool {
        matches!(self, Self::Clear)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(word: CoreStatusWord, app: &str) -> CoreStatusRecord {
        CoreStatusRecord::Report {
            word,
            app: CoreStatusApp::parse(app),
        }
    }

    #[test]
    fn records_encode_as_cores_root_records() {
        let cases = [
            (
                report(CoreStatusWord::Working, "claude"),
                "\x1b]7501;state=working:app=claude\x1b\\",
            ),
            (
                report(CoreStatusWord::Blocked, "codex"),
                "\x1b]7501;state=blocked:kind=permission:app=codex\x1b\\",
            ),
            (
                report(CoreStatusWord::Done, "pi"),
                "\x1b]7501;state=done:app=pi\x1b\\",
            ),
            (
                report(CoreStatusWord::Idle, "qwen-code"),
                "\x1b]7501;state=idle:app=qwen-code\x1b\\",
            ),
            (CoreStatusRecord::Clear, "\x1b]7501;state=clear\x1b\\"),
        ];
        for (record, expected) in cases {
            assert_eq!(String::from_utf8(record.encode()).unwrap(), expected);
        }
        let longest = CoreStatusRecord::Report {
            word: CoreStatusWord::Blocked,
            app: CoreStatusApp::parse(&"a".repeat(64)),
        };
        assert!(longest.encode().len() < 256);
    }

    #[test]
    fn unsafe_labels_are_left_out_not_rewritten() {
        for label in [
            "",
            "a:b",
            "a;b",
            "a\x1bb",
            "a\x07b",
            "a\nb",
            "a b",
            "a\\b",
            "clé",
            &"a".repeat(65),
        ] {
            assert_eq!(CoreStatusApp::parse(label), None, "{label:?}");
            assert_eq!(
                String::from_utf8(report(CoreStatusWord::Working, label).encode()).unwrap(),
                "\x1b]7501;state=working\x1b\\",
                "{label:?}"
            );
        }
        assert!(CoreStatusApp::parse("claude-code_2.1+x").is_some());
    }
}
