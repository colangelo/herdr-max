use serde::{Deserialize, Serialize};

pub(crate) const DEFAULT_TAB_BAR_COMMAND_INTERVAL_SECONDS: u64 = 5;
pub(crate) const DEFAULT_TAB_BAR_COMMAND_TIMEOUT_SECONDS: u64 = 2;
pub(crate) const MAX_TAB_BAR_COMMAND_INTERVAL_SECONDS: u64 = 31_536_000;
pub(crate) const MAX_TAB_BAR_COMMAND_TIMEOUT_SECONDS: u64 = 3_600;
pub(crate) const MAX_TAB_BAR_RIGHT_ENTRIES: usize = 16;

fn default_datetime_format() -> String {
    "%H:%M".to_string()
}

fn default_command_interval_seconds() -> u64 {
    DEFAULT_TAB_BAR_COMMAND_INTERVAL_SECONDS
}

fn default_command_timeout_seconds() -> u64 {
    DEFAULT_TAB_BAR_COMMAND_TIMEOUT_SECONDS
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TabBarRightEntryConfig {
    Zoom,
    Hostname,
    Datetime {
        #[serde(default = "default_datetime_format")]
        format: String,
    },
    Text {
        text: String,
    },
    Command {
        command: String,
        #[serde(default = "default_command_interval_seconds")]
        interval_seconds: u64,
        #[serde(default = "default_command_timeout_seconds")]
        timeout_seconds: u64,
    },
    /// An entry that could not be read (a wrong value type, a missing or
    /// unknown `type`, a key of another entry type). It shows nothing and
    /// `tab_bar_right_diagnostics` names it by position, so one bad entry
    /// does not cost the file (fork issue 163).
    #[serde(skip_serializing)]
    Invalid {
        reason: String,
    },
}

/// Written by hand rather than as an internally tagged enum with
/// `deny_unknown_fields` (fork issue 162): that buffers the table, so a key
/// from a newer build fails the entry and the error loses its name. An unknown
/// key is skipped through `IgnoredAny`, which the config loader reports by
/// path; a key that belongs to another entry type, or a wrong value type, is
/// still an error.
impl<'de> Deserialize<'de> for TabBarRightEntryConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct EntryVisitor;

        impl<'de> serde::de::Visitor<'de> for EntryVisitor {
            type Value = TabBarRightEntryConfig;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a tab_bar_right entry table with a `type`")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut kind: Option<String> = None;
                let mut format: Option<String> = None;
                let mut text: Option<String> = None;
                let mut command: Option<String> = None;
                let mut interval_seconds: Option<u64> = None;
                let mut timeout_seconds: Option<u64> = None;
                // What is wrong with this entry, if anything. A wrong-typed
                // value is read through `toml::Value` so the entry survives
                // as `Invalid` instead of failing the whole file.
                let mut problems: Vec<String> = Vec::new();
                fn lenient<'de, A, T>(
                    map: &mut A,
                    key: &str,
                    expected: &str,
                    problems: &mut Vec<String>,
                ) -> Result<Option<T>, A::Error>
                where
                    A: serde::de::MapAccess<'de>,
                    T: serde::de::DeserializeOwned,
                {
                    let value = map.next_value::<toml::Value>()?;
                    match T::deserialize(value) {
                        Ok(value) => Ok(Some(value)),
                        Err(_) => {
                            problems.push(format!(
                                "has a wrong value type for `{key}` (expected {expected})"
                            ));
                            Ok(None)
                        }
                    }
                }
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "type" => kind = lenient(&mut map, "type", "a string", &mut problems)?,
                        "format" => {
                            format = lenient(&mut map, "format", "a string", &mut problems)?
                        }
                        "text" => text = lenient(&mut map, "text", "a string", &mut problems)?,
                        "command" => {
                            command = lenient(&mut map, "command", "a string", &mut problems)?
                        }
                        "interval_seconds" => {
                            interval_seconds = lenient(
                                &mut map,
                                "interval_seconds",
                                "a whole number",
                                &mut problems,
                            )?
                        }
                        "timeout_seconds" => {
                            timeout_seconds = lenient(
                                &mut map,
                                "timeout_seconds",
                                "a whole number",
                                &mut problems,
                            )?
                        }
                        _ => {
                            map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }
                let invalid = |reason: String| Ok(TabBarRightEntryConfig::Invalid { reason });
                if !problems.is_empty() {
                    return invalid(problems.join(", "));
                }
                let Some(kind) = kind else {
                    return invalid("is missing `type`".to_string());
                };
                let stray = |allowed: &[&str]| -> Option<String> {
                    let given = [
                        ("format", format.is_some()),
                        ("text", text.is_some()),
                        ("command", command.is_some()),
                        ("interval_seconds", interval_seconds.is_some()),
                        ("timeout_seconds", timeout_seconds.is_some()),
                    ];
                    given
                        .iter()
                        .find(|(name, present)| *present && !allowed.contains(name))
                        .map(|(name, _)| {
                            format!("has `{name}`, which does not belong to type `{kind}`")
                        })
                };
                let allowed: &[&str] = match kind.as_str() {
                    "zoom" | "hostname" => &[],
                    "datetime" => &["format"],
                    "text" => &["text"],
                    "command" => &["command", "interval_seconds", "timeout_seconds"],
                    other => {
                        return invalid(format!(
                            "has an unknown type `{other}` (expected zoom, hostname, datetime, text or command)"
                        ))
                    }
                };
                if let Some(reason) = stray(allowed) {
                    return invalid(reason);
                }
                match kind.as_str() {
                    "zoom" => Ok(TabBarRightEntryConfig::Zoom),
                    "hostname" => Ok(TabBarRightEntryConfig::Hostname),
                    "datetime" => Ok(TabBarRightEntryConfig::Datetime {
                        format: format.unwrap_or_else(default_datetime_format),
                    }),
                    "text" => match text {
                        Some(text) => Ok(TabBarRightEntryConfig::Text { text }),
                        None => invalid("is missing `text`".to_string()),
                    },
                    _ => match command {
                        Some(command) => Ok(TabBarRightEntryConfig::Command {
                            command,
                            interval_seconds: interval_seconds
                                .unwrap_or_else(default_command_interval_seconds),
                            timeout_seconds: timeout_seconds
                                .unwrap_or_else(default_command_timeout_seconds),
                        }),
                        None => invalid("is missing `command`".to_string()),
                    },
                }
            }
        }

        deserializer.deserialize_map(EntryVisitor)
    }
}

pub(crate) fn parse_tab_bar_datetime_format(
    value: &str,
) -> Result<time::format_description::OwnedFormatItem, String> {
    if value.is_empty() {
        return Err("datetime format is empty".into());
    }
    let format = time::format_description::parse_strftime_owned(value)
        .map_err(|err| format!("invalid datetime format: {err}"))?;
    time::PrimitiveDateTime::MIN
        .format(&format)
        .map_err(|err| format!("unsupported datetime format: {err}"))?;
    Ok(format)
}

pub(crate) fn tab_bar_right_diagnostics(entries: &[TabBarRightEntryConfig]) -> Vec<String> {
    let mut diagnostics = Vec::new();
    if entries.len() > MAX_TAB_BAR_RIGHT_ENTRIES {
        diagnostics.push(format!(
            "ui.tab_bar_right may contain at most {MAX_TAB_BAR_RIGHT_ENTRIES} entries; ignoring extras"
        ));
    }

    for (index, entry) in entries.iter().enumerate().take(MAX_TAB_BAR_RIGHT_ENTRIES) {
        match entry {
            TabBarRightEntryConfig::Datetime { format } => {
                if format.is_empty() {
                    diagnostics.push(format!(
                        "ui.tab_bar_right[{index}] datetime format is empty; hiding entry"
                    ));
                } else if let Err(err) = parse_tab_bar_datetime_format(format) {
                    diagnostics.push(format!("ui.tab_bar_right[{index}] has {err}; hiding entry"));
                }
            }
            TabBarRightEntryConfig::Command {
                command,
                interval_seconds,
                timeout_seconds,
            } => {
                if command.trim().is_empty() {
                    diagnostics.push(format!(
                        "ui.tab_bar_right[{index}] command is empty; hiding entry"
                    ));
                }
                if *interval_seconds == 0 {
                    diagnostics.push(format!(
                        "ui.tab_bar_right[{index}] interval_seconds must be at least 1; hiding entry"
                    ));
                }
                if *interval_seconds > MAX_TAB_BAR_COMMAND_INTERVAL_SECONDS {
                    diagnostics.push(format!(
                        "ui.tab_bar_right[{index}] interval_seconds may be at most {MAX_TAB_BAR_COMMAND_INTERVAL_SECONDS}; hiding entry"
                    ));
                }
                if *timeout_seconds == 0 {
                    diagnostics.push(format!(
                        "ui.tab_bar_right[{index}] timeout_seconds must be at least 1; hiding entry"
                    ));
                }
                if *timeout_seconds > MAX_TAB_BAR_COMMAND_TIMEOUT_SECONDS {
                    diagnostics.push(format!(
                        "ui.tab_bar_right[{index}] timeout_seconds may be at most {MAX_TAB_BAR_COMMAND_TIMEOUT_SECONDS}; hiding entry"
                    ));
                }
            }
            TabBarRightEntryConfig::Invalid { reason } => {
                diagnostics.push(format!("ui.tab_bar_right[{index}] {reason}; hiding entry"));
            }
            TabBarRightEntryConfig::Zoom
            | TabBarRightEntryConfig::Hostname
            | TabBarRightEntryConfig::Text { .. } => {}
        }
    }

    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_bar_entries_parse_with_command_defaults() {
        #[derive(Deserialize)]
        struct Wrapper {
            entries: Vec<TabBarRightEntryConfig>,
        }

        let parsed: Wrapper = toml::from_str(
            r#"
entries = [
  { type = "zoom" },
  { type = "hostname" },
  { type = "datetime", format = "%H:%M" },
  { type = "text", text = "prod" },
  { type = "command", command = "status.sh" },
]
"#,
        )
        .expect("parse tab bar entries");

        assert_eq!(parsed.entries.len(), 5);
        assert!(matches!(
            &parsed.entries[4],
            TabBarRightEntryConfig::Command {
                interval_seconds: DEFAULT_TAB_BAR_COMMAND_INTERVAL_SECONDS,
                timeout_seconds: DEFAULT_TAB_BAR_COMMAND_TIMEOUT_SECONDS,
                ..
            }
        ));
    }

    #[test]
    fn diagnostics_reject_invalid_datetime_and_command_schedules() {
        let entries = vec![
            TabBarRightEntryConfig::Datetime {
                format: "%Q".into(),
            },
            TabBarRightEntryConfig::Datetime {
                format: "%z".into(),
            },
            TabBarRightEntryConfig::Command {
                command: String::new(),
                interval_seconds: 0,
                timeout_seconds: 0,
            },
            TabBarRightEntryConfig::Command {
                command: "status.sh".into(),
                interval_seconds: MAX_TAB_BAR_COMMAND_INTERVAL_SECONDS + 1,
                timeout_seconds: MAX_TAB_BAR_COMMAND_TIMEOUT_SECONDS + 1,
            },
        ];

        let diagnostics = tab_bar_right_diagnostics(&entries).join("\n");
        assert!(diagnostics.contains("invalid datetime format"));
        assert!(diagnostics.contains("unsupported datetime format"));
        assert!(diagnostics.contains("command is empty"));
        assert!(diagnostics.contains("interval_seconds must be at least 1"));
        assert!(diagnostics.contains("interval_seconds may be at most"));
        assert!(diagnostics.contains("timeout_seconds must be at least 1"));
        assert!(diagnostics.contains("timeout_seconds may be at most"));
        assert!(parse_tab_bar_datetime_format("").is_err());
    }
}
