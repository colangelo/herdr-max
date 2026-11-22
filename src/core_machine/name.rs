//! Names a core machine link deals in, parsed once at the boundary.

use std::fmt;

/// A core machine's name, the first part of its terminals' addresses
/// (`ac-mbm5/t7`) and the name of its state folder. 1-64 bytes of ASCII
/// letters, digits, `.`, `_` and `-`, starting with a letter or digit: a host
/// name fits, and nothing that could be a path part, an ssh option or a shell
/// word does.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct MachineName(String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NameError(pub String);

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NameError {}

pub(crate) const MAX_NAME_BYTES: usize = 64;

/// The rule of [`MachineName`], also used for a far core session's name.
pub(crate) fn is_plain_name(raw: &str) -> bool {
    let mut bytes = raw.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    raw.len() <= MAX_NAME_BYTES
        && first.is_ascii_alphanumeric()
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

impl MachineName {
    pub(crate) fn parse(raw: &str) -> Result<Self, NameError> {
        if is_plain_name(raw) {
            Ok(Self(raw.to_string()))
        } else {
            Err(NameError(format!(
                "machine name {raw:?} must be 1-{MAX_NAME_BYTES} letters, digits, '.', '_' or '-', starting with a letter or digit"
            )))
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MachineName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_like_names_parse() {
        for raw in ["ac-mbm5", "m4m", "10.0.0.7", "vm_1", "a", &"x".repeat(64)] {
            assert_eq!(MachineName::parse(raw).expect(raw).as_str(), raw);
        }
    }

    #[test]
    fn names_that_could_be_paths_options_or_words_are_refused() {
        for raw in [
            "",
            "-oProxyCommand=x",
            ".hidden",
            "..",
            "a/b",
            "a b",
            "a'b",
            "a;b",
            "$(x)",
            "é",
            &"x".repeat(65),
        ] {
            assert!(MachineName::parse(raw).is_err(), "{raw:?} was accepted");
        }
    }
}
